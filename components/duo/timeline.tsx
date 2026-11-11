"use client";

import { useState } from "react";
import { FINISHED, type AgentStatus } from "@/lib/types";
import { activeMs, clock, fmtDur, kindColor, label, treeOrder, type AgentView, type View } from "./reduce";
import { StatusText } from "./tree";

const ROW = 39;
const HEAD = 34;
const STEPS = [5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600].map((s) => s * 1000);

type Seg = { from: number; to: number; status: AgentStatus };

function segments(a: AgentView, end: number): Seg[] {
  const out: Seg[] = [];
  a.log.forEach((l, i) => {
    const to = a.log[i + 1]?.ts ?? (FINISHED.includes(l.status) ? l.ts : end);
    if (to > l.ts && !FINISHED.includes(l.status)) out.push({ from: l.ts, to, status: l.status });
  });
  return out;
}

/** Sum of time each status was held, across agents. */
function stats(v: View, t0: number, end: number) {
  const evs: { ts: number; d: number }[] = [];
  let waiting = 0;
  const waitBy = new Map<string, number>();
  for (const a of v.agents.values()) {
    for (const s of segments(a, end)) {
      if (s.status === "running") { evs.push({ ts: s.from, d: 1 }, { ts: s.to, d: -1 }); }
      if (s.status === "waiting" && a.info.depth > 0) { waiting += s.to - s.from; waitBy.set(a.info.id, (waitBy.get(a.info.id) ?? 0) + s.to - s.from); }
    }
  }
  evs.sort((x, y) => x.ts - y.ts || x.d - y.d);
  let cur = 0, peak = 0, area = 0, last = t0;
  for (const e of evs) { area += cur * (e.ts - last); last = e.ts; cur += e.d; peak = Math.max(peak, cur); }
  const most = [...waitBy.entries()].sort((x, y) => y[1] - x[1])[0];
  return { avg: area / Math.max(end - t0, 1), peak, waiting, most };
}

/** Follow, from the lead down, the child that finished last (or is still going): the slowest chain. */
function slowest(v: View, end: number): string[] {
  const finish = (a: AgentView): number => Math.max(FINISHED.includes(a.status) ? a.log[a.log.length - 1].ts : end, ...a.children.map((c) => finish(v.agents.get(c)!)));
  const chain: string[] = [];
  let cur = v.agents.get(v.leadId);
  while (cur) {
    chain.push(cur.info.id);
    const kids = cur.children.map((c) => v.agents.get(c)!).filter((k) => k.status !== "proposed" && k.log.some((l) => l.status === "running"));
    cur = kids.sort((x, y) => finish(y) - finish(x))[0];
  }
  return chain;
}

export function Timeline({ v, now, live, onOpen, onKill }: { v: View; now: number; live: boolean; onOpen: (id: string) => void; onKill: (id: string) => void }) {
  const [sel, setSel] = useState<string | null>(null);
  const t0 = v.startTs;
  const end = live ? now : v.lastTs;
  const span = Math.max(end - t0, 20_000) * 1.04;
  const pct = (ts: number) => `${(((ts - t0) / span) * 100).toFixed(3)}%`;
  const wpct = (a: number, b: number) => `${(((b - a) / span) * 100).toFixed(3)}%`;
  const step = STEPS.find((s) => span / s <= 8) ?? 3600_000;
  const ticks = Array.from({ length: Math.floor(span / step) + 1 }, (_, i) => i * step);

  const rows = treeOrder(v).filter(({ a }) => !(a.status === "killed" && !a.log.some((l) => l.status === "running")) && a.status !== "proposed");
  const idx = new Map(rows.map((r, i) => [r.a.info.id, i]));
  const st = stats(v, t0, end);
  const chain = slowest(v, end);
  const selA = sel ? v.agents.get(sel) : null;

  return (
    <div className="tl-wrap">
      <div className="cards">
        <div className="card"><span className="ck">Elapsed</span><span className="cv">{clock(end - t0)}</span><span className="cs">{live ? "still going" : v.status === "idle" ? "waiting for your next message" : "session ended"}</span></div>
        <div className="card"><span className="ck">Working in parallel</span><span className="cv">{st.avg.toFixed(1)}</span><span className="cs">agents on average, peak {st.peak}</span></div>
        <div className="card"><span className="ck">Time agents were blocked</span><span className="cv" style={{ color: st.waiting ? "var(--warn)" : undefined }}>{clock(st.waiting)}</span><span className="cs">{st.most ? `most: ${st.most[0]} (${fmtDur(st.most[1])})` : "nobody was blocked"}</span></div>
        <div className="card"><span className="ck">Slowest chain</span><span className="cv" style={{ fontSize: 15, lineHeight: "26px" }}>{chain.map((c, i) => (i ? " → " : "") + (i >= 2 ? `#${c.split("#")[1]}` : c)).join("")}</span><span className="cs">speeding up the last one ends the session sooner</span></div>
      </div>

      <div className="grid">
        <div className="grid-in">
          <div className="trow head">
            <span className="tlab mono dim" style={{ fontSize: 11 }}>agent</span>
            <div className="ttr mono" style={{ fontSize: 11, color: "var(--muted)" }}>
              {ticks.map((t, i) => (
                <span key={t} style={{ position: "absolute", top: 10, left: pct(t0 + t), transform: i ? "translateX(-50%)" : undefined }}>{clock(t)}</span>
              ))}
            </div>
            <span className="tcost" style={{ fontSize: 11 }}>cost</span>
          </div>

          <div aria-hidden="true" style={{ position: "absolute", left: 260, right: 72, top: HEAD, bottom: 0, pointerEvents: "none", zIndex: 1 }}>
            {ticks.slice(1).map((t) => <span key={t} style={{ position: "absolute", top: 0, bottom: 0, left: pct(t0 + t), borderLeft: "1px solid var(--line)" }} />)}
            {live && (
              <>
                <span style={{ position: "absolute", top: -6, bottom: 0, left: pct(now), borderLeft: "1.5px solid var(--run)" }} />
                <span className="mono" style={{ position: "absolute", top: -HEAD + 4, left: pct(now), transform: "translateX(-50%)", fontSize: 10.5, padding: "1px 5px", borderRadius: 4, background: "var(--run)", color: "#0c0d0f", fontWeight: 700 }}>now</span>
              </>
            )}
            {v.msgs.filter((m) => idx.has(m.from) && idx.has(m.to)).map((m, i) => {
              const a = idx.get(m.from)!, b = idx.get(m.to)!;
              const top = Math.min(a, b) * ROW + ROW / 2, h = Math.abs(a - b) * ROW;
              return (
                <span key={i}>
                  <span style={{ position: "absolute", left: pct(m.ts), top, height: h, borderLeft: "1.5px solid var(--you)" }} />
                  <span style={{ position: "absolute", left: pct(m.ts), top: b < a ? top - 6 : top + h - 2, marginLeft: -4, border: "4px solid transparent", [b < a ? "borderBottomColor" : "borderTopColor"]: "var(--you)" }} />
                </span>
              );
            })}
          </div>

          {rows.map(({ a, level }) => {
            const color = kindColor(a.info.kind);
            const solid = a.info.depth >= 2 ? `color-mix(in srgb, ${color} 72%, #fff)` : color;
            const segs = segments(a, end);
            const firstRun = segs.findIndex((s) => s.status === "running");
            const last = a.log[a.log.length - 1];
            return (
              <div key={a.info.id} className={"trow" + (sel === a.info.id ? " sel" : "")} onClick={() => setSel(sel === a.info.id ? null : a.info.id)} role="button" tabIndex={0} aria-pressed={sel === a.info.id} onKeyDown={(e) => { if (e.key === "Enter") setSel(a.info.id); }}>
                <span className="tlab" style={{ paddingLeft: 12 + level * 22 }}>
                  <span className="nd" style={{ background: color, opacity: a.info.depth >= 2 ? 0.65 : 1 }} />
                  <span className="nn">{a.info.depth >= 2 ? `#${a.info.id.split("#")[1]}` : label(a.info.id)}</span>
                  <span className="role">{a.info.depth === 0 ? "lead" : a.info.depth >= 2 ? "sub" : a.info.role}</span>
                  {a.info.depth >= 1 && <span className="dim" style={{ fontSize: 12, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{a.info.task}</span>}
                </span>
                <div className="ttr">
                  {a.info.depth > 0 && <span className="spawnm" style={{ left: pct(a.addedAt) }} />}
                  {segs.map((s, i) =>
                    s.status === "running" ? (
                      <span key={i} className="seg" style={{ left: pct(s.from), width: wpct(s.from, s.to), background: solid }} title={`running ${fmtDur(s.to - s.from)}`}>{i === firstRun && s.to - s.from > span / 14 ? a.info.role : ""}</span>
                    ) : s.status === "waiting" ? (
                      <span key={i} className="seg wait" style={{ left: pct(s.from), width: wpct(s.from, s.to) }} title={`waiting ${fmtDur(s.to - s.from)}`}>{s.to - s.from > span / 12 ? "waiting" : ""}</span>
                    ) : s.status === "idle" ? (
                      <span key={i} className="seg idle" style={{ left: pct(s.from), width: wpct(s.from, s.to), background: `color-mix(in srgb, ${color} 45%, transparent)` }} />
                    ) : s.status === "queued" ? (
                      <span key={i} className="seg queued" style={{ left: pct(s.from), width: wpct(s.from, s.to) }} />
                    ) : null,
                  )}
                  {a.errors.map((t, i) => <span key={"e" + i} className="mark" style={{ left: pct(t) }} title="a step failed" />)}
                  {last.status === "done" && <span className="mono ok" style={{ position: "absolute", top: 10, left: `calc(${pct(last.ts)} + 4px)`, fontSize: 12 }}>✓</span>}
                  {(last.status === "failed" || last.status === "killed") && a.log.some((l) => l.status === "running") && <span className="mono bad" style={{ position: "absolute", top: 10, left: `calc(${pct(last.ts)} + 4px)`, fontSize: 12 }}>✗</span>}
                  {a.status === "running" && live && <span className="pulse" style={{ position: "absolute", top: 14, left: pct(now), width: 10, height: 10, marginLeft: -5, borderRadius: "50%", background: "var(--run)", zIndex: 2 }} />}
                </div>
                <span className="tcost">{a.cost ? `$${a.cost.toFixed(2)}` : "–"}</span>
              </div>
            );
          })}
        </div>
      </div>

      <div style={{ display: "flex", gap: 16, flexWrap: "wrap", alignItems: "flex-start" }}>
        <div className="mono" style={{ display: "flex", flexWrap: "wrap", gap: 16, fontSize: 11, color: "var(--muted)", flex: 1, minWidth: 260 }}>
          <span style={{ display: "inline-flex", gap: 6, alignItems: "center" }}><span style={{ width: 18, height: 10, borderRadius: 3, background: "var(--codex)" }} />working</span>
          <span style={{ display: "inline-flex", gap: 6, alignItems: "center" }}><span style={{ width: 18, height: 2, background: "color-mix(in srgb, var(--claude) 45%, transparent)" }} />lead between turns</span>
          <span style={{ display: "inline-flex", gap: 6, alignItems: "center" }}><span className="seg wait" style={{ position: "static", width: 18, height: 10, padding: 0 }} />waiting</span>
          <span style={{ display: "inline-flex", gap: 6, alignItems: "center" }}><span style={{ width: 18, borderTop: "1px dashed var(--line2)" }} />queued</span>
          <span style={{ display: "inline-flex", gap: 6, alignItems: "center" }}><span style={{ width: 2, height: 12, background: "var(--bad)" }} />failed step</span>
          <span style={{ display: "inline-flex", gap: 6, alignItems: "center" }}><span style={{ height: 12, borderLeft: "1.5px solid var(--you)" }} />message between agents</span>
        </div>
        {selA ? (
          <div style={{ flex: "0 1 420px", minWidth: 260, border: "1px solid var(--line)", borderRadius: 12, background: "var(--pane2)", padding: "12px 14px", display: "flex", flexDirection: "column", gap: 6, fontSize: 13 }}>
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <span className="nd" style={{ background: kindColor(selA.info.kind) }} />
              <span style={{ fontWeight: 600 }}>{selA.info.id}</span><span className="role">{selA.info.role}</span>
              <span className="ns"><StatusText a={selA} /></span>
            </div>
            <span style={{ color: "var(--text)" }}>{selA.info.task}</span>
            <span className="mono dim" style={{ fontSize: 11 }}>
              {[fmtDur(activeMs(selA, end)), `${selA.tokens.toLocaleString()} tok`, `$${selA.cost.toFixed(2)}`, selA.info.parent ? `started by ${selA.info.parent}` : "started by you", selA.note].filter(Boolean).join(" · ")}
            </span>
            <div style={{ display: "flex", gap: 8, marginTop: 4 }}>
              <button className="btn" onClick={() => onOpen(selA.info.id)}>open its chat</button>
              {!FINISHED.includes(selA.status) && selA.info.depth > 0 && <button className="btn btn-bad" onClick={() => onKill(selA.info.id)}>stop it</button>}
            </div>
          </div>
        ) : (
          <span className="dim" style={{ fontSize: 12 }}>Click a row for details.</span>
        )}
      </div>
    </div>
  );
}
