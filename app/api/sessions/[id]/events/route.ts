import { getSession, readEvents } from "@/lib/session";
import type { DuoEvent } from "@/lib/types";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

// Server-Sent Events: everything so far, then live events.
export async function GET(req: Request, ctx: { params: Promise<{ id: string }> }) {
  const { id } = await ctx.params;
  const live = getSession(id);
  const past = live ? live.events : readEvents(id);
  if (!past) return new Response("Session not found", { status: 404 });

  const enc = new TextEncoder();
  const stream = new ReadableStream({
    start(controller) {
      const send = (ev: DuoEvent) => controller.enqueue(enc.encode(`data: ${JSON.stringify(ev)}\n\n`));
      past.forEach(send);
      if (!live) {
        controller.enqueue(enc.encode(`event: end\ndata: {}\n\n`));
        controller.close();
        return;
      }
      live.listeners.add(send);
      const ping = setInterval(() => controller.enqueue(enc.encode(`: ping\n\n`)), 15000);
      req.signal.addEventListener("abort", () => {
        live.listeners.delete(send);
        clearInterval(ping);
        try { controller.close(); } catch {}
      });
    },
  });
  return new Response(stream, { headers: { "Content-Type": "text/event-stream", "Cache-Control": "no-cache, no-transform", Connection: "keep-alive" } });
}
