import { NextResponse } from "next/server";
import { timingSafeEqual } from "node:crypto";
import { getSession } from "@/lib/session";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";
export const maxDuration = 3600;

// Called by mcp/duo-bridge.mjs when an agent uses a duo tool. `wait` and `propose_split` can block.
export async function POST(req: Request, ctx: { params: Promise<{ id: string }> }) {
  const { id } = await ctx.params;
  const s = getSession(id);
  if (!s) return NextResponse.json({ text: "Session not found", isError: true }, { status: 404 });
  const { agent, token, name, args } = await req.json().catch(() => ({}));
  const a = Buffer.from(String(token ?? ""));
  const b = Buffer.from(s.token);
  if (a.length !== b.length || !timingSafeEqual(a, b)) return NextResponse.json({ text: "Bad token", isError: true }, { status: 403 });
  const res = await s.handleTool(String(agent), String(name), (args ?? {}) as Record<string, unknown>);
  return NextResponse.json(res);
}
