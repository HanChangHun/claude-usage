//! Opt-in, read-only HTTP API on 127.0.0.1 so local CLIs and agents can read
//! the same quota the widget shows. Off by default (Settings toggle).
//!
//! Hardening: loopback bind only; the Host header must name the loopback
//! address (blocks DNS rebinding); browser requests (Origin or a cross-site
//! Sec-Fetch-Site) are refused; GET only; no CORS headers; account emails
//! are never part of the snapshot; forced refreshes are rate-limited.

use crate::{
    policy::{self, Options},
    snapshot::{self, Provider, Status},
};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    net::Ipv4Addr,
    path::PathBuf,
    sync::{
        atomic::{AtomicI64, Ordering},
        Mutex,
    },
    time::Duration,
};
use tauri::AppHandle;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

pub const DEFAULT_PORT: u16 = 47615;
pub const PORT_ENV: &str = "CLAUDE_USAGE_API_PORT";
const IDENTIFIER: &str = "io.github.hanchanghun.claude-usage";
const DISCOVERY_FILE: &str = "local-api.json";
const MAX_HEAD_BYTES: usize = 16 * 1024;
const HEAD_TIMEOUT: Duration = Duration::from_secs(5);
const REFRESH_MIN_INTERVAL_MS: i64 = 15_000;
const REFRESH_WAIT: Duration = Duration::from_secs(30);

struct Running {
    port: u16,
    task: tauri::async_runtime::JoinHandle<()>,
}

static RUNNING: Mutex<Option<Running>> = Mutex::new(None);
static TOGGLE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static LAST_FORCED_REFRESH: AtomicI64 = AtomicI64::new(0);

pub fn configured_port() -> Option<u16> {
    std::env::var(PORT_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<u16>().ok())
        .filter(|p| *p != 0)
}

/// `%LOCALAPPDATA%\io.github.hanchanghun.claude-usage\local-api.json`,
/// written while the API is listening so the CLI can find a fallback port.
pub fn discovery_file() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(base).join(IDENTIFIER).join(DISCOVERY_FILE))
}

fn write_discovery(port: u16) {
    let Some(path) = discovery_file() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let body = json!({
        "port": port,
        "base_url": format!("http://127.0.0.1:{port}"),
        "pid": std::process::id(),
        "app_version": env!("CARGO_PKG_VERSION"),
    });
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, body.to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

fn remove_discovery() {
    if let Some(path) = discovery_file() {
        let _ = std::fs::remove_file(path);
    }
}

fn running_port() -> Option<u16> {
    RUNNING.lock().ok()?.as_ref().map(|r| r.port)
}

fn stop() {
    let running = RUNNING.lock().ok().and_then(|mut r| r.take());
    if let Some(running) = running {
        running.task.abort();
        remove_discovery();
    }
}

async fn bind() -> Result<TcpListener, String> {
    let wanted = configured_port();
    let port = wanted.unwrap_or(DEFAULT_PORT);
    match TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await {
        Ok(listener) => Ok(listener),
        // The default port can be taken or reserved (Hyper-V excludes
        // ranges at boot); fall back to any free port and publish it in the
        // discovery file. An explicit port is a hard requirement.
        Err(_) if wanted.is_none() => TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|e| format!("Could not start the local API: {e}")),
        Err(e) => Err(format!(
            "Could not listen on 127.0.0.1:{port} ({PORT_ENV}): {e}"
        )),
    }
}

/// Start or stop the API; returns the port while it is listening.
#[tauri::command]
pub async fn set_local_api(app: AppHandle, enabled: bool) -> Result<Option<u16>, String> {
    let _guard = TOGGLE.lock().await;
    if !enabled {
        stop();
        return Ok(None);
    }
    if let Some(port) = running_port() {
        return Ok(Some(port));
    }
    let listener = bind().await?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("Could not start the local API: {e}"))?
        .port();
    let task = tauri::async_runtime::spawn(serve(listener, app, port));
    if let Ok(mut running) = RUNNING.lock() {
        *running = Some(Running { port, task });
    }
    write_discovery(port);
    Ok(Some(port))
}

async fn serve(listener: TcpListener, app: AppHandle, port: u16) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = handle(stream, app, port).await;
                });
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(200)).await,
        }
    }
}

async fn read_head(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            buf.truncate(end);
            return String::from_utf8(buf).ok();
        }
        if buf.len() > MAX_HEAD_BYTES {
            return None;
        }
    }
}

async fn handle(mut stream: TcpStream, app: AppHandle, port: u16) -> std::io::Result<()> {
    let (status, body) = match tokio::time::timeout(HEAD_TIMEOUT, read_head(&mut stream)).await {
        Ok(Some(head)) => match parse_request(&head).and_then(|req| check(&req, port).map(|_| req))
        {
            Ok(req) => route(&app, &req).await,
            Err((code, message)) => (code, error_body(&message)),
        },
        _ => (400, error_body("malformed request")),
    };
    let response = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: application/json; charset=utf-8\r\n\
         Cache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        reason_phrase(status),
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    }
}

fn error_body(message: &str) -> String {
    json!({ "error": message }).to_string()
}

#[derive(Debug, PartialEq)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

pub fn parse_request(head: &str) -> Result<Request, (u16, String)> {
    let bad = || (400, "malformed request".to_string());
    let mut lines = head.split("\r\n");
    let mut parts = lines.next().ok_or_else(bad)?.split(' ');
    let (method, target, version) = (
        parts.next().ok_or_else(bad)?,
        parts.next().ok_or_else(bad)?,
        parts.next().ok_or_else(bad)?,
    );
    if parts.next().is_some() || !version.starts_with("HTTP/1.") || !target.starts_with('/') {
        return Err(bad());
    }
    let url = url::Url::parse(&format!("http://localhost{target}")).map_err(|_| bad())?;
    let mut headers = Vec::new();
    for line in lines.filter(|l| !l.is_empty()) {
        let (name, value) = line.split_once(':').ok_or_else(bad)?;
        headers.push((name.trim().to_string(), value.trim().to_string()));
    }
    Ok(Request {
        method: method.to_string(),
        path: url.path().trim_end_matches('/').to_string(),
        query: url.query_pairs().into_owned().collect(),
        headers,
    })
}

pub fn check(req: &Request, port: u16) -> Result<(), (u16, String)> {
    if req.method != "GET" {
        return Err((405, "only GET is supported".into()));
    }
    let host = req.header("host").unwrap_or("").to_ascii_lowercase();
    if host != format!("127.0.0.1:{port}") && host != format!("localhost:{port}") {
        return Err((403, "the Host header must be 127.0.0.1 or localhost".into()));
    }
    let cross_site = req
        .header("sec-fetch-site")
        .is_some_and(|v| !v.eq_ignore_ascii_case("none"));
    if req.header("origin").is_some() || cross_site {
        return Err((403, "browser requests are not allowed".into()));
    }
    Ok(())
}

fn flag(value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "1" | "true" | "yes" => Ok(true),
        "0" | "false" | "no" => Ok(false),
        other => Err(format!("expected a boolean, got '{other}'")),
    }
}

fn percent(key: &str, value: &str) -> Result<f64, String> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|v| (0.0..=100.0).contains(v))
        .ok_or_else(|| format!("{key} must be a number from 0 to 100"))
}

/// Query parameters of `/v1/recommend` (and `refresh` for every route).
pub fn parse_options(query: &[(String, String)]) -> Result<(Options, bool), String> {
    let mut opts = Options::default();
    let mut refresh = false;
    let mut overrides: Vec<(Provider, f64)> = Vec::new();
    for (key, value) in query {
        match key.as_str() {
            "refresh" => refresh = flag(value)?,
            "prefer" => {
                let mut prefer = Vec::new();
                for name in value.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    let p = Provider::parse(name)
                        .ok_or_else(|| format!("unknown provider '{name}' in prefer"))?;
                    if !prefer.contains(&p) {
                        prefer.push(p);
                    }
                }
                if prefer.is_empty() {
                    return Err("prefer needs at least one provider".into());
                }
                opts.prefer = prefer;
            }
            "max_used" => opts.max_used = [percent(key, value)?; 3],
            "claude_model" => {
                opts.claude_model = Some(value.trim().to_string()).filter(|s| !s.is_empty())
            }
            "max_age" => {
                opts.max_age_secs = value
                    .trim()
                    .parse::<i64>()
                    .ok()
                    .filter(|v| (1..=7 * 86_400).contains(v))
                    .ok_or("max_age must be 1 to 604800 seconds")?
            }
            other => {
                let provider = other
                    .strip_prefix("max_used_")
                    .and_then(Provider::parse)
                    .ok_or_else(|| format!("unknown parameter '{other}'"))?;
                overrides.push((provider, percent(key, value)?));
            }
        }
    }
    for (provider, value) in overrides {
        let i = Provider::ALL
            .iter()
            .position(|p| *p == provider)
            .unwrap_or(0);
        opts.max_used[i] = value;
    }
    Ok((opts, refresh))
}

/// Ask every tracked provider for fresh numbers and wait (bounded) for the
/// answers. Rate-limited so a chatty client can't hammer claude.ai.
async fn refresh(app: &AppHandle) {
    let started = crate::now_ms();
    let last = LAST_FORCED_REFRESH.load(Ordering::SeqCst);
    if started - last < REFRESH_MIN_INTERVAL_MS
        || LAST_FORCED_REFRESH
            .compare_exchange(last, started, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
    {
        return;
    }
    // Claude is read here; Codex is read by the widget in response to the
    // refresh event this also emits. Antigravity is no longer read (v0.5.1).
    crate::fetch_usage(app, false).await;
    let deadline = tokio::time::Instant::now() + REFRESH_WAIT;
    while tokio::time::Instant::now() < deadline {
        let waiting = snapshot::current()
            .iter()
            .skip(1)
            .any(|s| s.status != Status::Disabled && s.checked_at_ms.unwrap_or(0) < started);
        if !waiting {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[derive(Serialize)]
struct RecommendBody {
    generated_at: Option<String>,
    #[serde(flatten)]
    recommendation: policy::Recommendation,
    options: Value,
}

fn options_json(opts: &Options) -> Value {
    json!({
        "prefer": opts.prefer.iter().map(|p| p.key()).collect::<Vec<_>>(),
        "max_used": {
            "claude": opts.max_used[0],
            "codex": opts.max_used[1],
            "antigravity": opts.max_used[2],
        },
        "claude_model": opts.claude_model,
        "max_age": opts.max_age_secs,
    })
}

async fn route(app: &AppHandle, req: &Request) -> (u16, String) {
    let pretty = |v: Value| serde_json::to_string_pretty(&v).unwrap_or_default();
    match req.path.as_str() {
        "" | "/v1" => (
            200,
            pretty(json!({
                "app_version": env!("CARGO_PKG_VERSION"),
                "endpoints": {
                    "/v1/health": "liveness",
                    "/v1/usage": "quota snapshot for every provider (?refresh=1 to re-read now)",
                    "/v1/recommend": "pick a provider: ?prefer=claude,codex&max_used=90&max_used_codex=80&claude_model=opus&max_age=600&refresh=1",
                },
            })),
        ),
        "/v1/health" => (
            200,
            pretty(json!({ "ok": true, "app_version": env!("CARGO_PKG_VERSION") })),
        ),
        "/v1/usage" => {
            let refresh_now = match req.query.iter().find(|(k, _)| k == "refresh") {
                Some((_, v)) => match flag(v) {
                    Ok(b) => b,
                    Err(e) => return (400, error_body(&format!("refresh: {e}"))),
                },
                None => false,
            };
            if refresh_now {
                refresh(app).await;
            }
            let view = snapshot::view(&snapshot::current(), crate::now_ms());
            (200, serde_json::to_string_pretty(&view).unwrap_or_default())
        }
        "/v1/recommend" => {
            let (opts, refresh_now) = match parse_options(&req.query) {
                Ok(parsed) => parsed,
                Err(e) => return (400, error_body(&e)),
            };
            if refresh_now {
                refresh(app).await;
            }
            let now = crate::now_ms();
            let body = RecommendBody {
                generated_at: snapshot::rfc3339_from_ms(now),
                recommendation: policy::recommend(&snapshot::current(), &opts, now),
                options: options_json(&opts),
            };
            (200, serde_json::to_string_pretty(&body).unwrap_or_default())
        }
        _ => (404, error_body("unknown endpoint; see /v1")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(head: &str) -> Request {
        parse_request(head).expect("request should parse")
    }

    #[test]
    fn parses_request_line_query_and_headers() {
        let r = req("GET /v1/recommend/?prefer=claude%2Ccodex&max_used=85 HTTP/1.1\r\nHost: 127.0.0.1:47615\r\nUser-Agent: curl/8");
        assert_eq!(r.method, "GET");
        assert_eq!(r.path, "/v1/recommend");
        assert_eq!(r.query[0], ("prefer".into(), "claude,codex".into()));
        assert_eq!(r.header("HOST"), Some("127.0.0.1:47615"));
        assert!(parse_request("GET\r\n").is_err());
        assert!(parse_request("GET http://evil/ HTTP/1.1\r\n").is_err());
    }

    #[test]
    fn rejects_rebinding_browsers_and_writes() {
        let ok = req("GET /v1/usage HTTP/1.1\r\nHost: localhost:47615");
        assert!(check(&ok, 47615).is_ok());
        let rebinding = req("GET /v1/usage HTTP/1.1\r\nHost: attacker.example:47615");
        assert_eq!(check(&rebinding, 47615).unwrap_err().0, 403);
        let wrong_port = req("GET /v1/usage HTTP/1.1\r\nHost: 127.0.0.1:80");
        assert_eq!(check(&wrong_port, 47615).unwrap_err().0, 403);
        let browser =
            req("GET /v1/usage HTTP/1.1\r\nHost: 127.0.0.1:47615\r\nOrigin: https://evil.example");
        assert_eq!(check(&browser, 47615).unwrap_err().0, 403);
        let cross =
            req("GET /v1/usage HTTP/1.1\r\nHost: 127.0.0.1:47615\r\nSec-Fetch-Site: cross-site");
        assert_eq!(check(&cross, 47615).unwrap_err().0, 403);
        let typed = req("GET /v1/usage HTTP/1.1\r\nHost: 127.0.0.1:47615\r\nSec-Fetch-Site: none");
        assert!(check(&typed, 47615).is_ok());
        let post = req("POST /v1/usage HTTP/1.1\r\nHost: 127.0.0.1:47615");
        assert_eq!(check(&post, 47615).unwrap_err().0, 405);
    }

    #[test]
    fn recommend_options_from_query() {
        let q = |s: &str| req(&format!("GET /v1/recommend?{s} HTTP/1.1\r\nHost: x")).query;
        let (opts, refresh) =
            parse_options(&q("max_used_codex=70&max_used=85&prefer=claude,codex,claude&claude_model=opus&max_age=120&refresh=1"))
                .unwrap();
        assert!(refresh);
        assert_eq!(opts.prefer, vec![Provider::Claude, Provider::Codex]);
        assert_eq!(opts.max_used, [85.0, 70.0, 85.0]);
        assert_eq!(opts.claude_model.as_deref(), Some("opus"));
        assert_eq!(opts.max_age_secs, 120);
        assert_eq!(parse_options(&q("")).unwrap().0, Options::default());
        assert!(parse_options(&q("prefer=gemini")).is_err());
        assert!(parse_options(&q("max_used=150")).is_err());
        assert!(parse_options(&q("max_used_foo=50")).is_err());
        assert!(parse_options(&q("typo=1")).is_err());
    }
}
