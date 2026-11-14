//! Notifications for the moments a session needs you: an approval, the work being done or stopped,
//! a failed agent, a message to you. The engine decides what is worth one; whoever embeds it
//! (the desktop app) decides how to show it. Same wording as web/src/duo/notify.ts.

use crate::types::{AgentStatus, EventBody, SessionStatus};
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub session: String,
    pub title: String,
    pub body: String,
}

type Sink = Box<dyn Fn(Notice) + Send + Sync>;
static SINK: OnceLock<Sink> = OnceLock::new();

/// Set once at start. Called while the session is locked: show it, never call back into the session.
pub fn set_sink(f: impl Fn(Notice) + Send + Sync + 'static) {
    let _ = SINK.set(Box::new(f));
}

pub fn send(n: Notice) {
    if let Some(f) = SINK.get() {
        f(n);
    }
}

fn short(s: &str, max: usize) -> String {
    let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > max { format!("{}…", one.chars().take(max - 1).collect::<String>()) } else { one }
}

/// None: nothing you need to know about right now.
pub fn notice(session: &str, task: &str, lead: &str, ev: &EventBody) -> Option<Notice> {
    let (title, body) = match ev {
        EventBody::ApprovalRequested { title, by, .. } => (format!("{by} needs your ok"), title.clone()),
        EventBody::SessionStatus { status: SessionStatus::Idle, .. } => (format!("{lead} is done"), "Waiting for your next message.".into()),
        EventBody::SessionStatus { status: SessionStatus::Failed, reason, .. } => ("Session stopped".into(), reason.clone().unwrap_or_default()),
        EventBody::AgentStatus { id, status: AgentStatus::Failed, note } => (format!("{id} failed"), note.clone().unwrap_or_default()),
        EventBody::Msg { from, to, text } if to == "you" => (format!("{from} says"), text.clone()),
        _ => return None,
    };
    Some(Notice { session: session.into(), title: short(&title, 60), body: format!("{}\n{}", short(&body, 140), short(task, 60)) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

    #[test]
    fn only_what_needs_you() {
        let n = |ev: EventBody| notice("s1", "Add a cache", "claude", &ev);
        let ask = EventBody::ApprovalRequested {
            id: "a".into(), kind: ApprovalKind::Merge, by: "claude".into(), title: "Merge 4 files?".into(), summary: None, items: None, detail: None,
        };
        assert_eq!(n(ask).unwrap(), Notice { session: "s1".into(), title: "claude needs your ok".into(), body: "Merge 4 files?\nAdd a cache".into() });
        assert!(n(EventBody::SessionStatus { status: SessionStatus::Stopped, reason: Some("Stopped by you".into()) }).is_none());
        assert!(n(EventBody::SessionStatus { status: SessionStatus::Running, reason: None }).is_none());
        assert_eq!(n(EventBody::SessionStatus { status: SessionStatus::Idle, reason: None }).unwrap().title, "claude is done");
        assert!(n(EventBody::Msg { from: "codex#1".into(), to: "claude".into(), text: "x".into() }).is_none());
        assert_eq!(n(EventBody::Msg { from: "codex#1".into(), to: "you".into(), text: "hi".into() }).unwrap().title, "codex#1 says");
        assert!(n(EventBody::Say { agent: "claude".into(), text: "x".into() }).is_none());
    }
}
