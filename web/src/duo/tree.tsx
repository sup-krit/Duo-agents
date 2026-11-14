

import type { ReactNode } from "react";
import { activeMs, fmtDur, kindColor, label, pathTo, type AgentView, type View } from "./reduce";

export function StatusText({ a }: { a: AgentView }) {
  switch (a.status) {
    case "running": return <><span className="run pulse">●</span> running</>;
    case "waiting": return <span className="warn">● waiting</span>;
    case "queued": return <>queued</>;
    case "proposed": return <>proposed</>;
    case "idle": return <>idle</>;
    case "done": return <><span className="ok">✓</span> done</>;
    case "failed": return <span className="bad">✗ failed</span>;
    case "killed": return <>stopped</>;
  }
}

function Node({ v, a, focus, onFocus, now, dim }: { v: View; a: AgentView; focus: string | null; onFocus: (id: string) => void; now: number; dim: boolean }) {
  const isLead = a.info.depth === 0;
  const subs = a.children.filter((c) => { const s = v.agents.get(c)?.status; return s && s !== "proposed" && s !== "killed"; }).length;
  const dur = activeMs(a, now);
  const cls = ["node", a.status === "proposed" ? "prop" : "", focus === a.info.id ? "sel" : "", dim ? "fade" : ""].join(" ");
  const border = focus === a.info.id ? undefined
    : a.status === "waiting" ? "color-mix(in srgb, var(--warn) 45%, transparent)"
    : isLead && a.info.id === v.leadId ? `color-mix(in srgb, ${kindColor(a.info.kind)} 50%, transparent)`
    : undefined;
  return (
    <button className={cls} style={border ? { borderColor: border } : undefined} onClick={() => onFocus(a.info.id)} aria-pressed={focus === a.info.id}>
      <span className="nh">
        <span className="nd" style={{ background: kindColor(a.info.kind), opacity: a.info.depth >= 2 ? 0.65 : 1 }} />
        <span className="nn">{a.info.depth >= 2 ? `#${a.info.id.split("#")[1]}` : label(a.info.id)}</span>
        <span className="role">{isLead ? (a.info.id === v.leadId ? "lead" : "former lead") : a.info.depth >= 2 ? "sub" : a.info.role}</span>
        <span className="ns"><StatusText a={a} /></span>
      </span>
      <span className="nt">{a.status === "running" && a.doing ? a.doing : a.info.task}</span>
      {a.status !== "proposed" && (
        <span className="nm">
          {[subs ? `${subs} sub${subs > 1 ? "s" : ""}` : null, dur ? fmtDur(dur) : null, a.cost ? `$${a.cost.toFixed(2)}` : null, a.note && a.status !== "running" ? a.note : null]
            .filter(Boolean)
            .join(" · ") || "—"}
        </span>
      )}
    </button>
  );
}

export function Tree({ v, focus, onFocus, now, footer }: { v: View; focus: string | null; onFocus: (id: string) => void; now: number; footer?: ReactNode }) {
  const onPath = focus ? new Set(pathTo(v, focus)) : null;
  const render = (id: string): ReactNode => {
    const a = v.agents.get(id);
    if (!a) return null;
    // Rejected or replaced proposals just clutter the tree.
    if (a.status === "killed" && a.log.every((l) => l.status === "proposed" || l.status === "killed")) return null;
    const dim = !!onPath && !onPath.has(id) && !(focus && pathTo(v, id).includes(focus));
    return (
      <div className="kid" key={id}>
        <Node v={v} a={a} focus={focus} onFocus={onFocus} now={now} dim={dim} />
        {a.children.length > 0 && <div className="kids">{a.children.map(render)}</div>}
      </div>
    );
  };
  const live = [...v.agents.values()].filter((a) => a.status !== "proposed" && a.status !== "killed").length;
  const proposed = [...v.agents.values()].filter((a) => a.status === "proposed").length;
  const maxDepth = Math.max(0, ...[...v.agents.values()].map((a) => a.info.depth));

  return (
    <>
      <div style={{ display: "flex", alignItems: "baseline", justifyContent: "space-between", gap: 8 }}>
        <span style={{ fontWeight: 600, fontSize: 14 }}>Agent tree</span>
        <span className="mono dim" style={{ fontSize: 11 }}>
          {live} agent{live === 1 ? "" : "s"}{proposed ? ` · ${proposed} proposed` : ""} · depth {maxDepth} · ${v.cost.toFixed(2)}
        </span>
      </div>
      <div>
        <div className={"node ghost" + (onPath ? " fade" : "")}>
          <span className="nh"><span className="nd" style={{ background: "var(--you)" }} /><span className="nn">You</span></span>
        </div>
        <div className="kids">{v.roots.map(render)}</div>
      </div>
      {footer}
    </>
  );
}
