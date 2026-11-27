//! Provider recommendation for agents: walk the caller's preference order
//! and pick the first provider that is not known to be at its limit.
//!
//! A provider whose quota is unknown (not tracked, stale, signed out) is not
//! skipped: the preference order wins until there is evidence the preferred
//! provider is exhausted, and `confident` tells the caller which case it got.

use crate::snapshot::{parse_rfc3339_ms, rfc3339_from_ms, Provider, ProviderState, Status};
use serde::Serialize;

pub const DEFAULT_MAX_USED: f64 = 90.0;
pub const DEFAULT_MAX_AGE_SECS: i64 = 600;

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub prefer: Vec<Provider>,
    /// Per-provider ceiling on used percent, indexed like `Provider::ALL`.
    pub max_used: [f64; 3],
    /// Only count Claude model-scoped limits for this model (e.g. "opus").
    /// Unscoped limits (session, all-models weekly) always count.
    pub claude_model: Option<String>,
    /// Rows older than this are treated as unknown.
    pub max_age_secs: i64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            prefer: vec![Provider::Claude, Provider::Codex],
            max_used: [DEFAULT_MAX_USED; 3],
            claude_model: None,
            max_age_secs: DEFAULT_MAX_AGE_SECS,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Available,
    Exhausted,
    Unknown,
}

#[derive(Debug, Serialize)]
pub struct Blocking {
    pub id: String,
    pub label: String,
    pub used_percent: f64,
    pub resets_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Candidate {
    pub provider: &'static str,
    pub verdict: Verdict,
    pub reason: String,
    pub max_used_percent: f64,
    pub peak_used_percent: Option<f64>,
    pub headroom_percent: Option<f64>,
    pub blocking: Vec<Blocking>,
    /// When every blocking window has reset (exhausted only).
    pub available_again_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Recommendation {
    /// `null` when every preferred provider is at its limit.
    pub provider: Option<&'static str>,
    pub confident: bool,
    pub reason: String,
    /// Earliest `available_again_at` when no provider is recommended.
    pub retry_at: Option<String>,
    pub candidates: Vec<Candidate>,
}

fn model_matches(window_model: &str, wanted: &str) -> bool {
    let a = window_model.to_ascii_lowercase();
    let b = wanted.trim().to_ascii_lowercase();
    !b.is_empty() && (a.contains(&b) || b.contains(&a))
}

fn evaluate(provider: Provider, state: &ProviderState, opts: &Options, now_ms: i64) -> Candidate {
    let max_used = opts.max_used[Provider::ALL
        .iter()
        .position(|p| *p == provider)
        .unwrap_or(0)];
    let unknown = |reason: String| Candidate {
        provider: provider.key(),
        verdict: Verdict::Unknown,
        reason,
        max_used_percent: max_used,
        peak_used_percent: None,
        headroom_percent: None,
        blocking: Vec::new(),
        available_again_at: None,
    };

    match state.status {
        Status::Disabled => {
            return unknown("not tracked; enable it in the Claude Usage widget settings".into())
        }
        Status::LoggedOut => {
            return unknown(format!(
                "signed out: {}",
                state.message.as_deref().unwrap_or("sign in again")
            ))
        }
        Status::Pending => return unknown("waiting for the first reading".into()),
        Status::Ok | Status::Error => {}
    }
    let Some(updated) = state.updated_at_ms else {
        return unknown(format!(
            "no reading yet: {}",
            state.message.as_deref().unwrap_or("unknown error")
        ));
    };
    let age = (now_ms - updated).max(0) / 1000;
    if age > opts.max_age_secs {
        return unknown(format!(
            "last reading is {age}s old (max {}s){}",
            opts.max_age_secs,
            state
                .message
                .as_deref()
                .map(|m| format!("; last error: {m}"))
                .unwrap_or_default()
        ));
    }

    let relevant: Vec<_> = state
        .windows
        .iter()
        .filter(|w| match (provider, &w.model, &opts.claude_model) {
            (Provider::Claude, Some(model), Some(wanted)) => model_matches(model, wanted),
            _ => true,
        })
        .collect();
    if relevant.is_empty() {
        return unknown("no limits reported".into());
    }

    let peak = relevant
        .iter()
        .map(|w| w.used_percent)
        .fold(f64::NEG_INFINITY, f64::max);
    let blocking: Vec<Blocking> = relevant
        .iter()
        .filter(|w| w.used_percent >= max_used)
        .map(|w| Blocking {
            id: w.id.clone(),
            label: w.label.clone(),
            used_percent: w.used_percent,
            resets_at: w.resets_at.clone(),
        })
        .collect();

    if blocking.is_empty() {
        return Candidate {
            provider: provider.key(),
            verdict: Verdict::Available,
            reason: format!("peak usage {peak:.0}% is below {max_used:.0}%"),
            max_used_percent: max_used,
            peak_used_percent: Some(peak),
            headroom_percent: Some(max_used - peak),
            blocking,
            available_again_at: None,
        };
    }

    // Available again once the last blocking window has reset; unknown if
    // any of them has no reset time.
    let resets: Option<Vec<i64>> = blocking
        .iter()
        .map(|b| b.resets_at.as_deref().and_then(parse_rfc3339_ms))
        .collect();
    let available_again_at = resets
        .and_then(|r| r.into_iter().max())
        .and_then(rfc3339_from_ms);
    let names: Vec<_> = blocking
        .iter()
        .map(|b| format!("{} {:.0}%", b.label, b.used_percent))
        .collect();
    Candidate {
        provider: provider.key(),
        verdict: Verdict::Exhausted,
        reason: format!("at or above {max_used:.0}%: {}", names.join(", ")),
        max_used_percent: max_used,
        peak_used_percent: Some(peak),
        headroom_percent: Some(max_used - peak),
        blocking,
        available_again_at,
    }
}

pub fn recommend(states: &[ProviderState; 3], opts: &Options, now_ms: i64) -> Recommendation {
    let candidates: Vec<Candidate> = opts
        .prefer
        .iter()
        .map(|p| {
            let i = Provider::ALL.iter().position(|q| q == p).unwrap_or(0);
            evaluate(*p, &states[i], opts, now_ms)
        })
        .collect();

    if let Some(pick) = candidates.iter().find(|c| c.verdict != Verdict::Exhausted) {
        let skipped: Vec<_> = candidates
            .iter()
            .take_while(|c| c.provider != pick.provider)
            .map(|c| c.provider)
            .collect();
        let mut reason = if skipped.is_empty() {
            format!("{} is the first preference", pick.provider)
        } else {
            format!("{} is at its limit", skipped.join(" and "))
        };
        if pick.verdict == Verdict::Unknown {
            reason.push_str(&format!(
                "; {} quota is unknown ({}), so the preference order is kept",
                pick.provider, pick.reason
            ));
        } else {
            reason.push_str(&format!("; {}: {}", pick.provider, pick.reason));
        }
        return Recommendation {
            provider: Some(pick.provider),
            confident: pick.verdict == Verdict::Available,
            reason,
            retry_at: None,
            candidates,
        };
    }

    let retry_at = candidates
        .iter()
        .filter_map(|c| c.available_again_at.as_deref().and_then(parse_rfc3339_ms))
        .min()
        .and_then(rfc3339_from_ms);
    Recommendation {
        provider: None,
        confident: !candidates.is_empty(),
        reason: if candidates.is_empty() {
            "no providers to choose from".into()
        } else {
            "every preferred provider is at or above its limit".into()
        },
        retry_at,
        candidates,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::Window;

    const NOW: i64 = 1_800_000_000_000;

    fn window(id: &str, used: f64, model: Option<&str>, resets_at: Option<&str>) -> Window {
        Window {
            id: id.into(),
            label: id.into(),
            used_percent: used,
            window_minutes: None,
            resets_at: resets_at.map(str::to_string),
            model: model.map(str::to_string),
        }
    }

    fn ok(windows: Vec<Window>) -> ProviderState {
        ProviderState {
            status: Status::Ok,
            message: None,
            plan: None,
            windows,
            extra_usage: None,
            updated_at_ms: Some(NOW - 30_000),
            checked_at_ms: Some(NOW - 30_000),
        }
    }

    fn with_status(status: Status) -> ProviderState {
        ProviderState {
            status,
            ..ok(Vec::new())
        }
    }

    #[test]
    fn prefers_claude_while_it_has_headroom() {
        let states = [
            ok(vec![
                window("session", 41.0, None, None),
                window("weekly_all", 63.0, None, None),
            ]),
            ok(vec![window("weekly", 5.0, None, None)]),
            with_status(Status::Disabled),
        ];
        let r = recommend(&states, &Options::default(), NOW);
        assert_eq!(r.provider, Some("claude"));
        assert!(r.confident);
        assert_eq!(r.candidates[0].headroom_percent, Some(27.0));
    }

    #[test]
    fn falls_back_to_codex_when_claude_session_is_spent() {
        let states = [
            ok(vec![
                window("session", 93.0, None, Some("2026-11-27T05:00:00Z")),
                window("weekly_all", 63.0, None, None),
            ]),
            ok(vec![window("weekly", 40.0, None, None)]),
            with_status(Status::Disabled),
        ];
        let r = recommend(&states, &Options::default(), NOW);
        assert_eq!(r.provider, Some("codex"));
        assert!(r.confident);
        assert_eq!(r.candidates[0].verdict, Verdict::Exhausted);
        assert_eq!(r.candidates[0].blocking[0].id, "session");
        assert_eq!(
            r.candidates[0].available_again_at.as_deref(),
            Some("2026-11-27T05:00:00Z")
        );
        assert!(r.reason.starts_with("claude is at its limit"));
    }

    #[test]
    fn per_provider_ceiling_protects_a_shared_account() {
        let states = [
            ok(vec![window(
                "session",
                95.0,
                None,
                Some("2026-11-27T05:00:00Z"),
            )]),
            ok(vec![window(
                "weekly",
                75.0,
                None,
                Some("2026-11-30T00:00:00Z"),
            )]),
            with_status(Status::Disabled),
        ];
        let mut opts = Options::default();
        opts.max_used[1] = 70.0;
        let r = recommend(&states, &opts, NOW);
        assert_eq!(r.provider, None);
        assert_eq!(r.retry_at.as_deref(), Some("2026-11-27T05:00:00Z"));
    }

    #[test]
    fn claude_model_filter_ignores_other_model_limits() {
        let states = [
            ok(vec![
                window("session", 20.0, None, None),
                window("weekly_model:fable", 99.0, Some("Fable"), None),
                window("weekly_model:opus", 50.0, Some("Opus"), None),
            ]),
            ok(vec![window("weekly", 5.0, None, None)]),
            with_status(Status::Disabled),
        ];
        // Without a model, every Claude limit counts (conservative).
        let r = recommend(&states, &Options::default(), NOW);
        assert_eq!(r.provider, Some("codex"));
        // Routing Opus work only cares about Opus and the shared limits.
        let opts = Options {
            claude_model: Some("opus".into()),
            ..Options::default()
        };
        let r = recommend(&states, &opts, NOW);
        assert_eq!(r.provider, Some("claude"));
        assert_eq!(r.candidates[0].peak_used_percent, Some(50.0));
    }

    #[test]
    fn unknown_preferred_provider_keeps_the_order_but_is_not_confident() {
        let mut stale = ok(vec![window("session", 10.0, None, None)]);
        stale.updated_at_ms = Some(NOW - 3_600_000);
        let states = [
            stale,
            ok(vec![window("weekly", 5.0, None, None)]),
            with_status(Status::Disabled),
        ];
        let r = recommend(&states, &Options::default(), NOW);
        assert_eq!(r.provider, Some("claude"));
        assert!(!r.confident);
        assert_eq!(r.candidates[0].verdict, Verdict::Unknown);
        assert!(r.candidates[0].reason.contains("old"));
    }

    #[test]
    fn disabled_fallback_is_unknown_and_still_chosen_after_exhaustion() {
        let states = [
            ok(vec![window("session", 100.0, None, None)]),
            with_status(Status::Disabled),
            with_status(Status::Disabled),
        ];
        let r = recommend(&states, &Options::default(), NOW);
        assert_eq!(r.provider, Some("codex"));
        assert!(!r.confident);
        assert!(r.candidates[1].reason.contains("not tracked"));
        // A blocking window without a reset time gives no retry estimate.
        assert_eq!(r.candidates[0].available_again_at, None);
    }

    #[test]
    fn error_status_still_uses_recent_rows() {
        let mut s = ok(vec![window("session", 95.0, None, None)]);
        s.status = Status::Error;
        s.message = Some("network: timeout".into());
        let states = [
            s,
            ok(vec![window("weekly", 5.0, None, None)]),
            with_status(Status::Disabled),
        ];
        let r = recommend(&states, &Options::default(), NOW);
        assert_eq!(r.provider, Some("codex"));
        assert_eq!(r.candidates[0].verdict, Verdict::Exhausted);
    }
}
