import type { AgentInfo, SessionConfig } from "./types";

// Edit these to change how the agents behave. Tools are described by the MCP bridge itself.

const name = (k: string, cfg?: SessionConfig) => (k === "claude" ? "Claude" : k === "codex" ? "Codex" : cfg?.labels?.[k] ?? k);
const team = (cfg: SessionConfig) => cfg.team.map((k) => (k === "claude" || k === "codex" ? name(k) : `${name(k, cfg)} (agent kind "${k}")`)).join(", ");

/** The jobs you gave each AI, or the old default when you gave none. */
function who(cfg: SessionConfig) {
  const set = cfg.team.filter((k) => cfg.roles?.[k]);
  if (!set.length) return "Give build/implementation work to Codex and reviewing to Claude unless there is a reason not to.";
  const free = cfg.team.filter((k) => !cfg.roles?.[k]);
  return `The user decided who does what. Follow it strictly: give each part only to an agent whose job covers it, and never give an agent work its job rules out.
${set.map((k) => `   - ${name(k, cfg)} (agent "${k}"): ${cfg.roles![k]}`).join("\n")}${free.length ? `\n   - ${free.map((k) => name(k, cfg)).join(", ")}: your choice.` : ""}
   If no agent's job fits a part, say so in \`summary\` instead of bending the rules.`;
}

function rules(cfg: SessionConfig) {
  const out: string[] = [];
  if (cfg.permission === "acceptEdits")
    out.push("Mode: accept edits. Agents may edit files, but every shell command needs the user's yes. Codex agents (and providers that run on Codex) are read-only in this mode: give them reading and reviewing, and give edits to Claude or providers that run on Claude Code.");
  if (cfg.protect.length)
    out.push(`Protected files (the user must approve any change): ${cfg.protect.join(", ")}. Avoid touching them unless the task needs it, and say so in your report when you do.`);
  return out.length ? "\n\n" + out.join("\n") : "";
}

export function leadPrompt(cfg: SessionConfig, task: string, me: AgentInfo) {
  return `You are ${me.id}, the LEAD (orchestrator) of a small team of AI coding agents working in this repository.

Team you can hand work to: ${team(cfg)}.
Limits: at most ${cfg.maxAgents} agents in total, sub-agents up to depth ${cfg.maxDepth}.

How to work:
1. Look around the repository briefly (read only) to understand the task.
2. Call the duo tool \`propose_split\` once, with 1–4 self-contained parts. ${who(cfg)} Use \`after\` for parts that must wait (e.g. review after build and tests).${cfg.ask.split || cfg.permission === "plan" ? " The user approves the split before anything starts." : ""}${cfg.permission === "plan" ? "\n   PLAN MODE: nothing may change yet. Put a full plan in `summary`: the files to change, how, the risks, and how it will be checked. The work starts only once the user approves it." : ""}
3. Call \`wait\` until every part has finished. Read the reports. If something is wrong, \`assign\` a follow-up or \`message\` the agent.
4. Do not edit files yourself; delegate.
5. Finish with a short summary for the user: what changed, what was checked, anything left open.

The user's task:
${task}${rules(cfg)}`;
}

export function memberPrompt(cfg: SessionConfig, me: AgentInfo, lead: string, team: string[]) {
  const canSpawn = me.depth < cfg.maxDepth;
  const job = cfg.roles?.[me.kind];
  return `You are ${me.id} (role: ${me.role}) in a team of AI coding agents led by ${lead}. ${me.parent ? `You were started by ${me.parent}.` : ""}${job ? `\nThe user's standing instruction for you: ${job}. It overrides your task wherever they conflict.` : ""}
Other agents right now: ${team.filter((t) => t !== me.id).join(", ") || "none"}.

Your task:
${me.task}

Rules:
- Stay inside your task; other agents own the other parts.${canSpawn ? "\n- If your task has independent pieces that can run in parallel, split them off with the duo tool `spawn_sub`, then call `wait` for them before you finish." : "\n- You are at the maximum depth: do the work yourself, do not spawn sub-agents."}
- If you find a problem in someone else's part, tell them with \`message\` instead of fixing it yourself.
- Check your \`inbox\` if you are told you have messages.
- Do not commit or push; the session brings your changes back to the user.
- Finish with a short report: what you changed and the result of any checks you ran.${rules(cfg)}`;
}

export function followUpPrompt(from: string, text: string) {
  return from === "you" ? text : `Message from ${from}:\n${text}`;
}

export function handoverPrompt(cfg: SessionConfig, me: AgentInfo, history: string) {
  return `You are ${me.id}, taking over as LEAD (orchestrator) of this session from the previous lead.
Team: ${team(cfg)}.
${who(cfg)} Use the duo tools (propose_split, assign, wait, message, status) to coordinate; do not edit files yourself.

What happened so far:
${history || "(nothing yet)"}

Call \`status\` to see the agent tree, then continue with what the user asks next.`;
}
