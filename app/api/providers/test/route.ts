import { NextResponse } from "next/server";
import { draftProvider, testProvider } from "@/lib/providers";
import { guard } from "../guard";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/** Tests what is in the form; an empty key uses the stored one (same base URL only). */
export async function POST(req: Request) {
  const g = guard(req);
  if (g) return g;
  const p = draftProvider(await req.json().catch(() => ({})));
  if (typeof p === "string") return NextResponse.json({ ok: false, error: p }, { status: 400 });
  const err = await testProvider(p);
  return NextResponse.json(err ? { ok: false, error: err } : { ok: true });
}
