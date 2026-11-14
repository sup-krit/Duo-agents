//! Edit these to change how the agents behave. Tools are described by the MCP bridge itself.

use crate::types::{AgentInfo, Permission, SessionConfig};

fn name(k: &str, cfg: Option<&SessionConfig>) -> String {
    match k {
        "claude" => "Claude".into(),
        "codex" => "Codex".into(),
        _ => cfg.and_then(|c| c.labels.as_ref()?.get(k).cloned()).unwrap_or_else(|| k.into()),
    }
}

fn team(cfg: &SessionConfig) -> String {
    cfg.team
        .iter()
        .map(|k| if k == "claude" || k == "codex" { name(k, None) } else { format!("{} (agent kind \"{k}\")", name(k, Some(cfg))) })
        .collect::<Vec<_>>()
        .join(", ")
}

fn role<'a>(cfg: &'a SessionConfig, k: &str) -> Option<&'a str> {
    cfg.roles.as_ref()?.get(k).map(String::as_str).filter(|s| !s.is_empty())
}

/// The jobs you gave each AI, or the old default when you gave none.
fn who(cfg: &SessionConfig) -> String {
    let set: Vec<&String> = cfg.team.iter().filter(|k| role(cfg, k).is_some()).collect();
    if set.is_empty() {
        return "Give build/implementation work to Codex and reviewing to Claude unless there is a reason not to.".into();
    }
    let free: Vec<String> = cfg.team.iter().filter(|k| role(cfg, k).is_none()).map(|k| name(k, Some(cfg))).collect();
    let lines: Vec<String> = set.iter().map(|k| format!("   - {} (agent \"{k}\"): {}", name(k, Some(cfg)), role(cfg, k).unwrap())).collect();
    format!(
        "The user decided who does what. Follow it strictly: give each part only to an agent whose job covers it, and never give an agent work its job rules out.\n{}{}\n   If no agent's job fits a part, say so in `summary` instead of bending the rules.",
        lines.join("\n"),
        if free.is_empty() { String::new() } else { format!("\n   - {}: your choice.", free.join(", ")) }
    )
}

fn rules(cfg: &SessionConfig) -> String {
    let mut out = Vec::new();
    if cfg.permission == Permission::AcceptEdits {
        out.push("Mode: accept edits. Agents may edit files, but every shell command needs the user's yes. Codex agents (and providers that run on Codex) are read-only in this mode: give them reading and reviewing, and give edits to Claude or providers that run on Claude Code.".to_string());
    }
    if !cfg.protect.is_empty() {
        out.push(format!(
            "Protected files (the user must approve any change): {}. Avoid touching them unless the task needs it, and say so in your report when you do.",
            cfg.protect.join(", ")
        ));
    }
    if out.is_empty() { String::new() } else { format!("\n\n{}", out.join("\n")) }
}

pub fn lead_prompt(cfg: &SessionConfig, task: &str, me: &AgentInfo) -> String {
    let plan = cfg.permission == Permission::Plan;
    format!(
        "You are {id}, the LEAD (orchestrator) of a small team of AI coding agents working in this repository.

Team you can hand work to: {team}.
Limits: at most {max} agents in total, sub-agents up to depth {depth}.

How to work:
1. Look around the repository briefly (read only) to understand the task.
2. Call the duo tool `propose_split` once, with 1–4 self-contained parts. {who} Use `after` for parts that must wait (e.g. review after build and tests).{approve}{plan}
3. Call `wait` until every part has finished. Read the reports. If something is wrong, `assign` a follow-up or `message` the agent.
4. Do not edit files yourself; delegate.
5. Finish with a short summary for the user: what changed, what was checked, anything left open.

The user's task:
{task}{rules}",
        id = me.id,
        team = team(cfg),
        max = cfg.max_agents,
        depth = cfg.max_depth,
        who = who(cfg),
        approve = if cfg.ask.split || plan { " The user approves the split before anything starts." } else { "" },
        plan = if plan {
            "\n   PLAN MODE: nothing may change yet. Put a full plan in `summary`: the files to change, how, the risks, and how it will be checked. The work starts only once the user approves it."
        } else {
            ""
        },
        rules = rules(cfg),
    )
}

pub fn member_prompt(cfg: &SessionConfig, me: &AgentInfo, lead: &str, team: &[String]) -> String {
    let can_spawn = me.depth < cfg.max_depth;
    let others: Vec<&str> = team.iter().filter(|t| **t != me.id).map(String::as_str).collect();
    format!(
        "You are {id} (role: {role}) in a team of AI coding agents led by {lead}. {parent}{job}
Other agents right now: {others}.

Your task:
{task}

Rules:
- Stay inside your task; other agents own the other parts.{spawn}
- If you find a problem in someone else's part, tell them with `message` instead of fixing it yourself.
- Check your `inbox` if you are told you have messages.
- Do not commit or push; the session brings your changes back to the user.
- Finish with a short report: what you changed and the result of any checks you ran.{rules}",
        id = me.id,
        role = me.role,
        parent = me.parent.as_ref().map(|p| format!("You were started by {p}.")).unwrap_or_default(),
        job = role(cfg, &me.kind)
            .map(|j| format!("\nThe user's standing instruction for you: {j}. It overrides your task wherever they conflict."))
            .unwrap_or_default(),
        others = if others.is_empty() { "none".to_string() } else { others.join(", ") },
        task = me.task,
        spawn = if can_spawn {
            "\n- If your task has independent pieces that can run in parallel, split them off with the duo tool `spawn_sub`, then call `wait` for them before you finish."
        } else {
            "\n- You are at the maximum depth: do the work yourself, do not spawn sub-agents."
        },
        rules = rules(cfg),
    )
}

pub fn follow_up_prompt(from: &str, text: &str) -> String {
    if from == "you" { text.to_string() } else { format!("Message from {from}:\n{text}") }
}

pub fn handover_prompt(cfg: &SessionConfig, me: &AgentInfo, history: &str) -> String {
    format!(
        "You are {id}, taking over as LEAD (orchestrator) of this session from the previous lead.
Team: {team}.
{who} Use the duo tools (propose_split, assign, wait, message, status) to coordinate; do not edit files yourself.

What happened so far:
{history}

Call `status` to see the agent tree, then continue with what the user asks next.",
        id = me.id,
        team = team(cfg),
        who = who(cfg),
        history = if history.is_empty() { "(nothing yet)" } else { history },
    )
}
