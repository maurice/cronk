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
}
