//! The flows the TypeScript version was tested with, on mock agents (20x speed).

use duo_core::session::{Session, create_session};
use duo_core::types::*;
use std::sync::Arc;
use std::time::Duration;

fn cfg(max_agents: u32, budget: f64) -> SessionConfig {
    SessionConfig {
        lead: "claude".into(),
        workspace: Workspace::Auto,
        team: vec!["codex".into(), "claude".into()],
        repo: "(mock)".into(),
        mode: Mode::Mock,
        max_depth: 2,
        max_agents,
        budget_usd: budget,
        ask: Ask { split: true, merge: true, spawn: false },
        permission: Permission::Auto,
        after_plan: Permission::Auto,
        protect: vec![],
        roles: None,
        labels: None,
    }
}

async fn start(c: SessionConfig) -> Arc<Session> {
    duo_core::set_data_dir(std::env::temp_dir().join("agent-duo-tests"));
    create_session(c, "Add a 60s cache to getUsers() with tests.".into(), "http://127.0.0.1:0".into(), 20.0).await.unwrap()
}

/// Wait (up to 20 s) until some event matches.
async fn until(s: &Session, what: &str, f: impl Fn(&EventBody) -> bool) {
    for _ in 0..2000 {
        let evs = s.lock().events.clone();
        if evs.iter().any(|e| f(&e.body)) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let all: Vec<String> = s.lock().events.iter().map(|e| serde_json::to_string(e).unwrap()).collect();
    panic!("timed out waiting for {what}\n{}", all.join("\n"));
}

fn count(s: &Session, f: impl Fn(&EventBody) -> bool) -> usize {
    s.lock().events.iter().filter(|e| f(&e.body)).count()
}

fn split_asks(s: &Session) -> usize {
    count(s, |b| matches!(b, EventBody::ApprovalRequested { kind: ApprovalKind::Split, .. }))
}

fn status_of(s: &Session, id: &str) -> Option<AgentStatus> {
    s.lock().events.iter().rev().find_map(|e| match &e.body {
        EventBody::AgentStatus { id: i, status, .. } if i == id => Some(*status),
        EventBody::AgentAdded { agent, status } if agent.id == id => Some(*status),
        _ => None,
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn edit_split_then_approve_subs_messages_merge() {
    let s = start(cfg(8, 5.0)).await;
    until(&s, "first split", |b| matches!(b, EventBody::ApprovalRequested { kind: ApprovalKind::Split, .. })).await;
    assert_eq!(s.input("e put the tests into the build part"), None);
    until(&s, "second split", |_| split_asks(&s) == 2).await;
    // the rejected numbers were given back: the new split starts at codex#1 again
    let ids: Vec<String> = s
        .lock()
        .events
        .iter()
        .rev()
        .find_map(|e| match &e.body {
            EventBody::ApprovalRequested { items: Some(i), .. } => Some(i.iter().map(|i| i.id.clone()).collect()),
            _ => None,
        })
        .unwrap();
    assert_eq!(ids, ["codex#1", "claude#2"]);
    assert_eq!(s.input("y"), None);

    until(&s, "subs", |b| matches!(b, EventBody::Spawn { id, .. } if id == "codex#1.b")).await;
    until(&s, "review queued then started", |b| matches!(b, EventBody::AgentStatus { id, status: AgentStatus::Running, .. } if id == "claude#2")).await;
    until(&s, "merge question", |b| matches!(b, EventBody::ApprovalRequested { kind: ApprovalKind::Merge, .. })).await;
    assert_eq!(status_of(&s, "codex#1"), Some(AgentStatus::Done));
    assert_eq!(status_of(&s, "codex#1.a"), Some(AgentStatus::Done));
    assert_eq!(s.input("yes"), None);
    until(&s, "merged", |b| matches!(b, EventBody::Log { text, .. } if text.starts_with("Merged into main (mock"))).await;
    until(&s, "idle", |b| matches!(b, EventBody::SessionStatus { status: SessionStatus::Idle, .. })).await;
    assert!(s.lock().total_cost > 0.0);

    // a follow-up to the lead runs a second turn; @agent, /help and unknown commands
    assert_eq!(s.input("thanks"), None);
    until(&s, "follow-up", |b| matches!(b, EventBody::Say { text, .. } if text.contains("(mock lead)"))).await;
    assert_eq!(s.input("@nobody hi").as_deref(), Some("No agent called nobody"));
    assert_eq!(s.input("/nope").as_deref(), Some("Unknown command /nope. Try /help."));
    assert_eq!(s.input("/mode edits"), None);
    assert!(count(&s, |b| matches!(b, EventBody::ModeChanged { permission: Permission::AcceptEdits, .. })) == 1);
    assert_eq!(s.input("/stop"), None);
    assert!(s.input("hi").unwrap().contains("ended"));

    // what was written to events.jsonl reads back the same
    let text = std::fs::read_to_string(s.dir.join("events.jsonl")).unwrap();
    let back: Vec<DuoEvent> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(back, s.lock().events);
}

#[tokio::test(flavor = "multi_thread")]
async fn agents_message_each_other() {
    let s = start(cfg(8, 5.0)).await;
    until(&s, "split", |b| matches!(b, EventBody::ApprovalRequested { kind: ApprovalKind::Split, .. })).await;
    s.input("y");
    until(&s, "tests tells build", |b| matches!(b, EventBody::Msg { from, to, .. } if from == "codex#2" && to == "codex#1")).await;
    until(&s, "build fixes it", |b| matches!(b, EventBody::Say { agent, text } if agent == "codex#1" && text.contains("fixing the expiry"))).await;
    s.stop("done", SessionStatus::Stopped);
}

#[tokio::test(flavor = "multi_thread")]
async fn kill_stops_an_agent_and_its_subs() {
    let s = start(cfg(8, 5.0)).await;
    until(&s, "split", |b| matches!(b, EventBody::ApprovalRequested { kind: ApprovalKind::Split, .. })).await;
    s.input("y");
    until(&s, "subs", |b| matches!(b, EventBody::Spawn { id, .. } if id == "codex#1.b")).await;
    assert_eq!(s.input("/kill codex#1"), None);
    for id in ["codex#1", "codex#1.a", "codex#1.b"] {
        assert_eq!(status_of(&s, id), Some(AgentStatus::Killed), "{id}");
    }
    assert_eq!(s.input("/kill ghost").as_deref(), Some("No agent called ghost"));
    // the killed builder stays killed when its turn unwinds
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(status_of(&s, "codex#1"), Some(AgentStatus::Killed));
    s.stop("done", SessionStatus::Stopped);
}

#[tokio::test(flavor = "multi_thread")]
async fn budget_stops_everything() {
    let s = start(cfg(8, 0.01)).await;
    until(&s, "split", |b| matches!(b, EventBody::ApprovalRequested { kind: ApprovalKind::Split, .. })).await;
    s.input("y");
    until(&s, "over budget", |b| matches!(b, EventBody::SessionStatus { status: SessionStatus::Failed, reason: Some(r) } if r.starts_with("Budget of $0.01 reached"))).await;
    assert!(s.ended());
    let busy = ["codex#1", "codex#2", "claude#3"].iter().filter(|id| matches!(status_of(&s, id), Some(AgentStatus::Running | AgentStatus::Waiting | AgentStatus::Queued))).count();
    assert_eq!(busy, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_limit_refuses_the_split() {
    let s = start(cfg(3, 5.0)).await; // lead + 3 parts would be 4
    until(&s, "lead gives up", |b| matches!(b, EventBody::Say { text, .. } if text == "Okay, I won't start anything.")).await;
    assert_eq!(split_asks(&s), 0);
    assert_eq!(count(&s, |b| matches!(b, EventBody::AgentAdded { .. })), 1);
    s.stop("done", SessionStatus::Stopped);
}

#[tokio::test(flavor = "multi_thread")]
async fn rejected_split_starts_nothing_and_orc_hands_over() {
    let s = start(cfg(8, 5.0)).await;
    until(&s, "split", |b| matches!(b, EventBody::ApprovalRequested { kind: ApprovalKind::Split, .. })).await;
    s.input("n not now");
    until(&s, "lead gives up", |b| matches!(b, EventBody::Say { text, .. } if text == "Okay, I won't start anything.")).await;
    until(&s, "idle", |b| matches!(b, EventBody::SessionStatus { status: SessionStatus::Idle, .. })).await;
    assert_eq!(status_of(&s, "codex#1"), Some(AgentStatus::Killed));
    assert_eq!(s.input("/orc codex"), None);
    until(&s, "lead changed", |b| matches!(b, EventBody::LeadChanged { to, .. } if to == "codex")).await;
    s.input("carry on");
    until(&s, "new lead answers", |b| matches!(b, EventBody::Say { agent, .. } if agent == "codex")).await;
    s.stop("done", SessionStatus::Stopped);
}
