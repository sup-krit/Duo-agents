// Shared by the session engine (server) and the UI (client).
// The UI only ever renders events, so a session can be replayed from runs/<id>/events.jsonl.

/** "claude", "codex", or the id of a provider from ~/.agent-duo/providers.json (e.g. "deepseek"). */
export type Kind = string;
/** auto: agents edit and run commands · acceptEdits: commands need your yes · plan: read-only until you approve the plan */
export type Permission = "auto" | "acceptEdits" | "plan";
export type Mode = "real" | "mock";

export type AgentStatus =
  | "proposed" // in a split the user has not approved yet
  | "queued" // approved, waiting for the items it depends on
  | "running"
  | "waiting" // blocked in `wait`, or on an approval
  | "idle" // the lead between turns
  | "done"
  | "failed"
  | "killed";

/** git: worktree on a new branch · copy: agents work on a copy, applied when you approve · direct: edit the folder in place */
export type Workspace = "git" | "copy" | "direct";

export type SessionConfig = {
  lead: Kind;
  /** "auto" is resolved when the session starts: git for a git repo, otherwise copy. */
  workspace: Workspace | "auto";
  team: Kind[];
  repo: string;
  mode: Mode;
  maxDepth: number;
  maxAgents: number;
  budgetUsd: number;
  ask: { split: boolean; merge: boolean; spawn: boolean };
  permission: Permission;
  /** The mode a plan session switches to once you approve the plan. */
  afterPlan: Exclude<Permission, "plan">;
  /** Globs of files that need your yes before an agent changes them. Empty: everything may change. */
  protect: string[];
  /** What you want each kind on the team to do, in your words ("write tests", "review only"). Missing: the lead decides. */
  roles?: Record<string, string>;
  /** Display names of provider kinds (no keys). */
  labels?: Record<string, string>;
};

export type AgentInfo = {
  id: string; // "claude", "codex#1", "codex#1.a"
  kind: Kind;
  role: string;
  task: string;
  parent: string | null; // null = started by you (the lead)
  depth: number; // lead 0, team 1, subs 2+
};

export type SplitItem = { agent: Kind; role: string; task: string; after?: number[] };

export type SessionStatus = "running" | "waiting" | "idle" | "stopped" | "failed";

type Base = { seq: number; ts: number };

export type EventBody =
  | { type: "session_started"; sessionId: string; task: string; config: SessionConfig; branch?: string; workdir?: string }
  | { type: "agent_added"; agent: AgentInfo; status: AgentStatus }
  | { type: "agent_status"; id: string; status: AgentStatus; note?: string }
  | { type: "agent_usage"; id: string; tokens: number; costUsd: number }
  | { type: "say"; agent: string; text: string }
  | { type: "tool"; agent: string; callId: string; verb: string; target: string; status: "running" | "ok" | "error"; detail?: string }
  | { type: "assign"; by: string; id: string; role: string; task: string; queued?: boolean }
  | { type: "spawn"; by: string; id: string; role: string; task: string }
  | { type: "msg"; from: string; to: string; text: string }
  | { type: "user"; to: string | null; text: string }
  | {
      type: "approval_requested";
      id: string;
      kind: "split" | "spawn" | "merge" | "command" | "file";
      by: string;
      title: string;
      summary?: string;
      items?: (SplitItem & { id: string })[];
      detail?: string;
    }
  | { type: "approval_resolved"; id: string; answer: "yes" | "no" | "edit"; note?: string }
  | { type: "lead_changed"; from: string; to: string }
  | { type: "mode_changed"; permission: Permission; by: string }
  | { type: "log"; level: "info" | "warn" | "error"; text: string }
  | { type: "session_status"; status: SessionStatus; reason?: string };

export type DuoEvent = EventBody extends infer E ? (E extends unknown ? E & Base : never) : never;

export type SessionSummary = {
  id: string;
  task: string;
  lead: Kind;
  repo: string;
  mode: Mode;
  status: SessionStatus;
  startedAt: number;
  costUsd: number;
  agents: number;
};

export const FINISHED: AgentStatus[] = ["done", "failed", "killed"];
