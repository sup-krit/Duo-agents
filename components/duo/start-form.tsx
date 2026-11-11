"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import type { PublicProvider } from "@/lib/providers";
import type { Kind, Mode, Permission, SessionSummary } from "@/lib/types";
import { OtherAI } from "./providers";
import { kindColor } from "./reduce";

/** Protected patterns are remembered per project folder (they are not secret). */
const protectKey = (repo: string) => `duo.protect:${repo.trim().replace(/\\/g, "/").replace(/\/$/, "").toLowerCase()}`;
const store = {
  get: (k: string) => { try { return localStorage.getItem(k); } catch { return null; } },
  set: (k: string, v: string) => { try { localStorage.setItem(k, v); } catch { /* private window */ } },
};

/** Shortcuts for "what does this AI do"; the text stays editable. */
const JOBS: [string, string][] = [
  ["build", "build: write and change the code"],
  ["tests", "tests: write and run the tests"],
  ["review", "review only: read the changes and report problems, do not edit files"],
  ["docs", "docs: write and update documentation"],
  ["research", "research: read the code and find answers, do not edit files"],
];
const DEFAULT_ROLES: Record<string, string> = { codex: JOBS[0][1], claude: JOBS[2][1] };

const MODES: { k: Permission; name: string; blurb: string }[] = [
  { k: "auto", name: "auto", blurb: "agents edit files and run commands on their own" },
  { k: "acceptEdits", name: "accept edits", blurb: "agents edit files; every command waits for your yes" },
  { k: "plan", name: "plan", blurb: "read-only until you approve the lead's plan" },
];

const Check = () => (
  <svg width="8" height="8" viewBox="0 0 8 8" aria-hidden="true"><path d="M1.5 4.2l1.6 1.6L6.5 2.2" fill="none" stroke="#0c0d0f" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" /></svg>
);

function Toggle({ on, set, children }: { on: boolean; set: (v: boolean) => void; children: React.ReactNode }) {
  return (
    <button type="button" className="tog" aria-pressed={on} onClick={() => set(!on)}>
      <span className="cb">{on && <Check />}</span>{children}
    </button>
  );
}

const STATUS_COLOR: Record<string, string> = { running: "var(--run)", waiting: "var(--warn)", idle: "var(--ok)", failed: "var(--bad)", stopped: "var(--muted)" };

export function StartForm({ sessions }: { sessions: SessionSummary[] }) {
  const router = useRouter();
  const [lead, setLead] = useState<Kind>("claude");
  const [team, setTeam] = useState<Set<Kind>>(new Set(["codex", "claude"]));
  const [providers, setProviders] = useState<PublicProvider[]>([]);
  const [permission, setPermission] = useState<Permission>("auto");
  const [afterPlan, setAfterPlan] = useState<"auto" | "acceptEdits">("auto");
  const [protect, setProtect] = useState("");
  const [roles, setRoles] = useState<Record<string, string>>(DEFAULT_ROLES);
  useEffect(() => { const v = store.get("duo.roles"); if (v) try { setRoles(JSON.parse(v)); } catch { /* ignore */ } }, []);
  const setRole = (k: Kind, v: string) => setRoles((r) => ({ ...r, [k]: v }));
  const [subs, setSubs] = useState(true);
  const [maxDepth, setMaxDepth] = useState(2);
  const [maxAgents, setMaxAgents] = useState(8);
  const [budget, setBudget] = useState(5);
  const [askSplit, setAskSplit] = useState(true);
  const [askMerge, setAskMerge] = useState(true);
  const [askSpawn, setAskSpawn] = useState(false);
  const [mode, setMode] = useState<Mode>("mock");
  const [repo, setRepo] = useState("");
  const [direct, setDirect] = useState(false);
  const [task, setTask] = useState("Add a 60s cache to getUsers() with tests. Keep the public API the same.");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");

  useEffect(() => {
    fetch("/api/providers").then((r) => r.json()).then((p) => Array.isArray(p) && setProviders(p)).catch(() => {});
  }, []);
  // A removed provider leaves the team and the lead.
  useEffect(() => {
    const ids = new Set(["claude", "codex", ...providers.map((p) => p.id)]);
    setTeam((t) => new Set([...t].filter((k) => ids.has(k))));
    setLead((l) => (ids.has(l) ? l : "claude"));
  }, [providers]);
  const loadProtect = () => { const v = store.get(protectKey(repo)); if (v !== null) setProtect(v); };
  const toggle = (k: Kind) => (on: boolean) => setTeam((t) => { const n = new Set(t); if (on) n.add(k); else n.delete(k); return n; });

  const start = async () => {
    setBusy(true);
    setErr("");
    const res = await fetch("/api/sessions", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        lead, mode, repo, task, workspace: direct ? "direct" : "auto", maxDepth, maxAgents, budgetUsd: budget, subAgents: subs,
        team: [...team],
        ask: { split: askSplit, merge: askMerge, spawn: askSpawn },
        permission, afterPlan,
        roles: Object.fromEntries([...team].map((k) => [k, roles[k] ?? ""])),
        protect: mode === "real" ? protect.split("\n").map((l) => l.trim()).filter(Boolean) : [],
      }),
    });
    if (mode === "real" && repo.trim()) store.set(protectKey(repo), protect);
    store.set("duo.roles", JSON.stringify(roles));
    const data = await res.json().catch(() => ({}));
    if (!res.ok) { setErr(data.error ?? "Could not start"); setBusy(false); return; }
    router.push(`/s/${data.id}`);
  };

  const nameOf = (k: Kind) => (k === "claude" ? "Claude" : k === "codex" ? "Codex" : providers.find((p) => p.id === k)?.label ?? k);
  const Lead = ({ k, blurb, extra }: { k: Kind; blurb: string; extra?: string }) => {
    const on = lead === k;
    const c = kindColor(k);
    return (
      <button type="button" role="radio" aria-checked={on} className="opt" onClick={() => setLead(k)}
        style={on ? { borderColor: c, background: `color-mix(in srgb, ${c} 8%, transparent)` } : undefined}>
        <span className="radio" style={on ? { border: `4px solid ${c}` } : undefined} />
        <span style={{ color: c, fontWeight: 700, minWidth: 64 }}>{nameOf(k)}</span>
        <span style={{ flex: 1 }}>{blurb}</span>
        {extra && <span className="dim" style={{ fontSize: 11 }}>{extra}</span>}
      </button>
    );
  };

  return (
    <div className="app start">
      <header className="top">
        <span className="brand"><span className="dots"><i style={{ background: "var(--claude)" }} /><i style={{ background: "var(--codex)" }} /></span>agent-duo</span>
        <span className="title">new session</span>
        <div className="tabs" role="radiogroup" aria-label="Mode">
          <button type="button" role="radio" className="tab" aria-checked={mode === "mock"} aria-selected={mode === "mock"} onClick={() => setMode("mock")}>mock (no keys)</button>
          <button type="button" role="radio" className="tab" aria-checked={mode === "real"} aria-selected={mode === "real"} onClick={() => setMode("real")}>real agents</button>
        </div>
      </header>

      <div className="frame">
        <section className="chat">
          <div className="term" style={{ padding: "24px 28px", gap: 18 }}>
            <div className="dim">agent-duo v0.3 · one agent leads, the team does the work, sub-agents split it further · type <span className="h">/help</span> in a session for commands</div>

            {mode === "real" && (
              <div style={{ display: "flex", flexDirection: "column", gap: 8, maxWidth: 640 }}>
                <label className="h" htmlFor="repo">Project folder</label>
                <input id="repo" className="field" placeholder="D:/projects/my-app" value={repo} onChange={(e) => setRepo(e.target.value)} onBlur={loadProtect} />
                <Toggle on={direct} set={setDirect}>edit this folder directly</Toggle>
                <span className="dim" style={{ fontSize: 12 }}>
                  {direct
                    ? "Agents change your files in place as they work. Nothing to approve at the end, and no undo unless the folder is in git."
                    : "Git repo: agents work on a new branch and you approve the merge. Any other folder: agents work on a copy, and changed files are copied back when you approve. node_modules and venvs are linked, not copied."}
                </span>
              </div>
            )}

            <div role="radiogroup" aria-label="Who leads" style={{ display: "flex", flexDirection: "column", gap: 8, maxWidth: 640 }}>
              <span className="h">Who leads this session?</span>
              <span className="dim" style={{ fontSize: 12 }}>The lead plans, splits the task, hands parts to the team and checks the results. Switch any time with <span className="h">/orc</span>.</span>
              <Lead k="claude" blurb="careful planner and reviewer" extra="recommended" />
              <Lead k="codex" blurb="fast, hands-on, likes to just start building" />
              {providers.map((p) => <Lead key={p.id} k={p.id} blurb={`${p.model} · ${p.engine === "claude" ? "on Claude Code" : "on Codex"}`} />)}
            </div>

            <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
              <span className="h">Team it can hand work to</span>
              <span className="dim" style={{ fontSize: 12 }}>Say what each AI does. The lead must split the work to match; leave a box empty to let the lead decide.</span>
              <div style={{ display: "flex", flexDirection: "column", gap: 8, maxWidth: 640 }}>
                {["codex", "claude", ...providers.map((p) => p.id)].map((k) => (
                  <div key={k} style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                    <Toggle on={team.has(k)} set={toggle(k)}><span style={{ color: kindColor(k), fontWeight: 700 }}>{nameOf(k)}</span></Toggle>
                    {team.has(k) && (
                      <div style={{ display: "flex", flexDirection: "column", gap: 6, paddingLeft: 24 }}>
                        <input className="field" value={roles[k] ?? ""} onChange={(e) => setRole(k, e.target.value)}
                          placeholder="what this AI does (empty: the lead decides)" aria-label={`What ${nameOf(k)} does`} />
                        <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                          {JOBS.map(([short, text]) => (
                            <button key={short} type="button" className="btn" style={{ fontSize: 11, padding: "2px 8px" }} aria-pressed={roles[k] === text} onClick={() => setRole(k, text)}>{short}</button>
                          ))}
                          {roles[k] && <button type="button" className="btn" style={{ fontSize: 11, padding: "2px 8px" }} onClick={() => setRole(k, "")}>clear</button>}
                        </div>
                      </div>
                    )}
                  </div>
                ))}
              </div>
            </div>

            <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
              <span className="h">Sub-agents</span>
              <div style={{ display: "flex", gap: 16, flexWrap: "wrap", alignItems: "center", fontSize: 12 }}>
                <Toggle on={subs} set={setSubs}>agents may spawn their own</Toggle>
                <label style={{ display: "inline-flex", gap: 8, alignItems: "center", opacity: subs ? 1 : 0.4 }}>max depth <input className="num" type="number" min={2} max={4} value={maxDepth} disabled={!subs} onChange={(e) => setMaxDepth(Number(e.target.value))} /></label>
                <label style={{ display: "inline-flex", gap: 8, alignItems: "center" }}>max agents <input className="num" type="number" min={1} max={20} value={maxAgents} onChange={(e) => setMaxAgents(Number(e.target.value))} /></label>
                <label style={{ display: "inline-flex", gap: 8, alignItems: "center" }}>budget $ <input className="num" type="number" min={0} step={0.5} value={budget} onChange={(e) => setBudget(Number(e.target.value))} /></label>
              </div>
            </div>

            <div role="radiogroup" aria-label="Mode" style={{ display: "flex", flexDirection: "column", gap: 8, maxWidth: 640 }}>
              <span className="h">Mode</span>
              {MODES.map((m) => {
                const on = permission === m.k;
                return (
                  <button key={m.k} type="button" role="radio" aria-checked={on} className="opt" onClick={() => setPermission(m.k)}
                    style={on ? { borderColor: "var(--you)", background: "color-mix(in srgb, var(--you) 8%, transparent)" } : undefined}>
                    <span className="radio" style={on ? { border: "4px solid var(--you)" } : undefined} />
                    <span style={{ fontWeight: 700, minWidth: 96 }}>{m.name}</span>
                    <span style={{ flex: 1 }}>{m.blurb}</span>
                  </button>
                );
              })}
              {permission === "plan" && (
                <label style={{ display: "inline-flex", gap: 8, alignItems: "center", fontSize: 12 }}>
                  after you approve the plan
                  <select className="field" style={{ width: "auto" }} value={afterPlan} onChange={(e) => setAfterPlan(e.target.value === "acceptEdits" ? "acceptEdits" : "auto")}>
                    <option value="auto">auto</option>
                    <option value="acceptEdits">accept edits</option>
                  </select>
                </label>
              )}
              <span className="dim" style={{ fontSize: 12 }}>Switch any time with <span className="h">/mode</span>. Codex cannot stop to ask, so in accept edits Codex agents are read-only.</span>
            </div>

            {mode === "real" && (
              <div style={{ display: "flex", flexDirection: "column", gap: 8, maxWidth: 640 }}>
                <label className="h" htmlFor="protect">Protected files</label>
                <span className="dim" style={{ fontSize: 12 }}>One pattern per line, from the project root. A pattern without <span className="mono">/</span> matches in any folder. Agents must get your yes before changing these; leave empty to allow everything. Remembered for this folder.</span>
                <textarea id="protect" className="field mono" rows={3} value={protect} onChange={(e) => setProtect(e.target.value)} placeholder={".env*\ndb/migrations/**\nsrc/payment/**"} />
              </div>
            )}

            {mode === "real" && <OtherAI providers={providers} setProviders={setProviders} />}

            <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
              <span className="h">Ask me before</span>
              <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
                <Toggle on={askSplit} set={setAskSplit}>starting the split</Toggle>
                <Toggle on={askMerge} set={setAskMerge}>merging</Toggle>
                <Toggle on={askSpawn} set={setAskSpawn}>each sub-agent</Toggle>
              </div>
            </div>
          </div>

          <form className="prompt" onSubmit={(e) => { e.preventDefault(); void start(); }}>
            <div className="pbox">
              <span className="mono c-you" style={{ fontWeight: 700, lineHeight: "22px" }}>›</span>
              <textarea
                rows={2}
                value={task}
                onChange={(e) => setTask(e.target.value)}
                onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); void start(); } }}
                placeholder="What should the team do?"
                aria-label="Task for the team"
              />
              <button className="btn btn-p" disabled={busy || !task.trim() || !team.size}>{busy ? "starting…" : "send ↵"}</button>
            </div>
            {err && <div className="err">{err}</div>}
            <div className="hints"><span><kbd>enter</kbd> start</span><span><kbd>shift enter</kbd> new line</span>{mode === "mock" && <span>mock agents follow a script and change no files</span>}</div>
          </form>
        </section>

        <aside className="side">
          <span style={{ fontWeight: 600, fontSize: 14 }}>Sessions</span>
          <div style={{ border: "1px solid var(--line)", borderRadius: 12, overflow: "hidden" }}>
            {sessions.length === 0 && <p className="dim" style={{ margin: 0, padding: "12px", fontSize: 13 }}>None yet. Try one in mock mode to see the flow.</p>}
            {sessions.slice(0, 12).map((s, i) => (
              <Link key={s.id} href={`/s/${s.id}`} className="srun" style={i === 0 ? { borderTop: 0 } : undefined}>
                <span style={{ fontSize: 13, fontWeight: 500, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{s.task.split("\n")[0]}</span>
                <span className="mono dim" style={{ fontSize: 11, display: "flex", gap: 8, flexWrap: "wrap" }}>
                  <span style={{ color: STATUS_COLOR[s.status] }}>● {s.status}</span>
                  <span style={{ color: kindColor(s.lead) }}>{s.lead} lead</span>
                  <span>{s.agents} agents</span><span>${s.costUsd.toFixed(2)}</span><span>{s.mode === "mock" ? "mock" : s.repo.split("/").pop()}</span>
                </span>
              </Link>
            ))}
          </div>
          <div className="legend" style={{ flexDirection: "column", gap: 6, fontFamily: "var(--sans)", fontSize: 12 }}>
            <span className="ink" style={{ fontWeight: 600 }}>How the tree works</span>
            <span>Only the lead hands work to team agents.</span>
            <span>Any agent can split its part into sub-agents, up to the depth you set.</span>
            <span>Click an agent to read just its conversation, or open the timeline.</span>
          </div>
        </aside>
      </div>
    </div>
  );
}
