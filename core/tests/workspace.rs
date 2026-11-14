//! Workspaces on temporary folders: copy mode (added, modified, deleted, conflict, skipped build
//! folders, copied back, a dependency junction) and git mode (worktree, diff, revert, commit, merge).

use duo_core::{copy, git};
use std::fs;
use std::path::Path;

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("duo-ws-{name}-{}", duo_core::rand_hex(4)));
    fs::create_dir_all(&d).unwrap();
    d
}

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

#[test]
fn copy_mode() {
    let src = tmp("src");
    write(&src, "a.txt", "one\ntwo\n");
    write(&src, "src/b.txt", "keep\n");
    write(&src, "gone.txt", "bye\n");
    write(&src, "both.txt", "base\n");
    write(&src, "dist/out.js", "built");
    write(&src, "node_modules/dep/index.js", "dep");
    let work = tmp("work").join("w");
    let mut m = copy::snapshot(&src, &work).unwrap();

    assert!(!work.join("dist").exists(), "build output is not copied");
    assert!(work.join("node_modules/dep/index.js").exists(), "dependencies are linked");
    assert!(!m.contains_key("node_modules/dep/index.js"));

    write(&work, "a.txt", "one\nTWO\n");
    write(&work, "new/c.txt", "hello\n");
    fs::remove_file(work.join("gone.txt")).unwrap();
    write(&work, "both.txt", "agent\n");
    write(&src, "both.txt", "you\n"); // you changed it too
    write(&work, "build/x.js", "ignored");

    let c = copy::changes(&src, &work, &m);
    assert_eq!(c.added, ["new/c.txt"]);
    assert_eq!(c.modified, ["a.txt"]);
    assert_eq!(c.deleted, ["gone.txt"]);
    assert_eq!(c.conflicts, ["both.txt"]);
    assert_eq!((c.insertions, c.deletions), (2, 1 + 2)); // a.txt 1, gone.txt 2 (split counts the empty last line, like the TS version)

    copy::apply(&src, &work, &c, &mut m).unwrap();
    assert_eq!(fs::read_to_string(src.join("a.txt")).unwrap(), "one\nTWO\n");
    assert_eq!(fs::read_to_string(src.join("new/c.txt")).unwrap(), "hello\n");
    assert!(!src.join("gone.txt").exists());
    assert_eq!(fs::read_to_string(src.join("both.txt")).unwrap(), "you\n", "a conflict is never applied");
    assert_eq!(copy::changes(&src, &work, &m).file_count(), 0, "applied changes are the new baseline");

    // revert puts a file back, and removes a new one
    write(&work, "src/b.txt", "changed\n");
    write(&work, "extra.txt", "x");
    copy::revert(&src, &work, "src/b.txt", &m);
    copy::revert(&src, &work, "extra.txt", &m);
    assert_eq!(fs::read_to_string(work.join("src/b.txt")).unwrap(), "keep\n");
    assert!(!work.join("extra.txt").exists());
}

async fn sh(dir: &Path, args: &[&str]) {
    let ok = tokio::process::Command::new("git").args(args).current_dir(dir).output().await.unwrap().status.success();
    assert!(ok, "git {args:?}");
}

#[tokio::test]
async fn git_mode() {
    let repo = tmp("git");
    sh(&repo, &["init", "-q", "-b", "main"]).await;
    write(&repo, "a.txt", "one\n");
    write(&repo, "secret.env", "x=1\n");
    sh(&repo, &["add", "-A"]).await;
    sh(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init"]).await;
    assert!(git::is_git_repo(&repo).await);
    assert!(!git::is_git_repo(&tmp("plain")).await);

    let wt = git::create_worktree(&repo, "run1").await.unwrap();
    assert_eq!(wt.branch, "duo/run1");
    write(&wt.dir, "a.txt", "one\ntwo\n");
    write(&wt.dir, "b.txt", "new\n");
    write(&wt.dir, "secret.env", "x=2\n");
    let files = git::changed_files(&wt.dir).await.unwrap();
    assert_eq!(files, ["a.txt", "b.txt", "secret.env"]);

    git::revert_file(&wt.dir, "secret.env").await.unwrap();
    git::revert_file(&wt.dir, "b.txt").await.unwrap();
    assert!(!wt.dir.join("b.txt").exists(), "a new file is removed");
    let d = git::diff_worktree(&wt.dir).await.unwrap();
    assert_eq!((d.files_changed, d.insertions, d.deletions), (1, 1, 0));

    assert!(git::commit_all(&wt.dir, "agent-duo: t").await.unwrap());
    git::merge_branch(&wt.root, &wt.branch).await.unwrap();
    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap().replace("\r\n", "\n"), "one\ntwo\n");
    assert_eq!(fs::read_to_string(repo.join("secret.env")).unwrap().replace("\r\n", "\n"), "x=1\n");
    let status = tokio::process::Command::new("git").args(["status", "--porcelain"]).current_dir(&repo).output().await.unwrap();
    assert!(String::from_utf8_lossy(&status.stdout).trim().is_empty(), "the worktree folder stays out of git status");
}
