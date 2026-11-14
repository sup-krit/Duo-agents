//! Running the agent CLIs: find them, stream their JSONL, and stop the whole process tree
//! (a job object on Windows, a process group elsewhere) on /kill or /stop.

use anyhow::{Result, bail};
use process_wrap::tokio::*;
use serde_json::Value;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio_util::sync::CancellationToken;

fn exe_in(dir: &Path, name: &str) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) { &[".exe", ".com"] } else { &[""] };
    exts.iter().map(|e| dir.join(format!("{name}{e}"))).find(|p| p.is_file())
}

/// A real executable for `name` (DUO_CLAUDE / DUO_CODEX override PATH). On Windows an npm shim
/// (codex.cmd) cannot take our arguments safely, so the native binary behind it is used.
pub fn find(name: &str) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(format!("DUO_{}", name.to_uppercase())) {
        return Some(PathBuf::from(p));
    }
    let dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Some(p) = dirs.iter().find_map(|d| exe_in(d, name)) {
        return Some(p);
    }
    if !cfg!(windows) {
        return None;
    }
    let extra = [crate::home_dir().join(".local").join("bin")];
    if let Some(p) = extra.iter().find_map(|d| exe_in(d, name)) {
        return Some(p);
    }
    let vendor = Path::new("node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/bin/codex.exe");
    dirs.iter().filter(|d| d.join(format!("{name}.cmd")).is_file()).find_map(|d| {
        let pkg = d.join("node_modules/@openai/codex");
        [pkg.join(vendor), d.join(vendor), pkg.join("vendor/x86_64-pc-windows-msvc/bin/codex.exe")].into_iter().find(|p| p.is_file())
    })
}

pub struct Spec<'a> {
    pub program: &'a Path,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: &'a Path,
    pub stdin: &'a str,
}

/// Run to the end, handing each JSON line of stdout to `on_line`. Cancelling `signal` kills the tree.
pub async fn run_jsonl(spec: Spec<'_>, signal: &CancellationToken, mut on_line: impl FnMut(Value) -> Result<()>) -> Result<()> {
    let mut cmd = tokio::process::Command::new(spec.program);
    cmd.args(&spec.args)
        .envs(spec.env.iter().map(|(k, v)| (k, v)))
        .current_dir(spec.cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut wrap = CommandWrap::from(cmd);
    #[cfg(windows)]
    wrap.wrap(JobObject);
    #[cfg(unix)]
    wrap.wrap(ProcessGroup::leader());
    wrap.wrap(KillOnDrop);
    let mut child = wrap.spawn().map_err(|e| anyhow::anyhow!("Could not start {}: {e}", spec.program.display()))?;

    if let Some(mut stdin) = child.stdin().take() {
        stdin.write_all(spec.stdin.as_bytes()).await?;
    } // dropped: the CLI sees the end of the prompt
    let mut stderr = child.stderr().take().unwrap();
    let err_task = tokio::spawn(async move {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s).await;
        s
    });
    let mut lines = BufReader::new(child.stdout().take().unwrap()).lines();

    let result: Result<()> = async {
        loop {
            tokio::select! {
                line = lines.next_line() => match line? {
                    Some(l) => {
                        if let Ok(v) = serde_json::from_str::<Value>(l.trim()) {
                            on_line(v)?;
                        }
                    }
                    None => return Ok(()),
                },
                _ = signal.cancelled() => bail!("stopped"),
            }
        }
    }
    .await;
    if result.is_err() {
        let _ = Box::into_pin(child.kill()).await;
        return result;
    }
    let status = child.wait().await?;
    if !status.success() {
        let err = err_task.await.unwrap_or_default();
        let tail: Vec<&str> = err.lines().filter(|l| !l.trim().is_empty()).collect();
        bail!("{} exited with {status}: {}", spec.program.file_stem().unwrap_or_default().to_string_lossy(), tail[tail.len().saturating_sub(5)..].join(" · "));
    }
    Ok(())
}

/// A TOML value for `codex -c key=value` (JSON strings are valid TOML basic strings).
pub fn toml(v: &Value) -> String {
    match v {
        Value::Array(a) => format!("[{}]", a.iter().map(toml).collect::<Vec<_>>().join(", ")),
        v => v.to_string(),
    }
}
