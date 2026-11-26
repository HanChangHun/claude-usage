use serde::{Deserialize, Serialize};
use std::{ffi::OsString, path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    sync::Mutex,
};

static QUERY: Mutex<()> = Mutex::const_new(());

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_STDOUT_BYTES: usize = 1024 * 1024;
const MAX_STDERR_BYTES: usize = 16 * 1024;

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub groups: Vec<Group>,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub name: String,
    pub buckets: Vec<Bucket>,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Bucket {
    pub id: Option<String>,
    pub name: Option<String>,
    pub window: Option<String>,
    pub remaining_fraction: f64,
    pub reset_time: Option<String>,
}

#[derive(Deserialize)]
struct CliOutput {
    status: Option<String>,
    command: Option<CliCommand>,
}

#[derive(Deserialize)]
struct CliCommand {
    name: Option<String>,
    data: Option<CliData>,
}

#[derive(Deserialize)]
struct CliData {
    #[serde(default)]
    groups: Vec<CliGroup>,
}

#[derive(Deserialize)]
struct CliGroup {
    #[serde(default)]
    name: String,
    #[serde(default)]
    buckets: Vec<CliBucket>,
}

#[derive(Deserialize)]
struct CliBucket {
    id: Option<String>,
    name: Option<String>,
    window: Option<String>,
    remaining_fraction: Option<f64>,
    reset_time: Option<String>,
}

fn find_executable(
    override_path: Option<OsString>,
    path_var: Option<OsString>,
    extra_roots: Vec<PathBuf>,
) -> Result<PathBuf, String> {
    if let Some(path) = override_path {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path).ok_or_else(|| {
            "ANTIGRAVITY_USAGE_CLI must point to the Antigravity CLI executable.".into()
        });
    }
    let binary = if cfg!(windows) { "agy.exe" } else { "agy" };
    let mut roots: Vec<PathBuf> = path_var
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    roots.extend(extra_roots);
    for root in roots {
        let candidate = root.join(binary);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err("Install Antigravity CLI (agy), sign in with your Google account, then retry. A custom executable can be set with ANTIGRAVITY_USAGE_CLI.".into())
}

fn executable() -> Result<PathBuf, String> {
    let mut extra_roots = Vec::new();
    if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") {
        let local = PathBuf::from(local_appdata);
        extra_roots.push(local.join("agy").join("bin"));
        extra_roots.push(local.join("Programs").join("antigravity").join("bin"));
    }
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        extra_roots.push(PathBuf::from(profile).join(".local").join("bin"));
    }
    find_executable(
        std::env::var_os("ANTIGRAVITY_USAGE_CLI"),
        std::env::var_os("PATH"),
        extra_roots,
    )
}

fn classify_failure(stderr: &str) -> String {
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("login")
        || lower.contains("sign in")
        || lower.contains("unauthenticated")
        || lower.contains("unauthorized")
        || lower.contains("credential")
        || lower.contains("oauth")
        || lower.contains("401")
        || lower.contains("403")
    {
        return "Sign in to Antigravity CLI (agy) with your Google account, then retry.".into();
    }
    if lower.contains("flag provided but not defined") || lower.contains("unknown flag") {
        return "Antigravity CLI returned an unsupported response. Update agy and retry.".into();
    }
    "Antigravity could not read subscription limits. Check your connection and Google login in agy, then retry.".into()
}

fn parse_usage(stdout: &str) -> Result<Usage, String> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Err("Antigravity returned no usage data.".into());
    }
    let parsed: CliOutput = serde_json::from_str(trimmed)
        .or_else(|_| {
            trimmed
                .lines()
                .rev()
                .find_map(|line| {
                    let line = line.trim();
                    line.starts_with('{')
                        .then(|| serde_json::from_str::<CliOutput>(line).ok())
                        .flatten()
                })
                .ok_or(())
        })
        .map_err(|_| {
            "Antigravity returned an unsupported response. Update agy and retry.".to_string()
        })?;

    if parsed.status.as_deref() != Some("SUCCESS") {
        return Err("Antigravity could not read subscription limits. Check your connection and Google login in agy, then retry.".into());
    }
    let command = parsed.command.ok_or_else(|| {
        "Antigravity returned an unsupported response. Update agy and retry.".to_string()
    })?;
    if command.name.as_deref() != Some("usage") {
        return Err(
            "Antigravity returned an unsupported response. Update agy and retry.".into(),
        );
    }
    let data = command
        .data
        .ok_or_else(|| "Antigravity returned no usage data.".to_string())?;

    let groups = data
        .groups
        .into_iter()
        .map(|group| {
            let buckets = group
                .buckets
                .into_iter()
                .filter_map(|bucket| {
                    let remaining_fraction = bucket.remaining_fraction?;
                    if !remaining_fraction.is_finite() {
                        return None;
                    }
                    Some(Bucket {
                        id: bucket.id,
                        name: bucket.name,
                        window: bucket.window,
                        remaining_fraction,
                        reset_time: bucket.reset_time,
                    })
                })
                .collect();
            Group {
                name: group.name,
                buckets,
            }
        })
        .collect();

    Ok(Usage { groups })
}

async fn read_stdout<R: AsyncRead + Unpin>(reader: R) -> Result<String, String> {
    let mut limited = reader.take((MAX_STDOUT_BYTES + 1) as u64);
    let mut buf = Vec::new();
    limited
        .read_to_end(&mut buf)
        .await
        .map_err(|_| "Could not read the Antigravity response.".to_string())?;
    if buf.len() > MAX_STDOUT_BYTES {
        return Err("Antigravity returned an unexpectedly large response.".into());
    }
    String::from_utf8(buf)
        .map_err(|_| "Antigravity returned an unsupported response. Update agy and retry.".into())
}

async fn read_stderr<R: AsyncRead + Unpin>(reader: R) -> String {
    let mut limited = reader.take(MAX_STDERR_BYTES as u64);
    let mut buf = Vec::new();
    let _ = limited.read_to_end(&mut buf).await;
    String::from_utf8_lossy(&buf).into_owned()
}

#[tauri::command]
pub async fn read_antigravity_usage() -> Result<Usage, String> {
    let _guard = QUERY
        .try_lock()
        .map_err(|_| "An Antigravity refresh is already running.".to_string())?;
    let mut command = Command::new(executable()?);
    command
        .args(["--print", "/usage", "--output-format", "json"])
        .current_dir(std::env::temp_dir())
        // Prevent passive quota checks from spawning agy's self-updater.
        .env("AGY_CLI_DISABLE_AUTO_UPDATE", "true")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let mut child = command.spawn().map_err(|_| {
        "Could not start Antigravity CLI (agy). Check the installation and retry.".to_string()
    })?;
    let result = tokio::time::timeout(COMMAND_TIMEOUT, async {
        let stdout = child
            .stdout
            .take()
            .ok_or("Antigravity output is unavailable.")?;
        let stderr = child
            .stderr
            .take()
            .ok_or("Antigravity error output is unavailable.")?;
        let (stdout_res, stderr_text, status_res) =
            tokio::join!(read_stdout(stdout), read_stderr(stderr), child.wait());
        let status = status_res
            .map_err(|_| "Antigravity exited unexpectedly. Check agy and retry.".to_string())?;
        if !status.success() {
            return Err(classify_failure(&stderr_text));
        }
        let stdout_text = stdout_res?;
        parse_usage(&stdout_text)
    })
    .await;
    let _ = child.kill().await;
    result.map_err(|_| "Antigravity refresh timed out. Check your connection and retry.".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_JSON: &str = r#"{
      "conversation_id": "",
      "status": "SUCCESS",
      "num_turns": 0,
      "command": {
        "name": "usage",
        "data": {
          "groups": [
            {
              "name": "Gemini Models",
              "buckets": [
                {
                  "id": "gemini-weekly",
                  "name": "Weekly Limit Remaining",
                  "window": "weekly",
                  "remaining_fraction": 0.9524,
                  "reset_time": "2026-12-03T01:14:34Z"
                },
                {
                  "id": "gemini-5h",
                  "name": "Five Hour Limit Remaining",
                  "window": "5h",
                  "remaining_fraction": 0.8291,
                  "reset_time": "2026-11-26T06:14:34Z"
                }
              ]
            },
            {
              "name": "Claude and GPT models",
              "buckets": [
                {
                  "id": "3p-weekly",
                  "name": "Weekly Limit Remaining",
                  "window": "weekly",
                  "remaining_fraction": 1,
                  "reset_time": "1970-01-01T00:00:00Z"
                }
              ]
            }
          ]
        }
      }
    }"#;

    #[test]
    fn parses_real_usage_payload() {
        let usage = parse_usage(SAMPLE_JSON).expect("sample payload should parse");
        assert_eq!(usage.groups.len(), 2);
        assert_eq!(usage.groups[0].name, "Gemini Models");
        assert_eq!(usage.groups[0].buckets.len(), 2);
        assert_eq!(usage.groups[0].buckets[0].id.as_deref(), Some("gemini-weekly"));
        assert_eq!(usage.groups[0].buckets[0].window.as_deref(), Some("weekly"));
        assert!((usage.groups[0].buckets[0].remaining_fraction - 0.9524).abs() < 1e-9);
        assert_eq!(
            usage.groups[0].buckets[1].reset_time.as_deref(),
            Some("2026-11-26T06:14:34Z")
        );
    }

    #[test]
    fn parses_json_after_preamble_lines() {
        let output = format!("Fetching usage...\n{}", SAMPLE_JSON.replace('\n', ""));
        let usage = parse_usage(&output).expect("trailing JSON line should parse");
        assert_eq!(usage.groups.len(), 2);
    }

    #[test]
    fn rejects_non_success_status() {
        let payload = r#"{"status":"ERROR","command":{"name":"usage","data":{"groups":[]}}}"#;
        assert!(parse_usage(payload).is_err());
    }

    #[test]
    fn rejects_non_usage_command() {
        let payload = r#"{"status":"SUCCESS","command":{"name":"credits","data":{"groups":[]}}}"#;
        assert!(parse_usage(payload).is_err());
    }

    #[test]
    fn classifies_auth_and_flag_errors() {
        assert!(classify_failure("401 Unauthorized: please sign in").contains("Sign in"));
        assert!(classify_failure("flag provided but not defined: -output-format")
            .contains("Update agy"));
        assert!(classify_failure("connection reset by peer")
            .contains("could not read subscription limits"));
    }

    #[test]
    fn override_must_point_to_existing_file() {
        let err = find_executable(
            Some(OsString::from("C:\\nonexistent\\path\\to\\agy.exe")),
            None,
            Vec::new(),
        )
        .unwrap_err();
        assert!(err.contains("ANTIGRAVITY_USAGE_CLI"));
    }
}
