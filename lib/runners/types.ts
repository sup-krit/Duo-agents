import type { Provider } from "../providers";
import type { AgentInfo, EventBody, Permission } from "../types";

export type ToolResult = { text: string; isError?: boolean };

/** What a runner gets for one turn of one agent. */
export type TurnContext = {
  agent: AgentInfo;
  prompt: string;
  cwd: string;
  /** "read": may only look at code. "write": may edit files and run commands. */
  access: "read" | "write";
  /** duo tools this agent may call (assign, spawn_sub, wait, …). */
  tools: string[];
  /** For real agents: where the MCP bridge reaches the session. */
  bridge: { baseUrl: string; sessionId: string; token: string; script: string };
  /** For mock agents: call a duo tool directly (same code path as the bridge). */
  callTool: (name: string, args: Record<string, unknown>) => Promise<ToolResult>;
  emit: (ev: EventBody) => void;
  signal: AbortSignal;
  /** The session's mode right now (it can change mid-turn with /mode or when a plan is approved). */
  permission: () => Permission;
  /** Is this file (absolute, or relative to cwd) one you marked as needing your yes? */
  isProtected: (file: string) => boolean;
  /** Ask you, and wait for the answer. `file` is remembered as approved when you say yes. */
  approve: (a: { kind: "command" | "file"; title: string; detail: string; file?: string }) => Promise<{ ok: boolean; note?: string }>;
  /** Set when this agent's kind is a provider (DeepSeek, Ollama, …): the brain to use instead of the default one. */
  provider?: Provider;
};

export type TurnResult = { text: string; tokens: number; costUsd: number };

/** One runner instance per agent, so follow-up turns keep the conversation. */
export interface Runner {
  turn(ctx: TurnContext): Promise<TurnResult>;
}

export function preview(v: unknown, max = 80): string {
  const s = typeof v === "string" ? v : JSON.stringify(v ?? "");
  const one = s.replace(/\s+/g, " ").trim();
  return one.length > max ? one.slice(0, max - 1) + "…" : one;
}
