//! Shared by the engine and the UI (TypeScript types are generated from these with ts-rs:
//! `cargo test -p duo-core` writes web/src/bindings/). The UI only ever renders events, so a
//! session can be replayed from runs/<id>/events.jsonl. Field order matters: the JSON must match
//! what the TypeScript version wrote, character for character.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// "claude", "codex", or the id of a provider from ~/.agent-duo/providers.json (e.g. "deepseek").
pub type Kind = String;

/// JS prints 5, not 5.0. Everything else (shortest round-trip, exponents) already matches ryu.
mod jsnum {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &f64, s: S) -> Result<S::Ok, S::Error> {
        if v.fract() == 0.0 && v.abs() < 1e15 { s.serialize_i64(*v as i64) } else { s.serialize_f64(*v) }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
        f64::deserialize(d)
    }
}

#[derive(Serialize, Deserialize, TS, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[ts(export)]
/// auto: agents edit and run commands · acceptEdits: commands need your yes · plan: read-only until you approve the plan
pub enum Permission {
    #[default]
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "acceptEdits")]
    AcceptEdits,
    #[serde(rename = "plan")]
    Plan,
}

#[derive(Serialize, Deserialize, TS, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Mode {
    Real,
    Mock,
}

#[derive(Serialize, Deserialize, TS, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum AgentStatus {
    /// in a split the user has not approved yet
    Proposed,
    /// approved, waiting for the items it depends on
    Queued,
    Running,
    /// blocked in `wait`, or on an approval
    Waiting,
    /// the lead between turns
    Idle,
    Done,
    Failed,
    Killed,
}

impl AgentStatus {
    pub fn finished(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Killed)
    }
}

/// git: worktree on a new branch · copy: agents work on a copy, applied when you approve · direct: edit the folder in place.
/// "auto" is resolved when the session starts: git for a git repo, otherwise copy.
#[derive(Serialize, Deserialize, TS, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Workspace {
    Git,
    Copy,
    Direct,
    Auto,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug, PartialEq)]
#[ts(export)]
pub struct Ask {
    pub split: bool,
    pub merge: bool,
    pub spawn: bool,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SessionConfig {
    pub lead: Kind,
    pub workspace: Workspace,
    pub team: Vec<Kind>,
    pub repo: String,
    pub mode: Mode,
    pub max_depth: u32,
    pub max_agents: u32,
    #[serde(with = "jsnum")]
    #[ts(type = "number")]
    pub budget_usd: f64,
    pub ask: Ask,
    // Sessions from v0.2 have no permission / afterPlan / protect.
    #[serde(default)]
    pub permission: Permission,
    /// The mode a plan session switches to once you approve the plan.
    #[serde(default)]
    pub after_plan: Permission,
    /// Globs of files that need your yes before an agent changes them. Empty: everything may change.
    #[serde(default)]
    pub protect: Vec<String>,
    /// What you want each kind on the team to do, in your words. Missing: the lead decides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub roles: Option<IndexMap<String, String>>,
    /// Display names of provider kinds (no keys).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub labels: Option<IndexMap<String, String>>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug, PartialEq)]
#[ts(export)]
pub struct AgentInfo {
    /// "claude", "codex#1", "codex#1.a"
    pub id: String,
    pub kind: Kind,
    pub role: String,
    pub task: String,
    /// null = started by you (the lead)
    pub parent: Option<String>,
    /// lead 0, team 1, subs 2+
    pub depth: u32,
}

/// One part of a proposed split, with the id the agent will get.
#[derive(Serialize, Deserialize, TS, Clone, Debug, PartialEq)]
#[ts(export)]
pub struct SplitItem {
    pub agent: Kind,
    pub role: String,
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub after: Option<Vec<i64>>,
    /// Empty while it is only a request from an agent.
    #[serde(default)]
    pub id: String,
}

#[derive(Serialize, Deserialize, TS, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum SessionStatus {
    Running,
    Waiting,
    Idle,
    Stopped,
    Failed,
}

#[derive(Serialize, Deserialize, TS, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum ToolStatus {
    Running,
    Ok,
    Error,
}

#[derive(Serialize, Deserialize, TS, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum ApprovalKind {
    Split,
    Spawn,
    Merge,
    Command,
    File,
}

#[derive(Serialize, Deserialize, TS, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Answer {
    Yes,
    No,
    Edit,
}

#[derive(Serialize, Deserialize, TS, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum EventBody {
    SessionStarted {
        #[serde(rename = "sessionId")]
        session_id: String,
        task: String,
        config: SessionConfig,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        branch: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        workdir: Option<String>,
    },
    AgentAdded {
        agent: AgentInfo,
        status: AgentStatus,
    },
    AgentStatus {
        id: String,
        status: AgentStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        note: Option<String>,
    },
    AgentUsage {
        id: String,
        #[ts(type = "number")]
        tokens: u64,
        #[serde(rename = "costUsd", with = "jsnum")]
        #[ts(type = "number")]
        cost_usd: f64,
    },
    Say {
        agent: String,
        text: String,
    },
    Tool {
        agent: String,
        #[serde(rename = "callId")]
        call_id: String,
        verb: String,
        target: String,
        status: ToolStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        detail: Option<String>,
    },
    Assign {
        by: String,
        id: String,
        role: String,
        task: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        queued: Option<bool>,
    },
    Spawn {
        by: String,
        id: String,
        role: String,
        task: String,
    },
    Msg {
        from: String,
        to: String,
        text: String,
    },
    User {
        to: Option<String>,
        text: String,
    },
    ApprovalRequested {
        id: String,
        kind: ApprovalKind,
        by: String,
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        summary: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        items: Option<Vec<SplitItem>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        detail: Option<String>,
    },
    ApprovalResolved {
        id: String,
        answer: Answer,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        note: Option<String>,
    },
    LeadChanged {
        from: String,
        to: String,
    },
    ModeChanged {
        permission: Permission,
        by: String,
    },
    Log {
        level: Level,
        text: String,
    },
    SessionStatus {
        status: SessionStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        reason: Option<String>,
    },
}

#[derive(Serialize, Deserialize, TS, Clone, Debug, PartialEq)]
#[ts(export)]
pub struct DuoEvent {
    #[serde(flatten)]
    pub body: EventBody,
    #[ts(type = "number")]
    pub seq: u64,
    #[ts(type = "number")]
    pub ts: u64,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SessionSummary {
    pub id: String,
    pub task: String,
    pub lead: Kind,
    pub repo: String,
    pub mode: Mode,
    pub status: SessionStatus,
    #[ts(type = "number")]
    pub started_at: u64,
    #[serde(with = "jsnum")]
    #[ts(type = "number")]
    pub cost_usd: f64,
    pub agents: u32,
}

/// claude: Anthropic-format API, runs on Claude Code · codex: OpenAI Responses API, runs on Codex
#[derive(Serialize, Deserialize, TS, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Engine {
    #[default]
    Claude,
    Codex,
}

/// Another AI (DeepSeek, Ollama, …). In the browser, `apiKey` is masked and `hasKey` says whether one is stored.
#[derive(Serialize, Deserialize, TS, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Provider {
    pub id: String,
    pub label: String,
    pub engine: Engine,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub usd_per_m_tok_in: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub usd_per_m_tok_out: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub has_key: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every events.jsonl the TypeScript version wrote must load; the v0.3 ones must come back byte for byte.
    #[test]
    fn reads_typescript_runs() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let mut n = 0;
        for f in std::fs::read_dir(&dir).unwrap() {
            let path = f.unwrap().path();
            let text = std::fs::read_to_string(&path).unwrap();
            let exact = path.to_string_lossy().contains("v03");
            for line in text.lines().filter(|l| !l.is_empty()) {
                let ev: DuoEvent = serde_json::from_str(line).unwrap_or_else(|e| panic!("{}: {e}\n{line}", path.display()));
                if exact {
                    assert_eq!(serde_json::to_string(&ev).unwrap(), line);
                }
                n += 1;
            }
        }
        assert!(n > 100, "only {n} events");
    }

    #[test]
    fn numbers_print_like_js() {
        let ev = EventBody::AgentUsage { id: "a".into(), tokens: 3, cost_usd: 5.0 };
        assert_eq!(serde_json::to_string(&ev).unwrap(), r#"{"type":"agent_usage","id":"a","tokens":3,"costUsd":5}"#);
        let ev = EventBody::AgentUsage { id: "a".into(), tokens: 3, cost_usd: 6100.0 * 0.000009 };
        assert_eq!(serde_json::to_string(&ev).unwrap(), r#"{"type":"agent_usage","id":"a","tokens":3,"costUsd":0.054900000000000004}"#);
    }
}
