"use client";

import Link from "next/link";
import { useEffect, useMemo, useState } from "react";
import type { DuoEvent } from "@/lib/types";
import { NumberTicker } from "@/components/ui/number-ticker";
import { activeMs, agentColor, fmtDur, label, MODE_LABEL, pathTo, reduce, type Approval } from "./reduce";
import { Prompt, Transcript } from "./chat";
import { StatusText, Tree } from "./tree";
import { Timeline } from "./timeline";

export function SessionView({ id }: { id: string }) {
  const [events, setEvents] = useState<DuoEvent[]>([]);
  const [missing, setMissing] = useState(false);
  const [tab, setTab] = useState<"chat" | "timeline">("chat");
  const [mtab, setMtab] = useState<"chat" | "tree">("chat");
  const [focus, setFocus] = useState<string | null>(null);
  const [hideTools, setHideTools] = useState(false);
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    setEvents([]);
    const es = new EventSource(`/api/sessions/${id}/events`);
    es.onmessage = (m) => {
      const ev = JSON.parse(m.data) as DuoEvent;
      setEvents((prev) => (ev.seq < prev.length ? prev : [...prev, ev]));
    };
    es.addEventListener("end", () => es.close());
    es.onerror = () => { if (es.readyState === EventSource.CLOSED) setMissing(true); };
    return () => es.close();
  }, [id]);

  const v = useMemo(() => reduce(events), [events]);
  const live = v.status !== "stopped" && v.status !== "failed" && !missing;
  const busyNow = v.status === "running" || v.status === "waiting";
  useEffect(() => {
    if (!busyNow) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [busyNow]);

  const send = async (text: string) => {
    const r = await fetch(`/api/sessions/${id}/input`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ text }) });
    return r.ok ? null : ((await r.json().catch(() => ({}))).error ?? "Could not send");
  };
  const answer = async (a: Approval, ans: "yes" | "no" | "edit", note?: string) => {
    await fetch(`/api/sessions/${id}/approvals`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ approvalId: a.id, answer: ans, note }) });
  };
  const open = (agent: string | null) => { setFocus(agent); setTab("chat"); setMtab("chat"); };

  if (missing && !events.length) return <div className="app" style={{ padding: 24 }}><p className="dim">Session not found. <Link href="/" className="ink">Start a new one</Link></p></div>;
  if (!v.config) return <div className="app" style={{ padding: 24 }}><span className="shim mono">connecting</span></div>;

  const lead = v.agents.get(v.leadId);
  const running = [...v.agents.values()].filter((a) => a.status === "running" || a.status === "waiting").length;
  const other = lead?.info.kind === "claude" ? "codex" : "claude";
  const team = [...v.agents.values()].filter((a) => a.info.depth === 1);
  const fa = focus ? v.agents.get(focus) : null;

  return (
    <div className="app">
      <header className="top">
        <Link href="/" className="brand"><span className="dots"><i style={{ background: "var(--claude)" }} /><i style={{ background: "var(--codex)" }} /></span>agent-duo</Link>
        <span className="title" title={v.task}>{v.task.split("\n")[0]}</span>
        {lead && (
          <button className="chip" style={{ cursor: "pointer" }} title={`Hand the lead to ${other}`} onClick={() => void send(`/orc ${other}`)} disabled={!live}>
            lead <b style={{ color: agentColor(v, lead.info.id) }}>{label(lead.info.kind)}</b> ⇄
          </button>
        )}
        <button className="chip" style={{ cursor: "pointer" }} disabled={!live}
          title="Mode: auto = agents act on their own · accept edits = commands need your yes · plan = read-only until you approve the plan. Click to switch."
          onClick={() => void send(`/mode ${({ auto: "edits", acceptEdits: "plan", plan: "auto" } as const)[v.config!.permission ?? "auto"]}`)}>
          mode <b className={v.config.permission === "auto" || !v.config.permission ? undefined : "warn"}>{MODE_LABEL[v.config.permission ?? "auto"]}</b>
        </button>
        {!!v.config.protect?.length && <span className="chip hide-sm" title={v.config.protect.join(", ")}><b>{v.config.protect.length}</b> protected</span>}
        {v.config.mode === "real" && (
          <span className="chip hide-sm" title={v.config.repo}>
            {v.config.workspace === "git" ? <>branch <b>{v.branch}</b></> : v.config.workspace === "copy" ? <>on a <b>copy</b></> : <b className="warn">editing in place</b>}
          </span>
        )}
        <span className="chip hide-sm">{v.status === "waiting" ? <span className="warn">● needs you</span> : running ? <><span className="run pulse">●</span> <b>{running}</b> working</> : <b>{v.status}</b>}</span>
        <span className="chip"><b><NumberTicker value={v.cost} format={{ style: "currency", currency: "USD", minimumFractionDigits: 2 }} /></b>{v.config.budgetUsd > 0 && ` / $${v.config.budgetUsd}`}</span>
        {live ? <button className="btn" onClick={() => void send("/stop")}>stop</button> : <Link href="/" className="btn">new session</Link>}
      </header>

      <div className="mtabs tabs" role="tablist" aria-label="View">
        <button role="tab" className="tab" aria-selected={mtab === "chat"} onClick={() => setMtab("chat")}>chat</button>
        <button role="tab" className="tab" aria-selected={mtab === "tree"} onClick={() => setMtab("tree")}>tree · {v.agents.size}</button>
      </div>

      <div className="frame" data-m={mtab}>
        <section className="chat">
          <div style={{ display: "flex", gap: 8, padding: "8px 16px", borderBottom: "1px solid var(--line)", flexWrap: "wrap", alignItems: "center" }}>
            <div className="tabs" role="tablist" aria-label="View">
              <button role="tab" className="tab" aria-selected={tab === "chat"} onClick={() => setTab("chat")}>chat</button>
              <button role="tab" className="tab" aria-selected={tab === "timeline"} onClick={() => setTab("timeline")}>timeline</button>
            </div>
            {tab === "chat" && !focus && (
              <div className="tabs" role="tablist" aria-label="Show messages from">
                <button role="tab" className="tab" aria-selected={!focus} onClick={() => setFocus(null)}>all</button>
                {[lead, ...team].filter(Boolean).slice(0, 6).map((a) => (
                  <button key={a!.info.id} role="tab" className="tab" aria-selected={false} onClick={() => setFocus(a!.info.id)}>
                    <span style={{ color: agentColor(v, a!.info.id) }}>{a!.info.id}</span>
                  </button>
                ))}
              </div>
            )}
            {tab === "chat" && focus && (
              <nav aria-label="Breadcrumb" className="mono" style={{ display: "flex", gap: 8, alignItems: "center", fontSize: 12, flexWrap: "wrap" }}>
                <button className="tab" onClick={() => setFocus(null)}>all</button>
                {pathTo(v, focus).map((p, i, arr) => (
                  <span key={p} style={{ display: "inline-flex", gap: 8, alignItems: "center" }}>
                    <span className="dim">›</span>
                    {i === arr.length - 1 ? <span style={{ color: agentColor(v, p), fontWeight: 700 }}>{p}</span>
                      : <button className="tab" style={{ color: agentColor(v, p) }} onClick={() => setFocus(p)}>{p}</button>}
                  </span>
                ))}
              </nav>
            )}
            <span style={{ flex: 1 }} />
            {tab === "chat" && <button className="tab hide-sm" aria-pressed={hideTools} onClick={() => setHideTools(!hideTools)}>{hideTools ? "show tool lines" : "hide tool lines"}</button>}
          </div>

          {tab === "chat" && fa && (
            <div style={{ display: "flex", gap: 16, alignItems: "flex-start", flexWrap: "wrap", padding: "14px 20px", borderBottom: "1px solid var(--line)", background: "var(--bg)" }}>
              <div style={{ display: "flex", flexDirection: "column", gap: 4, flex: 1, minWidth: 240 }}>
                <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
                  <span className="nd" style={{ width: 10, height: 10, background: agentColor(v, fa.info.id) }} />
                  <span style={{ fontWeight: 600, fontSize: 16 }}>{label(fa.info.id)}</span>
                  <span className="role">{fa.info.depth === 0 ? "lead" : fa.info.role}</span>
                  <span style={{ fontSize: 12 }}><StatusText a={fa} /></span>
                </div>
                <span style={{ fontSize: 13, color: "var(--text)" }}>{fa.info.parent ? `Assigned by ${fa.info.parent}: ` : ""}{fa.info.task}</span>
              </div>
              <div className="mono" style={{ display: "grid", gridTemplateColumns: "repeat(3, auto)", gap: "2px 18px", fontSize: 12, color: "var(--muted)" }}>
                <span>subs</span><span>time</span><span>cost</span>
                <span className="ink">{fa.children.length}</span><span className="ink">{fmtDur(activeMs(fa, now))}</span><span className="ink">${fa.cost.toFixed(2)}</span>
              </div>
              {fa.info.depth > 0 && live && ["running", "waiting", "queued"].includes(fa.status) && (
                <button className="btn btn-bad" onClick={() => void send(`/kill ${fa.info.id}`)}>stop it</button>
              )}
            </div>
          )}

          {tab === "chat" ? (
            <>
              <Transcript v={v} focus={focus} onFocus={open} hideTools={hideTools} answer={answer} />
              <Prompt v={v} focus={focus} send={send} />
            </>
          ) : (
            <Timeline v={v} now={now} live={busyNow && !missing} onOpen={open} onKill={(a) => void send(`/kill ${a}`)} />
          )}
        </section>

        {tab === "chat" && (
          <aside className="side">
            <Tree
              v={v}
              focus={focus}
              onFocus={(a) => open(focus === a ? null : a)}
              now={now}
              footer={
                <div className="legend">
                  <span><span className="run">●</span> running</span><span><span className="ok">✓</span> done</span>
                  <span><span className="warn">●</span> waiting</span><span><span className="bad">✗</span> failed</span><span>┄ proposed</span>
                  <span>click an agent to focus</span>
                </div>
              }
            />
          </aside>
        )}
      </div>
    </div>
  );
}
