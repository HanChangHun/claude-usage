//! In-memory, machine-readable copy of the quota the widget shows, shared by
//! the opt-in local API and the CLI. Rows are normalized the same way the
//! frontend renders them, so an agent sees exactly what the widget shows.
//! Account emails are deliberately never stored here.

use serde::Serialize;
use serde_json::Value;
use std::sync::{Mutex, OnceLock};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Claude,
    Codex,
    Antigravity,
}

impl Provider {
    pub const ALL: [Provider; 3] = [Provider::Claude, Provider::Codex, Provider::Antigravity];

    pub fn key(self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
            Provider::Antigravity => "antigravity",
        }
    }

    pub fn parse(value: &str) -> Option<Provider> {
        Provider::ALL
            .into_iter()
            .find(|p| p.key().eq_ignore_ascii_case(value.trim()))
    }

    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Tracked, but no answer yet.
    Pending,
    Ok,
    /// The last attempt failed; `windows` keeps the last good rows.
    Error,
    LoggedOut,
    /// Not enabled in the widget's Settings.
    Disabled,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Window {
    pub id: String,
    pub label: String,
    pub used_percent: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_minutes: Option<u64>,
    pub resets_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExtraUsage {
    pub enabled: bool,
    pub used_usd: f64,
    pub monthly_limit_usd: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProviderState {
    pub status: Status,
    pub message: Option<String>,
    pub plan: Option<String>,
    pub windows: Vec<Window>,
    pub extra_usage: Option<ExtraUsage>,
    /// Last successful read (unix ms).
    pub updated_at_ms: Option<i64>,
    /// Last attempt, successful or not (unix ms).
    pub checked_at_ms: Option<i64>,
}

impl ProviderState {
    fn new(status: Status) -> Self {
        ProviderState {
            status,
            message: None,
            plan: None,
            windows: Vec::new(),
            extra_usage: None,
            updated_at_ms: None,
            checked_at_ms: None,
        }
    }

    fn apply_ok(
        &mut self,
        windows: Vec<Window>,
        plan: Option<String>,
        extra_usage: Option<ExtraUsage>,
        now_ms: i64,
    ) {
        if self.status == Status::Disabled {
            return;
        }
        self.status = Status::Ok;
        self.message = None;
        self.plan = plan;
        self.windows = windows;
        self.extra_usage = extra_usage;
        self.updated_at_ms = Some(now_ms);
        self.checked_at_ms = Some(now_ms);
    }

    /// A sign-out drops the old rows (the next account may be a different
    /// one); a transient error keeps them, flagged by status and age.
    fn apply_failure(&mut self, status: Status, message: String, now_ms: i64) {
        if self.status == Status::Disabled {
            return;
        }
        self.status = status;
        self.message = Some(message);
        self.checked_at_ms = Some(now_ms);
        if status == Status::LoggedOut {
            self.plan = None;
            self.windows.clear();
            self.extra_usage = None;
            self.updated_at_ms = None;
        }
    }

    fn apply_tracked(&mut self, tracked: bool) {
        if !tracked {
            *self = ProviderState::new(Status::Disabled);
        } else if self.status == Status::Disabled {
            *self = ProviderState::new(Status::Pending);
        }
    }
}

fn store() -> &'static Mutex<[ProviderState; 3]> {
    static STORE: OnceLock<Mutex<[ProviderState; 3]>> = OnceLock::new();
    // Claude is always tracked; the optional providers report in once the
    // frontend has read their Settings toggles.
    STORE.get_or_init(|| {
        Mutex::new([
            ProviderState::new(Status::Pending),
            ProviderState::new(Status::Disabled),
            ProviderState::new(Status::Disabled),
        ])
    })
}

fn update(provider: Provider, f: impl FnOnce(&mut ProviderState)) {
    if let Ok(mut all) = store().lock() {
        f(&mut all[provider.index()]);
    }
}

pub fn current() -> [ProviderState; 3] {
    match store().lock() {
        Ok(all) => all.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

pub fn record_ok(
    provider: Provider,
    windows: Vec<Window>,
    plan: Option<String>,
    extra_usage: Option<ExtraUsage>,
    now_ms: i64,
) {
    update(provider, |s| s.apply_ok(windows, plan, extra_usage, now_ms));
}

pub fn record_failure(provider: Provider, status: Status, message: String, now_ms: i64) {
    update(provider, |s| s.apply_failure(status, message, now_ms));
}

/// Mirrors the widget's Settings toggle for an optional provider.
pub fn set_tracked(provider: Provider, tracked: bool) {
    update(provider, |s| s.apply_tracked(tracked));
}

// ---------- time helpers ----------

pub fn rfc3339_from_ms(ms: i64) -> Option<String> {
    OffsetDateTime::from_unix_timestamp(ms.div_euclid(1000))
        .ok()?
        .format(&Rfc3339)
        .ok()
}

pub fn parse_rfc3339_ms(value: &str) -> Option<i64> {
    let t = OffsetDateTime::parse(value.trim(), &Rfc3339).ok()?;
    i64::try_from(t.unix_timestamp_nanos() / 1_000_000).ok()
}

// ---------- normalization ----------

fn slug(value: &str) -> String {
    let mut out = String::new();
    for c in value.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    out.trim_end_matches('_').to_string()
}

fn non_empty(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Same labels as `limitLabel()` in `app/src/main.js`.
fn claude_limit(limit: &Value) -> Option<Window> {
    let used = limit.get("percent")?.as_f64().filter(|v| v.is_finite())?;
    let kind = non_empty(limit.get("kind"));
    let group = non_empty(limit.get("group"));
    let model = non_empty(limit.pointer("/scope/model/display_name"));
    let surface = non_empty(limit.pointer("/scope/surface/display_name"));
    let scope_name = model.clone().or(surface);

    let label = match (kind.as_deref(), &scope_name) {
        (Some("session"), _) => "Session (5h)".to_string(),
        (Some("weekly_all"), _) => "Weekly (all models)".to_string(),
        (_, Some(name)) => format!("{name} weekly"),
        _ if group.as_deref() == Some("weekly") => "Weekly".to_string(),
        (Some(k), None) => k.to_string(),
        (None, None) => "Limit".to_string(),
    };
    let base = kind.clone().unwrap_or_else(|| "limit".into());
    let id = match (kind.as_deref(), &scope_name) {
        (Some("session") | Some("weekly_all"), _) | (_, None) => base,
        (_, Some(name)) => format!("{base}:{}", slug(name)),
    };
    let window_minutes = match kind.as_deref() {
        Some("session") => Some(300),
        Some(k) if k.contains("weekly") => Some(10_080),
        _ if group.as_deref() == Some("weekly") => Some(10_080),
        _ => None,
    };
    let model = match kind.as_deref() {
        Some("session") | Some("weekly_all") => None,
        _ => model,
    };
    Some(Window {
        id,
        label,
        used_percent: used,
        window_minutes,
        resets_at: non_empty(limit.get("resets_at")),
        model,
    })
}

const CLAUDE_LEGACY: [(&str, &str, &str, Option<&str>); 4] = [
    ("five_hour", "session", "Session (5h)", None),
    ("seven_day", "weekly_all", "Weekly", None),
    ("seven_day_opus", "weekly:opus", "Opus weekly", Some("Opus")),
    (
        "seven_day_sonnet",
        "weekly:sonnet",
        "Sonnet weekly",
        Some("Sonnet"),
    ),
];

/// Rows from a claude.ai `/usage` response: the dynamic `limits` array, or
/// the legacy top-level `five_hour` / `seven_day*` objects.
pub fn claude_windows(data: &Value) -> Vec<Window> {
    if let Some(limits) = data
        .get("limits")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
    {
        return limits.iter().filter_map(claude_limit).collect();
    }
    CLAUDE_LEGACY
        .iter()
        .filter_map(|(key, id, label, model)| {
            let v = data.get(*key)?;
            let used = v.get("utilization")?.as_f64().filter(|u| u.is_finite())?;
            Some(Window {
                id: id.to_string(),
                label: label.to_string(),
                used_percent: used,
                window_minutes: Some(if *key == "five_hour" { 300 } else { 10_080 }),
                resets_at: non_empty(v.get("resets_at")),
                model: model.map(str::to_string),
            })
        })
        .collect()
}

pub fn claude_extra_usage(data: &Value) -> Option<ExtraUsage> {
    let extra = data.get("extra_usage")?;
    if extra.get("is_enabled").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let cents = |key: &str| extra.get(key).and_then(Value::as_f64).unwrap_or(0.0) / 100.0;
    Some(ExtraUsage {
        enabled: true,
        used_usd: cents("used_credits"),
        monthly_limit_usd: cents("monthly_limit"),
    })
}

// ---------- serialized view ----------

#[derive(Serialize)]
pub struct WindowView {
    #[serde(flatten)]
    pub window: Window,
    pub resets_in_seconds: Option<i64>,
}

#[derive(Serialize)]
pub struct ProviderView {
    pub tracked: bool,
    pub status: Status,
    pub message: Option<String>,
    pub plan: Option<String>,
    pub updated_at: Option<String>,
    pub age_seconds: Option<i64>,
    pub windows: Vec<WindowView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_usage: Option<ExtraUsage>,
}

#[derive(Serialize)]
pub struct Providers {
    pub claude: ProviderView,
    pub codex: ProviderView,
    pub antigravity: ProviderView,
}

#[derive(Serialize)]
pub struct SnapshotView {
    pub schema_version: u32,
    pub app_version: &'static str,
    pub generated_at: Option<String>,
    pub providers: Providers,
}

pub fn provider_view(state: &ProviderState, now_ms: i64) -> ProviderView {
    ProviderView {
        tracked: state.status != Status::Disabled,
        status: state.status,
        message: state.message.clone(),
        plan: state.plan.clone(),
        updated_at: state.updated_at_ms.and_then(rfc3339_from_ms),
        age_seconds: state.updated_at_ms.map(|t| (now_ms - t).max(0) / 1000),
        windows: state
            .windows
            .iter()
            .map(|w| WindowView {
                resets_in_seconds: w
                    .resets_at
                    .as_deref()
                    .and_then(parse_rfc3339_ms)
                    .map(|t| ((t - now_ms) / 1000).max(0)),
                window: w.clone(),
            })
            .collect(),
        extra_usage: state.extra_usage.clone(),
    }
}

pub fn view(states: &[ProviderState; 3], now_ms: i64) -> SnapshotView {
    SnapshotView {
        schema_version: SCHEMA_VERSION,
        app_version: env!("CARGO_PKG_VERSION"),
        generated_at: rfc3339_from_ms(now_ms),
        providers: Providers {
            claude: provider_view(&states[0], now_ms),
            codex: provider_view(&states[1], now_ms),
            antigravity: provider_view(&states[2], now_ms),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_limits_array_matches_widget_labels() {
        let data = json!({
            "limits": [
                {"kind": "session", "group": "session", "percent": 41.0, "resets_at": "2026-11-27T05:00:00.651205+00:00"},
                {"kind": "weekly_all", "group": "weekly", "percent": 63, "resets_at": "2026-12-01T00:00:00+00:00"},
                {"kind": "weekly_model", "group": "weekly", "percent": 12.5,
                 "scope": {"model": {"display_name": "Fable"}}, "resets_at": null},
                {"kind": "weekly_model", "group": "weekly", "percent": "n/a"}
            ],
            "extra_usage": {"is_enabled": true, "used_credits": 123, "monthly_limit": 5000}
        });
        let rows = claude_windows(&data);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].id, "session");
        assert_eq!(rows[0].label, "Session (5h)");
        assert_eq!(rows[0].window_minutes, Some(300));
        assert_eq!(rows[1].id, "weekly_all");
        assert_eq!(rows[1].label, "Weekly (all models)");
        assert_eq!(rows[1].model, None);
        assert_eq!(rows[2].id, "weekly_model:fable");
        assert_eq!(rows[2].label, "Fable weekly");
        assert_eq!(rows[2].model.as_deref(), Some("Fable"));
        assert_eq!(rows[2].resets_at, None);
        let extra = claude_extra_usage(&data).unwrap();
        assert!((extra.used_usd - 1.23).abs() < 1e-9);
        assert!((extra.monthly_limit_usd - 50.0).abs() < 1e-9);
    }

    #[test]
    fn claude_legacy_shape_is_still_read() {
        let data = json!({
            "five_hour": {"utilization": 10.0, "resets_at": "2026-11-27T05:00:00Z"},
            "seven_day": {"utilization": 20.0},
            "seven_day_opus": {"utilization": 95.0},
            "seven_day_sonnet": null
        });
        let rows = claude_windows(&data);
        let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["session", "weekly_all", "weekly:opus"]);
        assert_eq!(rows[2].model.as_deref(), Some("Opus"));
        assert!(claude_extra_usage(&data).is_none());
    }

    #[test]
    fn rfc3339_round_trip_and_offsets() {
        let ms = parse_rfc3339_ms("2026-11-27T05:00:00.651205+00:00").unwrap();
        assert_eq!(rfc3339_from_ms(ms).as_deref(), Some("2026-11-27T05:00:00Z"));
        assert_eq!(
            parse_rfc3339_ms("2026-11-27T14:00:00+09:00"),
            parse_rfc3339_ms("2026-11-27T05:00:00Z")
        );
        assert_eq!(parse_rfc3339_ms("not a date"), None);
    }

    #[test]
    fn view_reports_age_and_countdown() {
        let now = parse_rfc3339_ms("2026-11-27T04:00:00Z").unwrap();
        let mut state = ProviderState::new(Status::Ok);
        state.updated_at_ms = Some(now - 30_000);
        state.windows.push(Window {
            id: "session".into(),
            label: "Session (5h)".into(),
            used_percent: 40.0,
            window_minutes: Some(300),
            resets_at: Some("2026-11-27T05:00:00Z".into()),
            model: None,
        });
        let v = provider_view(&state, now);
        assert_eq!(v.age_seconds, Some(30));
        assert_eq!(v.windows[0].resets_in_seconds, Some(3600));
        let json = serde_json::to_value(&v).unwrap();
        assert_eq!(json["status"], "ok");
        assert_eq!(json["windows"][0]["used_percent"], 40.0);
        assert!(json.get("extra_usage").is_none());
    }

    #[test]
    fn state_transitions() {
        let row = Window {
            id: "x".into(),
            label: "x".into(),
            used_percent: 1.0,
            window_minutes: None,
            resets_at: None,
            model: None,
        };
        // Optional providers ignore results until the widget tracks them.
        let mut s = ProviderState::new(Status::Disabled);
        s.apply_ok(vec![row.clone()], None, None, 1);
        assert_eq!(s.status, Status::Disabled);
        s.apply_tracked(true);
        assert_eq!(s.status, Status::Pending);
        s.apply_ok(vec![row.clone()], Some("plus".into()), None, 1_000);
        assert_eq!(s.status, Status::Ok);
        assert_eq!(s.updated_at_ms, Some(1_000));
        // A transient error keeps the last rows and their timestamp.
        s.apply_failure(Status::Error, "network".into(), 2_000);
        assert_eq!(s.windows.len(), 1);
        assert_eq!(s.updated_at_ms, Some(1_000));
        assert_eq!(s.checked_at_ms, Some(2_000));
        // Re-enabling an already tracked provider keeps its data.
        s.apply_tracked(true);
        assert_eq!(s.windows.len(), 1);
        // A sign-out drops them.
        s.apply_failure(Status::LoggedOut, "sign in".into(), 3_000);
        assert!(s.windows.is_empty());
        assert_eq!(s.plan, None);
        assert_eq!(s.updated_at_ms, None);
        s.apply_tracked(false);
        assert_eq!(s, ProviderState::new(Status::Disabled));
    }
}
