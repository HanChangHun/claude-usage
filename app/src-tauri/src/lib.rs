use serde::Serialize;
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder,
};
use tauri_plugin_autostart::ManagerExt;
use url::Url;

const CLAUDE_BASE: &str = "https://claude.ai/";
const ORGANIZATIONS_URL: &str = "https://claude.ai/api/organizations";
const KOFI_URL: &str = "https://ko-fi.com/edgetpu";
const POLL_INTERVAL_SECS: u64 = 60;

// Surface the claude.ai sign-in window only after this many consecutive
// auth-looking failures. Avoids spurious pop-ups when WebView2 is still
// loading on startup, or during transient network/401 blips.
const LOGIN_FAIL_THRESHOLD: u32 = 3;

// Never auto-open the sign-in window more often than this — whether from
// repeated failure streaks (a false logged-out detection resets the streak
// on the next success) or from one long streak while the user stays
// signed out. Bounds how often the app can steal focus.
const LOGIN_POPUP_COOLDOWN_SECS: i64 = 6 * 60 * 60;

// Mirror the embedded WebView2's UA so the API sees the same client as the
// sign-in webview; obviously non-browser agents get challenged more often.
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36 Edg/126.0.0.0";

struct LoginFailures(AtomicU32);

/// Unix seconds when the sign-in window was last surfaced, by any path
/// (0 = never). Read by the auto-popup cooldown check.
struct LastLoginPopup(AtomicI64);

/// Org-ID resolution state. `cached` is the last org that worked (so a
/// missing/expired `lastActiveOrg` cookie doesn't read as a sign-out while
/// the session is still valid); `rejected` is a value the API refused (so
/// a stale cookie can't keep steering polls back to a revoked org).
#[derive(Default)]
struct OrgIds {
    cached: Option<String>,
    rejected: Option<String>,
}
struct OrgState(Mutex<OrgIds>);

fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(10))
            // Without a timeout, one request left half-open by e.g. a
            // sleep/resume cycle would wedge the poll loop forever.
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

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
            app.manage(LoginFailures(AtomicU32::new(0)));
            app.manage(LastLoginPopup(AtomicI64::new(0)));
            app.manage(OrgState(Mutex::new(OrgIds::default())));

            // ---- Main widget window ----
            let _main = WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::App("index.html".into()),
            )
            .title("Claude Usage")
            .inner_size(440.0, 420.0)
            .min_inner_size(300.0, 320.0)
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
                    fetch_usage(&app_handle, true).await;
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
                // Closing the sign-in window usually means the user just
                // signed in — refresh right away instead of waiting out the
                // poll interval, and give the new session a fresh streak.
                if window.label() == "claude" {
                    let app = window.app_handle().clone();
                    app.state::<LoginFailures>().0.store(0, Ordering::SeqCst);
                    trigger_fetch(app);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            manual_refresh,
            open_login,
            sign_out_cmd,
            toggle_autostart_cmd,
            is_autostart_enabled,
            open_kofi
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[tauri::command]
async fn manual_refresh(app: AppHandle) {
    fetch_usage(&app, false).await;
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

#[tauri::command]
fn open_kofi() {
    let _ = open::that_detached(KOFI_URL);
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn show_claude_login(app: &AppHandle) {
    // Every path that surfaces the window arms the auto-popup cooldown,
    // so the poll loop won't steal focus again right after the user saw
    // (and possibly dismissed) this window.
    app.state::<LastLoginPopup>()
        .0
        .store(now_ms() / 1000, Ordering::SeqCst);
    if let Some(w) = app.get_webview_window("claude") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn trigger_fetch(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        fetch_usage(&app, false).await;
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
    // Forget the org state and pre-arm the failure streak past the
    // threshold so the very next poll reports logged_out honestly instead
    // of staying quiet below the threshold. show_claude_login() below arms
    // the popup cooldown, so the poll loop won't re-steal focus either.
    if let Ok(mut org) = app.state::<OrgState>().0.lock() {
        *org = OrgIds::default();
    }
    app.state::<LoginFailures>()
        .0
        .store(LOGIN_FAIL_THRESHOLD + 1, Ordering::SeqCst);
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
    show_claude_login(&app);
    emit_status(
        &app,
        "logged_out",
        Some("Sign out requested. Use the claude.ai window to confirm.".into()),
    );
}

fn emit_status(app: &AppHandle, state: &str, message: Option<String>) {
    let _ = app.emit(
        "status",
        StatusEvent {
            state: state.into(),
            message,
        },
    );
}

/// Count an auth-looking failure and decide whether to surface the sign-in
/// window. Could be a real sign-out, but also fires on startup before
/// WebView2 has loaded claude.ai, or on a challenge page misread — so the
/// window pops only past the threshold, and never more often than
/// LOGIN_POPUP_COOLDOWN_SECS (show_claude_login arms the cooldown).
fn handle_auth_failure(app: &AppHandle, from_poll: bool, message: String) {
    let fails = app.state::<LoginFailures>();
    // Only poll failures count toward the threshold — user-initiated
    // refreshes can arrive in a burst and would collapse the "consecutive
    // failures over time" window to zero.
    let count = if from_poll {
        fails.0.fetch_add(1, Ordering::SeqCst) + 1
    } else {
        fails.0.load(Ordering::SeqCst)
    };
    if from_poll && count >= LOGIN_FAIL_THRESHOLD {
        let last = app.state::<LastLoginPopup>();
        let now_secs = now_ms() / 1000;
        if now_secs - last.0.load(Ordering::SeqCst) >= LOGIN_POPUP_COOLDOWN_SECS {
            show_claude_login(app);
        }
    }
    // Background polls stay quiet below the threshold (transient blip),
    // but a deliberate user refresh always gets an honest answer.
    if !from_poll || count >= LOGIN_FAIL_THRESHOLD {
        emit_status(app, "logged_out", Some(message));
    }
}

/// A 401/403 is a real sign-out only when it comes from the claude.ai API
/// itself (JSON body). An HTML body is an intermediary challenge page —
/// a transient error, not a reason to pop the sign-in window.
fn is_api_auth_response(r: &reqwest::Response) -> bool {
    r.headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("json"))
        .unwrap_or(false)
}

/// Org IDs are UUIDs. Anything else is a corrupt (or planted) value —
/// don't let it steer the authenticated request to an arbitrary path.
fn is_valid_org_id(v: &str) -> bool {
    !v.is_empty() && v.len() <= 40 && v.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

enum OrgLookup {
    Found(String),
    AuthFailure(String),
    Error(String),
}

/// Ask the API which orgs this session belongs to. Lets us keep working
/// when the `lastActiveOrg` cookie is missing/expired but the session
/// cookie is still valid, and doubles as a real auth check.
async fn discover_org_id(cookie_header: &str) -> OrgLookup {
    let resp = http_client()
        .get(ORGANIZATIONS_URL)
        .header("Cookie", cookie_header)
        .header("Accept", "application/json")
        .send()
        .await;
    let r = match resp {
        Ok(r) => r,
        Err(e) => return OrgLookup::Error(format!("network: {}", e)),
    };
    let status = r.status();
    if status == 401 || status == 403 {
        return if is_api_auth_response(&r) {
            OrgLookup::AuthFailure(format!("HTTP {} — sign in again", status))
        } else {
            OrgLookup::Error(format!("HTTP {} (challenge)", status))
        };
    }
    if !status.is_success() {
        return OrgLookup::Error(format!("HTTP {}", status));
    }
    let json: serde_json::Value = match r.json().await {
        Ok(j) => j,
        Err(e) => return OrgLookup::Error(format!("json parse: {}", e)),
    };
    let orgs = match json.as_array() {
        Some(a) => a,
        None => return OrgLookup::Error("unexpected organizations response".into()),
    };
    // Prefer the org this account chats under; fall back to the first.
    let picked = orgs
        .iter()
        .find(|o| {
            o.get("capabilities")
                .and_then(|c| c.as_array())
                .map(|caps| caps.iter().any(|v| v.as_str() == Some("chat")))
                .unwrap_or(false)
        })
        .or_else(|| orgs.first());
    match picked
        .and_then(|o| o.get("uuid"))
        .and_then(|u| u.as_str())
        .filter(|u| is_valid_org_id(u))
    {
        Some(id) => OrgLookup::Found(id.to_string()),
        None => OrgLookup::AuthFailure("No organization on this session — sign in again.".into()),
    }
}

/// The usage endpoint refused this org (401/403/404 from the API): forget
/// it and pin it as rejected so the stale `lastActiveOrg` cookie can't
/// steer the next poll back — discovery will find the right org instead,
/// or confirm the sign-out.
fn reject_org_id(app: &AppHandle, org_id: &str) {
    if let Ok(mut org) = app.state::<OrgState>().0.lock() {
        org.cached = None;
        org.rejected = Some(org_id.to_string());
    }
}

/// `from_poll` marks the 60s background loop (vs. a user-initiated refresh).
async fn fetch_usage(app: &AppHandle, from_poll: bool) {
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
    let cookies_result =
        tokio::task::spawn_blocking(move || claude.cookies_for_url(url)).await;
    let cookies = match cookies_result {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => {
            emit_status(app, "error", Some(format!("cookies_for_url failed: {}", e)));
            return;
        }
        Err(join_err) => {
            emit_status(
                app,
                "error",
                Some(format!("spawn_blocking join error: {}", join_err)),
            );
            return;
        }
    };

    if cookies.is_empty() {
        // No session at all (or webview not ready yet).
        handle_auth_failure(
            app,
            from_poll,
            "Sign in to claude.ai to start tracking usage.".into(),
        );
        return;
    }

    let cookie_header = cookies
        .iter()
        .map(|c| format!("{}={}", c.name(), c.value()))
        .collect::<Vec<_>>()
        .join("; ");

    // Org ID: lastActiveOrg cookie (unless the API already refused that
    // value) → last known-good value → API discovery.
    let known_org = {
        let cookie_org = cookies
            .iter()
            .find(|c| c.name() == "lastActiveOrg")
            .map(|c| c.value().to_string())
            .filter(|v| is_valid_org_id(v));
        match app.state::<OrgState>().0.lock() {
            Ok(org) => cookie_org
                .filter(|v| org.rejected.as_deref() != Some(v.as_str()))
                .or_else(|| org.cached.clone()),
            Err(_) => cookie_org,
        }
    };
    let org_id = match known_org {
        Some(id) => id,
        None => match discover_org_id(&cookie_header).await {
            OrgLookup::Found(id) => id,
            OrgLookup::AuthFailure(msg) => {
                handle_auth_failure(app, from_poll, msg);
                return;
            }
            OrgLookup::Error(msg) => {
                emit_status(app, "error", Some(msg));
                return;
            }
        },
    };

    let api_url = format!("https://claude.ai/api/organizations/{}/usage", org_id);

    let result = http_client()
        .get(&api_url)
        .header("Cookie", cookie_header)
        .header("Accept", "application/json")
        .send()
        .await;

    match result {
        Ok(r) => {
            let status = r.status();
            if status.is_success() {
                match r.json::<serde_json::Value>().await {
                    Ok(json) => {
                        app.state::<LoginFailures>().0.store(0, Ordering::SeqCst);
                        if let Ok(mut org) = app.state::<OrgState>().0.lock() {
                            org.cached = Some(org_id.clone());
                            // The org works (again) — e.g. re-added to a team.
                            if org.rejected.as_deref() == Some(org_id.as_str()) {
                                org.rejected = None;
                            }
                        }
                        let _ = app.emit(
                            "usage-update",
                            UsageEvent {
                                ts: now_ms(),
                                data: json,
                            },
                        );
                        emit_status(app, "logged_in", None);
                    }
                    Err(e) => {
                        emit_status(app, "error", Some(format!("json parse: {}", e)));
                    }
                }
            } else if status == 401 || status == 403 {
                if is_api_auth_response(&r) {
                    // Might be a stale org (seat revoked, account switched)
                    // rather than a dead session — the next poll rediscovers
                    // and either recovers or confirms the sign-out.
                    reject_org_id(app, &org_id);
                    handle_auth_failure(app, from_poll, format!("HTTP {} — sign in again", status));
                } else {
                    emit_status(app, "error", Some(format!("HTTP {} (challenge)", status)));
                }
            } else {
                if status == 404 {
                    reject_org_id(app, &org_id);
                }
                emit_status(app, "error", Some(format!("HTTP {}", status)));
            }
        }
        Err(e) => {
            emit_status(app, "error", Some(format!("network: {}", e)));
        }
    }
}
