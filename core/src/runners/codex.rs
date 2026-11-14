//! Codex through its CLI: `codex exec --json`, the duo tools as an MCP server set with `-c`, the
//! sandbox from the agent's access and the mode, and later turns through `codex exec resume`.

use super::proc::{Spec, find, run_jsonl, toml};
use super::{Access, TurnCtx, TurnResult, preview};
use crate::types::{EventBody, Level, Permission, ToolStatus};
use anyhow::{Result, bail};
use serde_json::{Value, json};

fn rate(var: &str) -> f64 {
    std::env::var(var).ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 1_000_000.0
}

/// One Codex thread per agent; later turns continue it.
#[derive(Default)]
pub struct Codex {
    thread: Option<String>,
}

impl Codex {
    pub async fn turn(&mut self, ctx: &TurnCtx) -> Result<TurnResult> {
        let Some(program) = find("codex") else { bail!("Codex was not found. Install it, or set DUO_CODEX to codex.exe.") };
        // Codex cannot ask you mid-turn (exec mode has no approvals), so it cannot honour "every command waits
        // for your yes" and runs read-only in accept edits as well as in plan mode.
        let read_only = ctx.access == Access::Read || ctx.permission() != Permission::Auto;
        let sandbox = if read_only { "read-only" } else { "workspace-write" };

        let mut cfg: Vec<(String, Value)> = vec![];
        let (exe, bridge_args) = ctx.bridge_cmd();
        cfg.push(("mcp_servers.duo.command".into(), json!(exe)));
        cfg.push(("mcp_servers.duo.args".into(), json!(bridge_args)));
        cfg.push(("mcp_servers.duo.startup_timeout_sec".into(), json!(30)));
        cfg.push(("mcp_servers.duo.tool_timeout_sec".into(), json!(3600))); // propose_split waits for you
        for (name, (desc, file)) in crate::shared::codex_agents(sandbox) {
            cfg.push((format!("agents.{name}.description"), json!(desc)));
            cfg.push((format!("agents.{name}.config_file"), json!(file.to_string_lossy())));
        }
        let mut env = vec![("CODEX_MANAGED_BY_NPM".to_string(), "1".to_string())];
        let p = &ctx.provider;
        if let Some(p) = p {
            // ponytail: Codex speaks only the Responses API (wire_api "chat" was removed)
            let base = format!("model_providers.{}", p.id);
            cfg.push((format!("{base}.name"), json!(p.label)));
            cfg.push((format!("{base}.base_url"), json!(p.base_url)));
            cfg.push((format!("{base}.env_key"), json!("DUO_PROVIDER_KEY")));
            cfg.push((format!("{base}.wire_api"), json!("responses")));
            cfg.push(("model_provider".into(), json!(p.id)));
            env.push(("DUO_PROVIDER_KEY".into(), if p.api_key.is_empty() { "none".into() } else { p.api_key.clone() }));
        }
        cfg.push(("approval_policy".into(), json!("never")));

        let mut args: Vec<String> = vec!["exec".into(), "--json".into()];
        for (k, v) in &cfg {
            args.extend(["--config".into(), format!("{k}={}", toml(v))]);
        }
        let model = p.as_ref().map(|p| p.model.clone()).or_else(|| std::env::var("CODEX_MODEL").ok()).filter(|m| !m.is_empty());
        if let Some(m) = model {
            args.extend(["--model".into(), m]);
        }
        args.extend(["--sandbox".into(), sandbox.into(), "--cd".into(), ctx.cwd.to_string_lossy().into_owned(), "--skip-git-repo-check".into()]);
        if let Some(t) = &self.thread {
            args.extend(["resume".into(), t.clone()]);
        }

        let agent = ctx.agent.id.clone();
        let mut out = TurnResult::default();
        let mut failed: Option<String> = None;
        let spec = Spec { program: &program, args, env, cwd: &ctx.cwd, stdin: &ctx.prompt };
        run_jsonl(spec, &ctx.signal, |ev| {
            let tool = |call_id: String, verb: String, target: String, status, detail| {
                ctx.emit(EventBody::Tool { agent: agent.clone(), call_id, verb, target, status, detail });
            };
            match ev["type"].as_str().unwrap_or("") {
                "thread.started" => self.thread = ev["thread_id"].as_str().map(String::from),
                "turn.failed" => failed = Some(format!("Codex failed: {}", ev["error"]["message"].as_str().unwrap_or(""))),
                "error" => failed = Some(format!("Codex error: {}", ev["message"].as_str().unwrap_or(""))),
                "turn.completed" => {
                    let u = &ev["usage"];
                    let n = |k: &str| u[k].as_u64().unwrap_or(0);
                    out.tokens = n("input_tokens") + n("output_tokens");
                    out.cost_usd = match p {
                        Some(p) => crate::providers::cost(p, n("input_tokens"), n("output_tokens")),
                        None => (n("input_tokens") - n("cached_input_tokens").min(n("input_tokens"))) as f64 * rate("CODEX_USD_PER_MTOK_IN")
                            + n("output_tokens") as f64 * rate("CODEX_USD_PER_MTOK_OUT"),
                    };
                }
                t @ ("item.started" | "item.completed") => {
                    let done = t == "item.completed";
                    let item = &ev["item"];
                    let id = item["id"].as_str().unwrap_or("").to_string();
                    let s = |k: &str| item[k].as_str().unwrap_or("").to_string();
                    match item["type"].as_str().unwrap_or("") {
                        "agent_message" if done && !s("text").trim().is_empty() => {
                            out.text = s("text");
                            ctx.emit(EventBody::Say { agent: agent.clone(), text: s("text").trim().into() });
                        }
                        "command_execution" => {
                            let code = item["exit_code"].as_i64();
                            let failed = s("status") == "failed" || (done && code.is_some_and(|c| c != 0));
                            let detail = failed.then(|| {
                                let out = s("aggregated_output");
                                let lines: Vec<&str> = out.lines().filter(|l| !l.is_empty()).collect();
                                preview(&json!(lines[lines.len().saturating_sub(2)..].join(" · ")), 160)
                            });
                            let status = if !done { ToolStatus::Running } else if failed { ToolStatus::Error } else { ToolStatus::Ok };
                            tool(id, "run".into(), preview(&item["command"], 70), status, detail);
                        }
                        "file_change" if done => {
                            for (i, c) in item["changes"].as_array().into_iter().flatten().enumerate() {
                                let verb = match c["kind"].as_str() {
                                    Some("add") => "add",
                                    Some("delete") => "delete",
                                    _ => "edit",
                                };
                                let status = if s("status") == "failed" { ToolStatus::Error } else { ToolStatus::Ok };
                                tool(format!("{id}-{i}"), verb.into(), c["path"].as_str().unwrap_or("").into(), status, None);
                            }
                        }
                        // duo calls are shown by the session as assign/spawn/msg
                        "mcp_tool_call" if s("server") != "duo" => {
                            let status = if !done { ToolStatus::Running } else if s("status") == "failed" { ToolStatus::Error } else { ToolStatus::Ok };
                            tool(id, format!("{}.{}", s("server"), s("tool")), preview(&item["arguments"], 60), status, item["error"]["message"].as_str().map(String::from));
                        }
                        "web_search" => tool(id, "search".into(), preview(&item["query"], 70), if done { ToolStatus::Ok } else { ToolStatus::Running }, None),
                        "error" => ctx.emit(EventBody::Log { level: Level::Warn, text: format!("{agent}: {}", s("message")) }),
                        _ => {}
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
