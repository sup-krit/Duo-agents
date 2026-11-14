//! `agent-duo bridge <baseUrl> <sessionId> <agentId> <token> <tool,tool,...>`
//! Stdio MCP server that each agent (Claude or Codex) runs as its "duo" tool server. It holds no state:
//! every tool call goes back to the session over HTTP, so the session decides who may assign, spawn,
//! wait or message, and the tree stays correct.
//!
//! `agent-duo hook <baseUrl> <sessionId> <agentId> <token>`
//! Claude Code's PreToolUse hook: reads the hook JSON on stdin, asks the session, prints the decision.

use rmcp::model::{CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock as Content, Implementation, InitializeResult, ListToolsResult, PaginatedRequestParams, ServerCapabilities, Tool};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::{Value, json};
use std::time::Duration;

fn obj(v: Value) -> serde_json::Map<String, Value> {
    v.as_object().cloned().unwrap_or_default()
}

/// The duo tools, with JSON Schemas the agents see. Which ones an agent gets is the session's decision.
pub fn tools() -> Vec<Tool> {
    let kind = json!({ "type": "string", "minLength": 1, "description": "Which kind of agent does this part" });
    let s = |d: &str| json!({ "type": "string", "description": d });
    let def = |name: &'static str, desc: &'static str, props: Value, required: &[&str]| {
        Tool::new(name, desc, obj(json!({ "type": "object", "properties": props, "required": required, "additionalProperties": false })))
    };
    vec![
        def(
            "propose_split",
            "Lead only. Propose how to split the user's task across team agents. The user reviews the split; once approved, every item starts as its own agent (items with `after` wait for those items to finish). Returns the new agent ids, or the user's requested changes.",
            json!({
                "summary": s("One or two sentences: why this split"),
                "items": { "type": "array", "minItems": 1, "items": {
                    "type": "object",
                    "properties": {
                        "agent": kind,
                        "role": s("Short role, e.g. build, tests, review"),
                        "task": s("What this agent must do, self-contained"),
                        "after": { "type": "array", "items": { "type": "integer" }, "description": "1-based indexes of items that must finish first" }
                    },
                    "required": ["agent", "role", "task"],
                    "additionalProperties": false
                }}
            }),
            &["summary", "items"],
        ),
        def(
            "assign",
            "Lead only. Start one more team agent for an extra piece of work after the split was approved.",
            json!({ "agent": { "type": "string", "minLength": 1 }, "role": { "type": "string" }, "task": { "type": "string" } }),
            &["agent", "role", "task"],
        ),
        def(
            "spawn_sub",
            "Split YOUR part: start a sub-agent of your own kind for an independent piece of your task. Use only when the piece can run in parallel. Call `wait` before you finish.",
            json!({ "role": s("Short role, e.g. sub, research, fix"), "task": { "type": "string" } }),
            &["role", "task"],
        ),
        def(
            "wait",
            "Wait for agents to finish (default: all agents you started). Returns their reports and any messages sent to you. Returns early after ~45s with progress; call again if some are still running.",
            json!({ "ids": { "type": "array", "items": { "type": "string" } } }),
            &[],
        ),
        def(
            "message",
            "Send a short message to another agent by id (e.g. codex#1), or to 'lead'. They read it on their next wait/inbox call.",
            json!({ "to": { "type": "string" }, "text": { "type": "string" } }),
            &["to", "text"],
        ),
        def("inbox", "Read messages sent to you by the user or other agents since your last check.", json!({}), &[]),
        def("status", "See the current agent tree: who is running, waiting or done, and what each is doing.", json!({}), &[]),
    ]
}

struct Bridge {
    base_url: String,
    session: String,
    agent: String,
    token: String,
    allowed: Vec<String>,
    http: reqwest::Client,
}

impl Bridge {
    async fn post(&self, path: &str, body: Value) -> Result<Value, String> {
        let url = format!("{}/api/sessions/{}/{path}", self.base_url, self.session);
        let res = self.http.post(url).json(&body).send().await.map_err(|e| format!("Agent Duo is not reachable: {e}"))?;
        let status = res.status();
        res.json::<Value>().await.map_err(|_| format!("HTTP {status}"))
    }
}

impl ServerHandler for Bridge {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(Implementation::new("duo", env!("CARGO_PKG_VERSION")))
    }

    async fn list_tools(&self, _: Option<PaginatedRequestParams>, _: RequestContext<RoleServer>) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tools().into_iter().filter(|t| self.allowed.iter().any(|a| a == t.name.as_ref())).collect()))
    }

    async fn call_tool(&self, req: CallToolRequestParams, _: RequestContext<RoleServer>) -> Result<CallToolResponse, ErrorData> {
        let name = req.name.to_string();
        if !self.allowed.contains(&name) {
            return Ok(CallToolResult::error(vec![Content::text(format!("Unknown tool {name}"))]).into());
        }
        let args = Value::Object(req.arguments.unwrap_or_default());
        let body = json!({ "agent": self.agent, "token": self.token, "name": name, "args": args });
        let (text, is_error) = match self.post("tool", body).await {
            Ok(v) => (v["text"].as_str().unwrap_or("").to_string(), v["isError"].as_bool().unwrap_or(false)),
            Err(e) => (e, true),
        };
        let content = vec![Content::text(text)];
        Ok(if is_error { CallToolResult::error(content) } else { CallToolResult::success(content) }.into())
    }
}

fn args(rest: &[String], n: usize, usage: &str) -> Vec<String> {
    if rest.len() < n || rest[..n].iter().any(String::is_empty) {
        eprintln!("usage: agent-duo {usage}");
        std::process::exit(2);
    }
    rest.to_vec()
}

fn http() -> reqwest::Client {
    // propose_split and wait block until you answer: no request timeout
    reqwest::Client::builder().no_proxy().connect_timeout(Duration::from_secs(10)).build().unwrap()
}

pub async fn run(rest: &[String]) -> anyhow::Result<()> {
    let a = args(rest, 4, "bridge <baseUrl> <sessionId> <agentId> <token> <tools>");
    let bridge = Bridge {
        base_url: a[0].clone(),
        session: a[1].clone(),
        agent: a[2].clone(),
        token: a[3].clone(),
        allowed: a.get(4).map(|t| t.split(',').filter(|s| !s.is_empty()).map(String::from).collect()).unwrap_or_default(),
        http: http(),
    };
    bridge.serve(rmcp::transport::stdio()).await?.waiting().await?;
    Ok(())
}

pub async fn hook(rest: &[String]) -> anyhow::Result<()> {
    let a = args(rest, 4, "hook <baseUrl> <sessionId> <agentId> <token>");
    let mut input = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)?;
    let input: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
    let bridge = Bridge { base_url: a[0].clone(), session: a[1].clone(), agent: a[2].clone(), token: a[3].clone(), allowed: vec![], http: http() };
    let body = json!({ "agent": bridge.agent, "token": bridge.token, "tool": input["tool_name"], "input": input["tool_input"] });
    // Fail closed: if the session cannot answer, the change does not happen.
    let out = bridge.post("gate", body).await.unwrap_or_else(|e| json!({ "decision": "deny", "reason": e }));
    let decision = out["decision"].as_str().unwrap_or("");
    if decision == "allow" || decision == "deny" {
        let mut h = json!({ "hookEventName": "PreToolUse", "permissionDecision": decision });
        if let Some(r) = out["reason"].as_str() {
            h["permissionDecisionReason"] = json!(r);
        }
        println!("{}", json!({ "hookSpecificOutput": h }));
    } else {
        println!("{{}}");
    }
    Ok(())
}
