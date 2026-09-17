//! Text grammar for `tasks` filter/sort queries — a scoped subset of the
//! Obsidian Tasks plugin's own query-block syntax
//! (<https://publish.obsidian.md/tasks/Queries/Filters>,
//! <https://publish.obsidian.md/tasks/Queries/Sorting>), so anyone who
//! already knows the plugin's search language can use it here too.
//!
//! Supported per line:
//! - Filter lines are AND'd together (matching the plugin's own semantics).
//! - One level of parenthesised boolean combination:
//!   `(expr) AND|OR|XOR|AND NOT|OR NOT (expr)`, or `NOT (expr)`.
//! - `sort by <key> [reverse]`.
//! - Blank lines and lines starting with `#` are ignored (comments).
//!
//! Not supported (see [`crate::tasks`] module docs for the full list of
//! deliberately out-of-scope features): date *ranges*, nested boolean
//! expressions beyond one level, and anything from the plugin's
//! `filter by function`/`sort by function` JavaScript layer.

use chrono::{Datelike, NaiveDate};

use crate::tasks::{DateCmp, Filter, Priority, PriorityCmp, SortKey, SortSpec, StatusType};

#[derive(Debug, Default, Clone, PartialEq)]
pub struct ParsedQuery {
    /// Filter lines, AND'd together.
    pub filters: Vec<Filter>,
    pub sort: Vec<SortSpec>,
}

/// Parse a multi-line query. `today` resolves relative dates (`today`,
/// `tomorrow`, `next monday`, ...) once, at parse time, so the resulting
/// filters are plain data — no wall-clock dependency survives parsing.
pub fn parse_query(text: &str, today: NaiveDate) -> Result<ParsedQuery, String> {
    let mut query = ParsedQuery::default();
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = strip_ci(line, "sort by ") {
            query.sort.push(parse_sort_spec(rest)?);
            continue;
        }
        query.filters.push(parse_filter_line(line, today)?);
    }
    Ok(query)
}

// ---------------------------------------------------------------------------
// Filter lines, including one level of boolean combination
// ---------------------------------------------------------------------------

fn parse_filter_line(line: &str, today: NaiveDate) -> Result<Filter, String> {
    if let Some(inner) = strip_ci(line, "not ") {
        let inner = inner.trim();
        if let Some(body) = parenthesized(inner) {
            return Ok(Filter::Not(Box::new(parse_simple_filter(body, today)?)));
        }
    }

    if let Some(first) = parenthesized_prefix(line) {
        let rest = line[first.len() + 2..].trim(); // +2 for the two '(' ')'
        if rest.is_empty() {
            return parse_simple_filter(first, today);
        }
        for (op, combine) in [
            ("AND NOT", combine_and_not as fn(Filter, Filter) -> Filter),
            ("OR NOT", combine_or_not),
            ("AND", combine_and),
            ("OR", combine_or),
            ("XOR", combine_xor),
        ] {
            if let Some(second) = strip_ci(rest, &format!("{op} ")).and_then(parenthesized) {
                let a = parse_simple_filter(first, today)?;
                let b = parse_simple_filter(second, today)?;
                return Ok(combine(a, b));
            }
        }
        return Err(format!("Unrecognized boolean combination: '{line}'"));
    }

    parse_simple_filter(line, today)
}

fn combine_and(a: Filter, b: Filter) -> Filter {
    Filter::And(vec![a, b])
}
fn combine_or(a: Filter, b: Filter) -> Filter {
    Filter::Or(vec![a, b])
}
fn combine_and_not(a: Filter, b: Filter) -> Filter {
    Filter::And(vec![a, Filter::Not(Box::new(b))])
}
fn combine_or_not(a: Filter, b: Filter) -> Filter {
    Filter::Or(vec![a, Filter::Not(Box::new(b))])
}
fn combine_xor(a: Filter, b: Filter) -> Filter {
    Filter::And(vec![
        Filter::Or(vec![a.clone(), b.clone()]),
        Filter::Not(Box::new(Filter::And(vec![a, b]))),
    ])
}

/// If `s` is exactly `(...)` (one balanced top-level group spanning the
/// whole string), returns its inner content.
fn parenthesized(s: &str) -> Option<&str> {
    let s = s.trim();
    let inner = parenthesized_prefix(s)?;
    (inner.len() + 2 == s.len()).then_some(inner)
}

/// If `s` starts with a balanced `(...)` group, returns its inner content
/// (not requiring the group to span the whole string).
fn parenthesized_prefix(s: &str) -> Option<&str> {
    let s = s.trim();
    if !s.starts_with('(') {
        return None;
    }
    let mut depth = 0i32;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[1..i]);
                }
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Simple (non-boolean) filter clauses
// ---------------------------------------------------------------------------

fn parse_simple_filter(s: &str, today: NaiveDate) -> Result<Filter, String> {
    let s = s.trim();

    match s.to_lowercase().as_str() {
        "done" => return Ok(Filter::Done),
        "not done" => return Ok(Filter::NotDone),
        "has due date" => return Ok(Filter::HasDue),
        "no due date" => return Ok(Filter::NoDue),
        "has done date" => return Ok(Filter::HasDone),
        "no done date" => return Ok(Filter::NoDone),
        "has tags" => return Ok(Filter::HasTags),
        "no tags" => return Ok(Filter::NoTags),
        _ => {}
    }

    if let Some(rest) = strip_ci(s, "due ") {
        return parse_date_filter(rest, today, true);
    }
    if let Some(rest) = strip_ci(s, "done ") {
        return parse_date_filter(rest, today, false);
    }
    if let Some(rest) = strip_ci(s, "priority is ") {
        return parse_priority_filter(rest);
    }
    if let Some(rest) = strip_ci(s, "tags include ").or_else(|| strip_ci(s, "tag includes ")) {
        return Ok(Filter::TagsInclude(rest.trim().to_string()));
    }
    if let Some(rest) =
        strip_ci(s, "tags do not include ").or_else(|| strip_ci(s, "tag does not include "))
    {
        return Ok(Filter::TagsExclude(rest.trim().to_string()));
    }
    if let Some(rest) = strip_ci(s, "path includes ") {
        return Ok(Filter::PathIncludes(rest.trim().to_string()));
    }
    if let Some(rest) = strip_ci(s, "path does not include ") {
        return Ok(Filter::PathExcludes(rest.trim().to_string()));
    }
    if let Some(rest) = strip_ci(s, "description includes ") {
        return Ok(Filter::DescriptionIncludes(rest.trim().to_string()));
    }
    if let Some(rest) = strip_ci(s, "description does not include ") {
        return Ok(Filter::DescriptionExcludes(rest.trim().to_string()));
    }
    if let Some(rest) = strip_ci(s, "status.name includes ") {
        return Ok(Filter::StatusNameIncludes(rest.trim().to_string()));
    }
    if let Some(rest) = strip_ci(s, "status.name does not include ") {
        return Ok(Filter::StatusNameExcludes(rest.trim().to_string()));
    }
    if let Some(rest) = strip_ci(s, "status.type is not ") {
        let kind = StatusType::parse_name(rest.trim())
            .ok_or_else(|| format!("Unknown status type '{}'", rest.trim()))?;
        return Ok(Filter::StatusTypeIsNot(kind));
    }
    if let Some(rest) = strip_ci(s, "status.type is ") {
        let kind = StatusType::parse_name(rest.trim())
            .ok_or_else(|| format!("Unknown status type '{}'", rest.trim()))?;
        return Ok(Filter::StatusTypeIs(kind));
    }

    Err(format!("Unrecognized filter: '{s}'"))
}

fn parse_date_filter(rest: &str, today: NaiveDate, is_due: bool) -> Result<Filter, String> {
    let (cmp, date_str) = if let Some(d) = strip_ci(rest, "on or before ") {
        (DateCmp::OnOrBefore, d)
    } else if let Some(d) = strip_ci(rest, "on or after ") {
        (DateCmp::OnOrAfter, d)
    } else if let Some(d) = strip_ci(rest, "before ") {
        (DateCmp::Before, d)
    } else if let Some(d) = strip_ci(rest, "after ") {
        (DateCmp::After, d)
    } else if let Some(d) = strip_ci(rest, "on ") {
        (DateCmp::On, d)
    } else {
        (DateCmp::On, rest)
    };
    let date = parse_date_value(date_str.trim(), today)?;
    Ok(if is_due {
        Filter::Due(cmp, date)
    } else {
        Filter::DoneCmp(cmp, date)
    })
}

fn parse_priority_filter(rest: &str) -> Result<Filter, String> {
    let (cmp, name) = if let Some(n) = strip_ci(rest, "above ") {
        (PriorityCmp::Above, n)
    } else if let Some(n) = strip_ci(rest, "below ") {
        (PriorityCmp::Below, n)
    } else if let Some(n) = strip_ci(rest, "not ") {
        (PriorityCmp::IsNot, n)
    } else {
        (PriorityCmp::Is, rest)
    };
    let name = name.trim();
    let priority = Priority::parse_name(name).ok_or_else(|| format!("Unknown priority '{name}'"))?;
    Ok(Filter::Priority(cmp, priority))
}

// ---------------------------------------------------------------------------
// Dates: absolute `YYYY-MM-DD`, plus a fixed relative vocabulary
// ---------------------------------------------------------------------------

fn parse_date_value(s: &str, today: NaiveDate) -> Result<NaiveDate, String> {
    let lower = s.to_lowercase();
    match lower.as_str() {
        "today" => return Ok(today),
        "tomorrow" => return Ok(today + chrono::Duration::days(1)),
        "yesterday" => return Ok(today - chrono::Duration::days(1)),
        _ => {}
    }
    if let Some(n) = lower
        .strip_suffix(" days ago")
        .and_then(|n| n.trim().parse::<i64>().ok())
    {
        return Ok(today - chrono::Duration::days(n));
    }
    if let Some(n) = lower
        .strip_prefix("in ")
        .and_then(|r| r.strip_suffix(" days"))
        .and_then(|n| n.trim().parse::<i64>().ok())
    {
        return Ok(today + chrono::Duration::days(n));
    }
    if let Some(w) = lower.strip_prefix("next ") {
        return next_weekday(today, w).ok_or_else(|| format!("Unknown weekday '{w}'"));
    }
    if let Some(w) = lower.strip_prefix("last ") {
        return last_weekday(today, w).ok_or_else(|| format!("Unknown weekday '{w}'"));
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| format!("Unrecognized date '{s}'"))
}

fn weekday_from_name(s: &str) -> Option<chrono::Weekday> {
    match s.trim() {
        "monday" => Some(chrono::Weekday::Mon),
        "tuesday" => Some(chrono::Weekday::Tue),
        "wednesday" => Some(chrono::Weekday::Wed),
        "thursday" => Some(chrono::Weekday::Thu),
        "friday" => Some(chrono::Weekday::Fri),
        "saturday" => Some(chrono::Weekday::Sat),
        "sunday" => Some(chrono::Weekday::Sun),
        _ => None,
    }
}

/// The next date after `today` (never `today` itself) that falls on
/// weekday `name`.
fn next_weekday(today: NaiveDate, name: &str) -> Option<NaiveDate> {
    let target = weekday_from_name(name)?;
    let mut d = today + chrono::Duration::days(1);
    while d.weekday() != target {
        d += chrono::Duration::days(1);
    }
    Some(d)
}

/// The most recent date before `today` (never `today` itself) that falls
/// on weekday `name`.
fn last_weekday(today: NaiveDate, name: &str) -> Option<NaiveDate> {
    let target = weekday_from_name(name)?;
    let mut d = today - chrono::Duration::days(1);
    while d.weekday() != target {
        d -= chrono::Duration::days(1);
    }
    Some(d)
}

// ---------------------------------------------------------------------------
// Sort lines
// ---------------------------------------------------------------------------

fn parse_sort_spec(rest: &str) -> Result<SortSpec, String> {
    let rest = rest.trim();
    let (key_str, reverse) = match strip_ci_suffix(rest, " reverse") {
        Some(k) => (k.trim(), true),
        None => (rest, false),
    };
    let key = match key_str.to_lowercase().as_str() {
        "due" => SortKey::Due,
        "done" => SortKey::Done,
        "priority" => SortKey::Priority,
        "status.type" | "status" => SortKey::StatusType,
        "status.name" => SortKey::StatusName,
        "path" => SortKey::Path,
        "description" => SortKey::Description,
        "tag" | "tags" => SortKey::Tags,
        other => return Err(format!("Unknown sort key '{other}'")),
    };
    Ok(SortSpec { key, reverse })
}

// ---------------------------------------------------------------------------
// Small string helpers
// ---------------------------------------------------------------------------

/// Case-insensitive prefix strip, preserving the remainder's original case.
fn strip_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    (s.len() >= prefix.len() && s.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes()))
        .then(|| &s[prefix.len()..])
}

fn strip_ci_suffix<'a>(s: &'a str, suffix: &str) -> Option<&'a str> {
    (s.len() >= suffix.len()
        && s.as_bytes()[s.len() - suffix.len()..].eq_ignore_ascii_case(suffix.as_bytes()))
    .then(|| &s[..s.len() - suffix.len()])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 15).unwrap() // a Tuesday
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn done_and_not_done() {
        assert_eq!(parse_filter_line("done", today()), Ok(Filter::Done));
        assert_eq!(parse_filter_line("not done", today()), Ok(Filter::NotDone));
        assert_eq!(
            parse_filter_line("Not Done", today()),
            Ok(Filter::NotDone),
            "case-insensitive"
        );
    }

    #[test]
    fn due_date_comparisons() {
        assert_eq!(
            parse_filter_line("due before 2025-01-01", today()),
            Ok(Filter::Due(DateCmp::Before, date(2025, 1, 1)))
        );
        assert_eq!(
            parse_filter_line("due on or after 2025-01-01", today()),
            Ok(Filter::Due(DateCmp::OnOrAfter, date(2025, 1, 1)))
        );
        assert_eq!(
            parse_filter_line("due 2025-01-01", today()),
            Ok(Filter::Due(DateCmp::On, date(2025, 1, 1))),
            "bare date implies 'on'"
        );
        assert_eq!(
            parse_filter_line("has due date", today()),
            Ok(Filter::HasDue)
        );
        assert_eq!(parse_filter_line("no due date", today()), Ok(Filter::NoDue));
    }

    #[test]
    fn done_date_is_disambiguated_from_bare_done_status() {
        assert_eq!(
            parse_filter_line("done before 2025-01-01", today()),
            Ok(Filter::DoneCmp(DateCmp::Before, date(2025, 1, 1)))
        );
        assert_eq!(parse_filter_line("done", today()), Ok(Filter::Done));
    }

    #[test]
    fn relative_dates() {
        assert_eq!(
            parse_filter_line("due today", today()),
            Ok(Filter::Due(DateCmp::On, today()))
        );
        assert_eq!(
            parse_filter_line("due tomorrow", today()),
            Ok(Filter::Due(DateCmp::On, date(2026, 9, 16)))
        );
        assert_eq!(
            parse_filter_line("due yesterday", today()),
            Ok(Filter::Due(DateCmp::On, date(2026, 9, 14)))
        );
        assert_eq!(
            parse_filter_line("due in 7 days", today()),
            Ok(Filter::Due(DateCmp::On, date(2026, 9, 22)))
        );
        assert_eq!(
            parse_filter_line("due 3 days ago", today()),
            Ok(Filter::Due(DateCmp::On, date(2026, 9, 12)))
        );
    }

    #[test]
    fn next_and_last_weekday_never_match_today() {
        // today() is Tuesday 2026-09-15.
        assert_eq!(
            parse_filter_line("due next tuesday", today()),
            Ok(Filter::Due(DateCmp::On, date(2026, 9, 22))),
            "next tuesday skips today"
        );
        assert_eq!(
            parse_filter_line("due last tuesday", today()),
            Ok(Filter::Due(DateCmp::On, date(2026, 9, 8))),
            "last tuesday skips today"
        );
    }

    #[test]
    fn priority_filters() {
        assert_eq!(
            parse_filter_line("priority is high", today()),
            Ok(Filter::Priority(PriorityCmp::Is, Priority::High))
        );
        assert_eq!(
            parse_filter_line("priority is above none", today()),
            Ok(Filter::Priority(PriorityCmp::Above, Priority::None))
        );
        assert_eq!(
            parse_filter_line("priority is not none", today()),
            Ok(Filter::Priority(PriorityCmp::IsNot, Priority::None))
        );
    }

    #[test]
    fn tag_filters() {
        assert_eq!(
            parse_filter_line("tags include #todo", today()),
            Ok(Filter::TagsInclude("#todo".to_string()))
        );
        assert_eq!(
            parse_filter_line("tag includes work", today()),
            Ok(Filter::TagsInclude("work".to_string()))
        );
        assert_eq!(
            parse_filter_line("tags do not include #todo", today()),
            Ok(Filter::TagsExclude("#todo".to_string()))
        );
    }

    #[test]
    fn path_and_description_filters() {
        assert_eq!(
            parse_filter_line("path includes Projects", today()),
            Ok(Filter::PathIncludes("Projects".to_string()))
        );
        assert_eq!(
            parse_filter_line("description does not include draft", today()),
            Ok(Filter::DescriptionExcludes("draft".to_string()))
        );
    }

    #[test]
    fn status_type_filters() {
        assert_eq!(
            parse_filter_line("status.type is IN_PROGRESS", today()),
            Ok(Filter::StatusTypeIs(StatusType::InProgress))
        );
        assert_eq!(
            parse_filter_line("status.type is not cancelled", today()),
            Ok(Filter::StatusTypeIsNot(StatusType::Cancelled))
        );
    }

    #[test]
    fn boolean_or_combination() {
        assert_eq!(
            parse_filter_line("(no due date) OR (due after 2021-04-04)", today()),
            Ok(Filter::Or(vec![
                Filter::NoDue,
                Filter::Due(DateCmp::After, date(2021, 4, 4)),
            ]))
        );
    }

    #[test]
    fn boolean_and_not_combination() {
        assert_eq!(
            parse_filter_line("(path includes GitHub) AND NOT (tags include #todo)", today()),
            Ok(Filter::And(vec![
                Filter::PathIncludes("GitHub".to_string()),
                Filter::Not(Box::new(Filter::TagsInclude("#todo".to_string()))),
            ]))
        );
    }

    #[test]
    fn boolean_not_prefix() {
        assert_eq!(
            parse_filter_line("NOT (done)", today()),
            Ok(Filter::Not(Box::new(Filter::Done)))
        );
    }

    #[test]
    fn single_parenthesized_group_without_operator() {
        assert_eq!(parse_filter_line("(done)", today()), Ok(Filter::Done));
    }

    #[test]
    fn sort_by_lines() {
        let q = parse_query("not done\nsort by due\nsort by priority reverse", today()).unwrap();
        assert_eq!(q.filters, vec![Filter::NotDone]);
        assert_eq!(
            q.sort,
            vec![
                SortSpec {
                    key: SortKey::Due,
                    reverse: false
                },
                SortSpec {
                    key: SortKey::Priority,
                    reverse: true
                },
            ]
        );
    }

    #[test]
    fn blank_lines_and_comments_are_ignored() {
        let q = parse_query("not done\n\n# a comment\ndue before 2025-01-01\n", today()).unwrap();
        assert_eq!(q.filters.len(), 2);
    }

    #[test]
    fn unrecognized_filter_is_an_error() {
        assert!(parse_filter_line("blah blah", today()).is_err());
    }

    #[test]
    fn unrecognized_sort_key_is_an_error() {
        assert!(parse_query("sort by nonsense", today()).is_err());
    }

    #[test]
    fn unrecognized_priority_name_is_an_error() {
        assert!(parse_filter_line("priority is extreme", today()).is_err());
    }
}
