pub mod claude;
pub mod codex;
pub mod mock;
pub mod proc;

use crate::session::Session;
use crate::types::{AgentInfo, EventBody, Permission, Provider};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Default)]
pub struct ToolResult {
    pub text: String,
    pub is_error: bool,
}

impl ToolResult {
    pub fn ok(t: impl Into<String>) -> Self {
        Self { text: t.into(), is_error: false }
    }
    pub fn err(t: impl Into<String>) -> Self {
        Self { text: t.into(), is_error: true }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// may only look at code
    Read,
    /// may edit files and run commands
    Write,
}

/// What a runner gets for one turn of one agent.
pub struct TurnCtx {
    pub session: Arc<Session>,
    pub agent: AgentInfo,
    pub prompt: String,
    pub cwd: PathBuf,
    pub access: Access,
    /// duo tools this agent may call (assign, spawn_sub, wait, …)
    pub tools: Vec<String>,
    pub signal: CancellationToken,
    /// Set when this agent's kind is a provider (DeepSeek, Ollama, …): the brain to use instead of the default one.
    pub provider: Option<Provider>,
}

impl TurnCtx {
    pub fn emit(&self, ev: EventBody) {
        self.session.emit(ev);
    }
    /// Mock agents call duo tools directly (the same code path as the bridge).
    pub async fn call_tool(&self, name: &str, args: Value) -> ToolResult {
        self.session.handle_tool(&self.agent.id, name, args).await
    }
    /// The session's mode right now (it can change mid-turn with /mode or when a plan is approved).
    pub fn permission(&self) -> Permission {
        self.session.permission()
    }
    /// `agent-duo bridge <baseUrl> <session> <agent> <token> <tools>`: the MCP server real agents run.
    pub fn bridge_cmd(&self) -> (String, Vec<String>) {
        let exe = std::env::current_exe().unwrap_or_else(|_| "agent-duo".into()).to_string_lossy().into_owned();
        let s = &self.session;
        (exe, vec!["bridge".into(), s.base_url(), s.id.clone(), self.agent.id.clone(), s.token.clone(), self.tools.join(",")])
    }
}

#[derive(Debug, Default)]
pub struct TurnResult {
    pub text: String,
    pub tokens: u64,
    pub cost_usd: f64,
}

/// One runner per agent, so follow-up turns keep the conversation.
pub enum Runner {
    Mock(mock::Mock),
    Claude(claude::Claude),
    Codex(codex::Codex),
}

impl Runner {
    pub async fn turn(&mut self, ctx: &TurnCtx) -> anyhow::Result<TurnResult> {
        match self {
            Runner::Mock(r) => r.turn(ctx).await,
            Runner::Claude(r) => r.turn(ctx).await,
            Runner::Codex(r) => r.turn(ctx).await,
        }
    }
}

/// One line, at most `max` characters.
pub fn preview(v: &Value, max: usize) -> String {
    let s = match v {
        Value::String(s) => s.clone(),
        Value::Null => "\"\"".into(),
        v => v.to_string(),
    };
    let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > max { format!("{}…", one.chars().take(max - 1).collect::<String>()) } else { one }
}

pub fn preview_str(s: &str, max: usize) -> String {
    preview(&Value::String(s.into()), max)
}
