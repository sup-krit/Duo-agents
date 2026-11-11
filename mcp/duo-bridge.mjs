#!/usr/bin/env node
// Stdio MCP server that each agent (Claude or Codex) runs as its "duo" tool server.
// It holds no state: every tool call is forwarded to the Agent Duo session over HTTP,
// so the session decides who may assign, spawn, wait or message, and the tree stays correct.
//
// usage: node duo-bridge.mjs <baseUrl> <sessionId> <agentId> <token> <tool,tool,...>

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";

const [baseUrl, sessionId, agentId, token, toolList = ""] = process.argv.slice(2);
if (!baseUrl || !sessionId || !agentId || !token) {
  console.error("usage: duo-bridge.mjs <baseUrl> <sessionId> <agentId> <token> <tools>");
  process.exit(2);
}
const allowed = new Set(toolList.split(",").filter(Boolean));

async function call(name, args) {
  try {
    const res = await fetch(`${baseUrl}/api/sessions/${sessionId}/tool`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ agent: agentId, token, name, args }),
    });
    const data = await res.json().catch(() => ({ text: `HTTP ${res.status}`, isError: true }));
    return { content: [{ type: "text", text: String(data.text ?? "") }], isError: !!data.isError };
  } catch (err) {
    return { content: [{ type: "text", text: `Agent Duo is not reachable: ${err.message}` }], isError: true };
  }
}

const agentKind = z.string().min(1); // "claude", "codex", or a provider id; the session checks it is on the team

const TOOLS = {
  propose_split: {
    description:
      "Lead only. Propose how to split the user's task across team agents. The user reviews the split; once approved, every item starts as its own agent (items with `after` wait for those items to finish). Returns the new agent ids, or the user's requested changes.",
    inputSchema: {
      summary: z.string().describe("One or two sentences: why this split"),
      items: z
        .array(
          z.object({
            agent: agentKind.describe("Which kind of agent does this part"),
            role: z.string().describe("Short role, e.g. build, tests, review"),
            task: z.string().describe("What this agent must do, self-contained"),
            after: z.array(z.number().int()).optional().describe("1-based indexes of items that must finish first"),
          }),
        )
        .min(1),
    },
  },
  assign: {
    description: "Lead only. Start one more team agent for an extra piece of work after the split was approved.",
    inputSchema: { agent: agentKind, role: z.string(), task: z.string() },
  },
  spawn_sub: {
    description:
      "Split YOUR part: start a sub-agent of your own kind for an independent piece of your task. Use only when the piece can run in parallel. Call `wait` before you finish.",
    inputSchema: { role: z.string().describe("Short role, e.g. sub, research, fix"), task: z.string() },
  },
  wait: {
    description:
      "Wait for agents to finish (default: all agents you started). Returns their reports and any messages sent to you. Returns early after ~45s with progress; call again if some are still running.",
    inputSchema: { ids: z.array(z.string()).optional() },
  },
  message: {
    description: "Send a short message to another agent by id (e.g. codex#1), or to 'lead'. They read it on their next wait/inbox call.",
    inputSchema: { to: z.string(), text: z.string() },
  },
  inbox: {
    description: "Read messages sent to you by the user or other agents since your last check.",
    inputSchema: {},
  },
  status: {
    description: "See the current agent tree: who is running, waiting or done, and what each is doing.",
    inputSchema: {},
  },
};

const server = new McpServer({ name: "duo", version: "0.2.0" });
for (const [name, def] of Object.entries(TOOLS)) {
  if (!allowed.has(name)) continue;
  server.registerTool(name, { description: def.description, inputSchema: def.inputSchema }, (args) => call(name, args ?? {}));
}
await server.connect(new StdioServerTransport());
