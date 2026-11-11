"use client";

import { useEffect, useRef, useState, type ReactNode } from "react";
import { agentColor, kindColor, pending, subtree, type Approval, type Line, type View } from "./reduce";

type Props = {
  v: View;
  focus: string | null;
  onFocus: (id: string | null) => void;
  hideTools: boolean;
  answer: (a: Approval, answer: "yes" | "no" | "edit", note?: string) => Promise<void>;
};

const speaker = (l: Line) =>
  l.k === "say" || l.k === "tool" ? l.agent : l.k === "assign" || l.k === "spawn" ? l.by : l.k === "msg" ? l.from : l.k === "user" ? "you" : l.k;

export function Transcript({ v, focus, onFocus, hideTools, answer }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const scope = focus ? subtree(v, focus) : null;

  const lines = v.lines.filter((l) => {
    if (hideTools && l.k === "tool" && l.status !== "error") return false;
    if (!scope) return true;
    switch (l.k) {
      case "say": case "tool": return scope.has(l.agent);
      case "assign": case "spawn": return scope.has(l.id) || scope.has(l.by);
      case "msg": return scope.has(l.from) || scope.has(l.to);
      case "user": return !!l.to && scope.has(l.to);
      case "approval": return scope.has(v.approvals.get(l.id)?.by ?? "");
      default: return false;
    }
  });

  // Follow new output unless you scrolled up to read.
  useEffect(() => {
    const el = ref.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [lines.length, v.lastTs]);

  const Ag = ({ id }: { id: string }) => (
    <button className="ag" style={{ color: agentColor(v, id) }} onClick={() => onFocus(id)}>{id}</button>
  );
  const Tag = ({ id }: { id: string }) => {
    const sub = (v.agents.get(id)?.info.depth ?? 0) >= 2;
    return <span className={"tag" + (sub ? " sub" : "")} style={{ color: agentColor(v, id) }}>{sub ? `#${id.split("#")[1]}` : id}</span>;
  };

  const out: ReactNode[] = [];
  let prev = "";
  for (const l of lines) {
    const who = speaker(l);
    if (prev && who !== prev && l.k !== "tool") out.push(<div className="gap" key={l.key + "g"} />);
    prev = who;
    switch (l.k) {
      case "user":
        out.push(
          <div className="l" key={l.key}>
            <span className="tag c-you">you ›</span>
            <span className="txt ink">{l.to && <><span className="dim">→ </span><Ag id={l.to} /> </>}{l.text}</span>
          </div>,
        );
        break;
      case "say":
        out.push(<div className="l" key={l.key}><Tag id={l.agent} /><span className="txt ink">{l.text}</span></div>);
        break;
      case "tool":
        out.push(
          <div className="l" key={l.key}>
            <Tag id={l.agent} />
            <span className="txt">
              {l.status === "running" ? <span className="run pulse">●</span> : l.status === "ok" ? <span className="ok">✓</span> : <span className="bad">✗</span>}{" "}
              <span style={{ display: "inline-block", minWidth: "6ch" }}>{l.verb}</span> {l.target}
              {l.status === "running" && <> <span className="shim">working</span></>}
              {l.detail && <span className={l.status === "error" ? "bad" : "dim"}>  {l.detail}</span>}
            </span>
          </div>,
        );
        break;
      case "assign":
      case "spawn":
        out.push(
          <div className="l" key={l.key}>
            <Tag id={l.by} />
            <span className="txt" style={{ display: "inline-flex", gap: 8, flexWrap: "wrap", alignItems: "baseline" }}>
              <span className="dim">⤷ {l.k === "assign" ? (l.queued ? "queue " : "assign") : "spawn "}</span>
              <Ag id={l.id} /><span className="dim">{l.role}</span><span>{l.task}</span>
            </span>
          </div>,
        );
        break;
      case "msg":
        out.push(
          <div className="l" key={l.key}>
            <Tag id={l.from} />
            <span className="txt"><span className="dim">→</span> {l.to === "you" ? <span className="c-you" style={{ fontWeight: 700 }}>you</span> : <Ag id={l.to} />} <span className="ink">{l.text}</span></span>
          </div>,
        );
        break;
      case "lead":
        out.push(<div className="l" key={l.key}><span className="tag dim">·</span><span className="txt dim">{l.from} handed the lead to <span style={{ color: agentColor(v, l.to), fontWeight: 700 }}>{l.to}</span></span></div>);
        break;
      case "log":
        out.push(<div className="l" key={l.key}><span className="tag dim">·</span><span className={"txt " + (l.level === "error" ? "bad" : l.level === "warn" ? "warn" : "dim")}>{l.text}</span></div>);
        break;
      case "approval": {
        const a = v.approvals.get(l.id);
        if (a) out.push(<div className="ind" key={l.key} style={{ margin: "6px 0" }}><ApprovalBox v={v} a={a} answer={answer} /></div>);
        break;
      }
    }
  }

  // What is happening right now, under the last line.
  const busy = [...v.agents.values()].filter((a) => a.status === "running" && (!scope || scope.has(a.info.id)) && !a.doing);
  return (
    <div
      className="term"
      ref={ref}
      onScroll={(e) => {
        const el = e.currentTarget;
        stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 60;
      }}
      aria-live="polite"
    >
      {out}
      {busy.length > 0 && v.status !== "stopped" && v.status !== "failed" && (
        <>
          <div className="gap" />
          {busy.map((a) => (
            <div className="l" key={"busy" + a.info.id}><Tag id={a.info.id} /><span className="shim">thinking</span></div>
          ))}
        </>
      )}
      {(v.status === "stopped" || v.status === "failed") && (
        <><div className="gap" /><div className="l"><span className="tag dim">·</span><span className={v.status === "failed" ? "bad" : "dim"}>session {v.status}{v.reason ? `: ${v.reason}` : ""}</span></div></>
      )}
    </div>
  );
}

function ApprovalBox({ v, a, answer }: { v: View; a: Approval; answer: Props["answer"] }) {
  const [editing, setEditing] = useState(false);
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const open = !a.answer;
  const go = async (x: "yes" | "no" | "edit", n?: string) => {
    setBusy(true);
    await answer(a, x, n);
    setBusy(false);
  };
  const state = a.answer === "yes" ? <span className="ok">✓ approved</span>
    : a.answer === "no" ? <span className="bad">✗ declined{a.note ? `: ${a.note}` : ""}</span>
    : a.answer === "edit" ? <span className="warn">✎ changes requested: {a.note}</span>
    : <span className="dim">· needs your ok</span>;

  return (
    <div className="box" style={open ? { borderColor: "color-mix(in srgb, var(--warn) 45%, transparent)" } : { opacity: 0.75 }}>
      <div className="box-h">
        <span className="warn">◆</span>
        <span className="ink" style={{ fontWeight: 700 }}>{a.title}</span>
        {state}
      </div>
      {a.summary && <div style={{ padding: "0 14px 10px", color: "var(--text)" }}>{a.summary}</div>}
      {a.items?.map((it, i) => (
        <div className="srow" key={it.id}>
          <span className="dim">{i + 1}</span>
          <span style={{ color: kindColor(it.agent), fontWeight: 700, whiteSpace: "nowrap" }}>{it.id}</span>
          <span className="dim hide-sm2">{it.role}</span>
          <span className="ink">{it.task}</span>
          <span className="dim hide-sm2" style={{ fontSize: 11 }}>{it.after?.length ? `after ${it.after.join(", ")}` : "starts now"}</span>
        </div>
      ))}
      {a.detail && <pre style={{ margin: 0, padding: "8px 14px", borderTop: "1px solid var(--line)", whiteSpace: "pre-wrap", font: "12px/1.6 var(--mono)", color: "var(--text)" }}>{a.detail}</pre>}
      {open && (
        <div className="box-f">
          {!editing ? (
            <>
              <button className="btn btn-p" disabled={busy} onClick={() => go("yes")}><kbd style={{ borderColor: "rgba(0,0,0,.25)", color: "#0c0d0f" }}>y</kbd> {a.kind === "merge" ? "merge" : a.kind === "command" || a.kind === "file" ? "allow" : "start"}</button>
              {a.kind === "split" && <button className="btn" disabled={busy} onClick={() => setEditing(true)}><kbd>e</kbd> edit split</button>}
              <button className="btn" disabled={busy} onClick={() => go("no")}><kbd>n</kbd> {a.kind === "merge" ? "keep on branch" : a.kind === "spawn" ? "decline" : a.kind === "command" || a.kind === "file" ? "deny" : "cancel"}</button>
            </>
          ) : (
            <form
              style={{ display: "flex", gap: 8, flex: 1, flexWrap: "wrap" }}
              onSubmit={(e) => { e.preventDefault(); if (note.trim()) void go("edit", note.trim()); }}
            >
              <input
                autoFocus
                className="field"
                style={{ flex: 1, minWidth: 200 }}
                placeholder={`What should ${v.leadId} change in the split?`}
                value={note}
                onChange={(e) => setNote(e.target.value)}
                aria-label="Changes to the split"
              />
              <button className="btn btn-p" disabled={busy || !note.trim()}>send</button>
              <button type="button" className="btn" onClick={() => setEditing(false)}>back</button>
            </form>
          )}
        </div>
      )}
    </div>
  );
}

export function Prompt({ v, focus, send }: { v: View; focus: string | null; send: (text: string) => Promise<string | null> }) {
  const [text, setText] = useState("");
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);
  const ended = v.status === "stopped" || v.status === "failed";
  const asking = pending(v).length > 0;
  const lead = v.leadId;

  const submit = async () => {
    const t = text.trim();
    if (!t || busy) return;
    setBusy(true);
    const full = focus && !t.startsWith("/") && !t.startsWith("@") && !(asking && /^(y|n|e)\b/i.test(t)) ? `@${focus} ${t}` : t;
    const e = await send(full);
    setBusy(false);
    if (e) setErr(e);
    else { setText(""); setErr(""); }
  };

  const placeholder = ended ? "This session has ended"
    : asking ? "y to approve · n to decline · e <what to change>"
    : focus ? `Message only ${focus}`
    : `Message ${lead} (lead) · @codex#1 to talk to one agent · /help`;

  return (
    <div className="prompt">
      <div className="pbox" style={focus ? { borderColor: `color-mix(in srgb, ${agentColor(v, focus)} 50%, transparent)` } : undefined}>
        {focus ? <span className="ag mono" style={{ color: agentColor(v, focus), fontSize: 12, lineHeight: "22px" }}>@{focus}</span>
          : <span className="mono c-you" style={{ fontWeight: 700, lineHeight: "22px" }}>›</span>}
        <textarea
          rows={1}
          value={text}
          disabled={ended}
          placeholder={placeholder}
          aria-label={placeholder}
          onChange={(e) => {
            setText(e.target.value);
            e.target.style.height = "auto";
            e.target.style.height = e.target.scrollHeight + "px";
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); void submit(); }
          }}
        />
        <button className="btn btn-p" onClick={submit} disabled={busy || ended || !text.trim()}>send ↵</button>
      </div>
      {err && <div className="err">{err}</div>}
      <div className="hints">
        <span><kbd>/orc {v.agents.get(lead)?.info.kind === "claude" ? "codex" : "claude"}</kbd> switch lead</span>
        <span><kbd>@agent</kbd> talk to one agent</span>
        <span className="hide-sm"><kbd>/kill id</kbd> stop one agent</span>
        <span className="hide-sm"><kbd>/merge</kbd></span>
        <span><kbd>/stop</kbd> stop all</span>
      </div>
    </div>
  );
}

