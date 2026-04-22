use serde::Serialize;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder,
};
use tauri_plugin_autostart::ManagerExt;
use url::Url;

const CLAUDE_BASE: &str = "https://claude.ai/";
const POLL_INTERVAL_SECS: u64 = 60;

#[derive(Serialize, Clone)]
struct UsageEvent {
    ts: i64,
    data: serde_json::Value,
}

#[derive(Serialize, Clone)]
struct StatusEvent {
    state: String, // "loading" | "logged_in" | "logged_out" | "error"
    message: Option<String>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            // ---- Main widget window ----
            let _main = WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::App("index.html".into()),
            )
            .title("Claude Usage")
            .inner_size(440.0, 420.0)
            .min_inner_size(360.0, 320.0)
            .resizable(true)
            .visible(true)
            .build()?;

            // ---- Hidden claude.ai webview (cookie host + login surface) ----
            let _claude = WebviewWindowBuilder::new(
                app,
                "claude",
                WebviewUrl::External(CLAUDE_BASE.parse().unwrap()),
            )
            .title("Claude — sign in")
            .inner_size(960.0, 720.0)
            .visible(false)
            .build()?;

            // ---- System tray ----
            let show_item = MenuItem::with_id(app, "show", "Show window", true, None::<&str>)?;
            let refresh_item =
                MenuItem::with_id(app, "refresh", "Refresh now", true, None::<&str>)?;
            let sep1 = PredefinedMenuItem::separator(app)?;
            let autostart_enabled = app
                .autolaunch()
                .is_enabled()
                .unwrap_or(false);
            let autostart_item = CheckMenuItem::with_id(
                app,
                "autostart",
                "Start with Windows",
                true,
                autostart_enabled,
                None::<&str>,
            )?;
            let signout_item =
                MenuItem::with_id(app, "signout", "Sign out of claude.ai…", true, None::<&str>)?;
            let sep2 = PredefinedMenuItem::separator(app)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[
                    &show_item,
                    &refresh_item,
                    &sep1,
                    &autostart_item,
                    &signout_item,
                    &sep2,
                    &quit_item,
                ],
            )?;

            let _tray = TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Claude Usage")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main(app),
                    "refresh" => trigger_fetch(app.clone()),
                    "autostart" => {
                        let _ = toggle_autostart(app);
                    }
                    "signout" => sign_out(app.clone()),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        show_main(app);
                    }
                })
                .build(app)?;

            // ---- Polling loop ----
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // Wait briefly so windows + webview are fully initialized before
                // first cookies_for_url call (some platforms need this).
                tokio::time::sleep(Duration::from_secs(2)).await;
                loop {
                    fetch_usage(&app_handle).await;
                    tokio::time::sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            // Close-to-tray for both windows: hide instead of quit
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" || window.label() == "claude" {
                    let _ = window.hide();
                    api.prevent_close();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            manual_refresh,
            open_login,
            sign_out_cmd,
            toggle_autostart_cmd,
            is_autostart_enabled
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[tauri::command]
async fn manual_refresh(app: AppHandle) {
    fetch_usage(&app).await;
}

#[tauri::command]
fn open_login(app: AppHandle) {
    show_claude_login(&app);
}

#[tauri::command]
fn sign_out_cmd(app: AppHandle) {
    sign_out(app);
}

#[tauri::command]
fn toggle_autostart_cmd(app: AppHandle) -> bool {
    toggle_autostart(&app)
}

#[tauri::command]
fn is_autostart_enabled(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn show_claude_login(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("claude") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn trigger_fetch(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        fetch_usage(&app).await;
    });
}

/// Toggle autostart and return the new state.
fn toggle_autostart(app: &AppHandle) -> bool {
    let manager = app.autolaunch();
    let currently = manager.is_enabled().unwrap_or(false);
    let _ = if currently {
        manager.disable()
    } else {
        manager.enable()
    };
    manager.is_enabled().unwrap_or(false)
}

/// Sign out of claude.ai by clearing accessible cookies and navigating
/// the embedded webview to the logout URL. The user can then sign in
/// again with a different account, or close the window.
fn sign_out(app: AppHandle) {
    let Some(claude) = app.get_webview_window("claude") else {
        return;
    };
    // Try to clear all non-HttpOnly cookies via JS, then navigate to /logout.
    // The HttpOnly sessionKey cookie can only be cleared by the server via
    // its logout response — that's why we hit /logout afterwards.
    let _ = claude.eval(
        r#"(function(){
            try {
                document.cookie.split(';').forEach(c => {
                    const name = c.split('=')[0].trim();
                    if (!name) return;
                    const expire = 'Expires=Thu, 01 Jan 1970 00:00:00 GMT';
                    document.cookie = `${name}=; Path=/; ${expire}`;
                    document.cookie = `${name}=; Path=/; Domain=.claude.ai; ${expire}`;
                    document.cookie = `${name}=; Path=/; Domain=claude.ai; ${expire}`;
                });
            } catch (e) {}
            location.href = 'https://claude.ai/logout';
        })();"#,
    );
    let _ = claude.show();
    let _ = claude.unminimize();
    let _ = claude.set_focus();
    let _ = app.emit(
        "status",
        StatusEvent {
            state: "logged_out".into(),
            message: Some("Sign out requested. Use the claude.ai window to confirm.".into()),
        },
    );
}

async fn fetch_usage(app: &AppHandle) {
    let claude = match app.get_webview_window("claude") {
        Some(w) => w,
        None => return,
    };

    let url = match Url::parse(CLAUDE_BASE) {
        Ok(u) => u,
        Err(_) => return,
    };

    // cookies_for_url is synchronous and deadlocks the WebView2 thread on Windows
    // when called from a sync context — wrap in spawn_blocking to be safe.
    let claude_for_blocking = claude.clone();
    let cookies_result =
        tokio::task::spawn_blocking(move || claude_for_blocking.cookies_for_url(url)).await;
    let cookies = match cookies_result {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => {
            let _ = app.emit(
                "status",
                StatusEvent {
                    state: "error".into(),
                    message: Some(format!("cookies_for_url failed: {}", e)),
                },
            );
            return;
        }
        Err(join_err) => {
            let _ = app.emit(
                "status",
                StatusEvent {
                    state: "error".into(),
                    message: Some(format!("spawn_blocking join error: {}", join_err)),
                },
            );
            return;
        }
    };

    let org_value = cookies
        .iter()
        .find(|c| c.name() == "lastActiveOrg")
        .map(|c| c.value().to_string());

    let Some(org_id) = org_value else {
        // Not logged in — surface the claude.ai webview so the user can sign in
        let _ = claude.show();
        let _ = claude.set_focus();
        let _ = app.emit(
            "status",
            StatusEvent {
                state: "logged_out".into(),
                message: Some("Sign in to claude.ai to start tracking usage.".into()),
            },
        );
        return;
    };

    let cookie_header = cookies
        .iter()
        .map(|c| format!("{}={}", c.name(), c.value()))
        .collect::<Vec<_>>()
        .join("; ");

    let api_url = format!("https://claude.ai/api/organizations/{}/usage", org_id);

    let result = reqwest::Client::new()
        .get(&api_url)
        .header("Cookie", cookie_header)
        .header("Accept", "application/json")
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) ClaudeUsageApp/0.1",
        )
        .send()
        .await;

    match result {
        Ok(r) => {
            let status = r.status();
            if status.is_success() {
                match r.json::<serde_json::Value>().await {
                    Ok(json) => {
                        let _ = app.emit(
                            "usage-update",
                            UsageEvent {
                                ts: now_ms(),
                                data: json,
                            },
                        );
                        let _ = app.emit(
                            "status",
                            StatusEvent {
                                state: "logged_in".into(),
                                message: None,
                            },
                        );
                    }
                    Err(e) => {
                        let _ = app.emit(
                            "status",
                            StatusEvent {
                                state: "error".into(),
                                message: Some(format!("json parse: {}", e)),
                            },
                        );
                    }
                }
            } else if status == 401 || status == 403 {
                let _ = claude.show();
                let _ = claude.set_focus();
                let _ = app.emit(
                    "status",
                    StatusEvent {
                        state: "logged_out".into(),
                        message: Some(format!("HTTP {} — sign in again", status)),
                    },
                );
            } else {
                let _ = app.emit(
                    "status",
                    StatusEvent {
                        state: "error".into(),
                        message: Some(format!("HTTP {}", status)),
                    },
                );
            }
        }
        Err(e) => {
            let _ = app.emit(
                "status",
                StatusEvent {
                    state: "error".into(),
                    message: Some(format!("network: {}", e)),
                },
            );
        }
    }
}
