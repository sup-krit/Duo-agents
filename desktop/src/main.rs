//! Agent Duo as a desktop app (Windows and macOS). The window shows the same page as agent-duo.exe,
//! served by the same engine on 127.0.0.1 inside this process. Agents call this executable back as
//! `bridge` / `hook`, so it answers those subcommands too, without opening a window.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::{Arc, Mutex};
use tauri::{Manager, UserAttentionType, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_notification::NotificationExt;

/// A fixed port keeps the page's origin the same between launches, so remembered form values stay.
const PORT: u16 = 47_300;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.first().map(String::as_str), Some("bridge" | "hook" | "doctor")) {
        duo_core::fix_path();
        let rt = tokio::runtime::Runtime::new().expect("tokio");
        if let Some(Err(e)) = rt.block_on(agent_duo::cli::subcommand(&args)) {
            eprintln!("{e:#}");
            std::process::exit(1);
        }
        return;
    }

    duo_core::fix_path();
    let (listener, base_url) = tauri::async_runtime::block_on(agent_duo::cli::bind(PORT)).expect("could not listen on 127.0.0.1");
    let app = agent_duo::api::router(agent_duo::api::App { base_url: base_url.clone() });
    tauri::async_runtime::spawn(async move {
        if let Err(e) = agent_duo::api::serve(listener, app).await {
            eprintln!("server stopped: {e}");
        }
    });

    // The session the last notification was about: opened when you come back to the window
    // (desktop notifications cannot report a click).
    let pending: Arc<Mutex<Option<String>>> = Arc::default();
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .setup(move |app| {
            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(base_url.parse()?))
                .title("Agent Duo")
                .inner_size(1400.0, 900.0)
                .min_inner_size(900.0, 600.0)
                // the page leaves notifications to the app (WKWebView has no web notifications anyway)
                .initialization_script("window.__DUO_DESKTOP__ = true;")
                .build()?;

            let (handle, waiting) = (app.handle().clone(), pending.clone());
            duo_core::notify::set_sink(move |n| {
                let (handle, waiting) = (handle.clone(), waiting.clone());
                // called while a session is locked: show it on another thread
                tauri::async_runtime::spawn_blocking(move || {
                    let Some(w) = handle.get_webview_window("main") else { return };
                    if w.is_focused().unwrap_or(false) {
                        return; // you are looking at it
                    }
                    *waiting.lock().unwrap() = Some(n.session.clone());
                    let shown = handle.notification().builder().title(&n.title).body(&n.body).show();
                    eprintln!("notification [{}] {}: {:?}", n.session, n.title, shown.map_err(|e| e.to_string()));
                    let _ = w.request_user_attention(Some(UserAttentionType::Informational));
                });
            });

            let (w, base, waiting) = (window.clone(), base_url.clone(), pending.clone());
            window.on_window_event(move |e| {
                if let WindowEvent::Focused(true) = e {
                    let _ = w.request_user_attention(None);
                    let Some(id) = waiting.lock().unwrap().take() else { return };
                    let here = w.url().map(|u| u.path().ends_with(&id)).unwrap_or(false);
                    if let (false, Ok(url)) = (here, format!("{base}/s/{id}").parse()) {
                        let _ = w.navigate(url);
                    }
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Agent Duo");
}
