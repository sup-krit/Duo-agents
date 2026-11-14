//! Other AI providers (DeepSeek, local Ollama, …) that join the team as their own agent kind.
//! Keys live only in ~/.agent-duo/providers.json on this machine (DUO_HOME to move it):
//! never in SessionConfig (that goes to events.jsonl), never sent to the browser in full.

use crate::types::{Engine, Provider};
use serde_json::Value;
use std::path::PathBuf;

pub const BUILTIN: [&str; 2] = ["claude", "codex"];

fn file() -> PathBuf {
    std::env::var_os("DUO_HOME").map(PathBuf::from).unwrap_or_else(|| crate::home_dir().join(".agent-duo")).join("providers.json")
}

pub fn list() -> Vec<Provider> {
    std::fs::read_to_string(file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn get(id: &str) -> Option<Provider> {
    list().into_iter().find(|p| p.id == id)
}

fn save(list: &[Provider]) -> std::io::Result<()> {
    let f = file();
    std::fs::create_dir_all(f.parent().unwrap())?;
    std::fs::write(&f, serde_json::to_string_pretty(list)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600))?;
    } // Windows: the ACLs of the user profile apply
    Ok(())
}

pub fn mask(k: &str) -> String {
    let n = k.chars().count();
    if n == 0 {
        String::new()
    } else if n <= 8 {
        "•".repeat(n)
    } else {
        format!("{}…{}", k.chars().take(3).collect::<String>(), k.chars().skip(n - 4).collect::<String>())
    }
}

/// What the browser sees.
pub fn to_public(p: &Provider) -> Provider {
    Provider { api_key: mask(&p.api_key), has_key: Some(!p.api_key.is_empty()), ..p.clone() }
}

pub fn list_public() -> Vec<Provider> {
    list().iter().map(to_public).collect()
}

fn s(v: &Value, k: &str) -> String {
    match &v[k] {
        Value::String(s) => s.trim().to_string(),
        Value::Null => String::new(),
        o => o.to_string(),
    }
}

fn base_url(v: &Value) -> Result<String, String> {
    let u = s(v, "baseUrl");
    let rest = u.strip_prefix("https://").or_else(|| u.strip_prefix("http://"));
    match rest {
        Some(r) if !r.is_empty() && !r.starts_with('/') && !r.contains(char::is_whitespace) => Ok(u.trim_end_matches('/').to_string()),
        _ if u.contains("://") => Err("base URL must be http or https".into()),
        _ => Err("base URL is not a valid URL".into()),
    }
}

fn rate(v: &Value, k: &str) -> Option<f64> {
    let n = v[k].as_f64().or_else(|| v[k].as_str()?.trim().parse().ok())?;
    (n > 0.0).then_some(n)
}

/// The same checks as a save, without saving: for testing what is in the form.
/// An empty key means the stored one, but only for the same base URL, so a changed URL never receives the old key.
pub fn draft(v: &Value) -> Result<Provider, String> {
    let id = s(v, "id").to_lowercase();
    let url = base_url(v)?;
    let model = s(v, "model");
    if model.is_empty() {
        return Err("model is required".into());
    }
    let old = get(&id);
    let key = s(v, "apiKey");
    Ok(Provider {
        label: Some(s(v, "label")).filter(|l| !l.is_empty()).unwrap_or_else(|| id.clone()),
        engine: if s(v, "engine") == "codex" { Engine::Codex } else { Engine::Claude },
        api_key: if key.is_empty() { old.filter(|o| o.base_url == url).map(|o| o.api_key).unwrap_or_default() } else { key },
        base_url: url,
        model,
        usd_per_m_tok_in: rate(v, "usdPerMTokIn"),
        usd_per_m_tok_out: rate(v, "usdPerMTokOut"),
        has_key: None,
        id,
    })
}

/// Validate and store. Returns an error message for the form.
pub fn upsert(v: &Value) -> Result<(), String> {
    let id = s(v, "id").to_lowercase();
    let ok_id = id.len() <= 24
        && id.starts_with(|c: char| c.is_ascii_lowercase())
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !ok_id {
        return Err("id: a-z, 0-9 and -, starting with a letter".into());
    }
    if BUILTIN.contains(&id.as_str()) {
        return Err(format!("\"{id}\" is built in; pick another id"));
    }
    let p = draft(v)?;
    let mut all: Vec<Provider> = list().into_iter().filter(|x| x.id != id).collect();
    all.push(p);
    save(&all).map_err(|e| e.to_string())
}

pub fn delete(id: &str) -> std::io::Result<()> {
    save(&list().into_iter().filter(|p| p.id != id).collect::<Vec<_>>())
}

pub fn engine_of(kind: &str) -> Engine {
    match kind {
        "codex" => Engine::Codex,
        "claude" => Engine::Claude,
        _ => get(kind).map(|p| p.engine).unwrap_or_default(),
    }
}

pub fn known_kinds() -> Vec<String> {
    BUILTIN.iter().map(|s| s.to_string()).chain(list().into_iter().map(|p| p.id)).collect()
}

pub fn cost(p: &Provider, tokens_in: u64, tokens_out: u64) -> f64 {
    (tokens_in as f64 * p.usd_per_m_tok_in.unwrap_or(0.0) + tokens_out as f64 * p.usd_per_m_tok_out.unwrap_or(0.0)) / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn masks_keys() {
        assert_eq!(mask("sk-1234567890abcd"), "sk-…abcd");
        assert_eq!(mask("short"), "•••••");
        assert_eq!(mask(""), "");
    }
    #[test]
    fn checks_urls() {
        let v = |u: &str| serde_json::json!({ "id": "x", "baseUrl": u, "model": "m" });
        assert_eq!(draft(&v("https://api.deepseek.com/anthropic/")).unwrap().base_url, "https://api.deepseek.com/anthropic");
        assert!(draft(&v("ftp://x")).is_err());
        assert!(draft(&v("nope")).is_err());
    }
}
