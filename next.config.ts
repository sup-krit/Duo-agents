import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  // The agent SDKs spawn their own CLI processes; keep them out of the bundle.
  serverExternalPackages: ["@anthropic-ai/claude-agent-sdk", "@openai/codex-sdk", "@openai/codex"],
};

export default nextConfig;
