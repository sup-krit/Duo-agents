import { NextResponse } from "next/server";
import { getSession } from "@/lib/session";

export const runtime = "nodejs";

export async function POST(req: Request, ctx: { params: Promise<{ id: string }> }) {
  const { id } = await ctx.params;
  const s = getSession(id);
  if (!s) return NextResponse.json({ error: "This session is no longer running" }, { status: 404 });
  const { approvalId, answer, note } = await req.json().catch(() => ({}));
  if (!["yes", "no", "edit"].includes(answer)) return NextResponse.json({ error: "answer must be yes, no or edit" }, { status: 400 });
  if (!s.answer(String(approvalId), answer, note ? String(note) : undefined))
    return NextResponse.json({ error: "No such pending approval" }, { status: 409 });
  return NextResponse.json({ ok: true });
}
