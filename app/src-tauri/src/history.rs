//! Local quota observations, sampled from successful polls without extra requests.
//! Identities stay in memory. Opaque segments prevent joining different logins
//! or app runs; a missing identity deliberately produces isolated observations.
use crate::snapshot::Window;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

const INTERVAL_MS: i64 = 300_000;
const DAY_MS: i64 = 86_400_000;
const RETENTION_DAYS: i64 = 90;

#[derive(Clone, Deserialize, Serialize)]
pub struct Sample {
    pub ts: i64,
    pub provider: String,
    pub segment: String,
    pub windows: Vec<Window>,
}

struct Cursor {
    identity: Option<String>,
    segment: String,
    bucket: i64,
}

struct History {
    directory: PathBuf,
    cursors: HashMap<String, Cursor>,
    sequence: u64,
    last_pruned_day: Option<i64>,
    error: Option<String>,
}

impl History {
    fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            cursors: HashMap::new(),
            sequence: 0,
            last_pruned_day: None,
            error: None,
        }
    }

    fn record(
        &mut self,
        provider: &str,
        identity: Option<&str>,
        windows: &[Window],
        ts: i64,
    ) -> std::io::Result<bool> {
        if windows.is_empty() || ts <= 0 {
            return Ok(false);
        }
        let bucket = ts.div_euclid(INTERVAL_MS);
        let cursor = self.cursors.get(provider);
        let same_identity =
            identity.is_some() && cursor.is_some_and(|c| c.identity.as_deref() == identity);
        if cursor.is_some_and(|c| c.identity.as_deref() == identity && c.bucket >= bucket) {
            return Ok(false);
        }
        let segment = if same_identity {
            cursor.unwrap().segment.clone()
        } else {
            self.sequence += 1;
            format!("{ts}-{}", self.sequence)
        };
        let sample = Sample {
            ts,
            provider: provider.into(),
            segment: segment.clone(),
            windows: windows.to_vec(),
        };
        fs::create_dir_all(&self.directory)?;
        let day = ts.div_euclid(DAY_MS);
        self.prune(day)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.directory.join(format!("{day}.jsonl")))?;
        // A leading newline isolates a partial final record after an interrupted write.
        let mut bytes = vec![b'\n'];
        serde_json::to_writer(&mut bytes, &sample)?;
        bytes.push(b'\n');
        file.write_all(&bytes)?;
        file.sync_data()?;
        self.cursors.insert(
            provider.into(),
            Cursor {
                identity: identity.map(str::to_string),
                segment,
                bucket,
            },
        );
        Ok(true)
    }

    fn prune(&mut self, day: i64) -> std::io::Result<()> {
        if self.last_pruned_day == Some(day) {
            return Ok(());
        }
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                continue;
            }
            let file_day = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<i64>().ok());
            if file_day.is_some_and(|d| d < day - RETENTION_DAYS) && entry.file_type()?.is_file() {
                fs::remove_file(path)?;
            }
        }
        self.last_pruned_day = Some(day);
        Ok(())
    }

    fn read(&self, provider: &str, hours: i64, now: i64) -> Result<HistoryView, String> {
        if !["claude", "codex"].contains(&provider) || ![6, 24, 168, 720, 2160].contains(&hours) {
            return Err("Unsupported history range or provider.".into());
        }
        let start = now - hours * 3_600_000;
        let mut samples = Vec::new();
        let mut skipped = 0;
        for day in start.div_euclid(DAY_MS)..=now.div_euclid(DAY_MS) {
            let file = match fs::File::open(self.directory.join(format!("{day}.jsonl"))) {
                Ok(file) => file,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(format!("Could not read usage history: {e}")),
            };
            for line in BufReader::new(file).lines() {
                let line = line.map_err(|e| format!("Could not read usage history: {e}"))?;
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Sample>(&line) {
                    Ok(s) if s.provider == provider && s.ts >= start && s.ts <= now => {
                        samples.push(s)
                    }
                    Ok(_) => {}
                    Err(_) => skipped += 1,
                }
            }
        }
        samples.sort_by_key(|s| s.ts);
        let warning = self.error.clone().or_else(|| {
            (skipped > 0).then(|| format!("Skipped {skipped} unreadable history records."))
        });
        Ok(HistoryView {
            samples,
            start,
            end: now,
            warning,
        })
    }
}

static STORE: OnceLock<Mutex<History>> = OnceLock::new();

pub fn init(directory: PathBuf) {
    let _ = STORE.set(Mutex::new(History::new(directory.join("usage-history-v1"))));
}

pub fn record(provider: &str, identity: Option<&str>, windows: &[Window], now: i64) {
    let Some(store) = STORE.get() else {
        return;
    };
    if let Ok(mut store) = store.lock() {
        match store.record(provider, identity, windows, now) {
            Ok(true) => store.error = None,
            Ok(false) => {}
            Err(e) => store.error = Some(format!("History recording failed: {e}")),
        }
    }
}

#[derive(Serialize)]
pub struct HistoryView {
    samples: Vec<Sample>,
    start: i64,
    end: i64,
    warning: Option<String>,
}

#[tauri::command]
pub async fn read_usage_history(provider: String, hours: i64) -> Result<HistoryView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        STORE
            .get()
            .ok_or("Usage history is not initialized.")?
            .lock()
            .map_err(|_| "Usage history is unavailable.")?
            .read(&provider, hours, crate::now_ms())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn store() -> History {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        History::new(std::env::temp_dir().join(format!("claude-usage-history-test-{id}")))
    }
    fn rows(percent: f64) -> Vec<Window> {
        vec![Window {
            id: "weekly".into(),
            label: "Weekly".into(),
            used_percent: percent,
            window_minutes: Some(10080),
            resets_at: Some("2026-12-01T00:00:00Z".into()),
            model: None,
        }]
    }
    #[test]
    fn samples_once_in_each_bucket_and_survives_restart() {
        let mut s = store();
        let now = 20_000 * DAY_MS;
        s.record("codex", Some("private@example.com"), &rows(10.0), now)
            .unwrap();
        s.record(
            "codex",
            Some("private@example.com"),
            &rows(11.0),
            now + 60_000,
        )
        .unwrap();
        s.record(
            "codex",
            Some("private@example.com"),
            &rows(12.0),
            now + INTERVAL_MS,
        )
        .unwrap();
        let mut restarted = History::new(s.directory.clone());
        restarted
            .record(
                "codex",
                Some("private@example.com"),
                &rows(13.0),
                now + 2 * INTERVAL_MS,
            )
            .unwrap();
        let view = restarted.read("codex", 24, now + 2 * INTERVAL_MS).unwrap();
        assert_eq!(view.samples.len(), 3);
        assert_eq!(view.samples[0].segment, view.samples[1].segment);
        assert_ne!(view.samples[1].segment, view.samples[2].segment);
        let disk = fs::read_to_string(s.directory.join("20000.jsonl")).unwrap();
        assert!(!disk.contains("private@example.com"));
        fs::remove_dir_all(s.directory).unwrap();
    }
    #[test]
    fn identity_changes_empty_results_and_missing_identity_do_not_join() {
        let mut s = store();
        let now = 20_000 * DAY_MS;
        s.record("codex", Some("a"), &rows(80.0), now).unwrap();
        s.record("codex", Some("b"), &rows(10.0), now + 1000)
            .unwrap();
        s.record("codex", None, &rows(11.0), now + INTERVAL_MS)
            .unwrap();
        s.record("codex", None, &rows(12.0), now + 2 * INTERVAL_MS)
            .unwrap();
        s.record("claude", Some("org"), &[], now).unwrap();
        let view = s.read("codex", 24, now + 2 * INTERVAL_MS).unwrap();
        assert_eq!(view.samples.len(), 4);
        assert!(view
            .samples
            .windows(2)
            .all(|pair| pair[0].segment != pair[1].segment));
        assert!(s.read("claude", 24, now).unwrap().samples.is_empty());
        fs::remove_dir_all(s.directory).unwrap();
    }
    #[test]
    fn retention_and_partial_records_preserve_valid_history() {
        let mut s = store();
        let now = 20_000 * DAY_MS;
        s.record("claude", Some("org"), &rows(1.0), now - 91 * DAY_MS)
            .unwrap();
        fs::write(s.directory.join("keep.txt"), "unrelated").unwrap();
        fs::write(s.directory.join("20000.jsonl"), "{partial").unwrap();
        s.record("claude", Some("org"), &rows(2.0), now).unwrap();
        let view = s.read("claude", 2160, now).unwrap();
        assert_eq!(view.samples.len(), 1);
        assert!(view.warning.is_some());
        assert!(!s.directory.join("19909.jsonl").exists());
        assert!(s.directory.join("keep.txt").exists());
        assert!(s.read("../other", 24, now).is_err());
        assert!(s.read("claude", 99999, now).is_err());
        fs::remove_dir_all(s.directory).unwrap();
    }
}
