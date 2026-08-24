//! Fetches usage from the same endpoint Claude Code's `/usage` command calls.
//!
//! Port of `menubar/Sources/ClaudeQuotaBar/OAuthUsageClient.swift`.
//!
//! This costs no quota -- it reports your windows, it doesn't consume them --
//! but it is an undocumented endpoint and it does rate limit, so `model` only
//! reaches for it when the local status line cache has gone stale.

use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use serde_json::Value;

use crate::credentials;
use crate::snapshot::{UsageEntry, UsageError, UsageSnapshot, UsageSource, UsageWindow, UsageWindowKind};

const ENDPOINT: &str = "https://api.anthropic.com/api/oauth/usage";
const BETA_HEADER: &str = "oauth-2025-04-20";
const USER_AGENT: &str = concat!("ClaudeQuota/", env!("CARGO_PKG_VERSION"));

/// Split so the caller can tell "your login needs attention" from "the network
/// was unhappy" -- only the second gets backed off.
#[derive(Debug, Clone, PartialEq)]
pub enum Failure {
    Credentials(UsageError),
    Transport(UsageError),
}

pub struct OAuthUsageClient {
    agent: ureq::Agent,
}

impl Default for OAuthUsageClient {
    fn default() -> Self {
        Self::new()
    }
}

impl OAuthUsageClient {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            // We need to read the status code and `Retry-After` off a 429
            // ourselves, so a non-2xx must not come back as a transport error.
            .http_status_as_error(false)
            .user_agent(USER_AGENT)
            .build();
        Self { agent: config.into() }
    }

    pub fn fetch(&self) -> Result<UsageSnapshot, Failure> {
        let credentials =
            credentials::load().ok_or(Failure::Credentials(UsageError::NoCredentials))?;
        if credentials.is_expired() {
            return Err(Failure::Credentials(UsageError::CredentialsExpired));
        }

        let mut response = self
            .agent
            .get(ENDPOINT)
            .header("Authorization", &format!("Bearer {}", credentials.access_token))
            .header("anthropic-beta", BETA_HEADER)
            .header("Accept", "application/json")
            .call()
            .map_err(|error| Failure::Transport(UsageError::Network(error.to_string())))?;

        let status = response.status().as_u16();
        let retry_after =
            response.headers().get("retry-after").and_then(|v| v.to_str().ok()).map(str::to_string);

        match status {
            200 => {
                let body = response
                    .body_mut()
                    .read_to_string()
                    .map_err(|error| Failure::Transport(UsageError::Network(error.to_string())))?;
                let mut snapshot =
                    parse(&body).ok_or(Failure::Transport(UsageError::NoData))?;
                // The plan badge isn't in the payload -- it rides on the
                // credential we just used, so stamp it here rather than
                // reading the credential file a second time from the view.
                snapshot.plan = credentials.plan_label();
                Ok(snapshot)
            }
            401 | 403 => Err(Failure::Credentials(UsageError::CredentialsExpired)),
            // Returned for accounts without subscription-backed windows.
            404 => Err(Failure::Credentials(UsageError::NotSubscribed)),
            429 => Err(Failure::Transport(UsageError::RateLimited {
                retry_at: retry_after.as_deref().and_then(|value| retry_date(value, Utc::now())),
            })),
            other => Err(Failure::Transport(UsageError::Network(format!("HTTP {other}")))),
        }
    }
}

/// Honour `Retry-After`, which may be either seconds or an HTTP date.
pub fn retry_date(value: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let trimmed = value.trim();
    if let Ok(seconds) = trimmed.parse::<f64>() {
        return Some(now + chrono::Duration::milliseconds((seconds * 1000.0) as i64));
    }
    // "Sun, 06 Nov 1994 08:49:37 GMT". The zone is always GMT in practice, and
    // an HTTP date that isn't is malformed, so parse the naive form and stamp
    // UTC on it rather than dragging in a timezone database.
    let naive = NaiveDateTime::parse_from_str(trimmed, "%a, %d %b %Y %H:%M:%S GMT").ok()?;
    Utc.from_utc_datetime(&naive).into()
}

pub fn parse(body: &str) -> Option<UsageSnapshot> {
    let root: Value = serde_json::from_str(body).ok()?;
    let root = root.as_object()?;

    // Prefer the `limits` array. It is self-describing (kind, percent, scope)
    // and it is the only place a model-scoped weekly window shows up. The
    // top-level keys are the fallback for accounts that don't report one.
    let mut windows = parse_limits(root.get("limits"));
    if windows.is_empty() {
        windows = parse_top_level_windows(root);
    }
    if windows.is_empty() {
        return None;
    }

    let mut snapshot = UsageSnapshot::new(windows, Utc::now(), UsageSource::OAuth);
    snapshot.extra_usage_enabled =
        root.get("extra_usage").and_then(|extra| extra.get("is_enabled")).and_then(Value::as_bool);
    Some(snapshot)
}

/// `kind` values we know how to display. Anything else (the endpoint also
/// reports several internal buckets) is skipped rather than guessed at.
fn limit_kind(name: &str) -> Option<UsageWindowKind> {
    match name {
        "session" => Some(UsageWindowKind::FiveHour),
        "weekly_all" => Some(UsageWindowKind::SevenDay),
        "weekly_scoped" => Some(UsageWindowKind::WeeklyScoped),
        _ => None,
    }
}

pub fn parse_limits(value: Option<&Value>) -> Vec<UsageEntry> {
    let Some(Value::Array(entries)) = value else { return Vec::new() };
    entries
        .iter()
        .filter_map(|entry| {
            let kind = limit_kind(entry.get("kind")?.as_str()?)?;
            let percent = decode_number(entry.get("percent"))?;
            let scope_label = entry
                .get("scope")
                .and_then(|scope| scope.get("model"))
                .and_then(|model| model.get("display_name"))
                .and_then(Value::as_str)
                .map(str::to_string);
            Some(UsageEntry::new(
                kind,
                UsageWindow::scoped(percent, decode_date(entry.get("resets_at")), scope_label),
            ))
        })
        .collect()
}

/// The per-key shape, for accounts that report no `limits` array. Still live
/// data, unlike the `seven_day_opus` / `seven_day_sonnet` keys that used to be
/// read here: those come back null now that scoped windows arrive through
/// `limits`.
pub fn parse_top_level_windows(root: &serde_json::Map<String, Value>) -> Vec<UsageEntry> {
    [(UsageWindowKind::FiveHour, "five_hour"), (UsageWindowKind::SevenDay, "seven_day")]
        .into_iter()
        .filter_map(|(kind, key)| {
            let object = root.get(key)?;
            let utilization = decode_number(object.get("utilization"))?;
            Some(UsageEntry::new(
                kind,
                UsageWindow::new(utilization, decode_date(object.get("resets_at"))),
            ))
        })
        .collect()
}

/// `percent` is an integer while `utilization` is a real, so accept either.
///
/// Booleans must not decode as 0/1. Swift needed a CoreFoundation type check
/// for this because `NSNumber` bridges both; `serde_json` keeps `Bool` and
/// `Number` apart, so `as_f64` already declines a boolean -- but getting it
/// wrong silently drops any window sitting at 0% or 1%, so it stays tested.
pub fn decode_number(value: Option<&Value>) -> Option<f64> {
    value?.as_f64()
}

/// `resets_at` comes back as an ISO-8601 string here and as epoch seconds in
/// the status line payload, so accept either.
pub fn decode_date(value: Option<&Value>) -> Option<DateTime<Utc>> {
    let value = value?;
    if let Some(seconds) = value.as_f64() {
        if seconds > 0.0 {
            return Utc.timestamp_millis_opt((seconds * 1000.0) as i64).single();
        }
        return None;
    }
    let text = value.as_str()?;
    if text.is_empty() {
        return None;
    }
    // Handles both the fractional-seconds form and the plain one, and any
    // offset -- normalised to UTC.
    DateTime::parse_from_rfc3339(text).ok().map(|date| date.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limits_array_wins_and_carries_scoped_windows() {
        let snapshot = parse(
            r#"{
              "limits": [
                {"kind": "session", "percent": 23, "resets_at": "2026-08-24T18:00:00Z"},
                {"kind": "weekly_all", "percent": 41.2, "resets_at": "2026-08-30T00:00:00Z"},
                {"kind": "weekly_scoped", "percent": 8,
                 "scope": {"model": {"display_name": "Fable"}}},
                {"kind": "some_internal_bucket", "percent": 99}
              ],
              "five_hour": {"utilization": 1.0},
              "extra_usage": {"is_enabled": true}
            }"#,
        )
        .unwrap();

        assert_eq!(snapshot.windows.len(), 3, "the unknown kind is skipped, not guessed at");
        assert_eq!(snapshot.window(UsageWindowKind::FiveHour).unwrap().used_percentage, 23.0);
        assert_eq!(snapshot.window(UsageWindowKind::SevenDay).unwrap().used_percentage, 41.2);
        let scoped = snapshot.window(UsageWindowKind::WeeklyScoped).unwrap();
        assert_eq!(scoped.scope_label.as_deref(), Some("Fable"));
        assert_eq!(snapshot.extra_usage_enabled, Some(true));
        assert_eq!(snapshot.source, UsageSource::OAuth);
    }

    #[test]
    fn the_top_level_shape_is_the_fallback_when_no_limits_array_arrives() {
        let snapshot = parse(
            r#"{"five_hour": {"utilization": 23.5, "resets_at": 1738425600},
                "seven_day": {"utilization": 41.2, "resets_at": 1738857600}}"#,
        )
        .unwrap();
        assert_eq!(snapshot.windows.len(), 2);
        assert_eq!(snapshot.window(UsageWindowKind::FiveHour).unwrap().used_percentage, 23.5);
        assert_eq!(
            snapshot.window(UsageWindowKind::FiveHour).unwrap().resets_at.unwrap().timestamp(),
            1_738_425_600
        );
    }

    #[test]
    fn percentages_are_zero_to_one_hundred_not_a_fraction() {
        // The legacy CLI headers used 0-1. If that scale ever leaked in here a
        // 41.2% window would render as 41 hundredths of a percent.
        let snapshot = parse(r#"{"limits": [{"kind": "weekly_all", "percent": 41.2}]}"#).unwrap();
        assert!(snapshot.window(UsageWindowKind::SevenDay).unwrap().used_percentage > 1.0);
    }

    #[test]
    fn a_boolean_is_not_a_percentage() {
        assert_eq!(decode_number(Some(&serde_json::json!(true))), None);
        assert_eq!(decode_number(Some(&serde_json::json!(false))), None);
        // ...but the literals it is confusable with must still decode.
        assert_eq!(decode_number(Some(&serde_json::json!(0))), Some(0.0));
        assert_eq!(decode_number(Some(&serde_json::json!(1))), Some(1.0));
    }

    #[test]
    fn a_window_sitting_at_zero_percent_is_kept_not_dropped() {
        let snapshot = parse(r#"{"limits": [{"kind": "session", "percent": 0}]}"#).unwrap();
        assert_eq!(snapshot.window(UsageWindowKind::FiveHour).unwrap().used_percentage, 0.0);
    }

    #[test]
    fn a_payload_with_nothing_usable_is_no_snapshot_at_all() {
        assert!(parse(r#"{"limits": []}"#).is_none());
        assert!(parse(r#"{"limits": [{"kind": "unknown", "percent": 5}]}"#).is_none());
        assert!(parse(r#"{"five_hour": {}}"#).is_none());
        assert!(parse("not json").is_none());
    }

    #[test]
    fn dates_decode_from_both_wire_formats() {
        let epoch = decode_date(Some(&serde_json::json!(1_738_425_600))).unwrap();
        assert_eq!(epoch.timestamp(), 1_738_425_600);

        let plain = decode_date(Some(&serde_json::json!("2026-08-24T18:00:00Z"))).unwrap();
        let fractional =
            decode_date(Some(&serde_json::json!("2026-08-24T18:00:00.512Z"))).unwrap();
        assert_eq!(plain.timestamp(), fractional.timestamp());

        let offset = decode_date(Some(&serde_json::json!("2026-08-24T20:00:00+02:00"))).unwrap();
        assert_eq!(offset.timestamp(), plain.timestamp());
    }

    #[test]
    fn an_unusable_date_is_absent_rather_than_epoch_zero() {
        assert_eq!(decode_date(Some(&serde_json::json!(0))), None);
        assert_eq!(decode_date(Some(&serde_json::json!(""))), None);
        assert_eq!(decode_date(Some(&serde_json::json!("whenever"))), None);
        assert_eq!(decode_date(None), None);
    }

    #[test]
    fn retry_after_reads_as_seconds_or_as_an_http_date() {
        let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        assert_eq!(retry_date("120", now).unwrap().timestamp(), 1_700_000_120);
        assert_eq!(retry_date("  120  ", now).unwrap().timestamp(), 1_700_000_120);

        let date = retry_date("Sun, 06 Nov 1994 08:49:37 GMT", now).unwrap();
        assert_eq!(date.timestamp(), 784_111_777);

        assert_eq!(retry_date("soon", now), None);
    }
}
