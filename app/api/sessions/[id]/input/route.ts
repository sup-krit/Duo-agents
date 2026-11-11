import { NextResponse } from "next/server";
import { getSession } from "@/lib/session";

export const runtime = "nodejs";

// Everything typed in the prompt: plain text to the lead, @agent text, /commands, y/n/e for approvals.
export async function POST(req: Request, ctx: { params: Promise<{ id: string }> }) {
  const { id } = await ctx.params;
  const s = getSession(id);
  if (!s) return NextResponse.json({ error: "This session is no longer running" }, { status: 404 });
  const { text } = await req.json().catch(() => ({ text: "" }));
  const error = s.input(String(text ?? ""));
  return error ? NextResponse.json({ error }, { status: 400 }) : NextResponse.json({ ok: true });
}
