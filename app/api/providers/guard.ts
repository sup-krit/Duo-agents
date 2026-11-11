import { NextResponse } from "next/server";

/**
 * These routes touch API keys. Only this app's own pages may call them: a JSON body (other sites
 * cannot send one without a CORS preflight, which is never answered) from the same origin.
 */
export function guard(req: Request): NextResponse | null {
  const origin = req.headers.get("origin");
  if (origin && new URL(origin).host !== new URL(req.url).host) return NextResponse.json({ error: "Forbidden" }, { status: 403 });
  if (req.method !== "GET" && !req.headers.get("content-type")?.includes("application/json"))
    return NextResponse.json({ error: "Send JSON" }, { status: 415 });
  return null;
}
