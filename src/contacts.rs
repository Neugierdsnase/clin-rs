//! Parsing Frontmatter-format contact notes, per the obsidian-contacts
//! plugin's "Frontmatter Format":
//! <https://github.com/vbeskrovnov/obsidian-contacts#frontmatter-format>
//!
//! Only this format is supported; the plugin's alternate "Custom Format"
//! (a `/---contact---/` Markdown table) is not read.

use std::borrow::Cow;

use chrono::{Datelike, NaiveDate};
use serde::Deserialize;

use crate::frontmatter;

/// A contact parsed from a note's YAML frontmatter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact {
    pub first_name: String,
    pub last_name: Option<String>,
    /// Birthdate. Only the month and day are meaningful for "is it
    /// today": the year is a birth year when `birth_year_known` is true,
    /// and otherwise an arbitrary leap-year placeholder (see
    /// [`parse_birthday`]) that callers must not display.
    pub birthday: NaiveDate,
    /// Whether `birthday`'s year is an actual birth year, rather than the
    /// placeholder substituted for the ISO 8601 reduced-precision
    /// `--MM-DD` form (a birthday with no known year).
    pub birth_year_known: bool,
}

impl Contact {
    /// "First Last", or just "First" when the note has no last name.
    pub fn display_name(&self) -> String {
        match &self.last_name {
            Some(last) => format!("{} {last}", self.first_name),
            None => self.first_name.clone(),
        }
    }
}

/// Frontmatter fields this reads; every other key (`phone`, `telegram`,
/// `linkedin`, `last_chat`, `friends`, ...) is ignored.
#[derive(Deserialize)]
struct RawFrontmatter {
    #[serde(rename = "type")]
    kind: Option<String>,
    name: Option<RawName>,
    birthday: Option<String>,
}

#[derive(Deserialize)]
struct RawName {
    first: Option<String>,
    last: Option<String>,
}

/// Parse a `birthday` frontmatter value into a date and whether its year
/// is actually known.
///
/// Accepts a full `YYYY-MM-DD` date, and also the ISO 8601
/// reduced-precision form `--MM-DD` that the obsidian-contacts plugin
/// accepts for a birthday whose year is unknown (its own parser matches
/// `[0-9-]+` and feeds that straight to JS `Date`, which special-cases the
/// leading `--`). For the `--MM-DD` form the returned date's year is an
/// arbitrary placeholder -- a leap year, so a leading-`--` February 29
/// stays constructible, matching how a full-year February 29 birthday is
/// only matched in leap years -- and the returned `bool` is `false` so
/// callers know not to display it.
fn parse_birthday(raw: &str) -> Option<(NaiveDate, bool)> {
    match raw.strip_prefix("--") {
        Some(month_day) => {
            let (month, day) = month_day.split_once('-')?;
            let date = NaiveDate::from_ymd_opt(4, month.parse().ok()?, day.parse().ok()?)?;
            Some((date, false))
        }
        None => Some((raw.parse().ok()?, true)),
    }
}

/// Parse a note's YAML frontmatter as a Frontmatter-format contact.
///
/// Returns `None` when the note has no frontmatter, its `type` isn't
/// `contact`, or it's missing a `name.first`/parseable `birthday` -- a
/// non-contact or malformed note is silently skipped, not an error. A
/// matched contact's frontmatter is always on line 1, per the plugin's
/// own "must be placed at the very top of the file" rule.
pub fn contact_in_note(source: &str) -> Option<Contact> {
    let (raw, _body) = frontmatter::extract_raw(source)?;

    let frontmatter: RawFrontmatter =
        serde_yaml_ng::from_str(&quote_reserved_indicator_scalars(raw)).ok()?;
    if frontmatter.kind.as_deref() != Some("contact") {
        return None;
    }
    let name = frontmatter.name?;

    let (birthday, birth_year_known) = parse_birthday(&frontmatter.birthday?)?;
    Some(Contact {
        first_name: name.first?,
        last_name: name.last,
        birthday,
        birth_year_known,
    })
}

/// Quote plain YAML scalars that begin with `@` or `` ` `` -- the two
/// "reserved for future use" indicator characters (YAML spec §5.3) that no
/// compliant parser accepts unquoted. The obsidian-contacts plugin's own
/// Frontmatter Format template writes `telegram: @handle` this way, which
/// otherwise fails the *entire* document's parse over one field we don't
/// even read, silently dropping the birthday along with it.
///
/// Only rewrites simple `key: value` lines (including nested ones, e.g.
/// `  first: value`); block scalars and flow collections are left alone.
fn quote_reserved_indicator_scalars(raw: &str) -> Cow<'_, str> {
    if !raw.lines().any(|line| unquoted_value(line).is_some()) {
        return Cow::Borrowed(raw);
    }
    let mut out = String::with_capacity(raw.len() + 8);
    for (i, line) in raw.lines().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        match unquoted_value(line) {
            Some(value) => {
                let prefix = &line[..line.len() - value.len()];
                let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
                out.push_str(prefix);
                out.push('"');
                out.push_str(&escaped);
                out.push('"');
            }
            None => out.push_str(line),
        }
    }
    Cow::Owned(out)
}

/// If `line` is a `key: value` pair whose value starts with a reserved YAML
/// indicator, returns that value (unquoted, as written).
fn unquoted_value(line: &str) -> Option<&str> {
    let colon = line.find(": ")?;
    if line[..colon].trim().is_empty() {
        return None;
    }
    let value = &line[colon + 2..];
    value.starts_with(['@', '`']).then_some(value)
}

/// The next calendar date `contact.birthday`'s month/day falls on, on or
/// after `today`. Checks `today`'s year and the next one; `None` only for
/// a February 29 birthday when neither year is a leap year -- it does not
/// roll over to March 1 the way the upstream plugin's JS `Date` arithmetic
/// does.
pub fn next_occurrence(contact: &Contact, today: NaiveDate) -> Option<NaiveDate> {
    [today.year(), today.year() + 1]
        .into_iter()
        .find_map(|year| {
            let occurrence =
                NaiveDate::from_ymd_opt(year, contact.birthday.month(), contact.birthday.day())?;
            (occurrence >= today).then_some(occurrence)
        })
}

/// Days from `today` until `contact`'s next birthday (`0` if it's today).
/// `None` exactly when [`next_occurrence`] is `None`.
pub fn days_until_birthday(contact: &Contact, today: NaiveDate) -> Option<i32> {
    next_occurrence(contact, today).map(|occurrence| (occurrence - today).num_days() as i32)
}

/// True if `contact`'s birthday falls on `today`.
pub fn is_birthday_today(contact: &Contact, today: NaiveDate) -> bool {
    days_until_birthday(contact, today) == Some(0)
}

/// The age `contact` turns on their next birthday ([`next_occurrence`]).
/// `None` when the birth year isn't known ([`Contact::birth_year_known`])
/// -- there's nothing to subtract from -- or when [`next_occurrence`]
/// itself is `None` (an unmatched February 29).
pub fn age_at_next_occurrence(contact: &Contact, today: NaiveDate) -> Option<i32> {
    if !contact.birth_year_known {
        return None;
    }
    let occurrence = next_occurrence(contact, today)?;
    Some(occurrence.year() - contact.birthday.year())
}

/// True if `contact`'s next birthday falls within the next `days` days,
/// counting `today` as the first of them (so `days == 1` matches only
/// today, `days == 7` today through six days out, and so on).
pub fn is_birthday_within(contact: &Contact, today: NaiveDate, days: i32) -> bool {
    days_until_birthday(contact, today).is_some_and(|until| until < days)
}

/// How far ahead `contacts birthdays` should look for a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BirthdayWindow {
    Today,
    Week,
    Month,
}

impl BirthdayWindow {
    /// Parse the `window` CLI argument (`today`, `week`, `month`).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "today" => Some(Self::Today),
            "week" => Some(Self::Week),
            "month" => Some(Self::Month),
            _ => None,
        }
    }

    /// Window size in days, counting today as the first of them.
    pub fn days(self) -> i32 {
        match self {
            Self::Today => 1,
            Self::Week => 7,
            Self::Month => 31,
        }
    }

    /// Printed when no contact's birthday falls inside the window.
    pub fn none_message(self) -> &'static str {
        match self {
            Self::Today => "no birthdays today",
            Self::Week => "no birthdays this week",
            Self::Month => "no birthdays this month",
        }
    }

    /// Whether listed contacts should be suffixed with their next
    /// birthday's date: redundant for `Today` (it's always today), useful
    /// once the window spans more than a single day.
    pub fn shows_date(self) -> bool {
        !matches!(self, Self::Today)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
    }

    fn contact(birthday: NaiveDate, birth_year_known: bool) -> Contact {
        Contact {
            first_name: "carl".to_string(),
            last_name: None,
            birthday,
            birth_year_known,
        }
    }

    #[test]
    fn parses_a_well_formed_frontmatter_contact() {
        let src = "\
---
name:
  first: carl
  last: johnson
phone: +1 555 555 5555
birthday: 1966-12-06
last_chat: 2022-12-06
type: contact
---

Body text.
";
        assert_eq!(
            contact_in_note(src),
            Some(Contact {
                first_name: "carl".to_string(),
                last_name: Some("johnson".to_string()),
                birthday: date(1966, 12, 6),
                birth_year_known: true,
            })
        );
    }

    #[test]
    fn unquoted_at_sign_field_does_not_break_the_rest_of_the_frontmatter() {
        // `@` is a reserved YAML indicator and can't start a plain scalar.
        // The obsidian-contacts plugin's own template writes
        // `telegram: @handle` unquoted, per its README's Frontmatter
        // Format example -- that one field must not sink the whole parse.
        let src = "\
---
name:
  first: carl
  last: johnson
telegram: @carlj567
birthday: 1966-12-06
type: contact
---
";
        assert_eq!(
            contact_in_note(src),
            Some(Contact {
                first_name: "carl".to_string(),
                last_name: Some("johnson".to_string()),
                birthday: date(1966, 12, 6),
                birth_year_known: true,
            })
        );
    }

    #[test]
    fn missing_last_name_is_none() {
        let src = "---\nname:\n  first: carl\nbirthday: 1966-12-06\ntype: contact\n---\n";
        assert_eq!(
            contact_in_note(src),
            Some(Contact {
                first_name: "carl".to_string(),
                last_name: None,
                birthday: date(1966, 12, 6),
                birth_year_known: true,
            })
        );
    }

    #[test]
    fn non_contact_frontmatter_is_none() {
        let src = "---\ntitle: Just a note\n---\n\nBody.\n";
        assert_eq!(contact_in_note(src), None);
    }

    #[test]
    fn missing_type_field_is_none() {
        let src = "---\nname:\n  first: carl\nbirthday: 1966-12-06\n---\n";
        assert_eq!(contact_in_note(src), None);
    }

    #[test]
    fn missing_birthday_is_none() {
        let src = "---\nname:\n  first: carl\ntype: contact\n---\n";
        assert_eq!(contact_in_note(src), None);
    }

    #[test]
    fn unparseable_birthday_is_none() {
        let src = "---\nname:\n  first: carl\nbirthday: not-a-date\ntype: contact\n---\n";
        assert_eq!(contact_in_note(src), None);
    }

    #[test]
    fn birthday_without_a_year_is_parsed() {
        // obsidian-contacts writes a birthday with no known birth year as
        // the ISO 8601 reduced-precision `--MM-DD` form.
        let src = "---\nname:\n  first: carl\nbirthday: --12-06\ntype: contact\n---\n";
        assert_eq!(
            contact_in_note(src),
            Some(Contact {
                first_name: "carl".to_string(),
                last_name: None,
                birthday: date(4, 12, 6),
                birth_year_known: false,
            })
        );
    }

    #[test]
    fn quoted_birthday_without_a_year_is_parsed() {
        let src = "---\nname:\n  first: carl\nbirthday: \"--12-06\"\ntype: contact\n---\n";
        assert_eq!(
            contact_in_note(src).map(|c| c.birthday),
            Some(date(4, 12, 6))
        );
    }

    #[test]
    fn birthday_without_a_year_matches_month_and_day() {
        let contact = contact(date(4, 12, 6), false);
        assert!(is_birthday_today(&contact, date(2026, 12, 6)));
        assert!(!is_birthday_today(&contact, date(2026, 12, 7)));
    }

    #[test]
    fn leap_day_birthday_without_a_year_matches_only_in_a_leap_year() {
        let contact = contact(date(4, 2, 29), false);
        assert!(!is_birthday_today(&contact, date(2026, 3, 1)));
        assert!(is_birthday_today(&contact, date(2028, 2, 29)));
    }

    #[test]
    fn malformed_dashed_birthday_is_none() {
        let src = "---\nname:\n  first: carl\nbirthday: --13-99\ntype: contact\n---\n";
        assert_eq!(contact_in_note(src), None);
    }

    #[test]
    fn missing_name_first_is_none() {
        let src = "---\nname:\n  last: johnson\nbirthday: 1966-12-06\ntype: contact\n---\n";
        assert_eq!(contact_in_note(src), None);
    }

    #[test]
    fn note_without_frontmatter_is_none() {
        assert_eq!(contact_in_note("# Just a heading\n\nBody.\n"), None);
    }

    #[test]
    fn custom_format_table_is_none() {
        let src = "\
/---contact---/
| key       | value                    |
| --------- | ------------------------ |
| Name      | carl                     |
| Birthday  | 1966-12-06               |
/---contact---/
";
        assert_eq!(contact_in_note(src), None);
    }

    #[test]
    fn malformed_yaml_is_none_not_a_panic() {
        let src = "---\nname: [unterminated\ntype: contact\n---\n";
        assert_eq!(contact_in_note(src), None);
    }

    #[test]
    fn birthday_matches_month_and_day_regardless_of_year() {
        let contact = contact(date(1966, 12, 6), true);
        assert!(is_birthday_today(&contact, date(2026, 12, 6)));
        assert!(!is_birthday_today(&contact, date(2026, 12, 7)));
        assert!(!is_birthday_today(&contact, date(2026, 6, 12)));
    }

    #[test]
    fn leap_day_birthday_does_not_match_in_a_non_leap_year() {
        let contact = contact(date(2000, 2, 29), true);
        assert!(!is_birthday_today(&contact, date(2026, 3, 1)));
        assert!(is_birthday_today(&contact, date(2028, 2, 29)));
    }

    #[test]
    fn days_until_birthday_counts_zero_on_the_day_itself() {
        let contact = contact(date(1966, 12, 6), true);
        assert_eq!(days_until_birthday(&contact, date(2026, 12, 6)), Some(0));
    }

    #[test]
    fn days_until_birthday_counts_forward_within_the_year() {
        let contact = contact(date(1966, 12, 6), true);
        assert_eq!(days_until_birthday(&contact, date(2026, 12, 1)), Some(5));
    }

    #[test]
    fn days_until_birthday_wraps_into_the_next_year() {
        let contact = contact(date(1966, 1, 3), true);
        // Dec 30 -> Jan 3 crosses a year boundary: 4 days, not "already
        // passed this year".
        assert_eq!(days_until_birthday(&contact, date(2026, 12, 30)), Some(4));
    }

    #[test]
    fn days_until_birthday_is_none_for_leap_day_across_two_non_leap_years() {
        let contact = contact(date(2000, 2, 29), true);
        // 2026 and 2027 are both non-leap, so neither has a February 29.
        assert_eq!(days_until_birthday(&contact, date(2026, 3, 1)), None);
    }

    #[test]
    fn age_at_next_occurrence_counts_years_since_birth_when_the_birthday_is_today() {
        let contact = contact(date(1966, 12, 6), true);
        assert_eq!(
            age_at_next_occurrence(&contact, date(2026, 12, 6)),
            Some(60)
        );
    }

    #[test]
    fn age_at_next_occurrence_uses_the_year_of_the_upcoming_birthday() {
        let contact = contact(date(1966, 1, 3), true);
        // Dec 30, 2026 -> next occurrence is Jan 3, 2027, turning 61, not 60.
        assert_eq!(
            age_at_next_occurrence(&contact, date(2026, 12, 30)),
            Some(61)
        );
    }

    #[test]
    fn age_at_next_occurrence_is_none_without_a_known_birth_year() {
        let contact = contact(date(4, 12, 6), false);
        assert_eq!(age_at_next_occurrence(&contact, date(2026, 12, 6)), None);
    }

    #[test]
    fn age_at_next_occurrence_is_none_for_leap_day_across_two_non_leap_years() {
        let contact = contact(date(2000, 2, 29), true);
        assert_eq!(age_at_next_occurrence(&contact, date(2026, 3, 1)), None);
    }

    #[test]
    fn is_birthday_within_includes_today_as_day_zero() {
        let contact = contact(date(1966, 12, 6), true);
        assert!(is_birthday_within(&contact, date(2026, 12, 6), 1));
    }

    #[test]
    fn is_birthday_within_matches_the_last_day_of_the_window_but_not_beyond() {
        let contact = contact(date(1966, 12, 6), true);
        let today = date(2026, 11, 30);
        assert!(is_birthday_within(&contact, today, 7)); // Dec 6 is 6 days out.
        assert!(!is_birthday_within(&contact, today, 6)); // window ends Dec 5.
    }

    #[test]
    fn is_birthday_within_excludes_a_birthday_already_passed_this_year() {
        let contact = contact(date(1966, 12, 6), true);
        assert!(!is_birthday_within(&contact, date(2026, 12, 7), 31));
    }

    #[test]
    fn display_name_omits_missing_last_name() {
        let contact = contact(date(1966, 12, 6), true);
        assert_eq!(contact.display_name(), "carl");
    }

    #[test]
    fn display_name_joins_first_and_last() {
        let mut contact = contact(date(1966, 12, 6), true);
        contact.last_name = Some("johnson".to_string());
        assert_eq!(contact.display_name(), "carl johnson");
    }

    #[test]
    fn birthday_window_parses_known_keywords_only() {
        assert_eq!(BirthdayWindow::parse("today"), Some(BirthdayWindow::Today));
        assert_eq!(BirthdayWindow::parse("week"), Some(BirthdayWindow::Week));
        assert_eq!(BirthdayWindow::parse("month"), Some(BirthdayWindow::Month));
        assert_eq!(BirthdayWindow::parse("year"), None);
    }

    #[test]
    fn birthday_window_days_and_shows_date() {
        assert_eq!(BirthdayWindow::Today.days(), 1);
        assert_eq!(BirthdayWindow::Week.days(), 7);
        assert_eq!(BirthdayWindow::Month.days(), 31);
        assert!(!BirthdayWindow::Today.shows_date());
        assert!(BirthdayWindow::Week.shows_date());
        assert!(BirthdayWindow::Month.shows_date());
    }
}
