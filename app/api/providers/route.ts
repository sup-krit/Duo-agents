import { NextResponse } from "next/server";
import { deleteProvider, listProviders, toPublic, upsertProvider } from "@/lib/providers";
import { guard } from "./guard";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/** Never returns full keys. */
export async function GET(req: Request) {
  return guard(req) ?? NextResponse.json(listProviders().map(toPublic));
}

export async function PUT(req: Request) {
  const g = guard(req);
  if (g) return g;
  const err = upsertProvider(await req.json().catch(() => ({})));
  return err ? NextResponse.json({ error: err }, { status: 400 }) : NextResponse.json(listProviders().map(toPublic));
}

export async function DELETE(req: Request) {
  const g = guard(req);
  if (g) return g;
  const { id } = await req.json().catch(() => ({}));
  deleteProvider(String(id ?? ""));
  return NextResponse.json(listProviders().map(toPublic));
}
