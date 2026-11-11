import { existsSync } from "node:fs";
import { query, type HookCallback } from "@anthropic-ai/claude-agent-sdk";
import { providerCost, type Provider } from "../providers";
import { SHARED } from "../shared";
import { preview, type Runner, type TurnContext, type TurnResult } from "./types";

// Skill + Agent: skills / subagents from the target repo's .claude/, ~/.claude/ and SHARED (as plugin "shared").
// Subagents still go through this allowlist, so a read-only agent's subagents stay read-only.
const READ_TOOLS = ["Read", "Glob", "Grep", "Skill", "Agent"];
const EDIT_TOOLS = ["Edit", "Write", "MultiEdit", "NotebookEdit"];
const SHELL_TOOLS = ["Bash", "PowerShell"]; // Claude Code on Windows runs commands through PowerShell
const WRITE_TOOLS = [...READ_TOOLS, ...EDIT_TOOLS, ...SHELL_TOOLS];

/**
 * Every Edit/Write/Bash (subagents' too) passes this gate. A hook, not canUseTool: hooks run even when
 * the user's own settings.json allow rules would let the call through without asking.
 */
function gate(ctx: TurnContext): HookCallback {
  return async (input) => {
    if (input.hook_event_name !== "PreToolUse") return {};
    const name = input.tool_name;
    const args = (input.tool_input ?? {}) as Record<string, unknown>;
    const deny = (why: string) => ({ hookSpecificOutput: { hookEventName: "PreToolUse" as const, permissionDecision: "deny" as const, permissionDecisionReason: why } });
    const allow = { hookSpecificOutput: { hookEventName: "PreToolUse" as const, permissionDecision: "allow" as const } };
    const isEdit = EDIT_TOOLS.includes(name);
    if (!isEdit && !SHELL_TOOLS.includes(name)) return {};

    if (ctx.access === "read") return deny("You are read-only. Delegate changes, or report what should change.");
    if (ctx.permission() === "plan") return deny("Plan mode: nothing may change until the user approves the plan. Describe the change instead.");

    if (isEdit) {
      const file = String(args.file_path ?? args.notebook_path ?? "");
      if (!file || !ctx.isProtected(file)) return {};
      const a = await ctx.approve({ kind: "file", title: `${ctx.agent.id} wants to change a protected file`, detail: `${name} ${file}`, file });
      return a.ok ? allow : deny(`The user said no to changing ${file}${a.note ? `: ${a.note}` : ""}. Leave it as it is.`);
    }
    if (ctx.permission() === "acceptEdits") {
      const cmd = String(args.command ?? "");
      const a = await ctx.approve({ kind: "command", title: `${ctx.agent.id} wants to run a command`, detail: cmd });
      return a.ok ? allow : deny(`The user said no to this command${a.note ? `: ${a.note}` : ""}.`);
    }
    return {};
  };
}

/** Claude Code pointed at another Anthropic-format API (DeepSeek, Kimi, GLM, OpenRouter, Ollama…). */
function providerEnv(p: Provider): Record<string, string> {
  const env: Record<string, string> = {};
  for (const [k, v] of Object.entries(process.env)) if (v !== undefined) env[k] = v;
  return {
    ...env,
    ANTHROPIC_BASE_URL: p.baseUrl,
    ANTHROPIC_AUTH_TOKEN: p.apiKey || "none", // local servers ignore it, but Claude Code wants one
    ANTHROPIC_API_KEY: "", // never send your Anthropic key to someone else's endpoint
    ANTHROPIC_MODEL: p.model,
    ANTHROPIC_DEFAULT_OPUS_MODEL: p.model,
    ANTHROPIC_DEFAULT_SONNET_MODEL: p.model,
    ANTHROPIC_DEFAULT_HAIKU_MODEL: p.model,
    ANTHROPIC_SMALL_FAST_MODEL: p.model,
    CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
  };
}

function line(name: string, input: Record<string, unknown>): { verb: string; target: string } | null {
  if (name.startsWith("mcp__duo__")) return null; // the session shows assign/spawn/msg itself
  const s = (k: string) => (typeof input[k] === "string" ? (input[k] as string) : "");
  switch (name) {
    case "Read": return { verb: "read", target: s("file_path") };
    case "Glob": return { verb: "glob", target: s("pattern") };
    case "Grep": return { verb: "grep", target: `"${s("pattern")}"` };
    case "Edit": case "MultiEdit": return { verb: "edit", target: s("file_path") };
    case "Write": return { verb: "write", target: s("file_path") };
    case "Bash": case "PowerShell": return { verb: "run", target: preview(s("command"), 70) };
    case "Skill": return { verb: "skill", target: s("skill") };
    case "Agent": return { verb: "agent", target: `${s("subagent_type") || "general"}: ${preview(s("description"), 50)}` };
    default: return { verb: name.replace(/^mcp__/, "").toLowerCase(), target: preview(input, 60) };
  }
}

function textOf(content: unknown): string {
  if (typeof content === "string") return content;
  if (Array.isArray(content)) return content.map((c) => (c && typeof c === "object" && "text" in c ? String(c.text) : "")).join("\n");
  return "";
}

/** One Claude Agent SDK conversation per agent; later turns resume it. */
export function createClaudeRunner(): Runner {
  let sessionId: string | undefined;
  let costSoFar = 0; // total_cost_usd carries over when a session is resumed

  return {
    async turn(ctx: TurnContext): Promise<TurnResult> {
      const { agent, emit } = ctx;
      const abortController = new AbortController();
      ctx.signal.addEventListener("abort", () => abortController.abort(), { once: true });

      const tools = (ctx.access === "write" ? WRITE_TOOLS : READ_TOOLS).concat(ctx.tools.map((t) => `mcp__duo__${t}`));
      const lines = new Map<string, boolean>(); // tool_use ids we showed

      let text = "";
      let costUsd = 0;
      let tokens = 0;

      const q = query({
        prompt: ctx.prompt,
        options: {
          cwd: ctx.cwd,
          abortController,
          resume: sessionId,
          model: ctx.provider?.model || process.env.CLAUDE_MODEL || undefined,
          env: ctx.provider ? providerEnv(ctx.provider) : undefined,
          hooks: { PreToolUse: [{ hooks: [gate(ctx)], timeout: 60 * 60, onFailure: "block" }] }, // waits for your answer
          allowedTools: tools,
          permissionMode: "dontAsk", // anything not in allowedTools is denied, never prompted
          skills: "all",
          plugins: existsSync(SHARED) ? [{ type: "local", path: SHARED }] : [],
          mcpServers: {
            duo: {
              type: "stdio",
              command: process.execPath,
              args: [ctx.bridge.script, ctx.bridge.baseUrl, ctx.bridge.sessionId, agent.id, ctx.bridge.token, ctx.tools.join(",")],
              timeout: 60 * 60 * 1000, // propose_split waits for you
            },
          },
        },
      });

      for await (const msg of q) {
        if ("session_id" in msg && typeof msg.session_id === "string") sessionId = msg.session_id;
        if (msg.type === "assistant" && !msg.parent_tool_use_id) {
          for (const block of msg.message.content) {
            if (block.type === "text" && block.text.trim()) emit({ type: "say", agent: agent.id, text: block.text.trim() });
            else if (block.type === "tool_use") {
              const l = line(block.name, (block.input ?? {}) as Record<string, unknown>);
              if (!l) continue;
              lines.set(block.id, true);
              emit({ type: "tool", agent: agent.id, callId: block.id, ...l, status: "running" });
            }
          }
        } else if (msg.type === "user" && Array.isArray(msg.message.content)) {
          for (const block of msg.message.content) {
            if (typeof block !== "object" || block.type !== "tool_result" || !lines.has(block.tool_use_id)) continue;
            emit({
              type: "tool",
              agent: agent.id,
              callId: block.tool_use_id,
              verb: "",
              target: "",
              status: block.is_error ? "error" : "ok",
              detail: block.is_error ? preview(textOf(block.content), 160) : undefined,
            });
          }
        } else if (msg.type === "result") {
          const total = msg.total_cost_usd ?? 0;
          costUsd = Math.max(total - costSoFar, 0);
          costSoFar = Math.max(total, costSoFar);
          const u = msg.usage;
          const tin = (u?.input_tokens ?? 0) + (u?.cache_read_input_tokens ?? 0) + (u?.cache_creation_input_tokens ?? 0);
          tokens = tin + (u?.output_tokens ?? 0);
          if (ctx.provider) costUsd = providerCost(ctx.provider, tin, u?.output_tokens ?? 0); // total_cost_usd is at Anthropic prices
          if (msg.subtype === "success") text = msg.result;
          else throw new Error(`Claude stopped: ${msg.subtype}`);
        }
      }
      return { text, tokens, costUsd };
    },
  };
}
