import { execFile } from "node:child_process";
import { promisify } from "node:util";
import path from "node:path";
import { appendFileSync, existsSync, mkdirSync, readFileSync } from "node:fs";

const run = promisify(execFile);

async function git(cwd: string, ...args: string[]) {
  const { stdout } = await run("git", args, { cwd, maxBuffer: 20 * 1024 * 1024 });
  return stdout.trim();
}

export async function isGitRepo(dir: string) {
  try {
    return (await git(dir, "rev-parse", "--is-inside-work-tree")) === "true";
  } catch {
    return false;
  }
}

/** Codex works on its own branch in its own folder; your checkout is never touched until merge. */
export async function createWorktree(repo: string, runId: string) {
  const root = await git(repo, "rev-parse", "--show-toplevel");
  const branch = `duo/${runId}`;
  const dir = path.join(root, ".duo", "worktrees", runId);
  await git(root, "worktree", "add", "-b", branch, dir, "HEAD");
  // Keep the worktree folder out of the user's `git status` (local exclude, nothing committed).
  const common = await git(root, "rev-parse", "--git-common-dir");
  const excludePath = path.join(path.isAbsolute(common) ? common : path.join(root, common), "info", "exclude");
  mkdirSync(path.dirname(excludePath), { recursive: true });
  if (!existsSync(excludePath) || !readFileSync(excludePath, "utf8").includes(".duo/"))
    appendFileSync(excludePath, "\n.duo/\n");
  return { root, dir, branch };
}

export type DiffInfo = { patch: string; filesChanged: number; insertions: number; deletions: number };

/** Everything changed in the worktree since it was created, including new files. */
export async function diffWorktree(dir: string): Promise<DiffInfo> {
  await git(dir, "add", "-A");
  const patch = await git(dir, "diff", "--cached", "HEAD");
  const stat = await git(dir, "diff", "--cached", "--shortstat", "HEAD");
  const n = (re: RegExp) => Number(stat.match(re)?.[1] ?? 0);
  return {
    patch,
    filesChanged: n(/(\d+) files? changed/),
    insertions: n(/(\d+) insertions?/),
    deletions: n(/(\d+) deletions?/),
  };
}

export async function commitAll(dir: string, message: string) {
  await git(dir, "add", "-A");
  const staged = await git(dir, "diff", "--cached", "--name-only");
  if (!staged) return false;
  await git(dir, "-c", "user.name=agent-duo", "-c", "user.email=agent-duo@localhost", "commit", "-m", message);
  return true;
}

export async function mergeBranch(root: string, branch: string) {
  await git(root, "merge", "--no-ff", "--no-edit", branch);
}

export async function removeWorktree(root: string, dir: string) {
  await git(root, "worktree", "remove", "--force", dir).catch(() => {});
}

/** Paths changed in the worktree since it was created (added, modified, deleted). */
export async function changedFiles(dir: string) {
  await git(dir, "add", "-A");
  return (await git(dir, "diff", "--cached", "--name-only", "HEAD")).split("\n").filter(Boolean);
}

/** Put one file back as it was at the start (a new file is removed). */
export async function revertFile(dir: string, file: string) {
  await git(dir, "restore", "--source=HEAD", "--staged", "--worktree", "--", file);
}
