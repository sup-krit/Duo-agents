//! What agent-duo (and the desktop app) do on start: serve the page, or run a subcommand.

use crate::{api, bridge};

use std::net::{Ipv4Addr, SocketAddr};

/// The subcommands agents (and you) run against this same executable. None: not a subcommand.
pub async fn subcommand(args: &[String]) -> Option<anyhow::Result<()>> {
    Some(match args.first().map(String::as_str)? {
        "bridge" => bridge::run(&args[1..]).await,
        "hook" => bridge::hook(&args[1..]).await,
        "doctor" => doctor().await,
        _ => return None,
    })
}

/// agent-duo [--port 3000] [--no-open] | doctor | bridge … | hook …
pub async fn main(args: Vec<String>) -> anyhow::Result<()> {
    if let Some(r) = subcommand(&args).await {
        return r;
    }
    match args.first().map(String::as_str) {
        Some("--version" | "-V") => {
            println!("agent-duo {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("--help" | "-h") => {
            println!("agent-duo [--port 3000] [--no-open] | doctor | bridge … | hook …");
            Ok(())
        }
        _ => serve(&args).await,
    }
}

/// Listen on 127.0.0.1 only (there is no login): `port`, or any free port when it is taken.
pub async fn bind(port: u16) -> anyhow::Result<(tokio::net::TcpListener, String)> {
    let listener = match tokio::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).await {
        Ok(l) => l,
        Err(_) => tokio::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await?,
    };
    let base_url = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    Ok((listener, base_url))
}

async fn serve(args: &[String]) -> anyhow::Result<()> {
    let flag = |f: &str| args.iter().position(|a| a == f);
    let port: u16 = flag("--port").and_then(|i| args.get(i + 1)?.parse().ok()).unwrap_or(3000);
    let (listener, base_url) = bind(port).await?;
    println!("Agent Duo {} is running at {base_url}", env!("CARGO_PKG_VERSION"));
    println!("Sessions are saved in {}", duo_core::runs_dir().display());
    println!("Close this window to stop it.");
    if flag("--no-open").is_none() {
        let _ = open::that_detached(&base_url);
    }
    axum::serve(listener, api::router(api::App { base_url })).await?;
    Ok(())
}

async fn doctor() -> anyhow::Result<()> {
    use duo_core::runners::proc::find;
    async fn out(p: &std::path::Path, args: &[&str]) -> Option<String> {
        let o = tokio::process::Command::new(p).args(args).env("CODEX_MANAGED_BY_NPM", "1").output().await.ok()?;
        let s = String::from_utf8_lossy(if o.stdout.is_empty() { &o.stderr } else { &o.stdout }).trim().to_string();
        o.status.success().then_some(s)
    }
    let mut ok = true;
    let mut report = |good: bool, what: &str, detail: String| {
        ok &= good;
        println!("{} {what:<14} {detail}", if good { "[ok]  " } else { "[FAIL]" });
    };

    match find("git") {
        Some(p) => report(out(&p, &["--version"]).await.is_some(), "git", p.display().to_string()),
        None => report(false, "git", "not found: install Git for Windows (needed for git repos)".into()),
    }

    match find("claude") {
        Some(p) => {
            let v = out(&p, &["--version"]).await;
            report(v.is_some(), "claude", format!("{} ({})", v.unwrap_or_default(), p.display()));
            let auth = out(&p, &["auth", "status"]).await.unwrap_or_default();
            let logged = auth.contains("\"loggedIn\": true") || std::env::var_os("ANTHROPIC_API_KEY").is_some();
            report(logged, "claude login", if logged { "logged in".into() } else { "run `claude` and log in, or set ANTHROPIC_API_KEY".into() });
        }
        None => report(false, "claude", "not found: install Claude Code (or set DUO_CLAUDE)".into()),
    }

    match find("codex") {
        Some(p) => {
            let v = out(&p, &["--version"]).await;
            report(v.is_some(), "codex", format!("{} ({})", v.unwrap_or_default(), p.display()));
            let status = out(&p, &["login", "status"]).await;
            let logged = status.as_deref().is_some_and(|s| s.contains("Logged in")) || std::env::var_os("OPENAI_API_KEY").is_some();
            report(logged, "codex login", status.filter(|_| logged).unwrap_or_else(|| "run `codex login`, or set OPENAI_API_KEY".into()));
        }
        None => report(false, "codex", "not found: npm install -g @openai/codex (or set DUO_CODEX)".into()),
    }

    println!("\nruns:   {}", duo_core::runs_dir().display());
    println!("shared: {}", duo_core::shared::dir().display());
    if !ok {
        println!("\nMock mode works without any of these. Real agents need the ones marked FAIL.");
        std::process::exit(1);
    }
    Ok(())
}
