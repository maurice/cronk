//! Short, locale-aware date formatting for GitLab's `YYYY-MM-DD` values.
//!
//! chrono exposes each locale's month names and numeric date pattern (`%x`) but no
//! "medium" pattern, so the numeric pattern is only used to learn the field order.
//! The result is e.g. `28 Sep 2026` (UK), `Sep 28, 2026` (US) or `2026-09-28`.

use chrono::{Locale, NaiveDate};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Order {
    Dmy,
    Mdy,
    Ymd,
}

/// The user's time locale from `LC_ALL`, `LC_TIME` or `LANG`; `None` for C/POSIX or unknown values.
pub fn locale_from_env() -> Option<Locale> {
    let value = ["LC_ALL", "LC_TIME", "LANG"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.trim().is_empty())?;
    parse_locale(&value)
}

fn parse_locale(value: &str) -> Option<Locale> {
    let name = value.split(['.', '@']).next().unwrap_or_default().trim();
    if name.is_empty() || name == "C" || name == "POSIX" {
        return None;
    }
    Locale::try_from(name).ok()
}

fn order(locale: Locale) -> Order {
    let probe = NaiveDate::from_ymd_opt(2001, 12, 25).expect("valid probe date");
    let numeric = probe.format_localized("%x", locale).to_string();
    let (day, month, year) = (numeric.find("25"), numeric.find("12"), numeric.find("01"));
    match (day, month, year) {
        (Some(d), Some(m), Some(y)) if y < d.min(m) => Order::Ymd,
        (Some(d), Some(m), _) if m < d => Order::Mdy,
        _ => Order::Dmy,
    }
}

pub fn format_date(date: NaiveDate, locale: Option<Locale>) -> String {
    let Some(locale) = locale else {
        return date.format("%-d %b %Y").to_string();
    };
    let pattern = match order(locale) {
        Order::Dmy => "%-d %b %Y",
        Order::Mdy => "%b %-d, %Y",
        Order::Ymd => "%Y-%m-%d",
    };
    date.format_localized(pattern, locale).to_string()
}

fn format_value(value: Option<&str>, locale: Option<Locale>) -> String {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => "?".into(),
        Some(text) => NaiveDate::parse_from_str(text, "%Y-%m-%d")
            .map(|date| format_date(date, locale))
            .unwrap_or_else(|_| text.to_owned()),
    }
}

pub fn format_range_with(start: Option<&str>, due: Option<&str>, locale: Option<Locale>) -> String {
    format!(
        "{} - {}",
        format_value(start, locale),
        format_value(due, locale)
    )
}

/// `28 Sep 2026 - 11 Oct 2026` in the user's locale.
pub fn format_range(start: Option<&str>, due: Option<&str>) -> String {
    format_range_with(start, due, locale_from_env())
}

/// Unix seconds for an RFC 3339 timestamp such as GitLab's `2026-09-28T10:15:00.000Z`.
pub fn parse_unix(timestamp: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(timestamp.trim())
        .ok()
        .map(|t| t.timestamp())
}

/// Coarse relative time (`just now`, `2 min ago`, `3 h ago`, `yesterday`, `5 days ago`),
/// falling back to the locale date after a month. `now` is explicit so captures stay deterministic.
pub fn format_relative(timestamp: &str, now_unix: i64) -> String {
    let Some(then) = parse_unix(timestamp) else {
        return timestamp.to_owned();
    };
    let delta = now_unix - then;
    if delta < 45 {
        "just now".into()
    } else if delta < 90 {
        "1 min ago".into()
    } else if delta < 3600 {
        format!("{} min ago", (delta + 30) / 60)
    } else if delta < 7200 {
        "1 h ago".into()
    } else if delta < 86_400 {
        format!("{} h ago", delta / 3600)
    } else if delta < 2 * 86_400 {
        "yesterday".into()
    } else if delta < 31 * 86_400 {
        format!("{} days ago", delta / 86_400)
    } else {
        chrono::DateTime::from_timestamp(then, 0)
            .map(|t| format_date(t.date_naive(), locale_from_env()))
            .unwrap_or_else(|| timestamp.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(locale: &str) -> String {
        format_range_with(Some("2026-09-28"), Some("2026-10-11"), parse_locale(locale))
    }

    #[test]
    fn formats_in_the_locale_field_order() {
        assert_eq!(range("en_GB.UTF-8"), "28 Sep 2026 - 11 Oct 2026");
        assert_eq!(range("en_US.UTF-8"), "Sep 28, 2026 - Oct 11, 2026");
        assert_eq!(range("de_DE@euro"), "28 Sep 2026 - 11 Okt 2026");
        assert_eq!(range("ja_JP.UTF-8"), "2026-09-28 - 2026-10-11");
    }

    #[test]
    fn unknown_locales_use_an_unambiguous_english_format() {
        for locale in ["C", "POSIX", "", "xx_YY"] {
            assert_eq!(range(locale), "28 Sep 2026 - 11 Oct 2026", "{locale}");
        }
    }

    #[test]
    fn missing_or_unparseable_dates_are_shown_plainly() {
        assert_eq!(format_range_with(None, Some("soon"), None), "? - soon");
    }

    #[test]
    fn relative_times_are_coarse() {
        let now = parse_unix("2026-09-28T12:00:00Z").unwrap();
        let rel = |t: &str| format_relative(t, now);
        assert_eq!(rel("2026-09-28T11:59:40Z"), "just now");
        assert_eq!(rel("2026-09-28T11:58:00Z"), "2 min ago");
        assert_eq!(rel("2026-09-28T09:10:00Z"), "2 h ago");
        assert_eq!(rel("2026-09-27T09:10:00Z"), "yesterday");
        assert_eq!(rel("2026-09-20T09:10:00Z"), "8 days ago");
        assert_eq!(rel("nonsense"), "nonsense");
    }
}
