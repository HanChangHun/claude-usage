//! `claude-usage-app <command>`: a thin client of the local API, so scripts
//! and agents can read quota without HTTP plumbing. Recognized commands run
//! before Tauri starts; anything else launches the widget as before.

use crate::local_api::{configured_port, discovery_file, DEFAULT_PORT};
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream},
    time::Duration,
};

const EXIT_OK: i32 = 0;
const EXIT_ERROR: i32 = 1;
const EXIT_UNREACHABLE: i32 = 2;
const EXIT_NO_PROVIDER: i32 = 3;

const HELP: &str = "\
Claude Usage CLI - reads the running widget's local API (Settings > Local API).

USAGE:
  claude-usage-app usage     [--json] [--refresh]
  claude-usage-app recommend [--prefer claude,codex] [--max-used N]
                             [--max-used-claude N] [--max-used-codex N]
                             [--max-used-antigravity N] [--claude-model NAME]
                             [--max-age SECONDS] [--refresh] [--json]
  claude-usage-app api       [--json]
  claude-usage-app help

COMMON OPTIONS:
  --json         print the API's JSON response
  --refresh      re-read every provider first (rate-limited to once per 15 s)
  --port N       API port (default: discovery file, then CLAUDE_USAGE_API_PORT, then 47615)

RECOMMEND:
  Prints the first provider in --prefer order (default claude,codex) that is
  not at or above its --max-used percent (default 90). A provider whose quota
  is unknown keeps its place in the order. With --claude-model, Claude
  per-model limits for other models are ignored.

EXIT CODES:
  0 ok, 1 error, 2 widget not running or Local API off, 3 no provider available
";

#[derive(Debug, PartialEq)]
enum Command {
    Usage,
    Recommend,
    Api,
    Help,
}

#[derive(Debug, PartialEq)]
struct Args {
    command: Command,
    json: bool,
    port: Option<u16>,
    /// Query string for the API request (without `?`).
    query: Vec<(String, String)>,
}

fn command_of(arg: &str) -> Option<Command> {
    match arg {
        "usage" | "status" => Some(Command::Usage),
        "recommend" => Some(Command::Recommend),
        "api" => Some(Command::Api),
        "help" | "--help" | "-h" => Some(Command::Help),
        _ => None,
    }
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let command =
        command_of(args.first().map(String::as_str).unwrap_or("help")).ok_or("unknown command")?;
    let mut parsed = Args {
        command,
        json: false,
        port: None,
        query: Vec::new(),
    };
    let mut rest = args.iter().skip(1);
    while let Some(arg) = rest.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (arg.as_str(), None),
        };
        let mut value = |name: &str| -> Result<String, String> {
            inline
                .clone()
                .or_else(|| rest.next().cloned())
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match name {
            "--json" => parsed.json = true,
            "--refresh" => parsed.query.push(("refresh".into(), "1".into())),
            "--help" | "-h" => parsed.command = Command::Help,
            "--port" => {
                let v = value(name)?;
                parsed.port = Some(
                    v.parse::<u16>()
                        .ok()
                        .filter(|p| *p != 0)
                        .ok_or_else(|| format!("invalid port '{v}'"))?,
                );
            }
            _ if parsed.command == Command::Recommend => {
                let key = match name {
                    "--prefer" => "prefer",
                    "--max-used" => "max_used",
                    "--max-used-claude" => "max_used_claude",
                    "--max-used-codex" => "max_used_codex",
                    "--max-used-antigravity" => "max_used_antigravity",
                    "--claude-model" => "claude_model",
                    "--max-age" => "max_age",
                    _ => return Err(format!("unknown option '{name}'")),
                };
                parsed.query.push((key.into(), value(name)?));
            }
            _ => return Err(format!("unknown option '{name}'")),
        }
    }
    Ok(parsed)
}

fn path_for(args: &Args) -> String {
    let path = match args.command {
        Command::Usage => "/v1/usage",
        Command::Recommend => "/v1/recommend",
        Command::Api | Command::Help => "/v1/health",
    };
    if args.query.is_empty() {
        return path.into();
    }
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(&args.query)
        .finish();
    format!("{path}?{query}")
}

/// Ports to try, most specific first, without duplicates.
fn candidate_ports(flag: Option<u16>) -> Vec<u16> {
    if let Some(p) = flag {
        return vec![p];
    }
    let discovered = discovery_file()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|v| v.get("port")?.as_u64())
        .and_then(|p| u16::try_from(p).ok());
    let mut ports = Vec::new();
    for p in [discovered, configured_port(), Some(DEFAULT_PORT)]
        .into_iter()
        .flatten()
    {
        if !ports.contains(&p) {
            ports.push(p);
        }
    }
    ports
}

enum GetError {
    Unreachable,
    Failed(String),
}

fn http_get(port: u16, path: &str, timeout: Duration) -> Result<(u16, String), GetError> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .map_err(|_| GetError::Unreachable)?;
    let _ = stream.set_read_timeout(Some(timeout));
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUser-Agent: claude-usage-cli/{}\r\n\
         Accept: application/json\r\nConnection: close\r\n\r\n",
        env!("CARGO_PKG_VERSION")
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|e| GetError::Failed(format!("request failed: {e}")))?;
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|e| GetError::Failed(format!("no response: {e}")))?;
    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| GetError::Failed("malformed response".into()))?;
    let status = head
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| GetError::Failed("malformed response".into()))?;
    Ok((status, body.to_string()))
}

fn fmt_duration(secs: i64) -> String {
    let m = secs.max(0) / 60;
    let (d, h, m) = (m / 1440, (m % 1440) / 60, m % 60);
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

/// Same prettifying as the widget's account line: `claude_team` -> `Team`.
fn plan_label(raw: &str) -> String {
    let trimmed = raw.trim();
    let plan = trimmed.strip_prefix("claude_").unwrap_or(trimmed);
    plan.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn render_usage(v: &Value) -> String {
    let mut out = String::new();
    for (key, title) in [
        ("claude", "Claude"),
        ("codex", "Codex"),
        ("antigravity", "Antigravity"),
    ] {
        let Some(p) = v.pointer(&format!("/providers/{key}")) else {
            continue;
        };
        let status = p["status"].as_str().unwrap_or("unknown");
        let mut line = format!("{title} - {status}");
        if status == "disabled" {
            line.push_str(" (enable it in the widget's Settings)");
        }
        if let Some(age) = p["age_seconds"].as_i64() {
            line.push_str(&format!(
                " - updated {} ago",
                if age < 60 {
                    format!("{age}s")
                } else {
                    fmt_duration(age)
                }
            ));
        }
        if let Some(plan) = p["plan"].as_str().map(plan_label).filter(|s| !s.is_empty()) {
            line.push_str(&format!(" - {plan}"));
        }
        out.push_str(&line);
        out.push('\n');
        if let Some(msg) = p["message"].as_str() {
            out.push_str(&format!("  ! {msg}\n"));
        }
        for w in p["windows"].as_array().into_iter().flatten() {
            let label = w["label"].as_str().unwrap_or("?");
            let used = w["used_percent"].as_f64().unwrap_or(0.0);
            let reset = w["resets_in_seconds"]
                .as_i64()
                .map(|s| format!("  resets in {}", fmt_duration(s)))
                .unwrap_or_default();
            out.push_str(&format!("  {label:<24}{:>4.0}%{reset}\n", used));
        }
        if let Some(extra) = p.get("extra_usage").filter(|e| e.is_object()) {
            out.push_str(&format!(
                "  {:<24}${:.2} of ${:.2}\n",
                "Extra usage",
                extra["used_usd"].as_f64().unwrap_or(0.0),
                extra["monthly_limit_usd"].as_f64().unwrap_or(0.0)
            ));
        }
    }
    out
}

#[cfg(windows)]
fn attach_parent_console() {
    // Release builds use the GUI subsystem, so a terminal that didn't
    // redirect our output gives us no stdout. Borrow the parent's console.
    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(n: u32) -> isize;
        fn AttachConsole(pid: u32) -> i32;
    }
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        if handle == 0 || handle == -1 {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

#[cfg(not(windows))]
fn attach_parent_console() {}

/// Runs a CLI command and returns its exit code, or `None` to start the app.
pub fn run_from_args() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    command_of(args.first()?)?;
    attach_parent_console();
    let code = run(&args);
    let _ = std::io::stdout().flush();
    Some(code)
}

fn run(args: &[String]) -> i32 {
    let args = match parse_args(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}\n\n{HELP}");
            return EXIT_ERROR;
        }
    };
    if args.command == Command::Help {
        print!("{HELP}");
        return EXIT_OK;
    }
    let refresh = args.query.iter().any(|(k, _)| k == "refresh");
    let timeout = Duration::from_secs(if refresh { 60 } else { 10 });
    let path = path_for(&args);
    let mut reached = None;
    for port in candidate_ports(args.port) {
        match http_get(port, &path, timeout) {
            Ok(response) => {
                reached = Some((port, response));
                break;
            }
            Err(GetError::Unreachable) => continue,
            Err(GetError::Failed(e)) => {
                eprintln!("error: 127.0.0.1:{port}: {e}");
                return EXIT_ERROR;
            }
        }
    }
    let Some((port, (status, body))) = reached else {
        eprintln!(
            "error: the Claude Usage local API is not reachable. Start the widget and turn on \
             Settings > Local API (CLI & agents)."
        );
        return EXIT_UNREACHABLE;
    };
    let json: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    if status != 200 {
        eprintln!(
            "error: HTTP {status}: {}",
            json["error"].as_str().unwrap_or(body.trim())
        );
        return EXIT_ERROR;
    }

    match args.command {
        Command::Usage if args.json => println!("{body}"),
        Command::Usage => print!("{}", render_usage(&json)),
        Command::Recommend => {
            if args.json {
                println!("{body}");
            } else {
                println!("{}", json["provider"].as_str().unwrap_or("none"));
                if let Some(reason) = json["reason"].as_str() {
                    match json["retry_at"].as_str() {
                        Some(at) => eprintln!("{reason}; the first one frees up at {at}"),
                        None => eprintln!("{reason}"),
                    }
                }
            }
            if json["provider"].is_null() {
                return EXIT_NO_PROVIDER;
            }
        }
        Command::Api if args.json => println!(
            "{}",
            serde_json::json!({
                "base_url": format!("http://127.0.0.1:{port}"),
                "app_version": json["app_version"],
            })
        ),
        Command::Api => println!("http://127.0.0.1:{port}"),
        Command::Help => {}
    }
    EXIT_OK
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Args, String> {
        parse_args(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn only_known_commands_enter_cli_mode() {
        assert_eq!(command_of("usage"), Some(Command::Usage));
        assert_eq!(command_of("recommend"), Some(Command::Recommend));
        assert_eq!(command_of("--minimized"), None);
        assert_eq!(command_of(""), None);
    }

    #[test]
    fn recommend_flags_become_query_parameters() {
        let a = args(&[
            "recommend",
            "--prefer=claude,codex",
            "--max-used",
            "92",
            "--max-used-codex",
            "75",
            "--claude-model",
            "opus",
            "--refresh",
            "--json",
            "--port",
            "5000",
        ])
        .unwrap();
        assert!(a.json);
        assert_eq!(a.port, Some(5000));
        assert_eq!(
            path_for(&a),
            "/v1/recommend?prefer=claude%2Ccodex&max_used=92&max_used_codex=75&claude_model=opus&refresh=1"
        );
    }

    #[test]
    fn rejects_bad_arguments() {
        assert!(args(&["usage", "--prefer", "claude"]).is_err());
        assert!(args(&["recommend", "--max-used"]).is_err());
        assert!(args(&["usage", "--port", "0"]).is_err());
        assert_eq!(args(&["usage", "-h"]).unwrap().command, Command::Help);
    }

    #[test]
    fn explicit_port_wins() {
        assert_eq!(candidate_ports(Some(1234)), vec![1234]);
        assert!(candidate_ports(None).contains(&DEFAULT_PORT));
    }

    #[test]
    fn renders_a_readable_summary() {
        let v = serde_json::json!({
            "providers": {
                "claude": {"status": "ok", "age_seconds": 12, "plan": "claude_max_20x", "message": null,
                    "windows": [{"label": "Session (5h)", "used_percent": 41.4, "resets_in_seconds": 7380}],
                    "extra_usage": {"used_usd": 1.5, "monthly_limit_usd": 50.0}},
                "codex": {"status": "disabled", "windows": []},
                "antigravity": {"status": "error", "message": "agy missing", "windows": []}
            }
        });
        let text = render_usage(&v);
        assert!(text.contains("Claude - ok - updated 12s ago - Max 20x"));
        assert!(text.contains("Session (5h)              41%  resets in 2h 3m"));
        assert!(text.contains("$1.50 of $50.00"));
        assert!(text.contains("Codex - disabled (enable it in the widget's Settings)"));
        assert!(text.contains("  ! agy missing"));
        assert_eq!(fmt_duration(3 * 86_400 + 4 * 3600 + 5), "3d 4h");
    }
}
