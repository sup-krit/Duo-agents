import { Codex, type Thread } from "@openai/codex-sdk";
import { providerCost } from "../providers";
import { codexAgents } from "../shared";
import { preview, type Runner, type TurnContext, type TurnResult } from "./types";

const rate = (v?: string) => (Number(v) || 0) / 1_000_000;
/** The SDK's env replaces process.env entirely, so pass it on. */
const env = () => Object.fromEntries(Object.entries(process.env).filter((e): e is [string, string] => e[1] !== undefined));

/** One Codex thread per agent; later turns continue it. The duo tools come in through MCP. */
export function createCodexRunner(): Runner {
  let codex: Codex | null = null;
  let thread: Thread | null = null;
  let policy = ""; // sandbox + approval the thread runs with; a new mode resumes it with new ones

  return {
    async turn(ctx: TurnContext): Promise<TurnResult> {
      const { agent, emit } = ctx;
      const p = ctx.provider;
      if (!codex) {
        const agents = codexAgents(ctx.access === "write" ? "workspace-write" : "read-only"); // shared/agents
        codex = new Codex({
          ...(p ? { env: { ...env(), DUO_PROVIDER_KEY: p.apiKey || "none" } } : {}),
          config: {
            ...(p
              ? {
                  // ponytail: Codex 0.167 speaks only the Responses API (wire_api "chat" was removed)
                  model_providers: { [p.id]: { name: p.label, base_url: p.baseUrl, env_key: "DUO_PROVIDER_KEY", wire_api: "responses" } },
                  model_provider: p.id,
                }
              : {}),
            ...(Object.keys(agents).length ? { agents } : {}), // `agents={}` would hide Codex's built-in roles
            mcp_servers: {
              duo: {
                command: process.execPath,
                args: [ctx.bridge.script, ctx.bridge.baseUrl, ctx.bridge.sessionId, agent.id, ctx.bridge.token, ctx.tools.join(",")],
                startup_timeout_sec: 30,
                tool_timeout_sec: 3600, // propose_split waits for you
              },
            },
          },
        });
      }
      // Codex cannot ask you mid-turn: exec mode has no approvals, "untrusted" was removed in 0.167, and inside the
      // workspace-write sandbox commands never need approval. So it cannot honour "every command waits for your yes"
      // and runs read-only in accept edits as well as in plan mode.
      const readOnly = ctx.access === "read" || ctx.permission() !== "auto";
      const opts = {
        workingDirectory: ctx.cwd,
        model: p?.model || process.env.CODEX_MODEL || undefined,
        sandboxMode: readOnly ? ("read-only" as const) : ("workspace-write" as const),
        approvalPolicy: "never" as const,
        skipGitRepoCheck: true,
      };
      const want = `${opts.sandboxMode}/${opts.approvalPolicy}`;
      if (!thread) thread = codex.startThread(opts);
      else if (want !== policy && thread.id) thread = codex.resumeThread(thread.id, opts);
      policy = want;

      let text = "";
      let tokens = 0;
      let costUsd = 0;
      const { events } = await thread.runStreamed(ctx.prompt, { signal: ctx.signal });

      for await (const ev of events) {
        if (ev.type === "turn.failed") throw new Error(`Codex failed: ${ev.error.message}`);
        if (ev.type === "error") throw new Error(`Codex error: ${ev.message}`);
        if (ev.type === "turn.completed") {
          const u = ev.usage;
          tokens = u.input_tokens + u.output_tokens;
          costUsd = p
            ? providerCost(p, u.input_tokens, u.output_tokens)
            : (u.input_tokens - u.cached_input_tokens) * rate(process.env.CODEX_USD_PER_MTOK_IN) +
              u.output_tokens * rate(process.env.CODEX_USD_PER_MTOK_OUT);
          continue;
        }
        if (ev.type !== "item.started" && ev.type !== "item.completed") continue;
        const item = ev.item;
        const done = ev.type === "item.completed";

        switch (item.type) {
          case "agent_message":
            if (done && item.text.trim()) {
              text = item.text;
              emit({ type: "say", agent: agent.id, text: item.text.trim() });
            }
            break;
          case "command_execution": {
            const failed = item.status === "failed" || (done && item.exit_code !== undefined && item.exit_code !== 0);
            emit({
              type: "tool", agent: agent.id, callId: item.id, verb: "run", target: preview(item.command, 70),
              status: !done ? "running" : failed ? "error" : "ok",
              detail: failed ? preview(item.aggregated_output.split("\n").filter(Boolean).slice(-2).join(" · "), 160) : undefined,
            });
            break;
          }
          case "file_change":
            if (!done) break;
            for (const [i, c] of item.changes.entries())
              emit({
                type: "tool", agent: agent.id, callId: `${item.id}-${i}`,
                verb: c.kind === "add" ? "add" : c.kind === "delete" ? "delete" : "edit", target: c.path,
                status: item.status === "failed" ? "error" : "ok",
              });
            break;
          case "mcp_tool_call":
            if (item.server === "duo") break; // shown by the session as assign/spawn/msg
            emit({
              type: "tool", agent: agent.id, callId: item.id, verb: `${item.server}.${item.tool}`, target: preview(item.arguments, 60),
              status: !done ? "running" : item.status === "failed" ? "error" : "ok", detail: item.error?.message,
            });
            break;
          case "web_search":
            emit({ type: "tool", agent: agent.id, callId: item.id, verb: "search", target: preview(item.query, 70), status: done ? "ok" : "running" });
            break;
          case "error":
            emit({ type: "log", level: "warn", text: `${agent.id}: ${item.message}` });
            break;
        }
      }
      return { text, tokens, costUsd };
    },
  };
}
