// agent-duo's own shared/skills and shared/agents, offered to every agent.
// Claude loads the folder as a plugin. Codex only reads <cwd>/.agents/skills (it ignores parent
// folders and skills.config paths), so the skills are copied there while Codex agents run and
// removed again before anything is diffed, merged or the session ends. shared/agents (Claude's
// .md format) become Codex agent roles: [agents.<name>] with a config_file of developer_instructions.

import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";

export const SHARED = path.resolve(/*turbopackIgnore: true*/ process.cwd(), "shared");

/** Copy shared skills into `workdir`/.agents/skills, skipping names the repo already has. Returns what was added. */
export function addCodexSkills(workdir: string): string[] {
  const src = path.join(SHARED, "skills");
  if (!existsSync(src)) return [];
  const added: string[] = [];
  for (const d of readdirSync(src, { withFileTypes: true })) {
    if (!d.isDirectory()) continue;
    const dest = path.join(workdir, ".agents", "skills", d.name);
    if (existsSync(dest)) continue; // the repo's own skill wins
    cpSync(path.join(src, d.name), dest, { recursive: true });
    added.push(dest);
  }
  return added;
}

/** Undo addCodexSkills, including the folders it had to create. */
export function removeCodexSkills(workdir: string, added: string[]) {
  for (const p of added) rmSync(p, { recursive: true, force: true });
  for (const dir of [path.join(workdir, ".agents", "skills"), path.join(workdir, ".agents")])
    if (existsSync(dir) && !readdirSync(dir).length) rmSync(dir, { recursive: true });
}

/**
 * shared/agents/*.md as Codex `agents` config, for spawn_agent's agent_type.
 * Claude-only fields (tools, model) are dropped; the role runs with the parent's sandbox.
 */
export function codexAgents(sandbox: "read-only" | "workspace-write"): Record<string, { description: string; config_file: string }> {
  const src = path.join(SHARED, "agents");
  if (!existsSync(src)) return {};
  const out = path.join(os.tmpdir(), "agent-duo-codex-agents", sandbox);
  mkdirSync(out, { recursive: true });
  const roles: Record<string, { description: string; config_file: string }> = {};
  for (const f of readdirSync(src)) {
    if (!f.endsWith(".md")) continue;
    const { meta, body } = frontmatter(readFileSync(path.join(src, f), "utf8"));
    const name = meta.name || f.slice(0, -3);
    if (!/^[\w-]+$/.test(name) || !body.trim()) continue;
    const file = path.join(out, `${name}.toml`);
    // JSON strings are valid TOML basic strings
    writeFileSync(file, `developer_instructions = ${JSON.stringify(body.trim())}\nsandbox_mode = ${JSON.stringify(sandbox)}\n`);
    roles[name] = { description: meta.description || name, config_file: file };
  }
  return roles;
}

// ponytail: flat `key: value` lines only; multi-line YAML values (| or >) are not read.
function frontmatter(text: string) {
  const m = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/.exec(text);
  if (!m) return { meta: {} as Record<string, string>, body: text };
  const meta: Record<string, string> = {};
  for (const line of m[1].split(/\r?\n/)) {
    const kv = /^(\w+):\s*(.*)$/.exec(line);
    if (kv) meta[kv[1]] = kv[2].trim().replace(/^(["'])(.*)\1$/, "$2");
  }
  return { meta, body: m[2] };
}
