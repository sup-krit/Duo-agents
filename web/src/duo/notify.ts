// Browser notifications for the command-line agent-duo (the desktop app shows its own, from the engine).
// Same wording as core/src/notify.rs. Only while this tab is in the background.
import type { DuoEvent } from "../types";

const desktop = () => (window as unknown as { __DUO_DESKTOP__?: boolean }).__DUO_DESKTOP__ === true;
const short = (s: string, max: number) => { const one = s.replace(/\s+/g, " ").trim(); return one.length > max ? one.slice(0, max - 1) + "…" : one; };

/** Ask once, from a click (browsers ignore it otherwise). */
export function askToNotify() {
  if (!desktop() && "Notification" in window && Notification.permission === "default") void Notification.requestPermission();
}

export function notify(e: DuoEvent, task: string, lead: string, since: number) {
  if (desktop() || !("Notification" in window) || Notification.permission !== "granted" || !document.hidden || e.ts < since) return;
  let title = "", body = "";
  if (e.type === "approval_requested") [title, body] = [`${e.by} needs your ok`, e.title];
  else if (e.type === "session_status" && e.status === "idle") [title, body] = [`${lead} is done`, "Waiting for your next message."];
  else if (e.type === "session_status" && e.status === "failed") [title, body] = ["Session stopped", e.reason ?? ""];
  else if (e.type === "agent_status" && e.status === "failed") [title, body] = [`${e.id} failed`, e.note ?? ""];
  else if (e.type === "msg" && e.to === "you") [title, body] = [`${e.from} says`, e.text];
  else return;
  const n = new Notification(short(title, 60), { body: `${short(body, 140)}\n${short(task, 60)}`, tag: `duo-${e.seq}` });
  n.onclick = () => { window.focus(); n.close(); };
}
