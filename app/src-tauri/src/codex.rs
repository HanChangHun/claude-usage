use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{ChildStdin, ChildStdout, Command},
    sync::Mutex,
};

static QUERY: Mutex<()> = Mutex::const_new(());

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    #[serde(default)]
    account: Option<Account>,
    rate_limits: Option<Bucket>,
    rate_limits_by_limit_id: Option<BTreeMap<String, Bucket>>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Account {
    #[serde(rename = "type")]
    kind: String,
    email: Option<String>,
    plan_type: Option<String>,
}

#[derive(Deserialize)]
struct AccountReply {
    account: Option<Account>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Bucket {
    limit_id: Option<String>,
    limit_name: Option<String>,
    primary: Option<Window>,
    secondary: Option<Window>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Window {
    used_percent: f64,
    window_duration_mins: Option<u64>,
    resets_at: Option<i64>,
}

#[derive(Deserialize)]
struct Reply<T> {
    id: Option<u64>,
    result: Option<T>,
    error: Option<RpcError>,
}

#[derive(Deserialize)]
struct RpcError {
    code: i64,
}

fn executable() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("CODEX_USAGE_CLI") {
        let path = PathBuf::from(path);
        return path
            .is_file()
            .then_some(path)
            .ok_or_else(|| "CODEX_USAGE_CLI must point to the native Codex executable.".into());
    }
    let mut roots: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        roots.push(PathBuf::from(appdata).join("npm"));
    }
    for root in roots {
        let native = root.join(if cfg!(windows) { "codex.exe" } else { "codex" });
        if native.is_file() {
            return Ok(native);
        }
        // npm installs a shell shim; launch its native binary without a shell.
        for platform in ["x64", "arm64"] {
            let triple = if platform == "x64" {
                "x86_64-pc-windows-msvc"
            } else {
                "aarch64-pc-windows-msvc"
            };
            for folder in ["bin", "codex"] {
                let native = root.join(format!(
                    "node_modules/@openai/codex/node_modules/@openai/codex-win32-{platform}/vendor/{triple}/{folder}/codex.exe"
                ));
                if native.is_file() {
                    return Ok(native);
                }
            }
        }
    }
    Err("Install Codex CLI, sign in with ChatGPT using codex login, then retry. A custom native executable can be set with CODEX_USAGE_CLI.".into())
}

async fn send(input: &mut ChildStdin, message: &str) -> Result<(), String> {
    input
        .write_all(message.as_bytes())
        .await
        .map_err(|_| "Could not send a request to Codex.".to_string())
}

async fn receive<T: DeserializeOwned>(
    output: &mut Lines<BufReader<ChildStdout>>,
    id: u64,
) -> Result<T, String> {
    while let Some(line) = output
        .next_line()
        .await
        .map_err(|_| "Could not read the Codex response.".to_string())?
    {
        let reply: Reply<T> = serde_json::from_str(&line).map_err(|_| {
            "Codex returned an unsupported response. Update Codex CLI and retry.".to_string()
        })?;
        if reply.id != Some(id) {
            continue;
        }
        if let Some(error) = reply.error {
            return Err(format!("Codex could not read subscription limits ({}). Check your connection and ChatGPT login in Codex CLI, then retry.", error.code));
        }
        return reply
            .result
            .ok_or_else(|| "Codex returned no usage data.".into());
    }
    Err("Codex exited before returning usage. Check Codex CLI and retry.".into())
}

#[tauri::command]
pub async fn read_codex_usage() -> Result<Usage, String> {
    let _guard = QUERY
        .try_lock()
        .map_err(|_| "A Codex refresh is already running.".to_string())?;
    let mut command = Command::new(executable()?);
    command
        .arg("app-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let mut child = command
        .spawn()
        .map_err(|_| "Could not start Codex CLI. Check the installation and retry.".to_string())?;
    let result = tokio::time::timeout(Duration::from_secs(25), async {
        let mut input = child.stdin.take().ok_or("Codex input is unavailable.")?;
        let mut output =
            BufReader::new(child.stdout.take().ok_or("Codex output is unavailable.")?).lines();
        send(
            &mut input,
            concat!(
                "{\"id\":1,\"method\":\"initialize\",\"params\":{\"clientInfo\":{",
                "\"name\":\"claude_usage\",\"version\":\"",
                env!("CARGO_PKG_VERSION"),
                "\"}}}\n"
            ),
        )
        .await?;
        receive::<serde::de::IgnoredAny>(&mut output, 1).await?;
        send(
            &mut input,
            "{\"method\":\"initialized\"}\n{\"id\":2,\"method\":\"account/read\",\"params\":{\"refreshToken\":false}}\n",
        )
        .await?;
        let account = receive::<AccountReply>(&mut output, 2)
            .await
            .ok()
            .and_then(|reply| reply.account);
        send(
            &mut input,
            "{\"id\":3,\"method\":\"account/rateLimits/read\"}\n",
        )
        .await?;
        let mut usage = receive::<Usage>(&mut output, 3).await?;
        usage.account = account;
        Ok(usage)
    })
    .await;
    let _ = child.kill().await;
    result.map_err(|_| "Codex refresh timed out. Check your connection and retry.".to_string())?
}
