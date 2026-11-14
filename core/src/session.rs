//! The heart: agent tree, duo tools, approvals, modes, protected files, commands, limits, merge.
//! A std Mutex guards the state and is never held across an await.

use crate::copy::{self, Manifest};
use crate::git::{self, Worktree};
use crate::prompts::{follow_up_prompt, handover_prompt, lead_prompt, member_prompt};
use crate::providers::{self, BUILTIN};
use crate::runners::{Access, Runner, ToolResult, TurnCtx, TurnResult, claude, codex, mock};
use crate::types::*;
use anyhow::{Result, bail};
use futures_util::future::BoxFuture;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::{Notify, broadcast, oneshot};
use tokio_util::sync::CancellationToken;

const WAIT_MS: u64 = 45_000;
const LEAD_TOOLS: [&str; 7] = ["propose_split", "assign", "spawn_sub", "wait", "message", "inbox", "status"];

struct Msg {
    from: String,
    text: String,
}

struct Rec {
    info: AgentInfo,
    status: AgentStatus,
    runner: Arc<tokio::sync::Mutex<Runner>>,
    inbox: Vec<Msg>,
    children: Vec<String>,
    deps: Vec<String>,
    report: Option<String>,
    abort: CancellationToken,
    busy: bool,
    turns: u32,
    /// overrides the default first prompt (lead handover)
    first_prompt: Option<String>,
}

type Reply = oneshot::Sender<(Answer, Option<String>)>;

pub struct State {
    sid: String,
    pub cfg: SessionConfig,
    pub task: String,
    pub events: Vec<DuoEvent>,
    agents: Vec<Rec>,
    pub lead_id: String,
    pub workdir: PathBuf,
    wt: Option<Worktree>,
    /// copy mode: hashes of the original folder at the start
    manifest: Option<Manifest>,
    mock_merged: bool,
    pub total_cost: f64,
    pub status: SessionStatus,
    team_counter: u32,
    approvals: Vec<(String, Reply)>,
    ended: bool,
    /// shared skills copied into the workdir for Codex; removed before merge / at the end.
    codex_skills: Vec<PathBuf>,
    /// Protected files you already said yes to (relative, forward slashes); not asked again at merge.
    approved_files: HashSet<String>,
    merging: bool,
    log: std::fs::File,
    tx: broadcast::Sender<DuoEvent>,
}

pub struct Session {
    pub id: String,
    pub dir: PathBuf,
    pub token: String,
    base_url: String,
    mock_speed: f64,
    st: Mutex<State>,
    pulse: Notify,
}

fn runner_for(kind: &str, mode: Mode) -> Arc<tokio::sync::Mutex<Runner>> {
    Arc::new(tokio::sync::Mutex::new(match (mode, providers::engine_of(kind)) {
        (Mode::Mock, _) => Runner::Mock(mock::Mock::default()),
        (_, Engine::Codex) => Runner::Codex(codex::Codex::default()),
        (_, Engine::Claude) => Runner::Claude(claude::Claude::default()),
    }))
}

impl State {
    fn emit(&mut self, body: EventBody) {
        let ev = DuoEvent { body, seq: self.events.len() as u64, ts: crate::now_ms() };
        if let Ok(line) = serde_json::to_string(&ev) {
            let _ = writeln!(self.log, "{line}");
        }
        let _ = self.tx.send(ev.clone());
        if let Some(n) = crate::notify::notice(&self.sid, &self.task, &self.lead_id, &ev.body) {
            crate::notify::send(n);
        }
        self.events.push(ev);
    }

    fn log(&mut self, level: Level, text: impl Into<String>) {
        self.emit(EventBody::Log { level, text: text.into() });
    }

    fn idx(&self, id: &str) -> Option<usize> {
        self.agents.iter().position(|r| r.info.id == id)
    }

    fn rec(&mut self, id: &str) -> Option<&mut Rec> {
        self.agents.iter_mut().find(|r| r.info.id == id)
    }

    fn status_of(&self, id: &str) -> Option<AgentStatus> {
        self.agents.iter().find(|r| r.info.id == id).map(|r| r.status)
    }

    fn set_status(&mut self, id: &str, status: AgentStatus, note: Option<String>) {
        let Some(rec) = self.rec(id) else { return };
        if rec.status == status && note.is_none() {
            return;
        }
        rec.status = status;
        self.emit(EventBody::AgentStatus { id: id.into(), status, note });
        self.recompute();
    }

    fn recompute(&mut self) {
        if self.ended {
            return;
        }
        let next = if !self.approvals.is_empty() {
            SessionStatus::Waiting
        } else if self.agents.iter().any(|r| r.busy || r.status == AgentStatus::Queued) {
            SessionStatus::Running
        } else {
            SessionStatus::Idle
        };
        if next != self.status {
            self.status = next;
            self.emit(EventBody::SessionStatus { status: next, reason: None });
        }
    }

    fn add_agent(&mut self, info: AgentInfo, status: AgentStatus) {
        let runner = runner_for(&info.kind, self.cfg.mode);
        if let Some(parent) = info.parent.as_deref().and_then(|p| self.agents.iter_mut().find(|r| r.info.id == p)) {
            parent.children.push(info.id.clone());
        }
        let rec = Rec {
            info: info.clone(),
            status,
            runner,
            inbox: vec![],
            children: vec![],
            deps: vec![],
            report: None,
            abort: CancellationToken::new(),
            busy: false,
            turns: 0,
            first_prompt: None,
        };
        self.agents.push(rec);
        self.emit(EventBody::AgentAdded { agent: info, status });
    }

    fn tools_for(&self, rec: &Rec) -> Vec<String> {
        if rec.info.id == self.lead_id {
            return LEAD_TOOLS.iter().map(|s| s.to_string()).collect();
        }
        let mut t: Vec<String> = vec![];
        if rec.info.depth < self.cfg.max_depth {
            t.push("spawn_sub".into());
        }
        t.extend(["wait", "message", "inbox", "status"].map(String::from));
        t
    }

    fn access_for(&self, rec: &Rec) -> Access {
        // ponytail: keyword match on your own words; say "do not edit" / "ห้ามแก้" to make an AI read-only for sure
        let job = self.cfg.roles.as_ref().and_then(|r| r.get(&rec.info.kind)).map(|s| s.to_lowercase()).unwrap_or_default();
        let no_edits = ["review only", "do not edit", "don't edit", "no edits", "read-only", "read only", "ห้ามแก้", "อ่านอย่างเดียว"]
            .iter()
            .any(|w| job.contains(w));
        if rec.info.depth == 0 || no_edits || rec.info.role.to_lowercase().contains("review") { Access::Read } else { Access::Write }
    }

    /// `file`: absolute, or relative to the workdir. Files outside the workdir are never "protected".
    fn is_protected(&self, file: &str) -> bool {
        if self.cfg.protect.is_empty() {
            return false;
        }
        let Some(r) = crate::rel_path(&self.workdir, file) else { return false };
        let base = r.rsplit('/').next().unwrap_or(&r);
        self.cfg.protect.iter().any(|p| {
            let Ok(g) = globset::GlobBuilder::new(p).literal_separator(true).build() else { return false };
            let m = g.compile_matcher();
            m.is_match(&r) || (!p.contains('/') && m.is_match(base))
        })
    }

    fn new_id(&mut self, parent: Option<&str>, kind: &str) -> String {
        let p = parent.and_then(|p| self.agents.iter().find(|r| r.info.id == p));
        match p {
            Some(p) if p.info.depth > 0 => {
                let n = p.children.iter().filter(|c| self.agents.iter().any(|r| &r.info.id == *c && r.info.depth == p.info.depth + 1)).count();
                format!("{}.{}", p.info.id, (b'a' + n as u8) as char)
            }
            _ => {
                self.team_counter += 1;
                format!("{kind}#{}", self.team_counter)
            }
        }
    }

    fn live(&self) -> usize {
        self.agents.iter().filter(|r| !matches!(r.status, AgentStatus::Killed | AgentStatus::Proposed)).count()
    }

    fn first_prompt_for(&self, id: &str) -> String {
        let team: Vec<String> =
            self.agents.iter().filter(|r| !r.status.finished() && r.status != AgentStatus::Proposed).map(|r| r.info.id.clone()).collect();
        let rec = self.agents.iter().find(|r| r.info.id == id).unwrap();
        member_prompt(&self.cfg, &rec.info, &self.lead_id, &team)
    }

    fn set_permission(&mut self, p: Permission, by: &str) {
        if self.cfg.permission == p {
            return;
        }
        self.cfg.permission = p;
        if p != Permission::Plan {
            self.cfg.after_plan = p;
        }
        self.emit(EventBody::ModeChanged { permission: p, by: by.into() });
    }

    fn drop_codex_skills(&mut self) {
        crate::shared::remove_codex_skills(&self.workdir, &self.codex_skills);
        self.codex_skills.clear();
    }

    fn kill(&mut self, id: &str, by: &str) -> bool {
        let Some(rec) = self.rec(id) else { return false };
        for c in rec.children.clone() {
            self.kill(&c, by);
        }
        let rec = self.rec(id).unwrap();
        if !rec.status.finished() && rec.status != AgentStatus::Idle {
            rec.abort.cancel();
            rec.busy = false;
            self.set_status(id, AgentStatus::Killed, Some(format!("stopped by {by}")));
        }
        true
    }
}

static SESSIONS: LazyLock<Mutex<HashMap<String, Arc<Session>>>> = LazyLock::new(Default::default);

pub fn get_session(id: &str) -> Option<Arc<Session>> {
    SESSIONS.lock().unwrap().get(id).cloned()
}

/// Start a session. `base_url` is where the MCP bridge of real agents calls back.
pub async fn create_session(cfg: SessionConfig, task: String, base_url: String, mock_speed: f64) -> Result<Arc<Session>> {
    let id = format!("{}-{}", crate::stamp(crate::now_ms()), crate::rand_hex(3));
    let s = Session::new(id.clone(), cfg, task, base_url, mock_speed)?;
    SESSIONS.lock().unwrap().insert(id, s.clone());
    if let Err(e) = s.start().await {
        s.stop(&e.to_string(), SessionStatus::Failed);
        return Err(e);
    }
    Ok(s)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}

pub fn read_events(id: &str) -> Option<Vec<DuoEvent>> {
    if !valid_id(id) {
        return None;
    }
    let text = std::fs::read_to_string(crate::runs_dir().join(id).join("events.jsonl")).ok()?;
    Some(text.lines().filter(|l| !l.is_empty()).filter_map(|l| serde_json::from_str(l).ok()).collect())
}

pub fn list_sessions() -> Vec<SessionSummary> {
    let Ok(rd) = std::fs::read_dir(crate::runs_dir()) else { return vec![] };
    let mut out: Vec<SessionSummary> = rd
        .flatten()
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().into_owned();
            let live = get_session(&id);
            let evs = match &live {
                Some(s) => s.lock().events.clone(),
                None => read_events(&id)?,
            };
            let start = evs.iter().find_map(|e| match &e.body {
                EventBody::SessionStarted { task, config, .. } => Some((e.ts, task.clone(), config.clone())),
                _ => None,
            })?;
            let mut status = if live.is_some() { SessionStatus::Running } else { SessionStatus::Stopped };
            let (mut cost, mut agents) = (0.0, 0);
            for e in &evs {
                match &e.body {
                    EventBody::SessionStatus { status: s, .. } => status = *s,
                    EventBody::AgentUsage { cost_usd, .. } => cost += cost_usd,
                    EventBody::AgentAdded { .. } => agents += 1,
                    _ => {}
                }
            }
            if live.is_none() && matches!(status, SessionStatus::Running | SessionStatus::Waiting) {
                status = SessionStatus::Stopped;
            }
            let (ts, task, cfg) = start;
            Some(SessionSummary { id, task, lead: cfg.lead, repo: cfg.repo, mode: cfg.mode, status, started_at: ts, cost_usd: cost, agents })
        })
        .collect();
    out.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    out
}

struct AskReq {
    kind: ApprovalKind,
    by: String,
    title: String,
    summary: Option<String>,
    items: Option<Vec<SplitItem>>,
    detail: Option<String>,
}

impl AskReq {
    fn new(kind: ApprovalKind, by: &str, title: impl Into<String>, detail: Option<String>) -> Self {
        Self { kind, by: by.into(), title: title.into(), summary: None, items: None, detail }
    }
}

fn arg_str(args: &Value, k: &str) -> String {
    match &args[k] {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}

impl Session {
    fn new(id: String, cfg: SessionConfig, task: String, base_url: String, mock_speed: f64) -> Result<Arc<Self>> {
        let dir = crate::runs_dir().join(&id);
        std::fs::create_dir_all(&dir)?;
        let log = std::fs::OpenOptions::new().create(true).write(true).truncate(true).open(dir.join("events.jsonl"))?;
        let st = State {
            sid: id.clone(),
            cfg,
            task,
            events: vec![],
            agents: vec![],
            lead_id: String::new(),
            workdir: PathBuf::new(),
            wt: None,
            manifest: None,
            mock_merged: false,
            total_cost: 0.0,
            status: SessionStatus::Running,
            team_counter: 0,
            approvals: vec![],
            ended: false,
            codex_skills: vec![],
            approved_files: HashSet::new(),
            merging: false,
            log,
            tx: broadcast::channel(4096).0,
        };
        Ok(Arc::new(Self { id, dir, token: crate::rand_hex(16), base_url, mock_speed, st: Mutex::new(st), pulse: Notify::new() }))
    }

    pub fn lock(&self) -> MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn emit(&self, body: EventBody) {
        self.lock().emit(body);
    }

    /// Everything so far and the live stream after it, with nothing lost in between.
    pub fn subscribe(&self) -> (Vec<DuoEvent>, broadcast::Receiver<DuoEvent>) {
        let st = self.lock();
        (st.events.clone(), st.tx.subscribe())
    }

    pub fn permission(&self) -> Permission {
        self.lock().cfg.permission
    }

    pub fn is_protected(&self, file: &str) -> bool {
        self.lock().is_protected(file)
    }

    /// What this agent may do right now (None: no such agent).
    pub fn access_of(&self, agent: &str) -> Option<Access> {
        let st = self.lock();
        st.agents.iter().find(|r| r.info.id == agent).map(|r| st.access_for(r))
    }

    pub fn workdir(&self) -> PathBuf {
        self.lock().workdir.clone()
    }

    pub fn ended(&self) -> bool {
        self.lock().ended
    }

    pub fn mock_speed(&self) -> f64 {
        self.mock_speed
    }

    pub fn base_url(&self) -> String {
        self.base_url.clone()
    }

    fn pulse(&self) {
        self.pulse.notify_waiters();
    }

    // ── lifecycle ───────────────────────────────────────────────────────────
    async fn start(self: &Arc<Self>) -> Result<()> {
        let mut cfg = self.lock().cfg.clone();
        let (mut wt, mut manifest) = (None, None);
        let workdir: PathBuf;
        if cfg.mode == Mode::Mock {
            cfg.workspace = Workspace::Copy;
            workdir = std::env::temp_dir().join(format!("duo-mock-{}", crate::rand_hex(3)));
            std::fs::create_dir_all(&workdir)?;
        } else {
            let repo = std::path::absolute(&cfg.repo)?;
            if !repo.is_dir() {
                bail!("Folder not found: {}", cfg.repo);
            }
            cfg.repo = repo.to_string_lossy().into_owned();
            if cfg.workspace == Workspace::Direct && !cfg.protect.is_empty() {
                cfg.workspace = Workspace::Auto; // editing in place has no merge step to hold protected files back
                self.lock().log(Level::Warn, "Protected files are set, so agents work on a branch or a copy instead of editing the folder directly.");
            }
            let is_git = git::is_git_repo(&repo).await;
            if cfg.workspace == Workspace::Auto {
                cfg.workspace = if is_git { Workspace::Git } else { Workspace::Copy };
            }
            if cfg.workspace == Workspace::Git && !is_git {
                cfg.workspace = Workspace::Copy;
            }
            match cfg.workspace {
                Workspace::Git => {
                    let w = git::create_worktree(&repo, &self.id).await?;
                    workdir = w.dir.clone();
                    wt = Some(w);
                }
                Workspace::Copy => {
                    workdir = self.dir.join("workspace");
                    let (r, w) = (repo.clone(), workdir.clone());
                    manifest = Some(tokio::task::spawn_blocking(move || copy::snapshot(&r, &w)).await??);
                }
                _ => workdir = repo,
            }
        }
        let mut st = self.lock();
        st.cfg = cfg.clone();
        st.workdir = workdir.clone();
        st.manifest = manifest;
        let branch = wt.as_ref().map(|w| w.branch.clone());
        st.wt = wt;
        let task = st.task.clone();
        st.emit(EventBody::SessionStarted {
            session_id: self.id.clone(),
            task: task.clone(),
            config: cfg.clone(),
            branch,
            workdir: Some(workdir.to_string_lossy().into_owned()),
        });
        let info = AgentInfo {
            id: cfg.lead.clone(),
            kind: cfg.lead.clone(),
            role: "orchestrator".into(),
            task: "Plan, split the work, check the results".into(),
            parent: None,
            depth: 0,
        };
        st.add_agent(info.clone(), AgentStatus::Idle);
        st.lead_id = info.id.clone();
        st.emit(EventBody::User { to: None, text: task.clone() });
        let prompt = lead_prompt(&st.cfg, &task, &info);
        self.start_turn(&mut st, &info.id, prompt);
        Ok(())
    }

    fn start_turn(self: &Arc<Self>, st: &mut State, id: &str, prompt: String) {
        let Some(i) = st.idx(id) else { return };
        if st.ended || st.agents[i].busy || st.agents[i].status == AgentStatus::Killed {
            return;
        }
        let codex = providers::engine_of(&st.agents[i].info.kind) == Engine::Codex;
        if codex && st.cfg.mode == Mode::Real && st.codex_skills.is_empty() {
            st.codex_skills = crate::shared::add_codex_skills(&st.workdir);
        }
        let rec = &mut st.agents[i];
        rec.busy = true;
        rec.turns += 1;
        if rec.abort.is_cancelled() {
            rec.abort = CancellationToken::new();
        }
        st.set_status(id, AgentStatus::Running, None);
        let rec = &st.agents[i];
        let ctx = TurnCtx {
            session: self.clone(),
            agent: rec.info.clone(),
            prompt,
            cwd: st.workdir.clone(),
            access: st.access_for(rec),
            tools: st.tools_for(rec),
            signal: rec.abort.clone(),
            provider: None,
        };
        let runner = rec.runner.clone();
        let real = st.cfg.mode == Mode::Real;
        let me = self.clone();
        tokio::spawn(async move {
            let mut ctx = ctx;
            let kind = ctx.agent.kind.clone();
            let res = if BUILTIN.contains(&kind.as_str()) || !real {
                runner.lock().await.turn(&ctx).await
            } else if let Some(p) = providers::get(&kind) {
                ctx.provider = Some(p);
                runner.lock().await.turn(&ctx).await
            } else {
                Err(anyhow::anyhow!("Provider \"{kind}\" is not set up any more. Add it again under Other AI."))
            };
            me.after_turn(&ctx.agent.id, res).await;
        });
    }

    async fn after_turn(self: &Arc<Self>, id: &str, res: Result<TurnResult>) {
        let lead_idle = {
            let mut st = self.lock();
            let (lead, ended) = (st.lead_id == id, st.ended);
            let Some(rec) = st.rec(id) else { return };
            rec.busy = false;
            match res {
                Ok(r) => {
                    rec.report = Some(r.text);
                    st.total_cost += r.cost_usd;
                    st.emit(EventBody::AgentUsage { id: id.into(), tokens: r.tokens, cost_usd: r.cost_usd });
                    // A turn that ends after /kill or /stop stays stopped.
                    let next = if st.status_of(id) == Some(AgentStatus::Killed) || st.ended {
                        AgentStatus::Killed
                    } else if lead {
                        AgentStatus::Idle
                    } else {
                        AgentStatus::Done
                    };
                    st.set_status(id, next, None);
                }
                Err(e) => {
                    if rec.abort.is_cancelled() || ended {
                        st.set_status(id, AgentStatus::Killed, None);
                    } else {
                        let msg = format!("{e:#}");
                        rec.report = Some(format!("Failed: {msg}"));
                        st.set_status(id, if lead { AgentStatus::Idle } else { AgentStatus::Failed }, Some(msg.clone()));
                        st.log(Level::Error, format!("{id} failed: {msg}"));
                    }
                }
            }
            self.pulse();
            if self.check_budget(&mut st) {
                return;
            }
            self.start_ready(&mut st);
            // Messages that arrived while it was busy: the user's (or anything for the lead) start a follow-up turn.
            self.deliver(&mut st, id);
            lead && !st.rec(id).is_some_and(|r| r.busy)
        };
        if lead_idle {
            self.offer_merge(false).await;
        }
        self.lock().recompute();
    }

    fn start_ready(self: &Arc<Self>, st: &mut State) {
        let ready: Vec<(String, Vec<String>)> = st
            .agents
            .iter()
            .filter(|r| r.status == AgentStatus::Queued)
            .filter_map(|r| {
                let deps: Vec<Option<AgentStatus>> = r.deps.iter().map(|d| st.status_of(d)).collect();
                deps.iter().all(|d| d.is_some_and(|s| s.finished())).then(|| {
                    let failed = r.deps.iter().filter(|d| st.status_of(d) != Some(AgentStatus::Done)).cloned().collect();
                    (r.info.id.clone(), failed)
                })
            })
            .collect();
        for (id, failed) in ready {
            let note = if failed.is_empty() { String::new() } else { format!("\n\nNote: {} did not finish successfully.", failed.join(", ")) };
            let prompt = st.first_prompt_for(&id) + &note;
            self.start_turn(st, &id, prompt);
        }
    }

    fn deliver(self: &Arc<Self>, st: &mut State, id: &str) {
        let is_lead = st.lead_id == id;
        let Some(rec) = st.rec(id) else { return };
        if rec.busy || rec.status == AgentStatus::Killed || rec.inbox.is_empty() {
            return;
        }
        if !is_lead && !rec.inbox.iter().any(|m| m.from == "you") {
            return;
        }
        let msgs: Vec<Msg> = rec.inbox.drain(..).collect();
        let mut prompt = msgs.iter().map(|m| follow_up_prompt(&m.from, &m.text)).collect::<Vec<_>>().join("\n\n");
        if rec.turns == 0 {
            if let Some(first) = &rec.first_prompt {
                prompt = format!("{first}\n\nThe user says:\n{prompt}");
            }
        }
        self.start_turn(st, id, prompt);
    }

    fn check_budget(&self, st: &mut State) -> bool {
        if st.cfg.budget_usd > 0.0 && st.total_cost > st.cfg.budget_usd {
            let reason = format!("Budget of ${} reached (${:.2})", js_num(st.cfg.budget_usd), st.total_cost);
            self.stop_locked(st, &reason, SessionStatus::Failed);
            return true;
        }
        false
    }

    pub fn stop(&self, reason: &str, status: SessionStatus) {
        let mut st = self.lock();
        self.stop_locked(&mut st, reason, status);
    }

    fn stop_locked(&self, st: &mut State, reason: &str, status: SessionStatus) {
        if st.ended {
            return;
        }
        for (id, reply) in std::mem::take(&mut st.approvals) {
            st.emit(EventBody::ApprovalResolved { id, answer: Answer::No, note: Some("session stopped".into()) });
            let _ = reply.send((Answer::No, None));
        }
        let ids: Vec<String> = st
            .agents
            .iter()
            .filter(|r| r.busy || matches!(r.status, AgentStatus::Queued | AgentStatus::Proposed))
            .map(|r| r.info.id.clone())
            .collect();
        for id in ids {
            let rec = st.rec(&id).unwrap();
            rec.abort.cancel();
            rec.busy = false;
            st.set_status(&id, AgentStatus::Killed, Some(reason.into()));
        }
        st.ended = true;
        st.drop_codex_skills();
        st.status = status;
        st.emit(EventBody::SessionStatus { status, reason: Some(reason.into()) });
        self.pulse();
    }

    pub fn kill(self: &Arc<Self>, id: &str, by: &str) -> bool {
        let mut st = self.lock();
        if !st.kill(id, by) {
            return false;
        }
        self.pulse();
        self.start_ready(&mut st);
        true
    }

    // ── approvals ───────────────────────────────────────────────────────────
    async fn ask(&self, req: AskReq) -> (Answer, Option<String>) {
        let rx = {
            let mut st = self.lock();
            if st.ended {
                return (Answer::No, None);
            }
            let id = crate::rand_hex(4);
            st.emit(EventBody::ApprovalRequested {
                id: id.clone(),
                kind: req.kind,
                by: req.by,
                title: req.title,
                summary: req.summary,
                items: req.items,
                detail: req.detail,
            });
            let (tx, rx) = oneshot::channel();
            st.approvals.push((id, tx));
            st.recompute();
            rx
        };
        rx.await.unwrap_or((Answer::No, None))
    }

    /// Answer one pending approval (the oldest when `id` is None).
    pub fn answer(&self, id: Option<&str>, answer: Answer, note: Option<String>) -> bool {
        let mut st = self.lock();
        let Some(i) = (match id {
            Some(id) => st.approvals.iter().position(|(k, _)| k == id),
            None => (!st.approvals.is_empty()).then_some(0),
        }) else {
            return false;
        };
        let (key, reply) = st.approvals.remove(i);
        st.emit(EventBody::ApprovalResolved { id: key, answer, note: note.clone() });
        let _ = reply.send((answer, note));
        st.recompute();
        true
    }

    pub fn pending_approvals(&self) -> usize {
        self.lock().approvals.len()
    }

    /// An agent's command or protected file waits for your yes. `file` is remembered as approved when you say yes.
    pub async fn approve(&self, agent: &str, kind: ApprovalKind, title: String, detail: String, file: Option<String>) -> (bool, Option<String>) {
        let before = {
            let mut st = self.lock();
            if st.ended {
                return (false, Some("the session has ended".into()));
            }
            let before = st.status_of(agent);
            st.set_status(agent, AgentStatus::Waiting, None);
            before
        };
        let (answer, note) = self.ask(AskReq::new(kind, agent, title, Some(detail))).await;
        let mut st = self.lock();
        if !st.ended && st.status_of(agent) == Some(AgentStatus::Waiting) && before != Some(AgentStatus::Waiting) {
            st.set_status(agent, AgentStatus::Running, None);
        }
        let ok = answer == Answer::Yes;
        if let Some(r) = file.as_deref().and_then(|f| crate::rel_path(&st.workdir, f)).filter(|_| ok) {
            st.approved_files.insert(r);
        }
        (ok, note)
    }

    // ── duo tools (called by agents through the MCP bridge, or directly by mock agents) ──
    pub fn handle_tool(self: &Arc<Self>, agent: &str, name: &str, args: Value) -> BoxFuture<'static, ToolResult> {
        let (me, agent, name) = (self.clone(), agent.to_string(), name.to_string());
        Box::pin(async move {
            {
                let mut st = me.lock();
                if st.rec(&agent).is_none() {
                    return ToolResult::err(format!("Unknown agent {agent}"));
                }
                if st.ended {
                    return ToolResult::err("The session has ended. Stop now.");
                }
            }
            let mut res = me.tool(&agent, &name, &args).await;
            let unread = me.lock().rec(&agent).map(|r| r.inbox.len()).unwrap_or(0);
            if name != "inbox" && name != "wait" && unread > 0 {
                res.text += &format!("\n\n(You have {unread} unread message{}. Call inbox.)", if unread > 1 { "s" } else { "" });
            }
            res
        })
    }

    async fn tool(self: &Arc<Self>, me: &str, name: &str, args: &Value) -> ToolResult {
        let is_lead = self.lock().lead_id == me;
        match name {
            "propose_split" => self.propose_split(me, is_lead, args).await,
            "assign" => {
                if !is_lead {
                    return ToolResult::err("Only the lead can assign work to the team.");
                }
                let kind = arg_str(args, "agent");
                let mut st = self.lock();
                if !st.cfg.team.contains(&kind) {
                    return ToolResult::err(format!("{kind} is not on this team"));
                }
                if st.live() >= st.cfg.max_agents as usize {
                    return ToolResult::err(format!("Agent limit ({}) reached.", st.cfg.max_agents));
                }
                let id = st.new_id(None, &kind);
                let role = Some(arg_str(args, "role")).filter(|r| !r.is_empty()).unwrap_or_else(|| "task".into());
                let info = AgentInfo { id: id.clone(), kind, role: role.clone(), task: arg_str(args, "task"), parent: Some(me.into()), depth: 1 };
                st.add_agent(info.clone(), AgentStatus::Queued);
                st.emit(EventBody::Assign { by: me.into(), id: id.clone(), role, task: info.task, queued: None });
                let p = st.first_prompt_for(&id);
                self.start_turn(&mut st, &id, p);
                ToolResult::ok(format!("Started {id}."))
            }
            "spawn_sub" => {
                let (depth, kind, ask) = {
                    let mut st = self.lock();
                    let rec = st.rec(me).unwrap();
                    let (depth, kind) = (rec.info.depth + 1, rec.info.kind.clone());
                    if depth > st.cfg.max_depth {
                        return ToolResult::err(format!("Max depth {} reached; do this yourself.", st.cfg.max_depth));
                    }
                    if st.live() >= st.cfg.max_agents as usize {
                        return ToolResult::err(format!("Agent limit ({}) reached; do this yourself.", st.cfg.max_agents));
                    }
                    (depth, kind, st.cfg.ask.spawn)
                };
                let task = arg_str(args, "task");
                if ask {
                    self.lock().set_status(me, AgentStatus::Waiting, None);
                    let (a, note) = self.ask(AskReq::new(ApprovalKind::Spawn, me, format!("{me} wants a sub-agent"), Some(task.clone()))).await;
                    let mut st = self.lock();
                    if !st.ended {
                        st.set_status(me, AgentStatus::Running, None);
                    }
                    if a != Answer::Yes {
                        return ToolResult::ok(format!("The user declined this sub-agent{}. Do it yourself.", note.map(|n| format!(": {n}")).unwrap_or_default()));
                    }
                }
                let mut st = self.lock();
                let id = st.new_id(Some(me), &kind);
                let role = Some(arg_str(args, "role")).filter(|r| !r.is_empty()).unwrap_or_else(|| "sub".into());
                st.add_agent(AgentInfo { id: id.clone(), kind, role: role.clone(), task: task.clone(), parent: Some(me.into()), depth }, AgentStatus::Queued);
                st.emit(EventBody::Spawn { by: me.into(), id: id.clone(), role, task });
                let p = st.first_prompt_for(&id);
                self.start_turn(&mut st, &id, p);
                ToolResult::ok(format!("Started {id}. Call wait before you finish."))
            }
            "wait" => self.wait(me, args).await,
            "message" => {
                let mut st = self.lock();
                let mut to = arg_str(args, "to");
                let text = arg_str(args, "text");
                if to == "lead" {
                    to = st.lead_id.clone();
                }
                if to == "you" || to == "user" {
                    st.emit(EventBody::Msg { from: me.into(), to: "you".into(), text });
                    return ToolResult::ok("Shown to the user.");
                }
                let Some(target) = st.rec(&to) else {
                    return ToolResult::err(format!("No agent called {to}. Call status to see ids."));
                };
                target.inbox.push(Msg { from: me.into(), text: text.clone() });
                st.emit(EventBody::Msg { from: me.into(), to: to.clone(), text });
                self.pulse();
                self.deliver(&mut st, &to);
                ToolResult::ok(format!("Delivered to {to}."))
            }
            "inbox" => {
                let mut st = self.lock();
                let msgs: Vec<Msg> = st.rec(me).unwrap().inbox.drain(..).collect();
                ToolResult::ok(if msgs.is_empty() { "No messages.".into() } else { msgs.iter().map(|m| format!("{}: {}", m.from, m.text)).collect::<Vec<_>>().join("\n") })
            }
            "status" => {
                let st = self.lock();
                ToolResult::ok(
                    st.agents
                        .iter()
                        .filter(|r| r.status != AgentStatus::Proposed)
                        .map(|r| {
                            let by = r.info.parent.as_ref().map(|p| format!(" · started by {p}")).unwrap_or_default();
                            format!("{} · {} · {} · {} · {}{by}", r.info.id, r.info.kind, r.info.role, status_word(r.status), r.info.task)
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
            }
            _ => ToolResult::err(format!("Unknown tool {name}")),
        }
    }

    async fn propose_split(self: &Arc<Self>, me: &str, is_lead: bool, args: &Value) -> ToolResult {
        if !is_lead {
            return ToolResult::err("Only the lead can propose a split. Use spawn_sub to split your own part.");
        }
        let items: Vec<SplitItem> = serde_json::from_value(args.get("items").cloned().unwrap_or(Value::Null)).unwrap_or_default();
        if items.is_empty() {
            return ToolResult::err("items must not be empty");
        }
        let (ids, ask, planning) = {
            let mut st = self.lock();
            if let Some(bad) = items.iter().find(|i| !st.cfg.team.contains(&i.agent)) {
                return ToolResult::err(format!("{} is not on this team (team: {})", bad.agent, st.cfg.team.join(", ")));
            }
            if st.live() + items.len() > st.cfg.max_agents as usize {
                return ToolResult::err(format!(
                    "That would exceed the limit of {} agents ({} already). Propose fewer parts.",
                    st.cfg.max_agents,
                    st.live()
                ));
            }
            let mut ids = vec![];
            for i in &items {
                let id = st.new_id(None, &i.agent);
                st.add_agent(
                    AgentInfo { id: id.clone(), kind: i.agent.clone(), role: i.role.clone(), task: i.task.clone(), parent: Some(me.into()), depth: 1 },
                    AgentStatus::Proposed,
                );
                ids.push(id);
            }
            for (k, i) in items.iter().enumerate() {
                let deps: Vec<String> =
                    i.after.iter().flatten().filter_map(|n| usize::try_from(*n - 1).ok().and_then(|n| ids.get(n)).cloned()).collect();
                st.rec(&ids[k]).unwrap().deps = deps;
            }
            let planning = st.cfg.permission == Permission::Plan;
            (ids, st.cfg.ask.split || planning, planning)
        };

        if ask {
            self.lock().set_status(me, AgentStatus::Waiting, None);
            let summary = args.get("summary").and_then(Value::as_str).map(String::from);
            let shown = items.iter().zip(&ids).map(|(i, id)| SplitItem { id: id.clone(), ..i.clone() }).collect();
            let req = AskReq { summary, items: Some(shown), ..AskReq::new(ApprovalKind::Split, me, "Proposed split", None) };
            let (a, note) = self.ask(req).await;
            let mut st = self.lock();
            if !st.ended {
                st.set_status(me, AgentStatus::Running, None);
            }
            if a != Answer::Yes {
                let why = if a == Answer::No { "rejected" } else { "replaced" };
                for id in &ids {
                    st.set_status(id, AgentStatus::Killed, Some(why.into()));
                }
                // Give the numbers back so the next proposal starts at #1 again.
                st.agents.retain(|r| !ids.contains(&r.info.id));
                if let Some(lead) = st.rec(me) {
                    lead.children.retain(|c| !ids.contains(c));
                }
                st.team_counter -= ids.len() as u32;
                let note = note.unwrap_or_default();
                return if a == Answer::No {
                    ToolResult::ok(format!("Rejected by the user{}. Do not start the work.", if note.is_empty() { String::new() } else { format!(": {note}") }))
                } else {
                    ToolResult::ok(format!(
                        "Changes requested by the user: {}. Call propose_split again with a new split.",
                        if note.is_empty() { "(no details)" } else { &note }
                    ))
                };
            }
            if planning {
                let to = st.cfg.after_plan;
                st.set_permission(to, "plan approved");
            }
        }
        let mut st = self.lock();
        let (mut started, mut queued) = (vec![], vec![]);
        for id in &ids {
            let rec = st.rec(id).unwrap();
            let (role, task, deps) = (rec.info.role.clone(), rec.info.task.clone(), rec.deps.clone());
            let q = !deps.is_empty();
            st.emit(EventBody::Assign { by: me.into(), id: id.clone(), role: role.clone(), task, queued: Some(q) });
            if q {
                st.set_status(id, AgentStatus::Queued, None);
                queued.push(format!("{id} ({role}, after {})", deps.join(", ")));
            } else {
                let p = st.first_prompt_for(id);
                self.start_turn(&mut st, id, p);
                started.push(format!("{id} ({role})"));
            }
        }
        ToolResult::ok(format!(
            "Approved. Started: {}.{} Call wait to collect their reports.",
            if started.is_empty() { "none".into() } else { started.join(", ") },
            if queued.is_empty() { String::new() } else { format!(" Queued: {}.", queued.join(", ")) }
        ))
    }

    async fn wait(self: &Arc<Self>, me: &str, args: &Value) -> ToolResult {
        let ids: Vec<String> = {
            let mut st = self.lock();
            let asked: Vec<String> = args.get("ids").and_then(Value::as_array).map(|a| a.iter().map(|v| v.as_str().unwrap_or("").to_string()).collect()).unwrap_or_default();
            let ids = if asked.is_empty() { st.rec(me).unwrap().children.clone() } else { asked };
            let unknown: Vec<&String> = ids.iter().filter(|i| st.idx(i).is_none()).collect();
            if !unknown.is_empty() {
                return ToolResult::err(format!("Unknown agent(s): {}", unknown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
            }
            if ids.is_empty() && st.rec(me).unwrap().inbox.is_empty() {
                return ToolResult::ok("Nothing to wait for: you have not started any agents.");
            }
            ids
        };
        let targets = |st: &State| -> Vec<(String, String, AgentStatus, Option<String>)> {
            ids.iter()
                .filter_map(|i| st.agents.iter().find(|r| &r.info.id == i))
                .filter(|r| r.status != AgentStatus::Killed || r.report.is_some())
                .map(|r| (r.info.id.clone(), r.info.role.clone(), r.status, r.report.clone()))
                .collect()
        };
        let blocked = |st: &mut State| {
            let pending = targets(st).iter().any(|t| !t.2.finished());
            let rec = st.rec(me).unwrap();
            pending && rec.inbox.is_empty() && !rec.abort.is_cancelled()
        };
        let deadline = tokio::time::Instant::now() + Duration::from_millis(WAIT_MS);
        {
            let mut st = self.lock();
            if blocked(&mut st) {
                st.set_status(me, AgentStatus::Waiting, None);
            }
        }
        loop {
            let notified = self.pulse.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut st = self.lock();
                if st.ended || !blocked(&mut st) || tokio::time::Instant::now() >= deadline {
                    break;
                }
            }
            let _ = tokio::time::timeout_at(deadline, notified).await;
        }
        let mut st = self.lock();
        let aborted = st.rec(me).unwrap().abort.is_cancelled();
        if !st.ended && !aborted && st.status_of(me) == Some(AgentStatus::Waiting) {
            st.set_status(me, AgentStatus::Running, None);
        }
        let mut parts: Vec<String> = vec![];
        let ts = targets(&st);
        for (id, role, status, report) in &ts {
            if status.finished() {
                parts.push(format!("## {id} ({role}) {}\n{}", status_word(*status), report.clone().unwrap_or_default()));
            }
        }
        let msgs: Vec<Msg> = st.rec(me).unwrap().inbox.drain(..).collect();
        if !msgs.is_empty() {
            parts.push(format!("## Messages\n{}", msgs.iter().map(|m| format!("{}: {}", m.from, m.text)).collect::<Vec<_>>().join("\n")));
        }
        let still: Vec<String> = ts.iter().filter(|t| !t.2.finished()).map(|t| format!("{} ({})", t.0, status_word(t.2))).collect();
        parts.push(if still.is_empty() { "All of them have finished.".into() } else { format!("Still running: {}. Call wait again.", still.join(", ")) });
        ToolResult::ok(parts.join("\n\n"))
    }

    // ── what you type ───────────────────────────────────────────────────────
    /// Plain text goes to the lead, `@agent text` to one agent, `/commands`, and y / n / e for the oldest approval.
    /// Returns an error to show under the prompt.
    pub fn input(self: &Arc<Self>, raw: &str) -> Option<String> {
        let text = raw.trim();
        if text.is_empty() {
            return None;
        }
        let mut st = self.lock();
        if st.ended {
            return Some("This session has ended. Start a new one.".into());
        }

        if !st.approvals.is_empty() {
            let word: String = text.chars().take_while(|c| c.is_ascii_alphabetic()).collect::<String>().to_lowercase();
            let boundary = !text[word.len()..].starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_');
            if boundary && ["y", "yes", "n", "no", "e", "edit"].contains(&word.as_str()) {
                let mut words = text.split_whitespace();
                let first = words.next().unwrap_or("").to_lowercase();
                let rest = words.collect::<Vec<_>>().join(" ");
                let a = if first.starts_with('y') {
                    Answer::Yes
                } else if first.starts_with('n') {
                    Answer::No
                } else {
                    Answer::Edit
                };
                st.emit(EventBody::User { to: None, text: text.into() });
                drop(st);
                self.answer(None, a, Some(rest).filter(|r| !r.is_empty()));
                return None;
            }
        }

        if let Some(cmdline) = text.strip_prefix('/') {
            let mut parts = cmdline.split_whitespace();
            let cmd = parts.next().unwrap_or("");
            let arg = parts.collect::<Vec<_>>().join(" ");
            match cmd {
                "help" => st.log(
                    Level::Info,
                    "/orc <agent kind> · /mode auto|edits|plan · @agent text · /kill <agent> · /budget <usd> · /merge · /stop · y / n / e <changes> to answer an approval",
                ),
                "stop" => self.stop_locked(&mut st, "Stopped by you", SessionStatus::Stopped),
                "kill" => {
                    drop(st);
                    return (!self.kill(&arg, "you")).then(|| format!("No agent called {arg}"));
                }
                "budget" => {
                    let a = arg.replace('$', "");
                    let n: f64 = if a.trim().is_empty() { 0.0 } else { a.trim().parse().unwrap_or(f64::NAN) };
                    if !(n >= 0.0) {
                        return Some("Usage: /budget 10".into());
                    }
                    st.cfg.budget_usd = n;
                    st.log(Level::Info, format!("Budget set to ${}", js_num(n)));
                }
                "merge" => {
                    let me = self.clone();
                    tokio::spawn(async move { me.offer_merge(true).await });
                }
                "mode" => {
                    let key: String = arg.to_lowercase().chars().filter(|c| !matches!(c, ' ' | '_' | '-')).collect();
                    let m = match key.as_str() {
                        "auto" => Permission::Auto,
                        "edits" | "accept" | "acceptedits" => Permission::AcceptEdits,
                        "plan" => Permission::Plan,
                        _ => return Some("Usage: /mode auto | edits | plan".into()),
                    };
                    st.set_permission(m, "you");
                }
                "orc" | "lead" => return self.switch_lead(&mut st, &arg),
                _ => return Some(format!("Unknown command /{cmd}. Try /help.")),
            }
            return None;
        }

        let at = text.strip_prefix('@').and_then(|r| {
            let (who, body) = r.split_once(char::is_whitespace)?;
            let body = body.trim_start();
            (!who.is_empty() && !body.is_empty()).then(|| (who.to_string(), body.to_string()))
        });
        let (to, body) = match &at {
            Some((w, b)) => (if w == "lead" { st.lead_id.clone() } else { w.clone() }, b.clone()),
            None => (st.lead_id.clone(), text.to_string()),
        };
        if st.idx(&to).is_none() {
            return Some(format!("No agent called {to}"));
        }
        st.emit(EventBody::User { to: at.as_ref().map(|_| to.clone()), text: body.clone() });
        st.rec(&to).unwrap().inbox.push(Msg { from: "you".into(), text: body });
        self.pulse();
        self.deliver(&mut st, &to);
        None
    }

    fn switch_lead(self: &Arc<Self>, st: &mut State, kind: &str) -> Option<String> {
        let known = providers::known_kinds();
        if !known.iter().any(|k| k == kind) {
            return Some(format!("Usage: /orc {}", known.join(" | ")));
        }
        let old_id = st.lead_id.clone();
        let old = st.rec(&old_id).unwrap();
        if old.info.kind == kind {
            return Some(format!("{old_id} already leads."));
        }
        if old.busy {
            return Some(format!("{old_id} is in the middle of a turn. Wait for it, or /stop."));
        }
        let mut id = kind.to_string();
        let mut n = 2;
        while st.idx(&id).is_some() {
            id = format!("{kind}-lead{n}");
            n += 1;
        }
        let lines: Vec<String> = st
            .events
            .iter()
            .filter_map(|e| match &e.body {
                EventBody::User { text, .. } => Some(format!("user: {text}")),
                EventBody::Say { agent, text } if *agent == old_id => Some(format!("{agent}: {text}")),
                _ => None,
            })
            .collect();
        let history = lines[lines.len().saturating_sub(16)..].join("\n");
        let info = AgentInfo { id: id.clone(), kind: kind.into(), role: "orchestrator".into(), task: "Lead the session (took over)".into(), parent: None, depth: 0 };
        st.add_agent(info.clone(), AgentStatus::Idle);
        let first = handover_prompt(&st.cfg, &info, &history);
        st.rec(&id).unwrap().first_prompt = Some(first);
        st.set_status(&old_id, AgentStatus::Done, Some(format!("handed over to {id}")));
        st.lead_id = id.clone();
        st.emit(EventBody::LeadChanged { from: old_id, to: id });
        None
    }

    // ── bringing the work back ──────────────────────────────────────────────
    /// Last gate for every engine (Codex cannot ask mid-turn, a shell can write anything): protected files you have
    /// not said yes to are asked about one by one, and put back if you say no.
    async fn hold_protected(&self) -> Result<()> {
        let (wt, manifest, repo, workdir, lead) = {
            let st = self.lock();
            if st.cfg.protect.is_empty() || st.cfg.mode != Mode::Real {
                return Ok(());
            }
            (st.wt.clone(), st.manifest.clone(), PathBuf::from(&st.cfg.repo), st.workdir.clone(), st.lead_id.clone())
        };
        let files = match (&wt, &manifest) {
            (Some(w), _) => git::changed_files(&w.dir).await?,
            (None, Some(m)) => {
                let c = copy::changes(&repo, &workdir, m);
                [c.added, c.modified, c.deleted].concat()
            }
            _ => vec![],
        };
        for f in files {
            {
                let st = self.lock();
                if !st.is_protected(&f) || st.approved_files.contains(&f) {
                    continue;
                }
            }
            let (a, _) = self.ask(AskReq::new(ApprovalKind::File, &lead, format!("Keep the change to protected file {f}?"), Some(f.clone()))).await;
            if self.ended() {
                return Ok(());
            }
            if a == Answer::Yes {
                self.lock().approved_files.insert(f);
            } else {
                match (&wt, &manifest) {
                    (Some(w), _) => git::revert_file(&w.dir, &f).await?,
                    (None, Some(m)) => copy::revert(&repo, &workdir, &f, m),
                    _ => {}
                }
                self.lock().log(Level::Info, format!("Put {f} back as it was."));
            }
        }
        Ok(())
    }

    pub async fn offer_merge(self: &Arc<Self>, force: bool) {
        {
            let mut st = self.lock();
            if st.merging || st.ended {
                return;
            }
            if st.agents.iter().any(|r| r.busy || r.status == AgentStatus::Queued) {
                if force {
                    st.log(Level::Warn, "Agents are still working; merge after they finish.");
                }
                return;
            }
            st.drop_codex_skills(); // nobody is running; keep them out of the diff (re-added on the next Codex turn)
            if st.cfg.workspace == Workspace::Direct && st.cfg.mode == Mode::Real {
                if force {
                    let t = format!("Agents edit {} directly; there is nothing to merge.", st.cfg.repo);
                    st.log(Level::Info, t);
                }
                return;
            }
            st.merging = true;
        }
        if let Err(e) = self.hold_protected().await {
            self.lock().log(Level::Error, format!("Could not check protected files, nothing merged: {e:#}"));
        } else if let Err(e) = self.merge(force).await {
            self.lock().log(Level::Error, format!("{e:#}"));
        }
        let mut st = self.lock();
        st.merging = false;
        st.recompute();
    }

    async fn merge(self: &Arc<Self>, force: bool) -> Result<()> {
        let (wt, manifest, repo, workdir, ask, lead, task) = {
            let st = self.lock();
            (st.wt.clone(), st.manifest.clone(), st.cfg.repo.clone(), st.workdir.clone(), st.cfg.ask.merge, st.lead_id.clone(), st.task.clone())
        };
        let log = |level: Level, t: String| self.lock().log(level, t);
        let decide = |title: String, detail: String| {
            let lead = lead.clone();
            async move { !ask || self.ask(AskReq::new(ApprovalKind::Merge, &lead, title, Some(detail))).await.0 == Answer::Yes }
        };
        if let Some(wt) = wt {
            let d = git::diff_worktree(&wt.dir).await?;
            if d.files_changed == 0 {
                if force {
                    log(Level::Info, "Nothing to merge.".into());
                }
                return Ok(());
            }
            let list = d.patch.lines().filter_map(|l| l.strip_prefix("+++ b/")).take(30).collect::<Vec<_>>().join("\n");
            let yes = decide(format!("Merge {} files (+{} −{}) into your current branch?", d.files_changed, d.insertions, d.deletions), list).await;
            let first: String = task.lines().next().unwrap_or("").chars().take(70).collect();
            git::commit_all(&wt.dir, &format!("agent-duo: {first}")).await?;
            if !yes {
                log(Level::Info, format!("Committed on {}. Not merged; /merge when ready.", wt.branch));
                return Ok(());
            }
            match git::merge_branch(&wt.root, &wt.branch).await {
                Ok(()) => log(Level::Info, format!("Merged {} into your current branch.", wt.branch)),
                Err(e) => log(Level::Error, format!("Merge failed, work kept on {}: {e:#}", wt.branch)),
            }
        } else if let Some(m) = manifest {
            let repo = PathBuf::from(repo);
            let c = copy::changes(&repo, &workdir, &m);
            if c.file_count() == 0 {
                if !c.conflicts.is_empty() {
                    log(Level::Error, format!("{} file(s) changed both here and in your folder; nothing applied:\n{}", c.conflicts.len(), c.describe()));
                } else if force {
                    log(Level::Info, "Nothing to apply.".into());
                }
                return Ok(());
            }
            let n = c.file_count();
            let yes = decide(format!("Apply {n} files (+{} −{}) to {}?", c.insertions, c.deletions, repo.display()), c.describe()).await;
            if !yes {
                log(Level::Info, format!("Kept in the session copy ({}). /merge to apply later.", workdir.display()));
                return Ok(());
            }
            {
                let mut st = self.lock();
                if let Some(m) = st.manifest.as_mut() {
                    copy::apply(&repo, &workdir, &c, m)?;
                }
            }
            let skipped =
                if c.conflicts.is_empty() { String::new() } else { format!(" Skipped {} you changed too: {}", c.conflicts.len(), c.conflicts.join(", ")) };
            log(Level::Info, format!("Applied {n} files to {}.{skipped}", repo.display()));
        } else {
            // mock: pretend there are changes, once
            let nothing = {
                let mut st = self.lock();
                let edited = st.events.iter().any(|e| matches!(&e.body, EventBody::Tool { verb, .. } if verb == "add" || verb == "edit"));
                let nothing = st.mock_merged || !edited;
                st.mock_merged |= !nothing;
                nothing
            };
            if nothing {
                if force {
                    log(Level::Info, "Nothing to merge.".into());
                }
                return Ok(());
            }
            let yes = decide(
                "Merge 4 files (+81 −6) into main (mock)?".into(),
                "M  src/api/users.ts\nA  src/lib/cache.ts\nA  tests/cache.test.ts\nM  src/api/index.ts".into(),
            )
            .await;
            log(Level::Info, if yes { "Merged into main (mock, no files changed)." } else { "Kept on the session branch (mock)." }.into());
        }
        Ok(())
    }
}

fn status_word(s: AgentStatus) -> String {
    serde_json::to_value(s).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
}

/// A number as JavaScript prints it in a template string (5, not 5.0).
fn js_num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 { format!("{}", n as i64) } else { format!("{n}") }
}
