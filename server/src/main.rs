//! agent-duo.exe: the web app (default), plus the subcommands agents and you run.
//!   agent-duo [--port 3000] [--no-open]
//!   agent-duo bridge <baseUrl> <sessionId> <agentId> <token> <tools>   (MCP server for one agent)
//!   agent-duo hook <baseUrl> <sessionId> <agentId> <token>             (Claude Code's permission gate)
//!   agent-duo doctor                                                   (are claude, codex and git ready?)

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    duo_core::fix_path();
    agent_duo::cli::main(std::env::args().skip(1).collect()).await
}
