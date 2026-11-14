//! The same endpoints as the Next.js version: sessions, SSE events, input, approvals, the bridge's
//! tool calls (plus Claude's permission gate) and the other-AI providers. The page itself is embedded.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use duo_core::providers;
use duo_core::session::{create_session, get_session, list_sessions, read_events};
use duo_core::types::*;
use futures_util::stream;
use tokio_stream::StreamExt;
use indexmap::IndexMap;
use serde_json::{Value, json};
use std::convert::Infallible;
use std::time::Duration;

#[derive(Clone)]
pub struct App {
    /// Where agents' bridges call back: http://127.0.0.1:<port>
    pub base_url: String,
}

#[derive(rust_embed::RustEmbed)]
#[folder = "../web/dist"]
struct Web;

pub fn router(app: App) -> Router {
    Router::new()
        .route("/api/sessions", get(list).post(create))
        .route("/api/sessions/{id}/events", get(events))
        .route("/api/sessions/{id}/input", post(input))
        .route("/api/sessions/{id}/approvals", post(approvals))
        .route("/api/sessions/{id}/tool", post(tool))
        .route("/api/sessions/{id}/gate", post(gate))
        .route("/api/providers", get(providers_list).put(providers_put).delete(providers_delete))
        .route("/api/providers/test", post(providers_test))
        .fallback(page)
        .with_state(app)
}

fn err(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

/// Only this app's own pages may use the API: the Host is this machine (no DNS rebinding), a browser
/// Origin is the same host, and changes come as JSON (other sites cannot send that without a CORS
/// preflight, which is never answered). The bridge's calls carry the session token instead.
fn guard(method: &Method, h: &HeaderMap) -> Option<Response> {
    let host = h.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    let name = host.rsplit_once(':').map_or(host, |(n, _)| n);
    if !matches!(name, "127.0.0.1" | "localhost" | "[::1]") {
        return Some(err(StatusCode::FORBIDDEN, "Forbidden"));
    }
    if let Some(o) = h.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        if o.split_once("://").map(|(_, r)| r) != Some(host) {
            return Some(err(StatusCode::FORBIDDEN, "Forbidden"));
        }
    }
    let json = h.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|c| c.contains("application/json"));
    if method != Method::GET && !json {
        return Some(err(StatusCode::UNSUPPORTED_MEDIA_TYPE, "Send JSON"));
    }
    None
}

fn body(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or(json!({}))
}

async fn list(method: Method, h: HeaderMap) -> Response {
    guard(&method, &h).unwrap_or_else(|| Json(list_sessions()).into_response())
}

/// `Number(v ?? d) || d`, clamped, like the TypeScript route (so 0 also means the default).
fn num(v: &Value, d: f64, lo: f64, hi: f64) -> f64 {
    let n = match v {
        Value::Null => d,
        Value::Number(n) => n.as_f64().unwrap_or(d),
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::String(s) if s.trim().is_empty() => 0.0,
        Value::String(s) => s.trim().parse().unwrap_or(f64::NAN),
        _ => f64::NAN,
    };
    let n = if n == 0.0 || n.is_nan() { d } else { n };
    n.max(lo).min(hi)
}

fn str_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}

async fn create(State(app): State<App>, method: Method, h: HeaderMap, raw: String) -> Response {
    if let Some(r) = guard(&method, &h) {
        return r;
    }
    let b = body(&raw);
    let task = str_of(&b["task"]).trim().to_string();
    let mode = if b["mode"] == "real" { Mode::Real } else { Mode::Mock };
    let repo = str_of(&b["repo"]).trim().to_string();
    if task.is_empty() {
        return err(StatusCode::BAD_REQUEST, "Type a task first");
    }
    if mode == Mode::Real && repo.is_empty() {
        return err(StatusCode::BAD_REQUEST, "Project folder is required");
    }
    let known = providers::known_kinds();
    let kind = |v: &Value| v.as_str().filter(|k| known.iter().any(|x| x == k)).unwrap_or("claude").to_string();
    let team_in = b["team"].as_array().cloned().unwrap_or_else(|| vec![json!("codex"), json!("claude")]);
    let mut team: Vec<String> = vec![];
    for k in team_in.iter().map(kind) {
        if !team.contains(&k) {
            team.push(k);
        }
    }
    if team.is_empty() {
        return err(StatusCode::BAD_REQUEST, "Pick at least one team agent");
    }
    let perm = |v: &Value| match v.as_str() {
        Some("acceptEdits") => Permission::AcceptEdits,
        Some("plan") => Permission::Plan,
        _ => Permission::Auto,
    };
    let mut protect: Vec<String> = vec![];
    for p in b["protect"].as_array().into_iter().flatten() {
        let p = str_of(p).trim().replace('\\', "/");
        let p = p.strip_prefix("./").unwrap_or(&p).to_string();
        if !p.is_empty() && !p.starts_with('#') && !protect.contains(&p) && protect.len() < 100 {
            protect.push(p);
        }
    }
    let lead = kind(&b["lead"]);
    let roles: IndexMap<String, String> = team
        .iter()
        .map(|k| (k.clone(), str_of(&b["roles"][k]).trim().chars().take(500).collect::<String>()))
        .filter(|(_, v)| !v.is_empty())
        .collect();
    // Display names only; keys stay in ~/.agent-duo/providers.json.
    let labels: IndexMap<String, String> =
        std::iter::once(&lead).chain(&team).map(|k| (k.clone(), providers::get(k).map(|p| p.label).unwrap_or_else(|| k.clone()))).collect();
    let cfg = SessionConfig {
        lead,
        workspace: if b["workspace"] == "direct" { Workspace::Direct } else { Workspace::Auto },
        team,
        repo: if mode == Mode::Mock { "(mock)".into() } else { repo },
        mode,
        max_depth: if b["subAgents"] == false { 1 } else { num(&b["maxDepth"], 2.0, 1.0, 4.0) as u32 },
        max_agents: num(&b["maxAgents"], 8.0, 1.0, 20.0) as u32,
        budget_usd: num(&b["budgetUsd"], 5.0, 0.0, 1000.0),
        ask: Ask { split: b["ask"]["split"] != false, merge: b["ask"]["merge"] != false, spawn: b["ask"]["spawn"].as_bool().unwrap_or(false) },
        permission: perm(&b["permission"]),
        after_plan: if b["afterPlan"] == "acceptEdits" { Permission::AcceptEdits } else { Permission::Auto },
        protect,
        roles: Some(roles),
        labels: Some(labels),
    };
    match create_session(cfg, task, app.base_url.clone(), 1.0).await {
        Ok(s) => Json(json!({ "id": s.id })).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

/// Server-Sent Events: everything so far, then live events.
async fn events(Path(id): Path<String>, method: Method, h: HeaderMap) -> Response {
    if let Some(r) = guard(&method, &h) {
        return r;
    }
    let data = |e: &DuoEvent| Ok::<_, Infallible>(Event::default().data(serde_json::to_string(e).unwrap_or_default()));
    if let Some(s) = get_session(&id) {
        let (past, rx) = s.subscribe();
        // A lagging browser loses the stream and reconnects; the replay skips what it already has.
        let live = tokio_stream::wrappers::BroadcastStream::new(rx).map_while(|r| r.ok());
        let all = stream::iter(past).chain(live).map(move |e| data(&e));
        return Sse::new(all).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)).text("ping")).into_response();
    }
    let Some(past) = read_events(&id) else {
        return (StatusCode::NOT_FOUND, "Session not found").into_response();
    };
    let end = Ok::<_, Infallible>(Event::default().event("end").data("{}"));
    Sse::new(stream::iter(past.iter().map(data).collect::<Vec<_>>()).chain(stream::once(async { end }))).into_response()
}

fn gone() -> Response {
    err(StatusCode::NOT_FOUND, "This session is no longer running")
}

/// Everything typed in the prompt: plain text to the lead, @agent text, /commands, y/n/e for approvals.
async fn input(Path(id): Path<String>, method: Method, h: HeaderMap, raw: String) -> Response {
    if let Some(r) = guard(&method, &h) {
        return r;
    }
    let Some(s) = get_session(&id) else { return gone() };
    match s.input(&str_of(&body(&raw)["text"])) {
        Some(e) => err(StatusCode::BAD_REQUEST, e),
        None => Json(json!({ "ok": true })).into_response(),
    }
}

async fn approvals(Path(id): Path<String>, method: Method, h: HeaderMap, raw: String) -> Response {
    if let Some(r) = guard(&method, &h) {
        return r;
    }
    let Some(s) = get_session(&id) else { return gone() };
    let b = body(&raw);
    let answer = match b["answer"].as_str() {
        Some("yes") => Answer::Yes,
        Some("no") => Answer::No,
        Some("edit") => Answer::Edit,
        _ => return err(StatusCode::BAD_REQUEST, "answer must be yes, no or edit"),
    };
    let note = Some(str_of(&b["note"])).filter(|n| !n.is_empty());
    if !s.answer(Some(&str_of(&b["approvalId"])), answer, note) {
        return err(StatusCode::CONFLICT, "No such pending approval");
    }
    Json(json!({ "ok": true })).into_response()
}

/// The bridge's calls carry the session token; compared in constant time.
fn token_ok(s: &duo_core::session::Session, t: &Value) -> bool {
    let (a, b) = (str_of(t), &s.token);
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Called by `agent-duo bridge` when an agent uses a duo tool. `wait` and `propose_split` can block.
async fn tool(Path(id): Path<String>, raw: String) -> Response {
    let Some(s) = get_session(&id) else {
        return (StatusCode::NOT_FOUND, Json(json!({ "text": "Session not found", "isError": true }))).into_response();
    };
    let b = body(&raw);
    if !token_ok(&s, &b["token"]) {
        return (StatusCode::FORBIDDEN, Json(json!({ "text": "Bad token", "isError": true }))).into_response();
    }
    let r = s.handle_tool(&str_of(&b["agent"]), &str_of(&b["name"]), b["args"].clone()).await;
    Json(json!({ "text": r.text, "isError": r.is_error })).into_response()
}

/// Called by `agent-duo hook` before Claude edits a file or runs a command.
async fn gate(Path(id): Path<String>, raw: String) -> Response {
    let Some(s) = get_session(&id) else { return Json(json!({ "decision": "deny", "reason": "This session is no longer running." })).into_response() };
    let b = body(&raw);
    if !token_ok(&s, &b["token"]) {
        return (StatusCode::FORBIDDEN, Json(json!({ "decision": "deny", "reason": "Bad token" }))).into_response();
    }
    let d = duo_core::runners::claude::gate(&s, &str_of(&b["agent"]), &str_of(&b["tool"]), &b["input"]).await;
    Json(match d {
        Some((ok, reason)) => json!({ "decision": if ok { "allow" } else { "deny" }, "reason": reason }),
        None => json!({}),
    })
    .into_response()
}

async fn providers_list(method: Method, h: HeaderMap) -> Response {
    guard(&method, &h).unwrap_or_else(|| Json(providers::list_public()).into_response())
}

async fn providers_put(method: Method, h: HeaderMap, raw: String) -> Response {
    if let Some(r) = guard(&method, &h) {
        return r;
    }
    match providers::upsert(&body(&raw)) {
        Ok(()) => Json(providers::list_public()).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, e),
    }
}

async fn providers_delete(method: Method, h: HeaderMap, raw: String) -> Response {
    if let Some(r) = guard(&method, &h) {
        return r;
    }
    let _ = providers::delete(&str_of(&body(&raw)["id"]));
    Json(providers::list_public()).into_response()
}

/// Tests what is in the form with the smallest real request; an empty key uses the stored one (same base URL only).
async fn providers_test(method: Method, h: HeaderMap, raw: String) -> Response {
    if let Some(r) = guard(&method, &h) {
        return r;
    }
    let p = match providers::draft(&body(&raw)) {
        Ok(p) => p,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({ "ok": false, "error": e }))).into_response(),
    };
    let claude = p.engine == Engine::Claude;
    let (url, payload) = if claude {
        (format!("{}/v1/messages", p.base_url), json!({ "model": p.model, "max_tokens": 1, "messages": [{ "role": "user", "content": "hi" }] }))
    } else {
        (format!("{}/responses", p.base_url), json!({ "model": p.model, "max_output_tokens": 16, "input": "hi" }))
    };
    let mut req = reqwest::Client::new().post(url).timeout(Duration::from_secs(30)).json(&payload);
    if claude {
        req = req.header("anthropic-version", "2023-06-01");
        if !p.api_key.is_empty() {
            req = req.header("x-api-key", &p.api_key);
        }
    }
    if !p.api_key.is_empty() {
        req = req.bearer_auth(&p.api_key);
    }
    let out = match req.send().await {
        Ok(r) if r.status().is_success() => json!({ "ok": true }),
        Ok(r) => {
            let status = r.status();
            let text: String = r.text().await.unwrap_or_default().chars().take(300).collect();
            let text = if p.api_key.is_empty() { text } else { text.replace(&p.api_key, "***") };
            json!({ "ok": false, "error": format!("{} {}: {text}", status.as_u16(), status.canonical_reason().unwrap_or("")) })
        }
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    };
    Json(out).into_response()
}

/// The embedded page; any other path is the app (the browser router shows / or /s/:id).
async fn page(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.starts_with("api/") {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    let (file, f) = match Web::get(path).filter(|_| !path.is_empty()) {
        Some(f) => (path, f),
        None => match Web::get("index.html") {
            Some(f) => ("index.html", f),
            None => return (StatusCode::NOT_FOUND, "The page was not built: run `npm run build` in web/ first.").into_response(),
        },
    };
    let cache = if file.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    ([(header::CONTENT_TYPE, f.metadata.mimetype().to_string()), (header::CACHE_CONTROL, cache.to_string())], f.data).into_response()
}

/// Serve the app on a listener from `cli::bind` (the desktop app runs this on Tauri's runtime).
pub async fn serve(listener: tokio::net::TcpListener, app: Router) -> std::io::Result<()> {
    axum::serve(listener, app).await
}
