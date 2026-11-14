//! Scripted agents for trying the whole flow without API keys. They call the same duo tools as real
//! agents (propose_split, spawn_sub, wait, message), so the tree, approvals, timeline and limits behave
//! exactly as in real mode. They do not touch any files.

use super::{ToolResult, TurnCtx, TurnResult};
use crate::types::{EventBody, ToolStatus};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::time::Duration;

#[derive(Default)]
pub struct Mock {
    turns: u32,
}

async fn sleep(ms: u64, ctx: &TurnCtx) -> Result<()> {
    let speed = ctx.session.mock_speed();
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_millis((ms as f64 / speed) as u64)) => Ok(()),
        _ = ctx.signal.cancelled() => bail!("stopped"),
    }
}

impl Mock {
    #[allow(unused_assignments)] // the last step!'s n += 1
    pub async fn turn(&mut self, ctx: &TurnCtx) -> Result<TurnResult> {
        self.turns += 1;
        let turns = self.turns;
        let agent = &ctx.agent;
        let mut n = 0;
        let say = |text: &str| ctx.emit(EventBody::Say { agent: agent.id.clone(), text: text.into() });
        macro_rules! step {
            ($verb:expr, $target:expr, $ms:expr) => {
                step!($verb, $target, $ms, None::<&str>)
            };
            ($verb:expr, $target:expr, $ms:expr, $fail:expr) => {{
                let call_id = format!("{}-{}-{}", agent.id, turns, n);
                n += 1;
                let tool = |status, detail: Option<String>| EventBody::Tool {
                    agent: agent.id.clone(),
                    call_id: call_id.clone(),
                    verb: $verb.into(),
                    target: $target.into(),
                    status,
                    detail,
                };
                ctx.emit(tool(ToolStatus::Running, None));
                sleep($ms, ctx).await?;
                let fail: Option<&str> = $fail;
                ctx.emit(tool(if fail.is_some() { ToolStatus::Error } else { ToolStatus::Ok }, fail.map(String::from)));
            }};
        }
        let call = |name: &'static str, args: Value| ctx.call_tool(name, args);
        let wait_all = |ids: Option<Vec<String>>| async move {
            let mut all = String::new();
            loop {
                let r = call("wait", ids.as_ref().map(|i| json!({ "ids": i })).unwrap_or(json!({}))).await;
                all += &r.text;
                all += "\n";
                if r.is_error || !r.text.to_lowercase().contains("still running") {
                    return all;
                }
            }
        };
        let rate = if agent.kind == "claude" { 0.000009 } else { 0.000005 };
        let done = |text: &str, tokens: u64| Ok(TurnResult { text: text.into(), tokens, cost_usd: tokens as f64 * rate });

        let role = if agent.depth == 0 {
            "lead"
        } else if agent.depth >= 2 {
            "sub"
        } else {
            agent.role.as_str()
        };

        if role == "lead" && (turns > 1 || ctx.prompt.contains("taking over as LEAD")) {
            sleep(800, ctx).await?;
            call("status", json!({})).await;
            let last = ctx.prompt.split('\n').next_back().unwrap_or("");
            say(&format!("Noted: \"{}\". Nothing else to hand out right now. (mock lead)", last.chars().take(80).collect::<String>()));
            return done("Acknowledged.", 900);
        }

        if role == "lead" {
            say("Looking at the code first.");
            step!("read", "src/api/users.ts", 700);
            step!("grep", "\"createUser|updateUser\"  2 hits", 600);
            step!("read", "package.json", 400);
            say("Three parts. Build and tests can run side by side; review waits for both.");
            let mut r: ToolResult = call(
                "propose_split",
                json!({
                    "summary": "Build and tests in parallel, then an independent review.",
                    "items": [
                        { "agent": "codex", "role": "build", "task": "Cache layer in src/lib/cache.ts, wire it into getUsers()" },
                        { "agent": "codex", "role": "tests", "task": "tests/cache.test.ts: hit, miss, expiry, invalidation" },
                        { "agent": "claude", "role": "review", "task": "Review the diff once build and tests pass", "after": [1, 2] },
                    ],
                }),
            )
            .await;
            if r.text.to_lowercase().starts_with("changes requested") {
                say("Understood, folding tests into the build part.");
                r = call(
                    "propose_split",
                    json!({
                        "summary": "One builder that also writes the tests, then a review.",
                        "items": [
                            { "agent": "codex", "role": "build", "task": "Cache layer + wire into getUsers() + tests/cache.test.ts" },
                            { "agent": "claude", "role": "review", "task": "Review the diff once the build passes", "after": [1] },
                        ],
                    }),
                )
                .await;
            }
            if r.is_error || r.text.to_lowercase().starts_with("rejected") {
                say("Okay, I won't start anything.");
                return done("Stopped before starting.", 3000);
            }
            wait_all(None).await;
            say("All parts are done and the review approved the change. getUsers() is cached for 60s, invalidated on createUser/updateUser, with 11 passing tests.");
            return done("Cache added, tests pass, review approved.", 6100);
        }

        if role == "build" {
            step!("read", "src/api/users.ts", 500);
            step!("grep", "\"getUsers\\(\"  4 call sites", 500);
            say("Two independent pieces, splitting them.");
            let a = call("spawn_sub", json!({ "role": "sub", "task": "Write the TTL cache module" })).await;
            let b = call("spawn_sub", json!({ "role": "sub", "task": "Invalidate on createUser / updateUser" })).await;
            if a.is_error || b.is_error {
                say(&format!("Could not split ({}); doing it myself.", if a.is_error { &a.text } else { &b.text }));
            }
            let got = wait_all(None).await;
            if got.contains("off-by-one") || got.contains(">=") {
                say("Right, fixing the expiry check in cache.ts.");
                step!("edit", "src/lib/cache.ts  +1 −1", 900);
            }
            step!("run", "npx tsc --noEmit", 1200);
            return done(
                "Added src/lib/cache.ts (+38) and wired getUsers() through it; invalidation in createUser/updateUser (+9 −4). Fixed expiry (>=). Typecheck clean.",
                18300,
            );
        }

        if role == "tests" {
            say("Checking how existing tests are set up first.");
            call("spawn_sub", json!({ "role": "sub", "task": "Find test helpers and DB mocks" })).await;
            wait_all(None).await;
            step!("add", "tests/cache.test.ts  +31", 1500);
            step!("run", "npm test", 1800, Some("1 failed · expires after ttl"));
            let status = call("status", json!({})).await.text;
            let build = status.lines().find(|l| l.split_once(" · codex · build").is_some_and(|(id, _)| !id.contains(char::is_whitespace)));
            if let Some(build) = build.and_then(|l| l.split(" · ").next()).map(String::from) {
                call("message", json!({ "to": build, "text": "Expiry check looks off by one: `now > exp` should be `now >= exp`." })).await;
                wait_all(Some(vec![build])).await;
            }
            step!("run", "npm test", 1600);
            return done("tests/cache.test.ts covers hit, miss, expiry and invalidation. npm test: 11 passed.", 14200);
        }

        if role == "review" {
            step!("run", "git diff --stat", 600);
            step!("read", "src/lib/cache.ts", 700);
            step!("read", "tests/cache.test.ts", 600);
            say("VERDICT: APPROVE. Invalidation covers both write paths and the tests exercise hit, miss, expiry and invalidation.");
            return done("APPROVE. No blocking issues.", 9400);
        }

        // sub-agents
        let t = agent.task.to_lowercase();
        if t.contains("helper") {
            step!("glob", "tests/**/*.ts", 700);
            step!("read", "tests/helpers/db.ts", 800);
            return done("Use createTestDb() from tests/helpers/db.ts and vi.useFakeTimers() for expiry.", 3100);
        }
        if t.contains("invalidate") {
            sleep(2500, ctx).await?; // starts after the cache module exists
            step!("edit", "src/api/users.ts  +9 −4", 2600);
            step!("run", "npx tsc --noEmit", 1400);
            return done("createUser/updateUser now clear the users cache.", 4200);
        }
        step!("add", "src/lib/cache.ts  +38", 2200);
        step!("run", "npx tsc --noEmit", 1300);
        done("TTL cache module in src/lib/cache.ts (Map + expiry).", 5200)
    }
}
