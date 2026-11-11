import { FINISHED, type AgentInfo, type AgentStatus, type DuoEvent, type SessionConfig, type SessionStatus, type SplitItem } from "@/lib/types";

export type AgentView = {
  info: AgentInfo;
  status: AgentStatus;
  note?: string;
  tokens: number;
  cost: number;
  addedAt: number;
  log: { ts: number; status: AgentStatus }[];
  errors: number[]; // timestamps of failed tool calls
  doing?: string; // last tool line, for the tree
  children: string[];
};

export type Line =
  | { k: "user"; key: string; ts: number; to: string | null; text: string }
  | { k: "say"; key: string; ts: number; agent: string; text: string }
  | { k: "tool"; key: string; ts: number; agent: string; verb: string; target: string; status: "running" | "ok" | "error"; detail?: string }
  | { k: "assign" | "spawn"; key: string; ts: number; by: string; id: string; role: string; task: string; queued?: boolean }
  | { k: "msg"; key: string; ts: number; from: string; to: string; text: string }
  | { k: "log"; key: string; ts: number; level: string; text: string }
  | { k: "lead"; key: string; ts: number; from: string; to: string }
  | { k: "approval"; key: string; ts: number; id: string };

export type Approval = {
  id: string;
  kind: "split" | "spawn" | "merge" | "command" | "file";
  by: string;
  title: string;
  summary?: string;
  items?: (SplitItem & { id: string })[];
  detail?: string;
  answer?: "yes" | "no" | "edit";
  note?: string;
};

export type View = {
  id?: string;
  task: string;
  config?: SessionConfig;
  branch?: string;
  status: SessionStatus;
  reason?: string;
  agents: Map<string, AgentView>;
  roots: string[]; // leads, in order
  leadId: string;
  lines: Line[];
  approvals: Map<string, Approval>;
  msgs: { ts: number; from: string; to: string }[];
  startTs: number;
  lastTs: number;
  cost: number;
  tokens: number;
};

export function reduce(events: DuoEvent[]): View {
  const v: View = {
    task: "", status: "running", agents: new Map(), roots: [], leadId: "", lines: [], approvals: new Map(),
    msgs: [], startTs: events[0]?.ts ?? Date.now(), lastTs: events[events.length - 1]?.ts ?? Date.now(), cost: 0, tokens: 0,
  };
  const toolIdx = new Map<string, number>();

  for (const e of events) {
    const key = `e${e.seq}`;
    switch (e.type) {
      case "session_started":
        v.id = e.sessionId; v.task = e.task; v.config = e.config; v.branch = e.branch; v.startTs = e.ts;
        break;
      case "agent_added": {
        // An id can come back after a rejected proposal gave its number back.
        const reused = v.agents.has(e.agent.id);
        v.agents.set(e.agent.id, { info: e.agent, status: e.status, tokens: 0, cost: 0, addedAt: e.ts, log: [{ ts: e.ts, status: e.status }], errors: [], children: [] });
        const parent = e.agent.parent ? v.agents.get(e.agent.parent) : null;
        if (reused && parent && parent.children.includes(e.agent.id)) break;
        if (parent) parent.children.push(e.agent.id);
        else { v.roots.push(e.agent.id); if (!v.leadId) v.leadId = e.agent.id; }
        break;
      }
      case "agent_status": {
        const a = v.agents.get(e.id);
        if (a) { a.status = e.status; a.note = e.note; a.log.push({ ts: e.ts, status: e.status }); if (FINISHED.includes(e.status)) a.doing = undefined; }
        break;
      }
      case "agent_usage": {
        const a = v.agents.get(e.id);
        if (a) { a.tokens += e.tokens; a.cost += e.costUsd; }
        v.cost += e.costUsd; v.tokens += e.tokens;
        break;
      }
      case "say":
        v.lines.push({ k: "say", key, ts: e.ts, agent: e.agent, text: e.text });
        break;
      case "tool": {
        const a = v.agents.get(e.agent);
        const i = toolIdx.get(e.callId);
        if (i !== undefined) {
          const prev = v.lines[i] as Extract<Line, { k: "tool" }>;
          v.lines[i] = { ...prev, status: e.status, detail: e.detail ?? prev.detail, verb: e.verb || prev.verb, target: e.target || prev.target };
        } else {
          toolIdx.set(e.callId, v.lines.length);
          v.lines.push({ k: "tool", key, ts: e.ts, agent: e.agent, verb: e.verb, target: e.target, status: e.status, detail: e.detail });
        }
        if (a) {
          const l = v.lines[toolIdx.get(e.callId)!] as Extract<Line, { k: "tool" }>;
          a.doing = e.status === "running" ? `${l.verb} ${l.target}` : a.doing;
          if (e.status === "error") a.errors.push(e.ts);
        }
        break;
      }
      case "assign":
      case "spawn":
        v.lines.push({ k: e.type, key, ts: e.ts, by: e.by, id: e.id, role: e.role, task: e.task, queued: e.type === "assign" ? e.queued : undefined });
        break;
      case "msg":
        v.lines.push({ k: "msg", key, ts: e.ts, from: e.from, to: e.to, text: e.text });
        v.msgs.push({ ts: e.ts, from: e.from, to: e.to });
        break;
      case "user":
        v.lines.push({ k: "user", key, ts: e.ts, to: e.to, text: e.text });
        break;
      case "log":
        v.lines.push({ k: "log", key, ts: e.ts, level: e.level, text: e.text });
        break;
      case "lead_changed":
        v.leadId = e.to;
        v.lines.push({ k: "lead", key, ts: e.ts, from: e.from, to: e.to });
        break;
      case "mode_changed":
        if (v.config) v.config = { ...v.config, permission: e.permission };
        v.lines.push({ k: "log", key, ts: e.ts, level: "info", text: `mode: ${MODE_LABEL[e.permission]} (${e.by})` });
        break;
      case "approval_requested":
        v.approvals.set(e.id, { id: e.id, kind: e.kind, by: e.by, title: e.title, summary: e.summary, items: e.items, detail: e.detail });
        v.lines.push({ k: "approval", key, ts: e.ts, id: e.id });
        break;
      case "approval_resolved": {
        const a = v.approvals.get(e.id);
        if (a) { a.answer = e.answer; a.note = e.note; }
        break;
      }
      case "session_status":
        v.status = e.status; v.reason = e.reason;
        break;
    }
  }
  return v;
}

export const pending = (v: View) => [...v.approvals.values()].filter((a) => !a.answer);

/** Agents in tree order (depth-first), each with its depth in the drawing. */
export function treeOrder(v: View): { a: AgentView; level: number }[] {
  const out: { a: AgentView; level: number }[] = [];
  const walk = (id: string, level: number) => {
    const a = v.agents.get(id);
    if (!a) return;
    out.push({ a, level });
    a.children.forEach((c) => walk(c, level + 1));
  };
  v.roots.forEach((r) => walk(r, 0));
  return out;
}

/** The agent plus everyone under it. */
export function subtree(v: View, id: string): Set<string> {
  const s = new Set<string>();
  const walk = (x: string) => { s.add(x); v.agents.get(x)?.children.forEach(walk); };
  walk(id);
  return s;
}

export function pathTo(v: View, id: string): string[] {
  const p: string[] = [];
  for (let x: string | null | undefined = id; x; x = v.agents.get(x)?.info.parent) p.unshift(x);
  return p;
}

// Provider kinds (deepseek, ollama, …) get a steady color from their id.
const EXTRA = ["#e5a84b", "#4fb8a8", "#c084fc", "#f07178", "#8fbf5a", "#60a5fa"];
export const kindColor = (k?: string) => {
  if (k === "codex") return "var(--codex)";
  if (k === "claude") return "var(--claude)";
  if (!k) return "var(--you)";
  let h = 0;
  for (const c of k) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return EXTRA[h % EXTRA.length];
};
export const MODE_LABEL = { auto: "auto", acceptEdits: "accept edits", plan: "plan" } as const;
export const agentColor = (v: View, id: string) => (id === "you" ? "var(--you)" : kindColor(v.agents.get(id)?.info.kind));

export function label(id: string) {
  return id.replace(/^([\w-]+)#/, "$1 #").replace(/^(\w)/, (c) => c.toUpperCase());
}

export function fmtDur(ms: number) {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  return m < 60 ? `${m}m ${String(s % 60).padStart(2, "0")}s` : `${Math.floor(m / 60)}h ${String(m % 60).padStart(2, "0")}m`;
}

export function clock(ms: number) {
  const s = Math.max(0, Math.round(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** Time an agent has been alive (from first running to finish or now). */
export function activeMs(a: AgentView, now: number) {
  const start = a.log.find((l) => l.status === "running")?.ts;
  if (!start) return 0;
  const end = FINISHED.includes(a.status) ? a.log[a.log.length - 1].ts : now;
  return end - start;
}
