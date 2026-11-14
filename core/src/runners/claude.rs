//! Claude Code through its CLI: `claude -p --output-format stream-json`, the duo tools through
//! `--mcp-config` (agent-duo bridge), permissions through `--allowedTools` + `--permission-mode dontAsk`,
//! and later turns through `--resume`. Every Edit/Write/Bash/PowerShell (subagents' too) passes the
//! PreToolUse hook (`agent-duo hook`), which runs even when your settings.json would allow the call.

use super::proc::{Spec, find, run_jsonl};
use super::{Access, TurnCtx, TurnResult, preview, preview_str};
use crate::session::Session;
use crate::types::{ApprovalKind, EventBody, Permission, Provider, ToolStatus};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::Arc;

// Skill + Agent: skills / subagents from the target repo's .claude/, ~/.claude/ and shared/ (as a plugin).
// Subagents still go through this allowlist, so a read-only agent's subagents stay read-only.
const READ_TOOLS: [&str; 5] = ["Read", "Glob", "Grep", "Skill", "Agent"];
const EDIT_TOOLS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];
const SHELL_TOOLS: [&str; 2] = ["Bash", "PowerShell"]; // Claude Code on Windows runs commands through PowerShell

/// The permission gate behind `agent-duo hook`. None: no opinion (the allowlist decides).
pub async fn gate(s: &Arc<Session>, agent: &str, tool: &str, input: &Value) -> Option<(bool, String)> {
    let is_edit = EDIT_TOOLS.contains(&tool);
    if !is_edit && !SHELL_TOOLS.contains(&tool) {
        return None;
    }
    if s.access_of(agent) != Some(Access::Write) {
        return Some((false, "You are read-only. Delegate changes, or report what should change.".into()));
    }
    let perm = s.permission();
    if perm == Permission::Plan {
        return Some((false, "Plan mode: nothing may change until the user approves the plan. Describe the change instead.".into()));
    }
    let note = |n: Option<String>| n.map(|n| format!(": {n}")).unwrap_or_default();
    if is_edit {
        let file = input["file_path"].as_str().or(input["notebook_path"].as_str()).unwrap_or("").to_string();
        if file.is_empty() || !s.is_protected(&file) {
            return None;
        }
        let (ok, n) = s.approve(agent, ApprovalKind::File, format!("{agent} wants to change a protected file"), format!("{tool} {file}"), Some(file.clone())).await;
        return Some(if ok { (true, String::new()) } else { (false, format!("The user said no to changing {file}{}. Leave it as it is.", note(n))) });
    }
    if perm == Permission::AcceptEdits {
        let cmd = input["command"].as_str().unwrap_or("").to_string();
        let (ok, n) = s.approve(agent, ApprovalKind::Command, format!("{agent} wants to run a command"), cmd, None).await;
        return Some(if ok { (true, String::new()) } else { (false, format!("The user said no to this command{}.", note(n))) });
    }
    None
}

/// Claude Code pointed at another Anthropic-format API (DeepSeek, Kimi, GLM, OpenRouter, Ollama…).
fn provider_env(p: &Provider) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = vec![
        ("ANTHROPIC_BASE_URL".into(), p.base_url.clone()),
        ("ANTHROPIC_AUTH_TOKEN".into(), if p.api_key.is_empty() { "none".into() } else { p.api_key.clone() }), // local servers ignore it
        ("ANTHROPIC_API_KEY".into(), String::new()), // never send your Anthropic key to someone else's endpoint
        ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".into(), "1".into()),
    ];
    for k in ["ANTHROPIC_MODEL", "ANTHROPIC_DEFAULT_OPUS_MODEL", "ANTHROPIC_DEFAULT_SONNET_MODEL", "ANTHROPIC_DEFAULT_HAIKU_MODEL", "ANTHROPIC_SMALL_FAST_MODEL"] {
        env.push((k.into(), p.model.clone()));
    }
    env
}

fn line(name: &str, input: &Value) -> Option<(String, String)> {
    if name.starts_with("mcp__duo__") {
        return None; // the session shows assign/spawn/msg itself
    }
    let s = |k: &str| input[k].as_str().unwrap_or("").to_string();
    Some(match name {
        "Read" => ("read".into(), s("file_path")),
        "Glob" => ("glob".into(), s("pattern")),
        "Grep" => ("grep".into(), format!("\"{}\"", s("pattern"))),
        "Edit" | "MultiEdit" => ("edit".into(), s("file_path")),
        "Write" => ("write".into(), s("file_path")),
        "Bash" | "PowerShell" => ("run".into(), preview_str(&s("command"), 70)),
        "Skill" => ("skill".into(), s("skill")),
        "Agent" => ("agent".into(), format!("{}: {}", Some(s("subagent_type")).filter(|t| !t.is_empty()).unwrap_or_else(|| "general".into()), preview_str(&s("description"), 50))),
        _ => (name.trim_start_matches("mcp__").to_lowercase(), preview(input, 60)),
    })
}

fn text_of(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(|c| c["text"].as_str().unwrap_or("")).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// Forward slashes: the hook command runs through a shell, and C:/x/agent-duo.exe works in all of them.
fn slashes(p: &str) -> String {
    if cfg!(windows) { p.replace('\\', "/") } else { p.to_string() }
}

/// One Claude Code conversation per agent; later turns resume it.
#[derive(Default)]
pub struct Claude {
    session_id: Option<String>,
    /// total_cost_usd carries over when a session is resumed
    cost_so_far: f64,
}

impl Claude {
    pub async fn turn(&mut self, ctx: &TurnCtx) -> Result<TurnResult> {
        let Some(program) = find("claude") else { bail!("Claude Code was not found. Install it, or set DUO_CLAUDE to claude.exe.") };
        let mut allowed: Vec<String> = READ_TOOLS.iter().map(|s| s.to_string()).collect();
        if ctx.access == Access::Write {
            allowed.extend(EDIT_TOOLS.iter().chain(&SHELL_TOOLS).map(|s| s.to_string()));
        }
        allowed.extend(ctx.tools.iter().map(|t| format!("mcp__duo__{t}")));

        let (exe, bridge_args) = ctx.bridge_cmd();
        let mcp = json!({ "mcpServers": { "duo": { "type": "stdio", "command": exe, "args": bridge_args, "timeout": 3_600_000 } } });
        let s = &ctx.session;
        let hook = format!("\"{}\" hook {} {} {} {}", slashes(&exe), s.base_url(), s.id, ctx.agent.id, s.token);
        let matcher = EDIT_TOOLS.iter().chain(&SHELL_TOOLS).copied().collect::<Vec<_>>().join("|");
        // timeout: the hook waits for your answer
        let settings = json!({ "hooks": { "PreToolUse": [{ "matcher": matcher, "hooks": [{ "type": "command", "command": hook, "timeout": 3600 }] }] } });

        let mut args: Vec<String> = ["-p", "--output-format", "stream-json", "--verbose", "--permission-mode", "dontAsk"].map(String::from).to_vec();
        args.extend(["--allowedTools".into(), allowed.join(",")]);
        args.extend(["--mcp-config".into(), mcp.to_string(), "--settings".into(), settings.to_string()]);
        let shared = crate::shared::dir();
        if shared.is_dir() {
            args.extend(["--plugin-dir".into(), shared.to_string_lossy().into_owned()]);
        }
        let model = ctx.provider.as_ref().map(|p| p.model.clone()).or_else(|| std::env::var("CLAUDE_MODEL").ok()).filter(|m| !m.is_empty());
        if let Some(m) = model {
            args.extend(["--model".into(), m]);
        }
        if let Some(id) = &self.session_id {
            args.extend(["--resume".into(), id.clone()]);
        }
        let mut env = vec![("MCP_TOOL_TIMEOUT".to_string(), "3600000".to_string())]; // propose_split waits for you
        if let Some(p) = &ctx.provider {
            env.extend(provider_env(p));
        }

        let agent = ctx.agent.id.clone();
        let mut shown: HashSet<String> = HashSet::new(); // tool_use ids we showed
        let mut out = TurnResult::default();
        let mut failed: Option<String> = None;
        let spec = Spec { program: &program, args, env, cwd: &ctx.cwd, stdin: &ctx.prompt };
        run_jsonl(spec, &ctx.signal, |msg| {
            if let Some(id) = msg["session_id"].as_str() {
                self.session_id = Some(id.to_string());
            }
            match msg["type"].as_str() {
                Some("assistant") if msg["parent_tool_use_id"].is_null() => {
                    for b in msg["message"]["content"].as_array().into_iter().flatten() {
                        match b["type"].as_str() {
                            Some("text") => {
                                let t = b["text"].as_str().unwrap_or("").trim();
                                if !t.is_empty() {
                                    ctx.emit(EventBody::Say { agent: agent.clone(), text: t.into() });
                                }
                            }
                            Some("tool_use") => {
                                let Some((verb, target)) = line(b["name"].as_str().unwrap_or(""), &b["input"]) else { continue };
                                let id = b["id"].as_str().unwrap_or("").to_string();
                                shown.insert(id.clone());
                                ctx.emit(EventBody::Tool { agent: agent.clone(), call_id: id, verb, target, status: ToolStatus::Running, detail: None });
                            }
                            _ => {}
                        }
                    }
                }
                Some("user") => {
                    for b in msg["message"]["content"].as_array().into_iter().flatten() {
                        let id = b["tool_use_id"].as_str().unwrap_or("");
                        if b["type"] != "tool_result" || !shown.contains(id) {
                            continue;
                        }
                        let err = b["is_error"].as_bool().unwrap_or(false);
                        ctx.emit(EventBody::Tool {
                            agent: agent.clone(),
                            call_id: id.into(),
                            verb: String::new(),
                            target: String::new(),
                            status: if err { ToolStatus::Error } else { ToolStatus::Ok },
                            detail: err.then(|| preview_str(&text_of(&b["content"]), 160)),
                        });
                    }
                }
                Some("result") => {
                    let total = msg["total_cost_usd"].as_f64().unwrap_or(0.0);
                    out.cost_usd = (total - self.cost_so_far).max(0.0);
                    self.cost_so_far = total.max(self.cost_so_far);
                    let u = &msg["usage"];
                    let n = |k: &str| u[k].as_u64().unwrap_or(0);
                    let tin = n("input_tokens") + n("cache_read_input_tokens") + n("cache_creation_input_tokens");
                    out.tokens = tin + n("output_tokens");
                    if let Some(p) = &ctx.provider {
                        out.cost_usd = crate::providers::cost(p, tin, n("output_tokens")); // total_cost_usd is at Anthropic prices
                    }
                    if msg["subtype"] == "success" {
                        out.text = msg["result"].as_str().unwrap_or("").to_string();
                    } else {
                        failed = Some(format!("Claude stopped: {}", msg["subtype"].as_str().unwrap_or("error")));
                    }
                }
                _ => {}
            }
            Ok(())
        })
        .await?;
        if let Some(f) = failed {
            bail!(f);
        }
        Ok(out)
    }
}
