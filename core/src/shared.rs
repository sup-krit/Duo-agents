//! agent-duo's own shared/skills and shared/agents, offered to every agent.
//! Claude loads the folder as a plugin. Codex only reads <cwd>/.agents/skills, so the skills are copied
//! there while Codex agents run and removed again before anything is diffed, merged or the session ends.
//! shared/agents (Claude's .md format) become Codex agent roles: [agents.<name>] with a config_file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// ./shared when you run from a checkout, otherwise %APPDATA%\agent-duo\shared.
pub fn dir() -> PathBuf {
    let here = PathBuf::from("shared");
    if here.is_dir() { std::path::absolute(here).unwrap_or_default() } else { crate::data_dir().join("shared") }
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let dest = to.join(e.file_name());
        if e.file_type()?.is_dir() { copy_dir(&e.path(), &dest)? } else { std::fs::copy(e.path(), dest).map(|_| ())? }
    }
    Ok(())
}

/// Copy shared skills into `workdir`/.agents/skills, skipping names the repo already has. Returns what was added.
pub fn add_codex_skills(workdir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir().join("skills")) else { return vec![] };
    let mut added = vec![];
    for d in rd.flatten().filter(|d| d.path().is_dir()) {
        let dest = workdir.join(".agents").join("skills").join(d.file_name());
        if dest.exists() {
            continue; // the repo's own skill wins
        }
        if copy_dir(&d.path(), &dest).is_ok() {
            added.push(dest);
        }
    }
    added
}

/// Undo add_codex_skills, including the folders it had to create.
pub fn remove_codex_skills(workdir: &Path, added: &[PathBuf]) {
    for p in added {
        let _ = std::fs::remove_dir_all(p);
    }
    for d in [workdir.join(".agents").join("skills"), workdir.join(".agents")] {
        if std::fs::read_dir(&d).is_ok_and(|mut r| r.next().is_none()) {
            let _ = std::fs::remove_dir(&d);
        }
    }
}

/// shared/agents/*.md as Codex `agents` config (name → description, config_file).
/// Claude-only fields (tools, model) are dropped; the role runs with the parent's sandbox.
pub fn codex_agents(sandbox: &str) -> BTreeMap<String, (String, PathBuf)> {
    let mut roles = BTreeMap::new();
    let Ok(rd) = std::fs::read_dir(dir().join("agents")) else { return roles };
    let out = std::env::temp_dir().join("agent-duo-codex-agents").join(sandbox);
    let _ = std::fs::create_dir_all(&out);
    for f in rd.flatten() {
        let path = f.path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let (meta, body) = frontmatter(&text);
        let name = meta.get("name").cloned().unwrap_or_else(|| path.file_stem().unwrap().to_string_lossy().into_owned());
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') || body.trim().is_empty() {
            continue;
        }
        let file = out.join(format!("{name}.toml"));
        // JSON strings are valid TOML basic strings
        let toml = format!(
            "developer_instructions = {}\nsandbox_mode = {}\n",
            serde_json::to_string(body.trim()).unwrap(),
            serde_json::to_string(sandbox).unwrap()
        );
        if std::fs::write(&file, toml).is_ok() {
            roles.insert(name.clone(), (meta.get("description").cloned().unwrap_or(name), file));
        }
    }
    roles
}

// ponytail: flat `key: value` lines only; multi-line YAML values (| or >) are not read.
fn frontmatter(text: &str) -> (BTreeMap<String, String>, String) {
    let mut meta = BTreeMap::new();
    let t = text.replace("\r\n", "\n");
    let Some(rest) = t.strip_prefix("---\n") else { return (meta, text.to_string()) };
    let Some(end) = rest.find("\n---") else { return (meta, text.to_string()) };
    for line in rest[..end].lines() {
        if let Some((k, v)) = line.split_once(':') {
            if !k.is_empty() && k.chars().all(|c| c.is_alphanumeric() || c == '_') {
                let v = v.trim();
                let v = v.strip_prefix('"').and_then(|x| x.strip_suffix('"')).or_else(|| v.strip_prefix('\'').and_then(|x| x.strip_suffix('\''))).unwrap_or(v);
                meta.insert(k.to_string(), v.to_string());
            }
        }
    }
    let body = rest[end + 4..].strip_prefix('\n').unwrap_or(&rest[end + 4..]).to_string();
    (meta, body)
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_frontmatter() {
        let (m, b) = super::frontmatter("---\nname: rev\ndescription: \"Reviews\"\n---\nBe strict.\n");
        assert_eq!(m["name"], "rev");
        assert_eq!(m["description"], "Reviews");
        assert_eq!(b, "Be strict.\n");
    }
}
