//! Parsing and querying Obsidian Tasks-plugin-style checklist items
//! (`- [ ] Do the thing 📅 2024-01-01 ⏫ #project/x`) across the vault.
//!
//! Scoped subset of <https://publish.obsidian.md/tasks>: due date, done
//! (completion) date, status, tags, priority, filtering and sorting.
//! Read-only — this module never edits a task line (no toggling status,
//! no writing dates back).
//!
//! Deliberately out of scope (not requested, and each is a meaningful
//! project by itself): recurrence, task dependencies (`id`/`dependsOn`),
//! the `on completion` action, scheduled/start/created/cancelled dates as
//! filter/sort keys (parsed, but not exposed), urgency, grouping, the
//! `filter by function`/`sort by function` JavaScript scripting layer,
//! and date *range* filters (`due 2023-11-25 2023-11-30`, `next week`) —
//! only single-date comparisons (`before`/`after`/`on`/...) are supported.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use anyhow::Result;
use chrono::NaiveDate;
use comrak::nodes::{AstNode, NodeValue};
use comrak::{Arena, Options, parse_document};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::config::TasksConfig;
use crate::storage::Storage;

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

/// The six status groups the Tasks plugin sorts and filters by. Which
/// checkbox symbols map to which type is configured in `[tasks]` (see
/// [`crate::config::TasksConfig`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StatusType {
    Todo,
    InProgress,
    OnHold,
    Done,
    Cancelled,
    NonTask,
}

impl StatusType {
    /// True for the types the plugin's `done` filter matches (`DONE`,
    /// `CANCELLED`, `NON_TASK`); false for `not done` (`TODO`,
    /// `IN_PROGRESS`, `ON_HOLD`).
    pub fn is_done_group(self) -> bool {
        matches!(self, Self::Done | Self::Cancelled | Self::NonTask)
    }

    /// Parse a `status.type is <TYPE>` value, case-insensitively.
    pub fn parse_name(s: &str) -> Option<Self> {
        match s.to_ascii_uppercase().as_str() {
            "TODO" => Some(Self::Todo),
            "IN_PROGRESS" => Some(Self::InProgress),
            "ON_HOLD" => Some(Self::OnHold),
            "DONE" => Some(Self::Done),
            "CANCELLED" => Some(Self::Cancelled),
            "NON_TASK" => Some(Self::NonTask),
            _ => None,
        }
    }

    /// Sort rank matching the plugin's `sort by status.type` order (not
    /// alphabetical): `IN_PROGRESS, TODO, ON_HOLD, DONE, CANCELLED,
    /// NON_TASK`.
    fn sort_rank(self) -> u8 {
        match self {
            Self::InProgress => 0,
            Self::Todo => 1,
            Self::OnHold => 2,
            Self::Done => 3,
            Self::Cancelled => 4,
            Self::NonTask => 5,
        }
    }
}

/// A task's resolved status: its checkbox `symbol`, display `name`, and
/// [`StatusType`] group, per the vault's `[tasks]` configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub symbol: char,
    pub name: String,
    pub kind: StatusType,
}

/// Resolves checkbox symbols to [`Status`] values per `[tasks]`
/// configuration, falling back to an `"Unknown"`/[`StatusType::Todo`]
/// status for any symbol not listed — matching the Tasks plugin's own
/// documented behaviour for statuses it doesn't recognise.
pub struct StatusRegistry(HashMap<char, Status>);

impl StatusRegistry {
    pub fn from_config(config: &TasksConfig) -> Self {
        Self(
            config
                .statuses
                .iter()
                .map(|s| {
                    (
                        s.symbol,
                        Status {
                            symbol: s.symbol,
                            name: s.name.clone(),
                            kind: s.kind,
                        },
                    )
                })
                .collect(),
        )
    }

    pub fn resolve(&self, symbol: char) -> Status {
        self.0.get(&symbol).cloned().unwrap_or(Status {
            symbol,
            name: "Unknown".to_string(),
            kind: StatusType::Todo,
        })
    }
}

// ---------------------------------------------------------------------------
// Priority
// ---------------------------------------------------------------------------

/// A task's priority. Declaration order is urgency order (most urgent
/// first), matching the plugin's `priorityNumber` (`Highest` = 0 ...
/// `Lowest` = 5); `derive(Ord)` gives the right comparisons for free.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Priority {
    Highest,
    High,
    Medium,
    #[default]
    None,
    Low,
    Lowest,
}

impl Priority {
    pub fn name(self) -> &'static str {
        match self {
            Self::Highest => "Highest",
            Self::High => "High",
            Self::Medium => "Medium",
            Self::None => "None",
            Self::Low => "Low",
            Self::Lowest => "Lowest",
        }
    }

    /// Parse a priority name from a `priority is <name>` filter, or a
    /// `sort by priority` context; case-insensitive.
    pub fn parse_name(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "highest" => Some(Self::Highest),
            "high" => Some(Self::High),
            "medium" => Some(Self::Medium),
            "none" => Some(Self::None),
            "low" => Some(Self::Low),
            "lowest" => Some(Self::Lowest),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Task
// ---------------------------------------------------------------------------

/// A single checklist item found in a note, with its Tasks-plugin
/// signifiers extracted.
#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    /// Vault-relative path of the note this task was found in.
    pub note_id: String,
    /// 1-based line number. Corrected to match the on-disk file for plain
    /// `.md` notes; for encrypted `.clin` notes this is relative to the
    /// decrypted content instead, since the on-disk file is ciphertext
    /// and has no meaningful line numbering of its own.
    pub line: u32,
    pub status: Status,
    /// Description with every recognised signifier (dates, priority)
    /// removed, but tags left in place — matching the plugin's own
    /// `description` semantics.
    pub description: String,
    pub due: Option<NaiveDate>,
    pub scheduled: Option<NaiveDate>,
    pub start: Option<NaiveDate>,
    pub done: Option<NaiveDate>,
    pub created: Option<NaiveDate>,
    pub cancelled: Option<NaiveDate>,
    pub priority: Priority,
    /// Tags including their leading `#`, in the order they appear.
    pub tags: Vec<String>,
}

impl Task {
    pub fn is_done(&self) -> bool {
        self.status.kind.is_done_group()
    }
}

// ---------------------------------------------------------------------------
// Parsing a single note
// ---------------------------------------------------------------------------

fn parse_options() -> Options<'static> {
    let mut options = Options::default();
    options.extension.strikethrough = true;
    options.extension.table = true;
    options.extension.tasklist = true;
    options.extension.footnotes = true;
    options.extension.autolink = true;
    options.extension.wikilinks_title_after_pipe = true;
    options.extension.description_lists = true;
    options
}

/// Flatten a node's inline text content to plain text, joining physical
/// lines with a space (checklist items are conventionally single source
/// lines, so this only matters for soft-wrapped continuations).
fn flatten_text<'a>(node: &'a AstNode<'a>, out: &mut String) {
    let data = node.data.borrow();
    match &data.value {
        NodeValue::Text(t) => out.push_str(t),
        NodeValue::Code(c) => out.push_str(&c.literal),
        NodeValue::SoftBreak | NodeValue::LineBreak => out.push(' '),
        _ => {
            drop(data);
            for child in node.children() {
                flatten_text(child, out);
            }
        }
    }
}

/// Text of a list item's own line(s), excluding any nested sub-list.
fn item_text<'a>(item: &'a AstNode<'a>) -> String {
    let mut text = String::new();
    for child in item.children() {
        if matches!(child.data.borrow().value, NodeValue::List(_)) {
            continue;
        }
        if !text.is_empty() {
            text.push(' ');
        }
        let mut child_text = String::new();
        flatten_text(child, &mut child_text);
        text.push_str(child_text.trim());
    }
    text
}

static CUSTOM_MARKER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\[(.)\]\s?").expect("valid regex"));

/// Parse a note's Markdown source into its checklist items, in document
/// order. Plain (non-checkbox) list items are not tasks.
pub fn tasks_in_note(source: &str, registry: &StatusRegistry) -> Vec<Task> {
    let arena = Arena::new();
    let options = parse_options();
    let root = parse_document(&arena, source, &options);

    root.descendants()
        .filter_map(|node| {
            let (symbol, raw_text, line) = {
                let data = node.data.borrow();
                match &data.value {
                    NodeValue::TaskItem(ti) => {
                        let symbol = ti.symbol.unwrap_or(' ');
                        let line = data.sourcepos.start.line as u32;
                        drop(data);
                        (symbol, item_text(node), line)
                    }
                    NodeValue::Item(_) => {
                        // Comrak's tasklist extension only recognises
                        // space/x/X as task checkboxes; any other symbol
                        // (e.g. `[-]`, `[/]`) is left as a plain item with
                        // the literal `[c] ` marker still in its text, so
                        // it has to be detected and stripped by hand here.
                        let line = data.sourcepos.start.line as u32;
                        drop(data);
                        let text = item_text(node);
                        let caps = CUSTOM_MARKER_RE.captures(&text)?;
                        let symbol = caps.get(1)?.as_str().chars().next()?;
                        let rest = text[caps.get(0)?.end()..].to_string();
                        (symbol, rest, line)
                    }
                    _ => return None,
                }
            };
            // Comrak's tasklist extension still classifies a checkbox with
            // nothing but whitespace after it (e.g. "- [ ]" alone) as a
            // task item; treat it like a plain list item instead, matching
            // GFM readers' visual "needs content" expectation.
            if raw_text.trim().is_empty() {
                return None;
            }
            let status = registry.resolve(symbol);
            let (description, sig) = extract_signifiers(&raw_text);
            Some(Task {
                note_id: String::new(),
                line,
                status,
                description,
                due: sig.due,
                scheduled: sig.scheduled,
                start: sig.start,
                done: sig.done,
                created: sig.created,
                cancelled: sig.cancelled,
                priority: sig.priority,
                tags: sig.tags,
            })
        })
        .collect()
}

struct Signifiers {
    due: Option<NaiveDate>,
    scheduled: Option<NaiveDate>,
    start: Option<NaiveDate>,
    done: Option<NaiveDate>,
    created: Option<NaiveDate>,
    cancelled: Option<NaiveDate>,
    priority: Priority,
    tags: Vec<String>,
}

static DUE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*📅\s*(\d{4}-\d{2}-\d{2})").expect("valid regex"));
static SCHEDULED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*⏳\s*(\d{4}-\d{2}-\d{2})").expect("valid regex"));
static START_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*🛫\s*(\d{4}-\d{2}-\d{2})").expect("valid regex"));
static DONE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*✅\s*(\d{4}-\d{2}-\d{2})").expect("valid regex"));
static CREATED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*➕\s*(\d{4}-\d{2}-\d{2})").expect("valid regex"));
static CANCELLED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*❌\s*(\d{4}-\d{2}-\d{2})").expect("valid regex"));
static PRIORITY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*(🔺|⏫|🔼|🔽|⏬️?)").expect("valid regex"));
// Tag character class per the Tasks plugin docs: any character except
// whitespace and `!@#$%^&*(),.?":{}|<>` (which includes `#` itself, so a
// tag cannot contain a nested `#`). `/` is kept, for nested tags.
static TAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"#[^\s!@#$%^&*(),.?":{}|<>]+"#).expect("valid regex"));
static WHITESPACE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s+").expect("valid regex"));

fn extract_date(re: &Regex, text: &mut String) -> Option<NaiveDate> {
    let caps = re.captures(text)?;
    let whole = caps.get(0)?.range();
    let date = caps.get(1)?.as_str().parse().ok();
    text.replace_range(whole, "");
    date
}

fn extract_signifiers(raw_text: &str) -> (String, Signifiers) {
    let mut text = raw_text.to_string();

    let due = extract_date(&DUE_RE, &mut text);
    let scheduled = extract_date(&SCHEDULED_RE, &mut text);
    let start = extract_date(&START_RE, &mut text);
    let done = extract_date(&DONE_RE, &mut text);
    let created = extract_date(&CREATED_RE, &mut text);
    let cancelled = extract_date(&CANCELLED_RE, &mut text);

    let priority = if let Some(caps) = PRIORITY_RE.captures(&text) {
        let whole = caps.get(0).expect("group 0 always matches").range();
        let priority = match caps.get(1).expect("group 1 always matches").as_str() {
            "🔺" => Priority::Highest,
            "⏫" => Priority::High,
            "🔼" => Priority::Medium,
            "🔽" => Priority::Low,
            _ => Priority::Lowest,
        };
        text.replace_range(whole, "");
        priority
    } else {
        Priority::None
    };

    let tags = TAG_RE
        .find_iter(&text)
        .map(|m| m.as_str().to_string())
        .collect();

    let description = WHITESPACE_RE.replace_all(text.trim(), " ").into_owned();
    (
        description,
        Signifiers {
            due,
            scheduled,
            start,
            done,
            created,
            cancelled,
            priority,
            tags,
        },
    )
}

// ---------------------------------------------------------------------------
// Vault-wide scan
// ---------------------------------------------------------------------------

/// Number of physical lines a plain `.md` note's frontmatter block (open
/// delimiter through the blank separator line) occupies on disk, so task
/// line numbers computed against the frontmatter-stripped body can be
/// corrected back to real on-disk line numbers.
fn frontmatter_line_offset(path: &Path) -> u32 {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return 0;
    };
    let Some((_, body)) = crate::frontmatter::extract_raw(&raw) else {
        return 0;
    };
    let prefix_len = raw.len() - body.len();
    raw[..prefix_len].matches('\n').count() as u32
}

/// Scan every note in the vault for checklist items.
pub fn scan_vault(storage: &Storage, registry: &StatusRegistry) -> Result<Vec<Task>> {
    let mut tasks = Vec::new();
    for id in storage.list_note_ids(false, false)? {
        let is_plain = id.ends_with(".md");
        if !is_plain && !id.ends_with(".clin") {
            continue;
        }
        let Ok(note) = storage.load_note(&id) else {
            continue;
        };
        let line_offset = if is_plain {
            frontmatter_line_offset(&storage.note_path(&id))
        } else {
            0
        };
        for mut task in tasks_in_note(&note.content, registry) {
            task.note_id = id.clone();
            task.line += line_offset;
            tasks.push(task);
        }
    }
    Ok(tasks)
}

// ---------------------------------------------------------------------------
// Filtering
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateCmp {
    On,
    Before,
    After,
    OnOrBefore,
    OnOrAfter,
}

impl DateCmp {
    fn matches(self, task_date: NaiveDate, bound: NaiveDate) -> bool {
        match self {
            Self::On => task_date == bound,
            Self::Before => task_date < bound,
            Self::After => task_date > bound,
            Self::OnOrBefore => task_date <= bound,
            Self::OnOrAfter => task_date >= bound,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorityCmp {
    Is,
    IsNot,
    Above,
    Below,
}

/// A filter predicate. `And`/`Or`/`Not` are fully recursive in this AST
/// even though [`crate::tasks_query`]'s text grammar only ever builds one
/// level of nesting (the plugin's own documented boolean-combination
/// scope) — the two concerns (what a filter *can* express vs. what the
/// text syntax *accepts*) are kept separate.
#[derive(Debug, Clone, PartialEq)]
pub enum Filter {
    Done,
    NotDone,
    StatusNameIncludes(String),
    StatusNameExcludes(String),
    StatusTypeIs(StatusType),
    StatusTypeIsNot(StatusType),
    HasDue,
    NoDue,
    Due(DateCmp, NaiveDate),
    HasDone,
    NoDone,
    DoneCmp(DateCmp, NaiveDate),
    Priority(PriorityCmp, Priority),
    HasTags,
    NoTags,
    TagsInclude(String),
    TagsExclude(String),
    PathIncludes(String),
    PathExcludes(String),
    DescriptionIncludes(String),
    DescriptionExcludes(String),
    And(Vec<Filter>),
    Or(Vec<Filter>),
    Not(Box<Filter>),
}

impl Filter {
    pub fn matches(&self, task: &Task) -> bool {
        match self {
            Self::Done => task.is_done(),
            Self::NotDone => !task.is_done(),
            Self::StatusNameIncludes(s) => task.status.name.to_lowercase().contains(&s.to_lowercase()),
            Self::StatusNameExcludes(s) => {
                !task.status.name.to_lowercase().contains(&s.to_lowercase())
            }
            Self::StatusTypeIs(t) => task.status.kind == *t,
            Self::StatusTypeIsNot(t) => task.status.kind != *t,
            Self::HasDue => task.due.is_some(),
            Self::NoDue => task.due.is_none(),
            Self::Due(cmp, bound) => task.due.is_some_and(|d| cmp.matches(d, *bound)),
            Self::HasDone => task.done.is_some(),
            Self::NoDone => task.done.is_none(),
            Self::DoneCmp(cmp, bound) => task.done.is_some_and(|d| cmp.matches(d, *bound)),
            Self::Priority(cmp, p) => match cmp {
                PriorityCmp::Is => task.priority == *p,
                PriorityCmp::IsNot => task.priority != *p,
                PriorityCmp::Above => task.priority < *p,
                PriorityCmp::Below => task.priority > *p,
            },
            Self::HasTags => !task.tags.is_empty(),
            Self::NoTags => task.tags.is_empty(),
            Self::TagsInclude(t) => tags_include(&task.tags, t),
            Self::TagsExclude(t) => !tags_include(&task.tags, t),
            Self::PathIncludes(s) => task.note_id.to_lowercase().contains(&s.to_lowercase()),
            Self::PathExcludes(s) => !task.note_id.to_lowercase().contains(&s.to_lowercase()),
            Self::DescriptionIncludes(s) => {
                task.description.to_lowercase().contains(&s.to_lowercase())
            }
            Self::DescriptionExcludes(s) => {
                !task.description.to_lowercase().contains(&s.to_lowercase())
            }
            Self::And(fs) => fs.iter().all(|f| f.matches(task)),
            Self::Or(fs) => fs.iter().any(|f| f.matches(task)),
            Self::Not(f) => !f.matches(task),
        }
    }
}

/// `tags include <tag>`: case-insensitive, `#` optional on the query, and
/// a partial (sub-string) match, so `foo` matches both `#foo` and
/// `#foo/bar`.
fn tags_include(tags: &[String], query: &str) -> bool {
    let query = query.strip_prefix('#').unwrap_or(query).to_lowercase();
    tags.iter().any(|t| {
        let t = t.strip_prefix('#').unwrap_or(t).to_lowercase();
        t.contains(&query)
    })
}

// ---------------------------------------------------------------------------
// Sorting
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Due,
    Done,
    Priority,
    StatusType,
    StatusName,
    Path,
    Description,
    Tags,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortSpec {
    pub key: SortKey,
    pub reverse: bool,
}

fn cmp_option_date(a: Option<NaiveDate>, b: Option<NaiveDate>) -> Ordering {
    match (a, b) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn cmp_one(a: &Task, b: &Task, key: SortKey) -> Ordering {
    match key {
        SortKey::Due => cmp_option_date(a.due, b.due),
        SortKey::Done => cmp_option_date(a.done, b.done),
        SortKey::Priority => a.priority.cmp(&b.priority),
        SortKey::StatusType => a.status.kind.sort_rank().cmp(&b.status.kind.sort_rank()),
        SortKey::StatusName => a.status.name.cmp(&b.status.name),
        SortKey::Path => a.note_id.cmp(&b.note_id),
        SortKey::Description => a.description.cmp(&b.description),
        SortKey::Tags => a.tags.join(",").cmp(&b.tags.join(",")),
    }
}

/// The Tasks plugin's own default sort order, minus `urgency` (a derived
/// score from several properties, out of scope here) and with `status`
/// simplified to its `type`: `status.type, due, priority, path`.
pub fn default_sort() -> Vec<SortSpec> {
    vec![
        SortSpec {
            key: SortKey::StatusType,
            reverse: false,
        },
        SortSpec {
            key: SortKey::Due,
            reverse: false,
        },
        SortSpec {
            key: SortKey::Priority,
            reverse: false,
        },
        SortSpec {
            key: SortKey::Path,
            reverse: false,
        },
    ]
}

/// Sort `tasks` in place by `specs`, each a tiebreaker for the previous.
pub fn sort_tasks(tasks: &mut [Task], specs: &[SortSpec]) {
    tasks.sort_by(|a, b| {
        specs.iter().fold(Ordering::Equal, |acc, spec| {
            acc.then_with(|| {
                let ord = cmp_one(a, b, spec.key);
                if spec.reverse { ord.reverse() } else { ord }
            })
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> StatusRegistry {
        StatusRegistry::from_config(&TasksConfig::default())
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
    }

    #[test]
    fn plain_checkbox_items_are_parsed() {
        let tasks = tasks_in_note("- [ ] Buy milk\n- [x] Ship it\n- Plain item\n", &registry());
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].description, "Buy milk");
        assert!(!tasks[0].is_done());
        assert_eq!(tasks[1].description, "Ship it");
        assert!(tasks[1].is_done());
    }

    #[test]
    fn empty_checkbox_is_not_a_task() {
        let tasks = tasks_in_note("- [ ]\n- [ ]   \n", &registry());
        assert!(tasks.is_empty());
    }

    #[test]
    fn custom_status_symbols_are_detected() {
        let tasks = tasks_in_note("- [-] Cancelled item\n- [/] In progress item\n", &registry());
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].status.kind, StatusType::Cancelled);
        assert_eq!(tasks[0].status.name, "Cancelled");
        assert_eq!(tasks[0].description, "Cancelled item");
        assert_eq!(tasks[1].status.kind, StatusType::InProgress);
    }

    #[test]
    fn unknown_status_symbol_falls_back_to_todo() {
        let tasks = tasks_in_note("- [?] A question\n", &registry());
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].status.kind, StatusType::Todo);
        assert_eq!(tasks[0].status.name, "Unknown");
    }

    #[test]
    fn due_date_is_extracted_and_stripped_from_description() {
        let tasks = tasks_in_note("- [ ] take out the trash 📅 2021-04-09\n", &registry());
        assert_eq!(tasks[0].due, Some(date(2021, 4, 9)));
        assert_eq!(tasks[0].description, "take out the trash");
    }

    #[test]
    fn done_date_is_extracted() {
        let tasks = tasks_in_note("- [x] take out the trash ✅ 2021-04-09\n", &registry());
        assert_eq!(tasks[0].done, Some(date(2021, 4, 9)));
    }

    #[test]
    fn scheduled_start_created_cancelled_are_parsed_but_not_lost() {
        let tasks = tasks_in_note(
            "- [ ] x ⏳ 2021-04-09 🛫 2021-04-10 ➕ 2021-04-08 ❌ 2021-04-11\n",
            &registry(),
        );
        let t = &tasks[0];
        assert_eq!(t.scheduled, Some(date(2021, 4, 9)));
        assert_eq!(t.start, Some(date(2021, 4, 10)));
        assert_eq!(t.created, Some(date(2021, 4, 8)));
        assert_eq!(t.cancelled, Some(date(2021, 4, 11)));
        assert_eq!(t.description, "x");
    }

    #[test]
    fn all_six_priorities_are_recognised() {
        let src = "\
- [ ] a 🔺
- [ ] b ⏫
- [ ] c 🔼
- [ ] d
- [ ] e 🔽
- [ ] f ⏬
";
        let tasks = tasks_in_note(src, &registry());
        let prios: Vec<Priority> = tasks.iter().map(|t| t.priority).collect();
        assert_eq!(
            prios,
            vec![
                Priority::Highest,
                Priority::High,
                Priority::Medium,
                Priority::None,
                Priority::Low,
                Priority::Lowest,
            ]
        );
    }

    #[test]
    fn priority_ordering_puts_low_below_none() {
        assert!(Priority::None < Priority::Low);
        assert!(Priority::Low < Priority::Lowest);
        assert!(Priority::Highest < Priority::None);
    }

    #[test]
    fn tags_are_extracted_and_kept_in_description() {
        let tasks = tasks_in_note("- [ ] call mom #family #context/home\n", &registry());
        assert_eq!(tasks[0].tags, vec!["#family", "#context/home"]);
        assert_eq!(tasks[0].description, "call mom #family #context/home");
    }

    #[test]
    fn tag_stops_at_a_dot_matching_plugin_character_class() {
        let tasks = tasks_in_note("- [ ] version #1.5 release\n", &registry());
        assert_eq!(tasks[0].tags, vec!["#1"]);
    }

    #[test]
    fn nested_task_inside_blockquote_is_detected() {
        let tasks = tasks_in_note("> [!note]\n> - [ ] In callout\n", &registry());
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].description, "In callout");
    }

    #[test]
    fn line_numbers_are_1_based_and_in_document_order() {
        let tasks = tasks_in_note("Text\n\n- [ ] First\n- [ ] Second\n", &registry());
        assert_eq!(tasks[0].line, 3);
        assert_eq!(tasks[1].line, 4);
    }

    #[test]
    fn filter_done_and_not_done_use_status_type_groups() {
        let done = Task {
            note_id: String::new(),
            line: 1,
            status: Status {
                symbol: '-',
                name: "Cancelled".into(),
                kind: StatusType::Cancelled,
            },
            description: String::new(),
            due: None,
            scheduled: None,
            start: None,
            done: None,
            created: None,
            cancelled: None,
            priority: Priority::None,
            tags: Vec::new(),
        };
        assert!(Filter::Done.matches(&done));
        assert!(!Filter::NotDone.matches(&done));
    }

    #[test]
    fn filter_due_comparisons() {
        let mut task = sample_task();
        task.due = Some(date(2024, 6, 15));
        assert!(Filter::Due(DateCmp::On, date(2024, 6, 15)).matches(&task));
        assert!(Filter::Due(DateCmp::Before, date(2024, 6, 16)).matches(&task));
        assert!(Filter::Due(DateCmp::After, date(2024, 6, 14)).matches(&task));
        assert!(Filter::Due(DateCmp::OnOrBefore, date(2024, 6, 15)).matches(&task));
        assert!(!Filter::Due(DateCmp::Before, date(2024, 6, 15)).matches(&task));
        assert!(!Filter::HasDue.matches(&sample_task()));
        assert!(Filter::NoDue.matches(&sample_task()));
    }

    #[test]
    fn filter_priority_above_and_below() {
        let mut task = sample_task();
        task.priority = Priority::High;
        assert!(Filter::Priority(PriorityCmp::Above, Priority::None).matches(&task));
        assert!(!Filter::Priority(PriorityCmp::Below, Priority::None).matches(&task));
        assert!(Filter::Priority(PriorityCmp::Is, Priority::High).matches(&task));
    }

    #[test]
    fn filter_tags_include_matches_subtags() {
        let mut task = sample_task();
        task.tags = vec!["#context/home".to_string()];
        assert!(Filter::TagsInclude("context".to_string()).matches(&task));
        assert!(Filter::TagsInclude("#context/home".to_string()).matches(&task));
        assert!(!Filter::TagsInclude("work".to_string()).matches(&task));
    }

    #[test]
    fn filter_and_or_not_combine() {
        let mut task = sample_task();
        task.priority = Priority::High;
        let filter = Filter::And(vec![
            Filter::NotDone,
            Filter::Priority(PriorityCmp::Above, Priority::None),
        ]);
        assert!(filter.matches(&task));
        assert!(Filter::Not(Box::new(Filter::Done)).matches(&task));
        assert!(Filter::Or(vec![Filter::Done, Filter::NotDone]).matches(&task));
    }

    #[test]
    fn sort_by_due_puts_none_last_and_earliest_first() {
        let mut tasks = vec![
            task_with_due(None),
            task_with_due(Some(date(2024, 1, 10))),
            task_with_due(Some(date(2024, 1, 5))),
        ];
        sort_tasks(
            &mut tasks,
            &[SortSpec {
                key: SortKey::Due,
                reverse: false,
            }],
        );
        assert_eq!(tasks[0].due, Some(date(2024, 1, 5)));
        assert_eq!(tasks[1].due, Some(date(2024, 1, 10)));
        assert_eq!(tasks[2].due, None);
    }

    #[test]
    fn sort_reverse_flips_order() {
        let mut tasks = vec![
            task_with_due(Some(date(2024, 1, 5))),
            task_with_due(Some(date(2024, 1, 10))),
        ];
        sort_tasks(
            &mut tasks,
            &[SortSpec {
                key: SortKey::Due,
                reverse: true,
            }],
        );
        assert_eq!(tasks[0].due, Some(date(2024, 1, 10)));
    }

    #[test]
    fn multiple_sort_keys_break_ties() {
        let mut a = sample_task();
        a.due = Some(date(2024, 1, 1));
        a.priority = Priority::Low;
        let mut b = sample_task();
        b.due = Some(date(2024, 1, 1));
        b.priority = Priority::High;
        let mut tasks = vec![a, b];
        sort_tasks(
            &mut tasks,
            &[
                SortSpec {
                    key: SortKey::Due,
                    reverse: false,
                },
                SortSpec {
                    key: SortKey::Priority,
                    reverse: false,
                },
            ],
        );
        assert_eq!(tasks[0].priority, Priority::High);
        assert_eq!(tasks[1].priority, Priority::Low);
    }

    fn sample_task() -> Task {
        Task {
            note_id: "Notes/a.md".to_string(),
            line: 1,
            status: Status {
                symbol: ' ',
                name: "Todo".into(),
                kind: StatusType::Todo,
            },
            description: "sample".to_string(),
            due: None,
            scheduled: None,
            start: None,
            done: None,
            created: None,
            cancelled: None,
            priority: Priority::None,
            tags: Vec::new(),
        }
    }

    fn task_with_due(due: Option<NaiveDate>) -> Task {
        let mut t = sample_task();
        t.due = due;
        t
    }
}
