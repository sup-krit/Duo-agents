//! A real MCP conversation over stdio with `agent-duo bridge`: each agent sees only its own tools,
//! and calls reach the session.

use duo_core::session::create_session;
use duo_core::types::*;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

async fn rpc(stdin: &mut tokio::process::ChildStdin, out: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>, id: u64, method: &str, params: Value) -> Value {
    let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    stdin.write_all(format!("{msg}\n").as_bytes()).await.unwrap();
    loop {
        let line = out.next_line().await.unwrap().expect("bridge closed");
        let v: Value = serde_json::from_str(&line).unwrap();
        if v["id"] == id {
            return v;
        }
    }
}

async fn tools_of(base: &str, sid: &str, agent: &str, token: &str, tools: &str) -> (Vec<String>, Value) {
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_agent-duo"))
        .args(["bridge", base, sid, agent, token, tools])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap()).lines();
    let init = rpc(&mut stdin, &mut out, 1, "initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "1" } })).await;
    assert_eq!(init["result"]["serverInfo"]["name"], "duo");
    stdin.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n").await.unwrap();
    let list = rpc(&mut stdin, &mut out, 2, "tools/list", json!({})).await;
    let names = list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    let call = rpc(&mut stdin, &mut out, 3, "tools/call", json!({ "name": "status", "arguments": {} })).await;
    (names, call["result"].clone())
}

#[tokio::test(flavor = "multi_thread")]
async fn bridge_lists_allowed_tools_and_calls_the_session() {
    duo_core::set_data_dir(std::env::temp_dir().join("agent-duo-tests"));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    tokio::spawn(axum::serve(listener, agent_duo::api::router(agent_duo::api::App { base_url: base.clone() })).into_future());

    let cfg = SessionConfig {
        lead: "claude".into(), workspace: Workspace::Auto, team: vec!["codex".into(), "claude".into()], repo: "(mock)".into(), mode: Mode::Mock,
        max_depth: 2, max_agents: 8, budget_usd: 5.0, ask: Ask { split: true, merge: true, spawn: false }, permission: Permission::Auto,
        after_plan: Permission::Auto, protect: vec![], roles: None, labels: None,
    };
    let s = create_session(cfg, "t".into(), base.clone(), 1.0).await.unwrap();

    let lead = "propose_split,assign,spawn_sub,wait,message,inbox,status";
    let (names, res) = tools_of(&base, &s.id, "claude", &s.token, lead).await;
    assert_eq!(names, lead.split(',').collect::<Vec<_>>());
    assert_eq!(res["isError"], false);
    assert!(res["content"][0]["text"].as_str().unwrap().starts_with("claude · claude · orchestrator"));

    let (names, _) = tools_of(&base, &s.id, "claude", &s.token, "wait,message,inbox,status").await;
    assert_eq!(names, ["wait", "message", "inbox", "status"]);

    let (_, res) = tools_of(&base, &s.id, "claude", "wrong-token", "status").await;
    assert_eq!(res["isError"], true);
    assert_eq!(res["content"][0]["text"], "Bad token");
    s.stop("done", SessionStatus::Stopped);
}
