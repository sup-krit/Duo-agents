

import { useState } from "react";
import type { PublicProvider } from "../types";
import { kindColor } from "./reduce";

// Starting points only: model names change often, so check them against the provider's docs.
// claude = Anthropic-format API (runs on Claude Code) · codex = OpenAI Responses API (runs on Codex)
const PRESETS: Record<string, { label: string; engine: "claude" | "codex"; baseUrl: string; model: string; local?: boolean }> = {
  deepseek: { label: "DeepSeek", engine: "claude", baseUrl: "https://api.deepseek.com/anthropic", model: "deepseek-chat" },
  kimi: { label: "Kimi", engine: "claude", baseUrl: "https://api.moonshot.ai/anthropic", model: "kimi-k2-turbo-preview" },
  glm: { label: "GLM", engine: "claude", baseUrl: "https://api.z.ai/api/anthropic", model: "glm-4.6" },
  openrouter: { label: "OpenRouter", engine: "claude", baseUrl: "https://openrouter.ai/api", model: "deepseek/deepseek-chat" },
  ollama: { label: "Ollama", engine: "claude", baseUrl: "http://localhost:11434", model: "qwen3-coder", local: true },
  lmstudio: { label: "LM Studio", engine: "codex", baseUrl: "http://localhost:1234/v1", model: "openai/gpt-oss-20b", local: true },
};

type Draft = { id: string; label: string; engine: "claude" | "codex"; baseUrl: string; apiKey: string; model: string; usdPerMTokIn: string; usdPerMTokOut: string };
const EMPTY: Draft = { id: "", label: "", engine: "claude", baseUrl: "", apiKey: "", model: "", usdPerMTokIn: "", usdPerMTokOut: "" };

const call = (method: string, url: string, body?: unknown) =>
  fetch(url, { method, headers: { "Content-Type": "application/json" }, body: body === undefined ? undefined : JSON.stringify(body) }).then(async (r) => ({ ok: r.ok, data: await r.json().catch(() => ({})) }));

export function OtherAI({ providers, setProviders }: { providers: PublicProvider[]; setProviders: (p: PublicProvider[]) => void }) {
  const [d, setD] = useState<Draft | null>(null);
  const [editing, setEditing] = useState<PublicProvider | null>(null);
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const set = (k: keyof Draft, v: string) => setD((x) => (x ? { ...x, [k]: v } : x));

  const preset = (k: string) => {
    const p = PRESETS[k];
    setMsg(null);
    setEditing(null);
    setD(p ? { ...EMPTY, id: k, label: p.label, engine: p.engine, baseUrl: p.baseUrl, model: p.model } : { ...EMPTY });
  };
  const edit = (p: PublicProvider) => {
    setMsg(null);
    setEditing(p);
    setD({ id: p.id, label: p.label, engine: p.engine, baseUrl: p.baseUrl, apiKey: "", model: p.model, usdPerMTokIn: String(p.usdPerMTokIn ?? ""), usdPerMTokOut: String(p.usdPerMTokOut ?? "") });
  };
  const test = async () => {
    if (!d) return;
    setBusy(true);
    const r = await call("POST", "/api/providers/test", d);
    setMsg(r.data.ok ? { ok: true, text: "connected" } : { ok: false, text: r.data.error ?? "failed" });
    setBusy(false);
  };
  const save = async () => {
    if (!d) return;
    setBusy(true);
    const r = await call("PUT", "/api/providers", d);
    setBusy(false);
    if (!r.ok) return setMsg({ ok: false, text: r.data.error ?? "could not save" });
    setProviders(r.data);
    setD(null);
    setEditing(null);
    setMsg(null);
  };
  const remove = async (id: string) => {
    const r = await call("DELETE", "/api/providers", { id });
    if (r.ok) setProviders(r.data);
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8, maxWidth: 640 }}>
      <span className="h">Other AI</span>
      <span className="dim" style={{ fontSize: 12 }}>
        Add DeepSeek, a local model or any other provider as a team member. Keys stay on this machine in <span className="mono">~/.agent-duo/providers.json</span> and are never shown again in full.
      </span>

      {providers.map((p) => (
        <div key={p.id} className="tog" style={{ justifyContent: "space-between", cursor: "default" }}>
          <span style={{ display: "inline-flex", gap: 8, alignItems: "center", minWidth: 0 }}>
            <span className="nd" style={{ background: kindColor(p.id), width: 8, height: 8, borderRadius: 99, display: "inline-block" }} />
            <b style={{ color: kindColor(p.id) }}>{p.label}</b>
            <span className="dim mono" style={{ fontSize: 11, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{p.id} · {p.model} · {p.engine === "claude" ? "Anthropic API" : "Responses API"} · key {p.hasKey ? p.apiKey : "none"}</span>
          </span>
          <span style={{ display: "inline-flex", gap: 6 }}>
            <button type="button" className="btn" onClick={() => edit(p)}>edit</button>
            <button type="button" className="btn" onClick={() => void remove(p.id)}>remove</button>
          </span>
        </div>
      ))}

      {!d ? (
        <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
          {Object.entries(PRESETS).map(([k, p]) => (
            <button key={k} type="button" className="btn" onClick={() => preset(k)} disabled={providers.some((x) => x.id === k)}>+ {p.label}{p.local ? " (local)" : ""}</button>
          ))}
          <button type="button" className="btn" onClick={() => preset("")}>+ other…</button>
        </div>
      ) : (
        <form
          style={{ display: "grid", gridTemplateColumns: "120px 1fr", gap: 8, alignItems: "center", border: "1px solid var(--line)", borderRadius: 12, padding: 12, fontSize: 12 }}
          onSubmit={(e) => { e.preventDefault(); void save(); }}
        >
          <label htmlFor="pv-id">id</label>
          <input id="pv-id" className="field" value={d.id} disabled={!!editing} onChange={(e) => set("id", e.target.value)} placeholder="deepseek" />
          <label htmlFor="pv-label">name</label>
          <input id="pv-label" className="field" value={d.label} onChange={(e) => set("label", e.target.value)} placeholder="DeepSeek" />
          <span>API format</span>
          <span style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
            <button type="button" className="tog" aria-pressed={d.engine === "claude"} onClick={() => set("engine", "claude")}>Anthropic (runs on Claude Code)</button>
            <button type="button" className="tog" aria-pressed={d.engine === "codex"} onClick={() => set("engine", "codex")}>OpenAI Responses (runs on Codex)</button>
          </span>
          <label htmlFor="pv-url">base URL</label>
          <input id="pv-url" className="field" value={d.baseUrl} onChange={(e) => set("baseUrl", e.target.value)} placeholder="https://api.example.com/anthropic" />
          <label htmlFor="pv-key">API key</label>
          <input id="pv-key" className="field" type="password" autoComplete="off" value={d.apiKey} onChange={(e) => set("apiKey", e.target.value)}
            placeholder={editing?.hasKey ? `${editing.apiKey} (leave empty to keep)` : "empty for local servers"} />
          <label htmlFor="pv-model">model</label>
          <input id="pv-model" className="field" value={d.model} onChange={(e) => set("model", e.target.value)} placeholder="check the provider's model list" />
          <span>$ per 1M tokens</span>
          <span style={{ display: "flex", gap: 8, alignItems: "center" }}>
            in <input className="num" type="number" min={0} step={0.01} value={d.usdPerMTokIn} onChange={(e) => set("usdPerMTokIn", e.target.value)} aria-label="USD per 1M input tokens" />
            out <input className="num" type="number" min={0} step={0.01} value={d.usdPerMTokOut} onChange={(e) => set("usdPerMTokOut", e.target.value)} aria-label="USD per 1M output tokens" />
            <span className="dim">optional, for the cost counter</span>
          </span>
          <span />
          <span style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
            <button type="submit" className="btn btn-p" disabled={busy}>save</button>
            <button type="button" className="btn" disabled={busy} onClick={() => void test()}>{busy ? "…" : "test connection"}</button>
            <button type="button" className="btn" onClick={() => { setD(null); setEditing(null); setMsg(null); }}>cancel</button>
            {msg && <span className={msg.ok ? "ok" : "bad"} style={{ wordBreak: "break-word" }}>{msg.text}</span>}
          </span>
        </form>
      )}
    </div>
  );
}
