//! git through the command line, like the TypeScript version.

use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

async fn git(cwd: &Path, args: &[&str]) -> Result<String> {
    let out = tokio::process::Command::new("git").args(args).current_dir(cwd).kill_on_drop(true).output().await?;
    if !out.status.success() {
        bail!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub async fn is_git_repo(dir: &Path) -> bool {
    git(dir, &["rev-parse", "--is-inside-work-tree"]).await.is_ok_and(|s| s == "true")
}

#[derive(Clone, Debug)]
pub struct Worktree {
    pub root: PathBuf,
    pub dir: PathBuf,
    pub branch: String,
}

/// Agents work on their own branch in their own folder; your checkout is never touched until merge.
pub async fn create_worktree(repo: &Path, run_id: &str) -> Result<Worktree> {
    let root = PathBuf::from(git(repo, &["rev-parse", "--show-toplevel"]).await?);
    let branch = format!("duo/{run_id}");
    let dir = root.join(".duo").join("worktrees").join(run_id);
    git(&root, &["worktree", "add", "-b", &branch, &dir.to_string_lossy(), "HEAD"]).await?;
    // Keep the worktree folder out of the user's `git status` (local exclude, nothing committed).
    let common = PathBuf::from(git(&root, &["rev-parse", "--git-common-dir"]).await?);
    let exclude = if common.is_absolute() { common } else { root.join(common) }.join("info").join("exclude");
    std::fs::create_dir_all(exclude.parent().unwrap())?;
    if !std::fs::read_to_string(&exclude).unwrap_or_default().contains(".duo/") {
        use std::io::Write;
        std::fs::OpenOptions::new().create(true).append(true).open(&exclude)?.write_all(b"\n.duo/\n")?;
    }
    Ok(Worktree { root, dir, branch })
}

#[derive(Debug, Default)]
pub struct DiffInfo {
    pub patch: String,
    pub files_changed: u64,
    pub insertions: u64,
    pub deletions: u64,
}

/// Everything changed in the worktree since it was created, including new files.
pub async fn diff_worktree(dir: &Path) -> Result<DiffInfo> {
    git(dir, &["add", "-A"]).await?;
    let patch = git(dir, &["diff", "--cached", "HEAD"]).await?;
    let stat = git(dir, &["diff", "--cached", "--shortstat", "HEAD"]).await?;
    let n = |word: &str| -> u64 {
        stat.split(',').find(|p| p.contains(word)).and_then(|p| p.split_whitespace().next()?.parse().ok()).unwrap_or(0)
    };
    Ok(DiffInfo { patch, files_changed: n("changed"), insertions: n("insertion"), deletions: n("deletion") })
}

pub async fn commit_all(dir: &Path, message: &str) -> Result<bool> {
    git(dir, &["add", "-A"]).await?;
    if git(dir, &["diff", "--cached", "--name-only"]).await?.is_empty() {
        return Ok(false);
    }
    git(dir, &["-c", "user.name=agent-duo", "-c", "user.email=agent-duo@localhost", "commit", "-m", message]).await?;
    Ok(true)
}

pub async fn merge_branch(root: &Path, branch: &str) -> Result<()> {
    git(root, &["merge", "--no-ff", "--no-edit", branch]).await.map(|_| ())
}

/// Paths changed in the worktree since it was created (added, modified, deleted).
pub async fn changed_files(dir: &Path) -> Result<Vec<String>> {
    git(dir, &["add", "-A"]).await?;
    Ok(git(dir, &["diff", "--cached", "--name-only", "HEAD"]).await?.lines().filter(|l| !l.is_empty()).map(String::from).collect())
}

/// Put one file back as it was at the start (a new file is removed).
pub async fn revert_file(dir: &Path, file: &str) -> Result<()> {
    git(dir, &["restore", "--source=HEAD", "--staged", "--worktree", "--", file]).await.map(|_| ())
}
