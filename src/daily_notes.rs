//! Obsidian Daily Notes plugin compatibility: locating and appending to the
//! vault's daily note.
//!
//! Reads the vault's own `.obsidian/daily-notes.json` so notes land exactly
//! where Obsidian's Daily Notes plugin would put them, and are picked up by
//! Obsidian without conversion.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{Datelike, NaiveDate, NaiveTime, Timelike, Weekday};

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Daily-note settings from `.obsidian/daily-notes.json`, falling back to
/// Obsidian's defaults (vault root, `YYYY-MM-DD`) when absent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyNotesConfig {
    /// Vault-relative folder for daily notes; empty means the vault root.
    pub folder: String,
    /// Filename pattern in Moment.js tokens, without the `.md` extension.
    /// May contain `/` to nest notes in per-year/month folders.
    pub format: String,
}

impl Default for DailyNotesConfig {
    fn default() -> Self {
        Self {
            folder: String::new(),
            format: "YYYY-MM-DD".to_string(),
        }
    }
}

/// Read the vault's daily-notes plugin config. A missing or malformed file
/// yields the defaults, matching how Obsidian treats an unconfigured plugin.
pub fn load_config(vault: &Path) -> DailyNotesConfig {
    let path = vault.join(".obsidian").join("daily-notes.json");
    let Ok(raw) = fs::read_to_string(path) else {
        return DailyNotesConfig::default();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return DailyNotesConfig::default();
    };
    let field = |key: &str| {
        json.get(key)
            .and_then(|value| value.as_str())
            .filter(|s| !s.is_empty())
    };
    DailyNotesConfig {
        folder: field("folder").unwrap_or("").trim_matches('/').to_string(),
        format: field("format").unwrap_or("YYYY-MM-DD").to_string(),
    }
}

/// Path of the daily note for `date` under `vault`.
pub fn note_path(vault: &Path, config: &DailyNotesConfig, date: NaiveDate) -> PathBuf {
    let mut path = vault.to_path_buf();
    if !config.folder.is_empty() {
        path.push(&config.folder);
    }
    path.push(format_date(&config.format, date) + ".md");
    path
}

/// Path of the daily note for `date` under `vault`, reading the vault's own
/// Daily Notes plugin configuration.
pub fn daily_note_path(vault: &Path, date: NaiveDate) -> PathBuf {
    note_path(vault, &load_config(vault), date)
}

/// Append a `[HH:MM] text` timestamped note to today's daily note, creating
/// the note and any missing folders. Returns the note's path.
pub fn append_note(vault: &Path, text: &str) -> io::Result<PathBuf> {
    let now = chrono::Local::now();
    append_note_on(vault, now.date_naive(), now.time(), text)
}

/// [`append_note`] for an explicit date and time. A blank line is always
/// inserted before the entry, regardless of how the note currently ends.
pub fn append_note_on(
    vault: &Path,
    date: NaiveDate,
    time: NaiveTime,
    text: &str,
) -> io::Result<PathBuf> {
    let entry = format!("[{:02}:{:02}] {text}", time.hour(), time.minute());
    append_entry_on(vault, date, &entry)
}

/// Append `entry` to `date`'s daily note preceded by a blank line, creating
/// the note and any missing folders. Returns the note's path.
fn append_entry_on(vault: &Path, date: NaiveDate, entry: &str) -> io::Result<PathBuf> {
    let path = daily_note_path(vault, date);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err),
    };
    content.push('\n');
    content.push_str(entry);
    content.push('\n');
    crate::fsutil::atomic_write_str(&path, &content)
        .map_err(|err| io::Error::other(err.to_string()))?;
    Ok(path)
}

/// Append `block` to today's daily note, creating the note and any missing
/// folders. Returns the note's path.
pub fn append_block(vault: &Path, block: &str) -> io::Result<PathBuf> {
    append_block_on(vault, chrono::Local::now().date_naive(), block)
}

/// [`append_block`] for an explicit date. The block is separated from
/// existing content by a newline and always ends with one — unlike
/// [`append_note_on`], no blank line is forced in front of it.
pub fn append_block_on(vault: &Path, date: NaiveDate, block: &str) -> io::Result<PathBuf> {
    let path = daily_note_path(vault, date);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err),
    };
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(block);
    if !block.ends_with('\n') {
        content.push('\n');
    }
    crate::fsutil::atomic_write_str(&path, &content)
        .map_err(|err| io::Error::other(err.to_string()))?;
    Ok(path)
}

/// Render `date` with the subset of Moment.js tokens daily-note formats
/// commonly use: `YYYY`, `YY`, `MMMM`, `MMM`, `MM`, `M`, `DD`, `D`, `dddd`,
/// `ddd`. Text in `[...]` is literal; everything else passes through.
pub fn format_date(format: &str, date: NaiveDate) -> String {
    format_date_impl(format, date, false)
}

/// Same as [`format_date`], but every `YYYY`/`YY` token renders as `?`
/// repeated to that token's width instead of digits. For a date whose
/// year is a meaningless placeholder -- e.g. the next occurrence computed
/// from a `contacts::Contact` birthday with no known birth year --
/// spelling out a real-looking year would misleadingly claim knowledge
/// the vault doesn't have.
pub fn format_date_with_masked_year(format: &str, date: NaiveDate) -> String {
    format_date_impl(format, date, true)
}

fn format_date_impl(format: &str, date: NaiveDate, mask_year: bool) -> String {
    let chars: Vec<char> = format.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' {
            i += 1;
            while i < chars.len() && chars[i] != ']' {
                out.push(chars[i]);
                i += 1;
            }
            i += 1;
            continue;
        }

        let run = chars[i..].iter().take_while(|&&c| c == chars[i]).count();
        let month = MONTHS[date.month0() as usize];
        let taken = match (chars[i], run) {
            ('Y', 4..) => {
                if mask_year {
                    out.push_str("????");
                } else {
                    out.push_str(&format!("{:04}", date.year()));
                }
                4
            }
            ('Y', 2..) => {
                if mask_year {
                    out.push_str("??");
                } else {
                    out.push_str(&format!("{:02}", date.year().rem_euclid(100)));
                }
                2
            }
            ('M', 4..) => {
                out.push_str(month);
                4
            }
            ('M', 3) => {
                out.push_str(&month[..3]);
                3
            }
            ('M', 2) => {
                out.push_str(&format!("{:02}", date.month()));
                2
            }
            ('M', _) => {
                out.push_str(&date.month().to_string());
                1
            }
            ('D', 2..) => {
                out.push_str(&format!("{:02}", date.day()));
                2
            }
            ('D', _) => {
                out.push_str(&date.day().to_string());
                1
            }
            ('d', 4..) => {
                out.push_str(weekday_name(date.weekday()));
                4
            }
            ('d', 3) => {
                out.push_str(&weekday_name(date.weekday())[..3]);
                3
            }
            (other, _) => {
                out.push(other);
                1
            }
        };
        i += taken;
    }
    out
}

fn weekday_name(weekday: Weekday) -> &'static str {
    match weekday {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 14).unwrap()
    }

    fn time() -> NaiveTime {
        NaiveTime::from_hms_opt(9, 5, 0).unwrap()
    }

    fn write_config(vault: &Path, json: &str) {
        let dir = vault.join(".obsidian");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("daily-notes.json"), json).unwrap();
    }

    #[test]
    fn load_config_missing_file_yields_defaults() {
        let dir = tempdir().unwrap();
        assert_eq!(load_config(dir.path()), DailyNotesConfig::default());
    }

    #[test]
    fn load_config_malformed_json_yields_defaults() {
        let dir = tempdir().unwrap();
        write_config(dir.path(), "not json");
        assert_eq!(load_config(dir.path()), DailyNotesConfig::default());
    }

    #[test]
    fn load_config_reads_folder_and_format() {
        let dir = tempdir().unwrap();
        write_config(
            dir.path(),
            r#"{"folder": "/Journal/", "format": "YYYY/MM/YYYY-MM-DD"}"#,
        );
        let config = load_config(dir.path());
        assert_eq!(config.folder, "Journal");
        assert_eq!(config.format, "YYYY/MM/YYYY-MM-DD");
    }

    #[test]
    fn note_path_nests_via_format_slashes_and_folder() {
        let config = DailyNotesConfig {
            folder: "Journal".to_string(),
            format: "YYYY/MM/YYYY-MM-DD".to_string(),
        };
        let path = note_path(Path::new("/vault"), &config, date());
        assert_eq!(
            path,
            Path::new("/vault/Journal/2026/07/2026-07-14.md")
        );
    }

    #[test]
    fn format_date_covers_supported_tokens() {
        assert_eq!(format_date("YYYY-MM-DD", date()), "2026-07-14");
        assert_eq!(format_date("YY-M-D", date()), "26-7-14");
        assert_eq!(format_date("MMMM D, YYYY", date()), "July 14, 2026");
        assert_eq!(format_date("MMM", date()), "Jul");
        assert_eq!(format_date("dddd", date()), "Tuesday");
        assert_eq!(format_date("ddd", date()), "Tue");
    }

    #[test]
    fn format_date_treats_brackets_as_literal() {
        assert_eq!(
            format_date("[Daily] YYYY-MM-DD", date()),
            "Daily 2026-07-14"
        );
    }

    #[test]
    fn format_date_with_masked_year_replaces_yyyy_with_question_marks() {
        assert_eq!(
            format_date_with_masked_year("YYYY-MM-DD", date()),
            "????-07-14"
        );
    }

    #[test]
    fn format_date_with_masked_year_replaces_yy_with_question_marks() {
        assert_eq!(format_date_with_masked_year("YY-MM-DD", date()), "??-07-14");
    }

    #[test]
    fn format_date_with_masked_year_leaves_non_year_tokens_alone() {
        assert_eq!(
            format_date_with_masked_year("dddd, MMMM D", date()),
            "Tuesday, July 14"
        );
    }

    #[test]
    fn append_note_on_creates_new_note_with_leading_blank_line() {
        let dir = tempdir().unwrap();
        let path = append_note_on(dir.path(), date(), time(), "Hello world").unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "\n[09:05] Hello world\n");
    }

    #[test]
    fn append_note_on_inserts_blank_line_even_after_existing_newline() {
        let dir = tempdir().unwrap();
        let config = DailyNotesConfig::default();
        let path = note_path(dir.path(), &config, date());
        fs::write(&path, "# Today\n").unwrap();
        append_note_on(dir.path(), date(), time(), "Hello world").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "# Today\n\n[09:05] Hello world\n"
        );
    }

    #[test]
    fn append_note_on_creates_missing_nested_folders() {
        let dir = tempdir().unwrap();
        write_config(dir.path(), r#"{"folder": "Journal", "format": "YYYY/MM-DD"}"#);
        let path = append_note_on(dir.path(), date(), time(), "hi").unwrap();
        assert!(path.exists());
        assert_eq!(path, dir.path().join("Journal/2026/07-14.md"));
    }

    #[test]
    fn append_block_on_appends_to_existing_note_ending_with_newline() {
        let dir = tempdir().unwrap();
        let config = DailyNotesConfig::default();
        let path = note_path(dir.path(), &config, date());
        fs::write(&path, "# Today\n").unwrap();
        append_block_on(dir.path(), date(), "- [ ] Call the dentist").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "# Today\n- [ ] Call the dentist\n"
        );
    }

    #[test]
    fn append_block_on_appends_to_existing_note_not_ending_with_newline() {
        let dir = tempdir().unwrap();
        let config = DailyNotesConfig::default();
        let path = note_path(dir.path(), &config, date());
        fs::write(&path, "# Today").unwrap();
        append_block_on(dir.path(), date(), "more text").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "# Today\nmore text\n"
        );
    }

    #[test]
    fn append_block_on_block_already_ending_with_newline_no_double() {
        let dir = tempdir().unwrap();
        let path = append_block_on(dir.path(), date(), "Block with newline\n").unwrap();
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "Block with newline\n"
        );
    }

    #[test]
    fn append_block_on_multiline_block_is_preserved_verbatim() {
        let dir = tempdir().unwrap();
        let path = append_block_on(dir.path(), date(), "Line one\nLine two").unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "Line one\nLine two\n");
    }
}
