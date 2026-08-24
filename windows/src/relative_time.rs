//! Small date formatting helpers shared by the popover and error strings.
//!
//! Port of `menubar/Sources/ClaudeQuotaBar/RelativeTime.swift`. The strings are
//! reproduced exactly: they are what makes the popover read like Claude's own
//! usage screen, and `presentation` tests pin every one of them.

use chrono::{DateTime, Local, Utc};

/// "3:45 PM", or "Sat 3:45 PM" once the date is more than a day out.
pub fn clock(date: DateTime<Utc>) -> String {
    clock_from(date, Utc::now())
}

pub fn clock_from(date: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let local = date.with_timezone(&Local);
    let seconds = (date - now).num_seconds();
    if seconds < 86_400 {
        // %-I is not portable; strip the leading zero by hand.
        strip_leading_zero(&local.format("%I:%M %p").to_string())
    } else {
        let rendered = local.format("%a %I:%M %p").to_string();
        // Only the hour field can carry the padding zero, and it sits after the
        // weekday and a space.
        match rendered.split_once(' ') {
            Some((weekday, rest)) => format!("{} {}", weekday, strip_leading_zero(rest)),
            None => rendered,
        }
    }
}

fn strip_leading_zero(value: &str) -> String {
    value.strip_prefix('0').unwrap_or(value).to_string()
}

/// A spelled-out countdown: "3 hr 19 min", "45 min", "2 days 4 hr".
/// Phrased the way Claude's usage screen phrases the same thing.
pub fn long_countdown(to: DateTime<Utc>, from: DateTime<Utc>) -> String {
    let seconds = (to - from).num_seconds();
    if seconds <= 0 {
        return "now".to_string();
    }
    if seconds < 60 {
        return format!("{} sec", seconds);
    }

    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{} min", minutes);
    }

    let hours = minutes / 60;
    if hours < 24 {
        let remainder = minutes % 60;
        return if remainder == 0 {
            format!("{} hr", hours)
        } else {
            format!("{} hr {} min", hours, remainder)
        };
    }

    let days = hours / 24;
    let remainder = hours % 24;
    let day_label = if days == 1 {
        "1 day".to_string()
    } else {
        format!("{} days", days)
    };
    if remainder == 0 {
        day_label
    } else {
        format!("{} {} hr", day_label, remainder)
    }
}

/// How long ago a snapshot was captured: "just now", "4m ago", "2h ago".
pub fn age(seconds: f64) -> String {
    let seconds = seconds.round().max(0.0) as i64;
    if seconds < 45 {
        return "just now".to_string();
    }
    if seconds < 3_600 {
        return format!("{}m ago", (seconds / 60).max(1));
    }
    if seconds < 86_400 {
        return format!("{}h ago", seconds / 3_600);
    }
    format!("{}d ago", seconds / 86_400)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + seconds, 0).unwrap()
    }

    #[test]
    fn countdown_uses_the_coarsest_useful_unit() {
        let now = at(0);
        assert_eq!(long_countdown(at(30), now), "30 sec");
        assert_eq!(long_countdown(at(60), now), "1 min");
        assert_eq!(long_countdown(at(45 * 60), now), "45 min");
        assert_eq!(long_countdown(at(3 * 3600), now), "3 hr");
        assert_eq!(long_countdown(at(3 * 3600 + 19 * 60), now), "3 hr 19 min");
        assert_eq!(long_countdown(at(24 * 3600), now), "1 day");
        assert_eq!(long_countdown(at(2 * 86400 + 4 * 3600), now), "2 days 4 hr");
    }

    #[test]
    fn a_countdown_that_has_already_elapsed_reads_as_now() {
        assert_eq!(long_countdown(at(-5), at(0)), "now");
        assert_eq!(long_countdown(at(0), at(0)), "now");
    }

    #[test]
    fn age_rounds_down_to_whole_units_but_never_to_zero() {
        assert_eq!(age(3.0), "just now");
        assert_eq!(age(44.0), "just now");
        // Rounds first, then compares -- so 44.9 has already become 45.
        assert_eq!(age(44.9), "1m ago");
        // 45s is under a minute, but "0m ago" would read as fresher than it is.
        assert_eq!(age(45.0), "1m ago");
        assert_eq!(age(240.0), "4m ago");
        assert_eq!(age(7_200.0), "2h ago");
        assert_eq!(age(200_000.0), "2d ago");
    }

    #[test]
    fn clock_drops_the_padding_zero_from_the_hour() {
        // Rendered in local time, so assert on the shape rather than the value:
        // a leading zero would mean "03:45 PM" where Claude shows "3:45 PM".
        let rendered = clock_from(at(3600), at(0));
        assert!(!rendered.starts_with('0'), "{rendered}");
        assert!(rendered.ends_with("AM") || rendered.ends_with("PM"), "{rendered}");
    }

    #[test]
    fn clock_gains_a_weekday_once_it_is_more_than_a_day_out() {
        let rendered = clock_from(at(3 * 86_400), at(0));
        assert_eq!(rendered.split(' ').count(), 3, "{rendered}");
    }
}
