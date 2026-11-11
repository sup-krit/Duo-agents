// Other AI providers (DeepSeek, local Ollama, …) that join the team as their own agent kind.
// Keys live only in ~/.agent-duo/providers.json on this machine (DUO_HOME to move it):
// never in SessionConfig (that goes to events.jsonl), never sent to the browser in full.

import { chmodSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";

/** claude: Anthropic-format API, runs on Claude Code · codex: OpenAI Responses API, runs on Codex */
export type Engine = "claude" | "codex";

export type Provider = {
  id: string;
  label: string;
  engine: Engine;
  baseUrl: string;
  apiKey: string;
  model: string;
  usdPerMTokIn?: number;
  usdPerMTokOut?: number;
};

/** What the browser sees. */
export type PublicProvider = Omit<Provider, "apiKey"> & { apiKey: string; hasKey: boolean };

const DIR = process.env.DUO_HOME || path.join(os.homedir(), ".agent-duo");
const FILE = path.join(DIR, "providers.json");
export const BUILTIN = ["claude", "codex"];

export function listProviders(): Provider[] {
  if (!existsSync(FILE)) return [];
  try {
    const v = JSON.parse(readFileSync(FILE, "utf8"));
    return Array.isArray(v) ? v : [];
  } catch {
    return [];
  }
}

export const getProvider = (id: string) => listProviders().find((p) => p.id === id);

function save(list: Provider[]) {
  mkdirSync(DIR, { recursive: true });
  writeFileSync(FILE, JSON.stringify(list, null, 2), { mode: 0o600 });
  try { chmodSync(FILE, 0o600); } catch { /* Windows: ACLs of the user profile apply */ }
}

export const mask = (k: string) => (!k ? "" : k.length <= 8 ? "•".repeat(k.length) : `${k.slice(0, 3)}…${k.slice(-4)}`);

export const toPublic = (p: Provider): PublicProvider => ({ ...p, apiKey: mask(p.apiKey), hasKey: !!p.apiKey });

/** Validate and store. An empty apiKey keeps the stored one. Returns an error message or null. */
export function upsertProvider(input: Partial<Provider>): string | null {
  const id = String(input.id ?? "").trim().toLowerCase();
  if (!/^[a-z][a-z0-9-]{0,23}$/.test(id)) return "id: a-z, 0-9 and -, starting with a letter";
  if (BUILTIN.includes(id)) return `"${id}" is built in; pick another id`;
  const engine: Engine = input.engine === "codex" ? "codex" : "claude";
  let baseUrl: URL;
  try { baseUrl = new URL(String(input.baseUrl ?? "")); } catch { return "base URL is not a valid URL"; }
  if (!/^https?:$/.test(baseUrl.protocol)) return "base URL must be http or https";
  const model = String(input.model ?? "").trim();
  if (!model) return "model is required";
  const rate = (v: unknown) => (Number(v) > 0 ? Number(v) : undefined);

  const list = listProviders();
  const old = list.find((p) => p.id === id);
  const p: Provider = {
    id,
    label: String(input.label ?? "").trim() || id,
    engine,
    baseUrl: baseUrl.toString().replace(/\/$/, ""),
    // A stored key is kept only for the same base URL, so a changed URL can never receive the old key.
    apiKey: String(input.apiKey ?? "").trim() || (old && old.baseUrl === baseUrl.toString().replace(/\/$/, "") ? old.apiKey : ""),
    model,
    usdPerMTokIn: rate(input.usdPerMTokIn),
    usdPerMTokOut: rate(input.usdPerMTokOut),
  };
  save([...list.filter((x) => x.id !== id), p]);
  return null;
}

export function deleteProvider(id: string) {
  save(listProviders().filter((p) => p.id !== id));
}

export const engineOf = (kind: string): Engine => (kind === "codex" ? "codex" : kind === "claude" ? "claude" : getProvider(kind)?.engine ?? "claude");

export const knownKinds = () => [...BUILTIN, ...listProviders().map((p) => p.id)];

export const providerCost = (p: Provider, tokensIn: number, tokensOut: number) =>
  (tokensIn * (p.usdPerMTokIn ?? 0) + tokensOut * (p.usdPerMTokOut ?? 0)) / 1_000_000;

/** Same validation as upsert, without saving: for testing what is in the form. */
export function draftProvider(input: Partial<Provider>): Provider | string {
  const id = String(input.id ?? "").trim().toLowerCase();
  const old = getProvider(id);
  let url: string;
  try { url = new URL(String(input.baseUrl ?? "")).toString().replace(/\/$/, ""); } catch { return "base URL is not a valid URL"; }
  const model = String(input.model ?? "").trim();
  if (!model) return "model is required";
  return {
    id, label: id, engine: input.engine === "codex" ? "codex" : "claude", baseUrl: url, model,
    apiKey: String(input.apiKey ?? "").trim() || (old && old.baseUrl === url ? old.apiKey : ""),
  };
}

/** Smallest real request to the provider's own base URL. Returns null when it answered. */
export async function testProvider(p: Provider): Promise<string | null> {
  const body =
    p.engine === "claude"
      ? { url: `${p.baseUrl}/v1/messages`, json: { model: p.model, max_tokens: 1, messages: [{ role: "user", content: "hi" }] } }
      : { url: `${p.baseUrl}/responses`, json: { model: p.model, max_output_tokens: 16, input: "hi" } };
  const headers: Record<string, string> = { "content-type": "application/json" };
  if (p.engine === "claude") {
    headers["anthropic-version"] = "2023-06-01";
    if (p.apiKey) { headers["x-api-key"] = p.apiKey; headers.authorization = `Bearer ${p.apiKey}`; }
  } else if (p.apiKey) headers.authorization = `Bearer ${p.apiKey}`;
  try {
    const res = await fetch(body.url, { method: "POST", headers, body: JSON.stringify(body.json), signal: AbortSignal.timeout(30_000) });
    if (res.ok) return null;
    const text = (await res.text()).slice(0, 300).split(p.apiKey || "\u0000").join("***");
    return `${res.status} ${res.statusText}: ${text}`;
  } catch (err) {
    return (err as Error).message;
  }
}
