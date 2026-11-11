import { NextResponse } from "next/server";
import { createSession, listSessions } from "@/lib/session";
import { getProvider, knownKinds } from "@/lib/providers";
import type { Kind, Permission, SessionConfig } from "@/lib/types";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

export async function GET() {
  return NextResponse.json(listSessions());
}

const kind = (v: unknown): Kind => (typeof v === "string" && knownKinds().includes(v) ? v : "claude");
const perm = (v: unknown): Permission => (v === "acceptEdits" || v === "plan" ? v : "auto");
const num = (v: unknown, d: number, lo: number, hi: number) => Math.min(Math.max(Number(v ?? d) || d, lo), hi);

export async function POST(req: Request) {
  const b = await req.json().catch(() => ({}));
  const task = String(b.task ?? "").trim();
  const mode = b.mode === "real" ? "real" : "mock";
  const repo = String(b.repo ?? "").trim();
  if (!task) return NextResponse.json({ error: "Type a task first" }, { status: 400 });
  if (mode === "real" && !repo) return NextResponse.json({ error: "Project folder is required" }, { status: 400 });
  const team = (Array.isArray(b.team) ? b.team : ["codex", "claude"]).map(kind).filter((k: Kind, i: number, a: Kind[]) => a.indexOf(k) === i);
  if (!team.length) return NextResponse.json({ error: "Pick at least one team agent" }, { status: 400 });

  const cfg: SessionConfig = {
    lead: kind(b.lead),
    workspace: b.workspace === "direct" ? "direct" : "auto",
    team,
    repo: mode === "mock" ? "(mock)" : repo,
    mode,
    maxDepth: b.subAgents === false ? 1 : num(b.maxDepth, 2, 1, 4),
    maxAgents: num(b.maxAgents, 8, 1, 20),
    budgetUsd: num(b.budgetUsd, 5, 0, 1000),
    ask: { split: b.ask?.split !== false, merge: b.ask?.merge !== false, spawn: !!b.ask?.spawn },
    permission: perm(b.permission),
    afterPlan: b.afterPlan === "acceptEdits" ? "acceptEdits" : "auto",
    protect: (Array.isArray(b.protect) ? b.protect : [])
      .map((p: unknown) => String(p).trim().replace(/\\/g, "/").replace(/^\.\//, ""))
      .filter((p: string, i: number, a: string[]) => p && !p.startsWith("#") && a.indexOf(p) === i)
      .slice(0, 100),
  };
  const roles = b.roles && typeof b.roles === "object" ? (b.roles as Record<string, unknown>) : {};
  cfg.roles = Object.fromEntries(team.map((k: Kind) => [k, String(roles[k] ?? "").trim().slice(0, 500)]).filter(([, v]: string[]) => v));
  // Display names only; keys stay in ~/.agent-duo/providers.json.
  cfg.labels = Object.fromEntries([cfg.lead, ...team].map((k) => [k, getProvider(k)?.label ?? k]));
  try {
    // Agents reach the session through this origin (the MCP bridge calls back here).
    const s = await createSession(cfg, task, new URL(req.url).origin);
    return NextResponse.json({ id: s.id });
  } catch (err) {
    return NextResponse.json({ error: (err as Error).message }, { status: 400 });
  }
}
