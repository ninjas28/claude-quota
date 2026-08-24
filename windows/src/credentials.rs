//! The OAuth credential Claude Code stores after you sign in.
//!
//! Port of `menubar/Sources/ClaudeQuotaBar/ClaudeCredentials.swift`, with the
//! one genuine platform difference in the whole app: macOS keeps this in the
//! Keychain under the service "Claude Code-credentials", Windows keeps it in a
//! plain JSON file at `~/.claude/.credentials.json`. The blob inside is the
//! same shape, so only the loading half differs.
//!
//! This is strictly read-only. We never write, refresh, or rotate the
//! credential: refreshing would rotate the refresh token out from under Claude
//! Code and could sign you out of it. When the token is expired we surface that
//! and wait for Claude Code to renew it on its own. The token is never logged,
//! never written anywhere, and never handed to a child process -- in particular
//! we never set `CLAUDE_CODE_OAUTH_TOKEN`, which makes Claude Code delete its
//! stored credential on exit.

use std::path::PathBuf;

use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct ClaudeCredentials {
    pub access_token: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub subscription_type: Option<String>,
    pub rate_limit_tier: Option<String>,
}

impl ClaudeCredentials {
    /// Treat a token as unusable slightly before its stated expiry so we don't
    /// fire a request that is guaranteed to 401 mid-flight.
    pub fn is_expired_at(&self, now: DateTime<Utc>) -> bool {
        match self.expires_at {
            None => false,
            Some(expires_at) => (expires_at - now).num_seconds() < 30,
        }
    }

    pub fn is_expired(&self) -> bool {
        self.is_expired_at(Utc::now())
    }

    pub fn plan_label(&self) -> Option<String> {
        plan_label(self.subscription_type.as_deref(), self.rate_limit_tier.as_deref())
    }
}

/// The plan badge Claude's own usage screen shows, rebuilt from what the
/// credential happens to carry: `default_claude_max_5x` -> "Max (5x)".
/// Falls back to `subscription_type` when the tier is missing or unfamiliar.
pub fn plan_label(subscription_type: Option<&str>, rate_limit_tier: Option<&str>) -> Option<String> {
    if let Some(tier) = rate_limit_tier {
        let tier = tier.to_lowercase();
        if !tier.is_empty() {
            let cleaned = tier.replace("default_", "").replace("claude_", "");
            let mut parts: Vec<String> =
                cleaned.split('_').filter(|p| !p.is_empty()).map(|p| p.to_string()).collect();

            // A trailing "5x" is the plan multiplier, not part of its name.
            let mut multiplier: Option<String> = None;
            if let Some(last) = parts.last() {
                let stem = &last[..last.len().saturating_sub(1)];
                if last.ends_with('x') && !stem.is_empty() && stem.parse::<i64>().is_ok() {
                    multiplier = Some(last.clone());
                    parts.pop();
                }
            }

            let name = parts.iter().map(|p| capitalized(p)).collect::<Vec<_>>().join(" ");
            if !name.is_empty() {
                return Some(match multiplier {
                    Some(multiplier) => format!("{} ({})", name, multiplier),
                    None => name,
                });
            }
        }
    }

    let subscription_type = subscription_type?;
    if subscription_type.is_empty() {
        return None;
    }
    Some(capitalized(subscription_type))
}

/// Swift's `String.capitalized` uppercases the first character and lowercases
/// the rest of each word. Our inputs are single lowercase words by this point.
fn capitalized(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
    }
}

/// Where Claude Code stores the credential on Windows.
///
/// `CLAUDE_QUOTA_CREDENTIALS` overrides it, which is how the tests exercise the
/// reader without touching the real login.
pub fn credentials_path() -> PathBuf {
    if let Ok(override_path) = std::env::var("CLAUDE_QUOTA_CREDENTIALS") {
        if !override_path.is_empty() {
            return PathBuf::from(override_path);
        }
    }
    crate::paths::claude_home().join(".credentials.json")
}

pub fn load() -> Option<ClaudeCredentials> {
    let data = std::fs::read_to_string(credentials_path()).ok()?;
    parse(&data)
}

/// The stored blob is `{"claudeAiOauth": {"accessToken": ..., "expiresAt": <ms>}}`.
/// Older builds stored the inner object directly, so accept both shapes. The
/// file also holds unrelated `mcpOAuth` entries for plugin logins, which we
/// must not mistake for the subscription credential.
pub fn parse(data: &str) -> Option<ClaudeCredentials> {
    let root: Value = serde_json::from_str(data).ok()?;
    let payload = root.get("claudeAiOauth").unwrap_or(&root);

    let token = payload.get("accessToken")?.as_str()?;
    if token.is_empty() {
        return None;
    }

    let expires_at = payload
        .get("expiresAt")
        .and_then(Value::as_f64)
        .and_then(|milliseconds| Utc.timestamp_millis_opt(milliseconds as i64).single());

    Some(ClaudeCredentials {
        access_token: token.to_string(),
        expires_at,
        subscription_type: payload
            .get("subscriptionType")
            .and_then(Value::as_str)
            .map(str::to_string),
        rate_limit_tier: payload.get("rateLimitTier").and_then(Value::as_str).map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_nested_shape_claude_code_writes() {
        let credentials = parse(
            r#"{"mcpOAuth": {"whatever": {}},
                "claudeAiOauth": {
                  "accessToken": "sk-ant-oat-xyz",
                  "expiresAt": 1700000000000,
                  "subscriptionType": "max",
                  "rateLimitTier": "default_claude_max_5x"
                }}"#,
        )
        .unwrap();
        assert_eq!(credentials.access_token, "sk-ant-oat-xyz");
        assert_eq!(credentials.expires_at.unwrap().timestamp(), 1_700_000_000);
        assert_eq!(credentials.plan_label().unwrap(), "Max (5x)");
    }

    #[test]
    fn reads_the_legacy_flat_shape_too() {
        let credentials = parse(r#"{"accessToken": "flat", "subscriptionType": "pro"}"#).unwrap();
        assert_eq!(credentials.access_token, "flat");
        assert_eq!(credentials.expires_at, None);
        assert_eq!(credentials.plan_label().unwrap(), "Pro");
    }

    #[test]
    fn a_file_holding_only_plugin_logins_is_not_a_subscription_credential() {
        // The real file on Windows carries `mcpOAuth` entries for every plugin
        // the user has connected. None of those is the Claude.ai login, and
        // reading one as if it were would show someone else's quota.
        assert_eq!(parse(r#"{"mcpOAuth": {"plugin:github|abc": {"accessToken": "gho_x"}}}"#), None);
    }

    #[test]
    fn an_empty_or_missing_token_is_not_a_credential() {
        assert_eq!(parse(r#"{"claudeAiOauth": {"accessToken": ""}}"#), None);
        assert_eq!(parse(r#"{"claudeAiOauth": {}}"#), None);
        assert_eq!(parse("not json"), None);
    }

    #[test]
    fn expiry_carries_a_thirty_second_skew() {
        use chrono::Duration;
        let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        let with_expiry = |offset: i64| ClaudeCredentials {
            access_token: "t".into(),
            expires_at: Some(now + Duration::seconds(offset)),
            subscription_type: None,
            rate_limit_tier: None,
        };
        assert!(with_expiry(-1).is_expired_at(now));
        assert!(with_expiry(29).is_expired_at(now), "inside the skew, so unusable");
        assert!(!with_expiry(31).is_expired_at(now));
    }

    #[test]
    fn a_credential_with_no_stated_expiry_never_expires() {
        let credentials = ClaudeCredentials {
            access_token: "t".into(),
            expires_at: None,
            subscription_type: None,
            rate_limit_tier: None,
        };
        assert!(!credentials.is_expired());
    }

    #[test]
    fn plan_labels_are_rebuilt_from_whatever_the_credential_carries() {
        assert_eq!(plan_label(None, Some("default_claude_max_5x")).unwrap(), "Max (5x)");
        assert_eq!(plan_label(None, Some("default_claude_max_20x")).unwrap(), "Max (20x)");
        assert_eq!(plan_label(None, Some("default_claude_pro")).unwrap(), "Pro");
        // Unfamiliar tier: still better than nothing, just title-cased.
        assert_eq!(plan_label(None, Some("enterprise_seat")).unwrap(), "Enterprise Seat");
        // No tier at all: fall back to the subscription type.
        assert_eq!(plan_label(Some("max"), None).unwrap(), "Max");
        assert_eq!(plan_label(Some("max"), Some("")).unwrap(), "Max");
        assert_eq!(plan_label(None, None), None);
        assert_eq!(plan_label(Some(""), None), None);
    }

    #[test]
    fn a_trailing_x_that_is_not_a_multiplier_stays_part_of_the_name() {
        // "max" ends in no digit-x, and a bare "x" has no number in front of
        // it -- neither should be eaten as a multiplier.
        assert_eq!(plan_label(None, Some("default_claude_x")).unwrap(), "X");
    }
}
