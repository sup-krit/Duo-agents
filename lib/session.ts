import { appendFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { randomBytes, randomUUID } from "node:crypto";
import {
  FINISHED,
  type AgentInfo,
  type AgentStatus,
  type DuoEvent,
  type EventBody,
  type Kind,
  type Permission,
  type SessionConfig,
  type SessionStatus,
  type SessionSummary,
  type SplitItem,
} from "./types";
import type { Runner, ToolResult } from "./runners/types";
import { createMockRunner } from "./runners/mock";
import { createClaudeRunner } from "./runners/claude";
import { createCodexRunner } from "./runners/codex";
import { followUpPrompt, handoverPrompt, leadPrompt, memberPrompt } from "./prompts";
import { addCodexSkills, removeCodexSkills } from "./shared";
import { BUILTIN, engineOf, getProvider, knownKinds } from "./providers";
import { changedFiles, commitAll, createWorktree, diffWorktree, isGitRepo, mergeBranch, revertFile } from "./git";
import { apply as applyCopy, changes as copyChanges, describe as describeCopy, fileCount, revert as revertCopy, snapshot, type Manifest } from "./workspace-copy";

export const RUNS_DIR = path.resolve(/*turbopackIgnore: true*/ process.cwd(), process.env.RUNS_DIR || "runs");
const BRIDGE = path.resolve(/*turbopackIgnore: true*/ process.cwd(), "mcp/duo-bridge.mjs");
const WAIT_MS = 45_000;

type Msg = { from: string; text: string };
type Rec = {
  info: AgentInfo;
  status: AgentStatus;
  runner: Runner;
  inbox: Msg[];
  children: string[];
  deps: string[];
  report?: string;
  abort: AbortController;
  busy: boolean;
  turns: number;
  firstPrompt?: string; // overrides the default first prompt (lead handover)
};

const LEAD_TOOLS = ["propose_split", "assign", "spawn_sub", "wait", "message", "inbox", "status"];

export class Session {
  readonly id: string;
  readonly dir: string;
  readonly token = randomBytes(16).toString("hex");
  cfg: SessionConfig;
  task: string;
  baseUrl: string;
  events: DuoEvent[] = [];
  listeners = new Set<(e: DuoEvent) => void>();
  agents = new Map<string, Rec>();
  leadId = "";
  workdir = "";
  wt: { root: string; dir: string; branch: string } | null = null;
  private manifest: Manifest | null = null; // copy mode: hashes of the original folder at the start
  private mockMerged = false;
  totalCost = 0;
  status: SessionStatus = "running";
  private teamCounter = 0;
  private approvals = new Map<string, (a: { answer: "yes" | "no" | "edit"; note?: string }) => void>();
  private pulses = new Set<() => void>();
  private ended = false;
  /** shared skills copied into the workdir for Codex; removed before merge / at the end. */
  private codexSkills: string[] = [];
  /** Protected files you already said yes to (relative, forward slashes); not asked again at merge. */
  private approvedFiles = new Set<string>();

  constructor(id: string, cfg: SessionConfig, task: string, baseUrl: string) {
    this.id = id;
    this.cfg = cfg;
    this.task = task;
    this.baseUrl = baseUrl;
    this.dir = path.join(RUNS_DIR, id);
    mkdirSync(this.dir, { recursive: true });
    writeFileSync(path.join(this.dir, "events.jsonl"), "");
  }

  // ── events ──────────────────────────────────────────────────────────────
  emit(body: EventBody) {
    const ev = { ...body, seq: this.events.length, ts: Date.now() } as DuoEvent;
    this.events.push(ev);
    appendFileSync(path.join(this.dir, "events.jsonl"), JSON.stringify(ev) + "\n");
    for (const l of this.listeners) l(ev);
    return ev;
  }

  private pulse() {
    const ps = [...this.pulses];
    this.pulses.clear();
    ps.forEach((p) => p());
  }

  private nextPulse(ms: number) {
    return new Promise<void>((resolve) => {
      const t = setTimeout(() => (this.pulses.delete(done), resolve()), ms);
      const done = () => (clearTimeout(t), resolve());
      this.pulses.add(done);
    });
  }

  // ── lifecycle ───────────────────────────────────────────────────────────
  async start() {
    if (this.cfg.mode === "mock") {
      this.cfg.workspace = "copy";
      this.workdir = mkdtempSync(path.join(os.tmpdir(), "duo-mock-"));
    } else {
      const repo = path.resolve(this.cfg.repo);
      if (!existsSync(repo) || !statSync(repo).isDirectory()) throw new Error(`Folder not found: ${this.cfg.repo}`);
      this.cfg.repo = repo;
      if (this.cfg.workspace === "direct" && this.cfg.protect.length) {
        this.cfg.workspace = "auto"; // editing in place has no merge step to hold protected files back
        this.emit({ type: "log", level: "warn", text: "Protected files are set, so agents work on a branch or a copy instead of editing the folder directly." });
      }
      const git = await isGitRepo(repo);
      if (this.cfg.workspace === "auto") this.cfg.workspace = git ? "git" : "copy";
      if (this.cfg.workspace === "git" && !git) this.cfg.workspace = "copy";
      if (this.cfg.workspace === "git") {
        this.wt = await createWorktree(repo, this.id);
        this.workdir = this.wt.dir;
      } else if (this.cfg.workspace === "copy") {
        this.workdir = path.join(this.dir, "workspace");
        this.manifest = snapshot(repo, this.workdir);
      } else {
        this.workdir = repo;
      }
    }
    this.emit({ type: "session_started", sessionId: this.id, task: this.task, config: this.cfg, branch: this.wt?.branch, workdir: this.workdir });
    const lead = this.addAgent({ id: this.cfg.lead, kind: this.cfg.lead, role: "orchestrator", task: "Plan, split the work, check the results", parent: null, depth: 0 }, "idle");
    this.leadId = lead.info.id;
    this.emit({ type: "user", to: null, text: this.task });
    this.startTurn(lead, leadPrompt(this.cfg, this.task, lead.info));
  }

  private runnerFor(kind: Kind): Runner {
    if (this.cfg.mode === "mock") return createMockRunner();
    return engineOf(kind) === "codex" ? createCodexRunner() : createClaudeRunner();
  }

  private addAgent(info: AgentInfo, status: AgentStatus): Rec {
    const rec: Rec = { info, status, runner: this.runnerFor(info.kind), inbox: [], children: [], deps: [], abort: new AbortController(), busy: false, turns: 0 };
    this.agents.set(info.id, rec);
    if (info.parent) this.agents.get(info.parent)?.children.push(info.id);
    this.emit({ type: "agent_added", agent: info, status });
    return rec;
  }

  private setStatus(rec: Rec, status: AgentStatus, note?: string) {
    if (rec.status === status && !note) return;
    rec.status = status;
    this.emit({ type: "agent_status", id: rec.info.id, status, note });
    this.recompute();
  }

  private recompute() {
    if (this.ended) return;
    const recs = [...this.agents.values()];
    const next: SessionStatus = this.approvals.size
      ? "waiting"
      : recs.some((r) => r.busy || r.status === "queued")
        ? "running"
        : "idle";
    if (next !== this.status) {
      this.status = next;
      this.emit({ type: "session_status", status: next });
    }
  }

  private toolsFor(rec: Rec) {
    if (rec.info.id === this.leadId) return LEAD_TOOLS;
    return [...(rec.info.depth < this.cfg.maxDepth ? ["spawn_sub"] : []), "wait", "message", "inbox", "status"];
  }

  private accessFor(rec: Rec): "read" | "write" {
    // ponytail: keyword match on your own words; say "do not edit" / "ห้ามแก้" to make an AI read-only for sure
    const noEdits = /review only|do not edit|don't edit|no edits|read[- ]only|ห้ามแก้|อ่านอย่างเดียว/i.test(this.cfg.roles?.[rec.info.kind] ?? "");
    return rec.info.depth === 0 || noEdits || /review/i.test(rec.info.role) ? "read" : "write";
  }

  /** `file`: absolute, or relative to the workdir. Files outside the workdir are never "protected". */
  isProtected(file: string) {
    if (!this.cfg.protect.length) return false;
    const r = this.relPath(file);
    if (r === null) return false;
    const base = r.split("/").pop()!;
    return this.cfg.protect.some((p) => path.matchesGlob(r, p) || (!p.includes("/") && path.matchesGlob(base, p)));
  }

  private relPath(file: string) {
    const r = path.relative(this.workdir, path.resolve(this.workdir, file)).split(path.sep).join("/");
    return r.startsWith("..") || path.isAbsolute(r) ? null : r;
  }

  private async approve(rec: Rec, a: { kind: "command" | "file"; title: string; detail: string; file?: string }) {
    if (this.ended) return { ok: false, note: "the session has ended" };
    const before = rec.status;
    this.setStatus(rec, "waiting");
    const res = await this.ask({ kind: a.kind, by: rec.info.id, title: a.title, detail: a.detail });
    if (!this.ended && rec.status === "waiting" && before !== "waiting") this.setStatus(rec, "running");
    const ok = res.answer === "yes";
    const r = a.file ? this.relPath(a.file) : null;
    if (ok && r) this.approvedFiles.add(r);
    return { ok, note: res.note };
  }

  setPermission(p: Permission, by: string) {
    if (this.cfg.permission === p) return;
    this.cfg.permission = p;
    if (p !== "plan") this.cfg.afterPlan = p;
    this.emit({ type: "mode_changed", permission: p, by });
  }

  private startTurn(rec: Rec, prompt: string) {
    if (this.ended || rec.busy || rec.status === "killed") return;
    rec.busy = true;
    rec.turns++;
    if (engineOf(rec.info.kind) === "codex" && this.cfg.mode === "real" && !this.codexSkills.length) this.codexSkills = addCodexSkills(this.workdir);
    if (rec.abort.signal.aborted) rec.abort = new AbortController();
    this.setStatus(rec, "running");

    void (async () => {
      try {
        const provider = BUILTIN.includes(rec.info.kind) ? undefined : getProvider(rec.info.kind);
        if (!provider && !BUILTIN.includes(rec.info.kind) && this.cfg.mode === "real")
          throw new Error(`Provider "${rec.info.kind}" is not set up any more. Add it again under Other AI.`);
        const res = await rec.runner.turn({
          agent: rec.info,
          prompt,
          cwd: this.workdir,
          access: this.accessFor(rec),
          tools: this.toolsFor(rec),
          bridge: { baseUrl: this.baseUrl, sessionId: this.id, token: this.token, script: BRIDGE },
          callTool: (name, args) => this.handleTool(rec.info.id, name, args),
          emit: (ev) => this.emit(ev),
          signal: rec.abort.signal,
          permission: () => this.cfg.permission,
          isProtected: (f) => this.isProtected(f),
          approve: (a) => this.approve(rec, a),
          provider,
        });
        rec.report = res.text;
        this.totalCost += res.costUsd;
        this.emit({ type: "agent_usage", id: rec.info.id, tokens: res.tokens, costUsd: res.costUsd });
        rec.busy = false;
        // A turn that ends after /kill or /stop stays stopped.
        if (rec.status === "killed" || this.ended) this.setStatus(rec, "killed");
        else this.setStatus(rec, rec.info.id === this.leadId ? "idle" : "done");
      } catch (err) {
        rec.busy = false;
        if (rec.abort.signal.aborted || this.ended) this.setStatus(rec, "killed");
        else {
          rec.report = `Failed: ${(err as Error).message}`;
          this.setStatus(rec, rec.info.id === this.leadId ? "idle" : "failed", (err as Error).message);
          this.emit({ type: "log", level: "error", text: `${rec.info.id} failed: ${(err as Error).message}` });
        }
      }
      this.pulse();
      if (this.checkBudget()) return;
      this.startReady();
      // Messages that arrived while it was busy: the user's (or anything for the lead) start a follow-up turn.
      this.deliver(rec);
      if (rec.info.id === this.leadId && !rec.busy) await this.offerMerge();
      this.recompute();
    })();
  }

  private startReady() {
    for (const rec of this.agents.values()) {
      if (rec.status !== "queued") continue;
      const deps = rec.deps.map((d) => this.agents.get(d));
      if (deps.every((d) => d && FINISHED.includes(d.status))) {
        const failed = deps.filter((d) => d && d.status !== "done").map((d) => d!.info.id);
        const note = failed.length ? `\n\nNote: ${failed.join(", ")} did not finish successfully.` : "";
        this.startTurn(rec, this.firstPromptFor(rec) + note);
      }
    }
  }

  private firstPromptFor(rec: Rec) {
    const team = [...this.agents.values()].filter((r) => !FINISHED.includes(r.status) && r.status !== "proposed").map((r) => r.info.id);
    return memberPrompt(this.cfg, rec.info, this.leadId, team);
  }

  private deliver(rec: Rec) {
    if (rec.busy || rec.status === "killed" || !rec.inbox.length) return;
    const isLead = rec.info.id === this.leadId;
    if (!isLead && !rec.inbox.some((m) => m.from === "you")) return;
    const msgs = rec.inbox.splice(0);
    let prompt = msgs.map((m) => followUpPrompt(m.from, m.text)).join("\n\n");
    if (rec.turns === 0 && rec.firstPrompt) prompt = `${rec.firstPrompt}\n\nThe user says:\n${prompt}`;
    this.startTurn(rec, prompt);
  }

  private checkBudget() {
    if (this.cfg.budgetUsd > 0 && this.totalCost > this.cfg.budgetUsd) {
      this.stop(`Budget of $${this.cfg.budgetUsd} reached ($${this.totalCost.toFixed(2)})`, "failed");
      return true;
    }
    return false;
  }

  stop(reason: string, status: "stopped" | "failed" = "stopped") {
    if (this.ended) return;
    for (const [id, resolve] of this.approvals) {
      this.emit({ type: "approval_resolved", id, answer: "no", note: "session stopped" });
      resolve({ answer: "no" });
    }
    this.approvals.clear();
    for (const rec of this.agents.values()) {
      if (rec.busy || rec.status === "queued" || rec.status === "proposed") {
        rec.abort.abort();
        rec.busy = false;
        this.setStatus(rec, "killed", reason);
      }
    }
    this.ended = true;
    this.dropCodexSkills();
    this.status = status;
    this.emit({ type: "session_status", status, reason });
    this.pulse();
  }

  kill(id: string, by = "you") {
    const rec = this.agents.get(id);
    if (!rec) return false;
    for (const c of rec.children) this.kill(c, by);
    if (!FINISHED.includes(rec.status) && rec.status !== "idle") {
      rec.abort.abort();
      rec.busy = false;
      this.setStatus(rec, "killed", `stopped by ${by}`);
    }
    this.pulse();
    this.startReady();
    return true;
  }

  // ── approvals ───────────────────────────────────────────────────────────
  private ask(ev: Omit<Extract<EventBody, { type: "approval_requested" }>, "type" | "id">) {
    const id = randomUUID().slice(0, 8);
    this.emit({ type: "approval_requested", id, ...ev });
    return new Promise<{ answer: "yes" | "no" | "edit"; note?: string }>((resolve) => {
      this.approvals.set(id, resolve);
      this.recompute();
    });
  }

  answer(id: string | undefined, answer: "yes" | "no" | "edit", note?: string) {
    const key = id ?? [...this.approvals.keys()][0];
    const resolve = key && this.approvals.get(key);
    if (!key || !resolve) return false;
    this.approvals.delete(key);
    this.emit({ type: "approval_resolved", id: key, answer, note });
    resolve({ answer, note });
    this.recompute();
    return true;
  }

  get pendingApprovals() {
    return this.approvals.size;
  }

  // ── duo tools (called by agents through the MCP bridge, or directly by mock agents) ──
  private newId(parent: Rec | null, kind: Kind) {
    if (!parent || parent.info.depth === 0) return `${kind}#${++this.teamCounter}`;
    const n = parent.children.filter((c) => this.agents.get(c)?.info.depth === parent.info.depth + 1).length;
    return `${parent.info.id}.${String.fromCharCode(97 + n)}`;
  }

  private live() {
    return [...this.agents.values()].filter((r) => !["killed", "proposed"].includes(r.status)).length;
  }

  async handleTool(agentId: string, name: string, args: Record<string, unknown>): Promise<ToolResult> {
    const rec = this.agents.get(agentId);
    if (!rec) return { text: `Unknown agent ${agentId}`, isError: true };
    if (this.ended) return { text: "The session has ended. Stop now.", isError: true };
    const res = await this.tool(rec, name, args);
    if (name !== "inbox" && name !== "wait" && rec.inbox.length)
      res.text += `\n\n(You have ${rec.inbox.length} unread message${rec.inbox.length > 1 ? "s" : ""}. Call inbox.)`;
    return res;
  }

  private async tool(rec: Rec, name: string, args: Record<string, unknown>): Promise<ToolResult> {
    const me = rec.info.id;
    const isLead = me === this.leadId;
    switch (name) {
      case "propose_split": {
        if (!isLead) return { text: "Only the lead can propose a split. Use spawn_sub to split your own part.", isError: true };
        const items = (Array.isArray(args.items) ? args.items : []) as SplitItem[];
        if (!items.length) return { text: "items must not be empty", isError: true };
        const bad = items.find((i) => !this.cfg.team.includes(i.agent));
        if (bad) return { text: `${bad.agent} is not on this team (team: ${this.cfg.team.join(", ")})`, isError: true };
        if (this.live() + items.length > this.cfg.maxAgents)
          return { text: `That would exceed the limit of ${this.cfg.maxAgents} agents (${this.live()} already). Propose fewer parts.`, isError: true };

        const recs = items.map((i) =>
          this.addAgent({ id: this.newId(null, i.agent), kind: i.agent, role: i.role, task: i.task, parent: me, depth: 1 }, "proposed"),
        );
        recs.forEach((r, k) => (r.deps = (items[k].after ?? []).map((n) => recs[n - 1]?.info.id).filter(Boolean) as string[]));

        const planning = this.cfg.permission === "plan";
        if (this.cfg.ask.split || planning) {
          this.setStatus(rec, "waiting");
          const a = await this.ask({
            kind: "split",
            by: me,
            title: "Proposed split",
            summary: typeof args.summary === "string" ? args.summary : undefined,
            items: items.map((i, k) => ({ ...i, id: recs[k].info.id })),
          });
          if (!this.ended) this.setStatus(rec, "running");
          if (a.answer !== "yes") {
            recs.forEach((r) => this.setStatus(r, "killed", a.answer === "no" ? "rejected" : "replaced"));
            // Give the numbers back so the next proposal starts at #1 again.
            for (const r of recs) {
              this.agents.delete(r.info.id);
              rec.children = rec.children.filter((c) => c !== r.info.id);
            }
            this.teamCounter -= recs.length;
            return a.answer === "no"
              ? { text: `Rejected by the user${a.note ? `: ${a.note}` : ""}. Do not start the work.` }
              : { text: `Changes requested by the user: ${a.note || "(no details)"}. Call propose_split again with a new split.` };
          }
          if (planning) this.setPermission(this.cfg.afterPlan, "plan approved");
        }
        const started: string[] = [];
        const queued: string[] = [];
        for (const r of recs) {
          const q = r.deps.length > 0;
          this.emit({ type: "assign", by: me, id: r.info.id, role: r.info.role, task: r.info.task, queued: q });
          if (q) {
            this.setStatus(r, "queued");
            queued.push(`${r.info.id} (${r.info.role}, after ${r.deps.join(", ")})`);
          } else {
            this.startTurn(r, this.firstPromptFor(r));
            started.push(`${r.info.id} (${r.info.role})`);
          }
        }
        return {
          text: `Approved. Started: ${started.join(", ") || "none"}.${queued.length ? ` Queued: ${queued.join(", ")}.` : ""} Call wait to collect their reports.`,
        };
      }

      case "assign": {
        if (!isLead) return { text: "Only the lead can assign work to the team.", isError: true };
        const kind = args.agent as Kind;
        if (!this.cfg.team.includes(kind)) return { text: `${kind} is not on this team`, isError: true };
        if (this.live() >= this.cfg.maxAgents) return { text: `Agent limit (${this.cfg.maxAgents}) reached.`, isError: true };
        const r = this.addAgent({ id: this.newId(null, kind), kind, role: String(args.role || "task"), task: String(args.task || ""), parent: me, depth: 1 }, "queued");
        this.emit({ type: "assign", by: me, id: r.info.id, role: r.info.role, task: r.info.task });
        this.startTurn(r, this.firstPromptFor(r));
        return { text: `Started ${r.info.id}.` };
      }

      case "spawn_sub": {
        const depth = rec.info.depth + 1;
        if (depth > this.cfg.maxDepth) return { text: `Max depth ${this.cfg.maxDepth} reached; do this yourself.`, isError: true };
        if (this.live() >= this.cfg.maxAgents) return { text: `Agent limit (${this.cfg.maxAgents}) reached; do this yourself.`, isError: true };
        const task = String(args.task || "");
        if (this.cfg.ask.spawn) {
          this.setStatus(rec, "waiting");
          const a = await this.ask({ kind: "spawn", by: me, title: `${me} wants a sub-agent`, detail: task });
          if (!this.ended) this.setStatus(rec, "running");
          if (a.answer !== "yes") return { text: `The user declined this sub-agent${a.note ? `: ${a.note}` : ""}. Do it yourself.` };
        }
        const r = this.addAgent({ id: this.newId(rec, rec.info.kind), kind: rec.info.kind, role: String(args.role || "sub"), task, parent: me, depth }, "queued");
        this.emit({ type: "spawn", by: me, id: r.info.id, role: r.info.role, task });
        this.startTurn(r, this.firstPromptFor(r));
        return { text: `Started ${r.info.id}. Call wait before you finish.` };
      }

      case "wait": {
        const ids = Array.isArray(args.ids) && args.ids.length ? (args.ids as string[]) : rec.children;
        const unknown = ids.filter((i) => !this.agents.has(i));
        if (unknown.length) return { text: `Unknown agent(s): ${unknown.join(", ")}`, isError: true };
        if (!ids.length && !rec.inbox.length) return { text: "Nothing to wait for: you have not started any agents." };
        const targets = () => ids.map((i) => this.agents.get(i)!).filter((r) => r.status !== "killed" || r.report);
        const pending = () => targets().filter((r) => !FINISHED.includes(r.status));
        const deadline = Date.now() + WAIT_MS;
        if (pending().length && !rec.inbox.length) this.setStatus(rec, "waiting");
        while (pending().length && !rec.inbox.length && Date.now() < deadline && !this.ended && !rec.abort.signal.aborted)
          await this.nextPulse(deadline - Date.now());
        if (!this.ended && !rec.abort.signal.aborted && rec.status === "waiting") this.setStatus(rec, "running");

        const parts: string[] = [];
        for (const t of targets()) {
          if (FINISHED.includes(t.status)) parts.push(`## ${t.info.id} (${t.info.role}) ${t.status}\n${t.report ?? ""}`);
        }
        const msgs = rec.inbox.splice(0);
        if (msgs.length) parts.push(`## Messages\n${msgs.map((m) => `${m.from}: ${m.text}`).join("\n")}`);
        const still = pending().map((r) => `${r.info.id} (${r.status})`);
        parts.push(still.length ? `Still running: ${still.join(", ")}. Call wait again.` : "All of them have finished.");
        return { text: parts.join("\n\n") };
      }

      case "message": {
        let to = String(args.to || "");
        const text = String(args.text || "");
        if (to === "lead") to = this.leadId;
        if (to === "you" || to === "user") {
          this.emit({ type: "msg", from: me, to: "you", text });
          return { text: "Shown to the user." };
        }
        const target = this.agents.get(to);
        if (!target) return { text: `No agent called ${to}. Call status to see ids.`, isError: true };
        target.inbox.push({ from: me, text });
        this.emit({ type: "msg", from: me, to, text });
        this.pulse();
        this.deliver(target);
        return { text: `Delivered to ${to}.` };
      }

      case "inbox": {
        const msgs = rec.inbox.splice(0);
        return { text: msgs.length ? msgs.map((m) => `${m.from}: ${m.text}`).join("\n") : "No messages." };
      }

      case "status":
        return {
          text: [...this.agents.values()]
            .filter((r) => r.status !== "proposed")
            .map((r) => `${r.info.id} · ${r.info.kind} · ${r.info.role} · ${r.status} · ${r.info.task}${r.info.parent ? ` · started by ${r.info.parent}` : ""}`)
            .join("\n"),
        };
    }
    return { text: `Unknown tool ${name}`, isError: true };
  }

  // ── what you type ───────────────────────────────────────────────────────
  input(raw: string): string | null {
    const text = raw.trim();
    if (!text) return null;
    if (this.ended) return "This session has ended. Start a new one.";

    if (this.approvals.size && /^(y|yes|n|no|e|edit)\b/i.test(text)) {
      const [w, ...rest] = text.split(/\s+/);
      const a = /^y/i.test(w) ? "yes" : /^n/i.test(w) ? "no" : "edit";
      this.emit({ type: "user", to: null, text });
      this.answer(undefined, a, rest.join(" ") || undefined);
      return null;
    }

    if (text.startsWith("/")) {
      const [cmd, ...rest] = text.slice(1).split(/\s+/);
      const arg = rest.join(" ");
      switch (cmd) {
        case "help":
          this.emit({ type: "log", level: "info", text: "/orc <agent kind> · /mode auto|edits|plan · @agent text · /kill <agent> · /budget <usd> · /merge · /stop · y / n / e <changes> to answer an approval" });
          return null;
        case "stop":
          this.stop("Stopped by you");
          return null;
        case "kill":
          return this.kill(arg) ? null : `No agent called ${arg}`;
        case "budget": {
          const n = Number(arg.replace("$", ""));
          if (!(n >= 0)) return "Usage: /budget 10";
          this.cfg.budgetUsd = n;
          this.emit({ type: "log", level: "info", text: `Budget set to $${n}` });
          return null;
        }
        case "merge":
          void this.offerMerge(true);
          return null;
        case "mode": {
          const m = ({ auto: "auto", edits: "acceptEdits", accept: "acceptEdits", acceptedits: "acceptEdits", plan: "plan" } as Record<string, Permission>)[arg.toLowerCase().replace(/[\s_-]/g, "")];
          if (!m) return "Usage: /mode auto | edits | plan";
          this.setPermission(m, "you");
          return null;
        }
        case "orc":
        case "lead":
          return this.switchLead(arg as Kind);
        default:
          return `Unknown command /${cmd}. Try /help.`;
      }
    }

    const at = text.match(/^@(\S+)\s+([\s\S]+)/);
    const to = at ? (at[1] === "lead" ? this.leadId : at[1]) : this.leadId;
    const body = at ? at[2] : text;
    const target = this.agents.get(to);
    if (!target) return `No agent called ${to}`;
    this.emit({ type: "user", to: at ? to : null, text: body });
    target.inbox.push({ from: "you", text: body });
    this.pulse();
    this.deliver(target);
    return null;
  }

  private switchLead(kind: Kind): string | null {
    if (!knownKinds().includes(kind)) return `Usage: /orc ${knownKinds().join(" | ")}`;
    const old = this.agents.get(this.leadId)!;
    if (old.info.kind === kind) return `${old.info.id} already leads.`;
    if (old.busy) return `${old.info.id} is in the middle of a turn. Wait for it, or /stop.`;
    let id: string = kind;
    for (let n = 2; this.agents.has(id); n++) id = `${kind}-lead${n}`;
    const history = this.events
      .filter((e) => e.type === "user" || (e.type === "say" && e.agent === old.info.id))
      .slice(-16)
      .map((e) => (e.type === "user" ? `user: ${e.text}` : e.type === "say" ? `${e.agent}: ${e.text}` : ""))
      .join("\n");
    const rec = this.addAgent({ id, kind, role: "orchestrator", task: "Lead the session (took over)", parent: null, depth: 0 }, "idle");
    rec.firstPrompt = handoverPrompt(this.cfg, rec.info, history);
    this.setStatus(old, "done", `handed over to ${id}`);
    this.leadId = id;
    this.emit({ type: "lead_changed", from: old.info.id, to: id });
    return null;
  }

  private dropCodexSkills() {
    removeCodexSkills(this.workdir, this.codexSkills);
    this.codexSkills = [];
  }

  // ── bringing the work back ──────────────────────────────────────────────
  /** Last gate for every engine (Codex cannot ask mid-turn, Bash can write anything): protected files you have
   *  not said yes to are asked about one by one, and put back if you say no. */
  private async holdProtected() {
    if (!this.cfg.protect.length || this.cfg.mode !== "real") return;
    let files: string[] = [];
    if (this.wt) files = await changedFiles(this.wt.dir);
    else if (this.manifest) {
      const c = copyChanges(this.cfg.repo, this.workdir, this.manifest);
      files = [...c.added, ...c.modified, ...c.deleted];
    }
    for (const f of files) {
      if (!this.isProtected(f) || this.approvedFiles.has(f)) continue;
      const a = await this.ask({ kind: "file", by: this.leadId, title: `Keep the change to protected file ${f}?`, detail: f });
      if (this.ended) return;
      if (a.answer === "yes") this.approvedFiles.add(f);
      else {
        if (this.wt) await revertFile(this.wt.dir, f);
        else if (this.manifest) revertCopy(this.cfg.repo, this.workdir, f, this.manifest);
        this.emit({ type: "log", level: "info", text: `Put ${f} back as it was.` });
      }
    }
  }

  private merging = false;
  private async offerMerge(force = false) {
    if (this.merging || this.ended) return;
    if ([...this.agents.values()].some((r) => r.busy || r.status === "queued")) {
      if (force) this.emit({ type: "log", level: "warn", text: "Agents are still working; merge after they finish." });
      return;
    }
    this.dropCodexSkills(); // nobody is running; keep them out of the diff (re-added on the next Codex turn)
    if (this.cfg.workspace === "direct" && this.cfg.mode === "real") {
      if (force) this.emit({ type: "log", level: "info", text: `Agents edit ${this.cfg.repo} directly; there is nothing to merge.` });
      return;
    }
    this.merging = true;
    const log = (text: string, level: "info" | "error" = "info") => this.emit({ type: "log", level, text });
    try {
      await this.holdProtected();
    } catch (err) {
      this.merging = false;
      return log(`Could not check protected files, nothing merged: ${(err as Error).message}`, "error");
    }
    const decide = async (title: string, detail: string) =>
      !this.cfg.ask.merge || (await this.ask({ kind: "merge", by: this.leadId, title, detail })).answer === "yes";
    try {
      if (this.wt) {
        const d = await diffWorktree(this.wt.dir);
        if (!d.filesChanged) { if (force) log("Nothing to merge."); return; }
        const list = d.patch.split("\n").filter((l) => l.startsWith("+++ b/")).map((l) => l.slice(6)).slice(0, 30).join("\n");
        const yes = await decide(`Merge ${d.filesChanged} files (+${d.insertions} −${d.deletions}) into your current branch?`, list);
        await commitAll(this.wt.dir, `agent-duo: ${this.task.split("\n")[0].slice(0, 70)}`);
        if (!yes) return log(`Committed on ${this.wt.branch}. Not merged; /merge when ready.`);
        try {
          await mergeBranch(this.wt.root, this.wt.branch);
          log(`Merged ${this.wt.branch} into your current branch.`);
        } catch (err) {
          log(`Merge failed, work kept on ${this.wt.branch}: ${(err as Error).message}`, "error");
        }
      } else if (this.manifest) {
        const c = copyChanges(this.cfg.repo, this.workdir, this.manifest);
        if (!fileCount(c)) {
          if (c.conflicts.length) log(`${c.conflicts.length} file(s) changed both here and in your folder; nothing applied:\n${describeCopy(c)}`, "error");
          else if (force) log("Nothing to apply.");
          return;
        }
        const yes = await decide(`Apply ${fileCount(c)} files (+${c.insertions} −${c.deletions}) to ${this.cfg.repo}?`, describeCopy(c));
        if (!yes) return log(`Kept in the session copy (${this.workdir}). /merge to apply later.`);
        applyCopy(this.cfg.repo, this.workdir, c, this.manifest);
        log(`Applied ${fileCount(c)} files to ${this.cfg.repo}.` + (c.conflicts.length ? ` Skipped ${c.conflicts.length} you changed too: ${c.conflicts.join(", ")}` : ""));
      } else {
        // mock: pretend there are changes, once
        if (this.mockMerged || !this.events.some((e) => e.type === "tool" && ["add", "edit"].includes(e.verb))) { if (force) log("Nothing to merge."); return; }
        this.mockMerged = true;
        const yes = await decide("Merge 4 files (+81 −6) into main (mock)?", "M  src/api/users.ts\nA  src/lib/cache.ts\nA  tests/cache.test.ts\nM  src/api/index.ts");
        log(yes ? "Merged into main (mock, no files changed)." : "Kept on the session branch (mock).");
      }
    } finally {
      this.merging = false;
      this.recompute();
    }
  }
}

// ── registry ──────────────────────────────────────────────────────────────
const g = globalThis as unknown as { __duoSessions?: Map<string, Session> };
const sessions = (g.__duoSessions ??= new Map<string, Session>());

export async function createSession(cfg: SessionConfig, task: string, baseUrl: string) {
  const id = new Date().toISOString().slice(0, 19).replace(/[-:T]/g, "") + "-" + randomUUID().slice(0, 6);
  const s = new Session(id, cfg, task, baseUrl);
  sessions.set(id, s);
  try {
    await s.start();
  } catch (err) {
    s.stop((err as Error).message, "failed");
    throw err;
  }
  return s;
}

export const getSession = (id: string) => sessions.get(id);

export function readEvents(id: string): DuoEvent[] | null {
  if (!/^[\w-]+$/.test(id)) return null;
  const file = path.join(RUNS_DIR, id, "events.jsonl");
  if (!existsSync(file)) return null;
  return readFileSync(file, "utf8").split("\n").filter(Boolean).map((l) => JSON.parse(l) as DuoEvent);
}

export function listSessions(): SessionSummary[] {
  if (!existsSync(RUNS_DIR)) return [];
  return readdirSync(RUNS_DIR)
    .map((id): SessionSummary | null => {
      const evs = sessions.get(id)?.events ?? readEvents(id);
      const start = evs?.find((e) => e.type === "session_started");
      if (!evs || !start || start.type !== "session_started") return null;
      let status: SessionStatus = sessions.has(id) ? "running" : "stopped";
      let cost = 0;
      let agents = 0;
      for (const e of evs) {
        if (e.type === "session_status") status = e.status;
        if (e.type === "agent_usage") cost += e.costUsd;
        if (e.type === "agent_added") agents++;
      }
      if (!sessions.has(id) && (status === "running" || status === "waiting")) status = "stopped";
      return { id, task: start.task, lead: start.config.lead, repo: start.config.repo, mode: start.config.mode, status, startedAt: start.ts, costUsd: cost, agents };
    })
    .filter((s): s is SessionSummary => !!s)
    .sort((a, b) => b.startedAt - a.startedAt);
}
