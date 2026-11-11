// Workspace for folders that are not git repos: agents work on a copy, and changes are
// copied back only when you approve. A manifest of file hashes taken at the start tells
// what the agents changed, and whether you changed the same file meanwhile (a conflict).

import { createHash } from "node:crypto";
import { copyFileSync, existsSync, lstatSync, mkdirSync, readdirSync, readFileSync, rmSync, symlinkSync } from "node:fs";
import path from "node:path";

/** Build output and caches: not copied, agents can regenerate them. */
const SKIP = new Set([".git", ".duo", ".next", ".nuxt", ".turbo", ".cache", "dist", "build", "out", "coverage", "target", "__pycache__", ".pytest_cache", ".mypy_cache"]);
/** Dependencies: linked, not copied, so tests and builds still run in the copy. */
const LINK = new Set(["node_modules", ".venv", "venv"]);

export type Manifest = Map<string, string>; // relative path (forward slashes) → sha1

const hash = (file: string) => createHash("sha1").update(readFileSync(file)).digest("hex");
const rel = (root: string, file: string) => path.relative(root, file).split(path.sep).join("/");

function walk(root: string, dir: string, out: string[]) {
  for (const name of readdirSync(dir)) {
    if (SKIP.has(name) || LINK.has(name)) continue;
    const full = path.join(dir, name);
    const st = lstatSync(full);
    if (st.isSymbolicLink()) continue;
    if (st.isDirectory()) walk(root, full, out);
    else if (st.isFile()) out.push(full);
  }
}

export function listFiles(root: string) {
  const out: string[] = [];
  walk(root, root, out);
  return out;
}

/** Copy `src` into `dest` and return the hashes of what was copied. */
export function snapshot(src: string, dest: string): Manifest {
  const m: Manifest = new Map();
  mkdirSync(dest, { recursive: true });
  for (const file of listFiles(src)) {
    const r = rel(src, file);
    const to = path.join(dest, r);
    mkdirSync(path.dirname(to), { recursive: true });
    copyFileSync(file, to);
    m.set(r, hash(file));
  }
  // Link dependency folders at any depth we passed (top level is the common case).
  for (const name of LINK) {
    const from = path.join(src, name);
    if (existsSync(from)) {
      try {
        symlinkSync(from, path.join(dest, name), process.platform === "win32" ? "junction" : "dir");
      } catch {
        /* agents can still install dependencies themselves */
      }
    }
  }
  return m;
}

export type Changes = {
  added: string[];
  modified: string[];
  deleted: string[];
  conflicts: string[]; // you changed these in the original folder too; they are not applied
  insertions: number;
  deletions: number;
};

function lineDiff(a: string, b: string) {
  const count = (s: string) => {
    const m = new Map<string, number>();
    for (const l of s.split("\n")) m.set(l, (m.get(l) ?? 0) + 1);
    return m;
  };
  const ca = count(a), cb = count(b);
  let ins = 0, del = 0;
  for (const [l, n] of cb) ins += Math.max(0, n - (ca.get(l) ?? 0));
  for (const [l, n] of ca) del += Math.max(0, n - (cb.get(l) ?? 0));
  return { ins, del };
}

export function changes(src: string, work: string, m: Manifest): Changes {
  const c: Changes = { added: [], modified: [], deleted: [], conflicts: [], insertions: 0, deletions: 0 };
  const seen = new Set<string>();
  for (const file of listFiles(work)) {
    const r = rel(work, file);
    seen.add(r);
    const h = hash(file);
    const before = m.get(r);
    if (before === h) continue;
    const orig = path.join(src, r);
    const origNow = existsSync(orig) ? hash(orig) : undefined;
    if (origNow !== before) { c.conflicts.push(r); continue; }
    if (before === undefined) c.added.push(r);
    else c.modified.push(r);
    const d = lineDiff(before === undefined ? "" : readFileSync(orig, "utf8"), readFileSync(file, "utf8"));
    c.insertions += d.ins;
    c.deletions += d.del;
  }
  for (const [r, h] of m) {
    if (seen.has(r)) continue;
    const orig = path.join(src, r);
    if (!existsSync(orig)) continue;
    if (hash(orig) !== h) { c.conflicts.push(r); continue; }
    c.deleted.push(r);
    c.deletions += readFileSync(orig, "utf8").split("\n").length;
  }
  return c;
}

/** Copy the agents' changes into the original folder, and record them as the new baseline. */
export function apply(src: string, work: string, c: Changes, m: Manifest) {
  for (const r of [...c.added, ...c.modified]) {
    const to = path.join(src, r);
    mkdirSync(path.dirname(to), { recursive: true });
    copyFileSync(path.join(work, r), to);
    m.set(r, hash(to));
  }
  for (const r of c.deleted) {
    rmSync(path.join(src, r), { force: true });
    m.delete(r);
  }
}

export const fileCount = (c: Changes) => c.added.length + c.modified.length + c.deleted.length;

export function describe(c: Changes) {
  return [
    ...c.added.map((f) => `A  ${f}`),
    ...c.modified.map((f) => `M  ${f}`),
    ...c.deleted.map((f) => `D  ${f}`),
    ...c.conflicts.map((f) => `!  ${f}  (you changed it too, not applied)`),
  ].slice(0, 30).join("\n");
}

/** Put one file in the copy back as it was at the start (a new file is removed). */
export function revert(src: string, work: string, r: string, m: Manifest) {
  const to = path.join(work, r);
  if (!m.has(r)) return rmSync(to, { force: true });
  const orig = path.join(src, r);
  if (!existsSync(orig) || hash(orig) !== m.get(r)) return; // you changed it meanwhile: a conflict, never applied anyway
  mkdirSync(path.dirname(to), { recursive: true });
  copyFileSync(orig, to);
}
