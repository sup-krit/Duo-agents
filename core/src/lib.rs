pub mod copy;
pub mod git;
pub mod notify;
pub mod prompts;
pub mod providers;
pub mod runners;
pub mod session;
pub mod shared;
pub mod types;

use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

static DATA: OnceLock<PathBuf> = OnceLock::new();

/// Where runs/ (and shared/ when there is none next to you) live: %APPDATA%\agent-duo on Windows,
/// ~/Library/Application Support/agent-duo on macOS. RUNS_DIR still overrides runs/ alone.
pub fn data_dir() -> &'static Path {
    DATA.get_or_init(|| {
        let base = if cfg!(windows) {
            std::env::var_os("APPDATA").map(PathBuf::from)
        } else if cfg!(target_os = "macos") {
            Some(home_dir().join("Library").join("Application Support"))
        } else {
            std::env::var_os("XDG_DATA_HOME").map(PathBuf::from)
        };
        base.unwrap_or_else(|| home_dir().join(".local").join("share")).join("agent-duo")
    })
}

/// Apps opened from Finder / the Dock get a bare PATH (/usr/bin:/bin…), so claude, codex and node are
/// not found. Take the PATH your login shell sets up, plus the usual install folders. Call it first
/// thing in main, before any thread starts. Does nothing on Windows.
pub fn fix_path() {
    if cfg!(windows) {
        return;
    }
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let from_shell = std::process::Command::new(shell)
        .args(["-ilc", "printf '\\n__PATH__%s' \"$PATH\""])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).rsplit_once("__PATH__").map(|(_, p)| p.trim().to_string()))
        .unwrap_or_default();
    let home = home_dir();
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&from_shell).collect();
    dirs.extend(std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default());
    dirs.extend([home.join(".local/bin"), "/opt/homebrew/bin".into(), "/usr/local/bin".into(), home.join(".npm-global/bin")]);
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|d| !d.as_os_str().is_empty() && seen.insert(d.clone()));
    if let Ok(p) = std::env::join_paths(dirs) {
        // SAFETY: called at the very start of main, before other threads exist.
        unsafe { std::env::set_var("PATH", p) };
    }
}

/// Tests and embedders can point everything somewhere else before the first session starts.
pub fn set_data_dir(p: PathBuf) {
    let _ = DATA.set(p);
}

pub fn runs_dir() -> PathBuf {
    match std::env::var_os("RUNS_DIR") {
        Some(p) => std::path::absolute(PathBuf::from(p)).unwrap_or_default(),
        None => data_dir().join("runs"),
    }
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_else(|| ".".into())
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// `20261114093000` (UTC), like `new Date().toISOString()` without separators.
pub fn stamp(ms: u64) -> String {
    let secs = ms / 1000;
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    // Howard Hinnant's civil_from_days
    let z = (secs / 86400) as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(mo <= 2);
    format!("{y:04}{mo:02}{d:02}{h:02}{m:02}{s:02}")
}

pub fn rand_hex(bytes: usize) -> String {
    let mut s = String::new();
    while s.len() < bytes * 2 {
        s.push_str(&uuid::Uuid::new_v4().simple().to_string());
    }
    s.truncate(bytes * 2);
    s
}

/// `file` (absolute, or relative to `root`) as a forward-slash path inside `root`; None when outside.
pub fn rel_path(root: &Path, file: &str) -> Option<String> {
    let mut out: Vec<String> = Vec::new();
    let full = root.join(file);
    let mut base: Vec<String> = Vec::new();
    for (v, p) in [(&mut base, root), (&mut out, full.as_path())] {
        for c in p.components() {
            match c {
                Component::ParentDir => {
                    v.pop();
                }
                Component::CurDir => {}
                c => v.push(c.as_os_str().to_string_lossy().into_owned()),
            }
        }
    }
    let same = |a: &String, b: &String| if cfg!(windows) { a.eq_ignore_ascii_case(b) } else { a == b };
    if out.len() < base.len() || !base.iter().zip(&out).all(|(a, b)| same(a, b)) {
        return None;
    }
    Some(out[base.len()..].join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_matches_iso() {
        assert_eq!(stamp(1794402387828), "20261111130627");
    }

    #[test]
    fn rel_paths() {
        let root = if cfg!(windows) { Path::new(r"C:\w\repo") } else { Path::new("/w/repo") };
        assert_eq!(rel_path(root, "src/a.ts").as_deref(), Some("src/a.ts"));
        assert_eq!(rel_path(root, "../x").as_deref(), None);
        assert_eq!(rel_path(root, &root.join("b").join("c.rs").to_string_lossy()).as_deref(), Some("b/c.rs"));
        if cfg!(windows) {
            assert_eq!(rel_path(root, r"c:\W\Repo\.env").as_deref(), Some(".env"));
        }
    }
}
