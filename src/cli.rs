use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "clin",
    version,
    about = "Feature-packed terminal note management app inspired by Obsidian"
)]
pub struct Cli {
    /// Override the config file location for this run.
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    /// Override the storage/vault path for this run (~ and $VAR expanded).
    #[arg(long, global = true)]
    pub vault: Option<PathBuf>,

    /// Force the first-run setup wizard, even if config already exists.
    #[arg(long)]
    pub setup: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Note operations.
    Notes {
        #[command(subcommand)]
        action: NotesCmd,
    },
    /// Storage / vault path management.
    Storage {
        #[command(subcommand)]
        action: StorageCmd,
    },
    /// Keybind management.
    Keybinds {
        #[command(subcommand)]
        action: KeybindsCmd,
    },
    /// Template management.
    Templates {
        #[command(subcommand)]
        action: TemplatesCmd,
    },
    /// Config management.
    Config {
        #[command(subcommand)]
        action: ConfigCmd,
    },
    /// Cached data management.
    Cache {
        #[command(subcommand)]
        action: CacheCmd,
    },
    /// Quick-capture a note into today's daily note (Obsidian Daily Notes
    /// plugin compatible).
    Jot {
        /// Text to append. May start with `-` (e.g. a markdown checklist
        /// item or bullet) without needing a `--` separator.
        #[arg(allow_hyphen_values = true)]
        text: String,
        /// Append as a raw block instead of a `[HH:MM]` timestamped entry.
        #[arg(long)]
        append: bool,
    },
    /// Contact notes (obsidian-contacts plugin Frontmatter Format).
    Contacts {
        #[command(subcommand)]
        action: ContactsCmd,
    },
    /// Checklist items across the vault (Obsidian Tasks plugin compatible
    /// subset: due/done dates, status, tags, priority, filtering, sorting).
    Tasks {
        #[command(subcommand)]
        action: TasksCmd,
    },
}

#[derive(Subcommand, Debug)]
pub enum TasksCmd {
    /// List checklist items across the vault.
    List {
        /// `open` (not done), `done`, or `all` (default: no status filter).
        #[arg(long)]
        status: Option<String>,
        /// Due-date filter, e.g. `before 2025-01-01`, `today`, `2025-01-01`.
        #[arg(long)]
        due: Option<String>,
        /// Done-date filter, same syntax as `--due`.
        #[arg(long)]
        done: Option<String>,
        /// Priority filter, e.g. `high`, `above none`, `not none`.
        #[arg(long)]
        priority: Option<String>,
        /// Require a tag (repeatable; combined with AND). `#` is optional.
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Only tasks whose note path contains this text.
        #[arg(long)]
        path: Option<String>,
        /// Sort key (repeatable, in priority order), e.g. `due`,
        /// `priority reverse`. Defaults to status/due/priority/path.
        #[arg(long = "sort")]
        sort: Vec<String>,
        /// Raw multi-line query in Tasks-plugin syntax (see
        /// <https://publish.obsidian.md/tasks/Queries/Filters>); when set,
        /// the flags above are ignored.
        #[arg(long)]
        query: Option<String>,
        /// Also print each task's note path and line number.
        #[arg(short, long)]
        verbose: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum ContactsCmd {
    /// List contacts whose birthday falls within a window of today.
    Birthdays {
        /// How far ahead to look: `today`, `week`, or `month`.
        #[arg(default_value = "today")]
        window: String,
        /// Also print each match's note path and frontmatter line.
        #[arg(short, long)]
        verbose: bool,
        /// Moment.js-style format for the birthday date shown by the
        /// `week`/`month` windows (default: YYYY-MM-DD).
        #[arg(long)]
        date_format: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum NotesCmd {
    /// List note titles.
    List,
    /// Create a new note and open it in the TUI.
    New {
        /// Create the note from this template.
        #[arg(short, long)]
        template: Option<String>,
        /// Initial body content. When set, the note is created and the TUI is not opened.
        #[arg(long)]
        body: Option<String>,
        /// Create the note and exit without opening the TUI.
        #[arg(long)]
        no_tui: bool,
        /// Optional title for the note.
        title: Option<String>,
    },
    /// Open a note by title in the TUI.
    Open {
        /// Title of the note to open.
        title: String,
    },
    /// Print a note's body to stdout.
    Cat {
        /// Title of the note to print (case-insensitive match).
        title: String,
    },
    /// Create a quick note from content and exit (no TUI).
    Quick {
        /// Body content of the note.
        content: String,
        /// Optional title for the note.
        title: Option<String>,
    },
    /// Search notes by title and content.
    Search {
        /// Query string.
        query: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum StorageCmd {
    /// Show the current storage path.
    Show,
    /// Set a custom (absolute) storage path.
    Set { path: PathBuf },
    /// Reset to the default storage path.
    Reset,
    /// Migrate data from a previous storage location.
    Migrate,
}

#[derive(Subcommand, Debug)]
pub enum KeybindsCmd {
    /// Show current keybindings.
    Show,
    /// Export keybinds as TOML.
    Export,
    /// Reset keybinds to defaults.
    Reset,
}

#[derive(Subcommand, Debug)]
pub enum TemplatesCmd {
    /// List available templates.
    List,
    /// Create example templates.
    Init,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCmd {
    /// Print the config file path.
    Show,
    /// Open the config file in $VISUAL or $EDITOR.
    Edit,
    /// Reset the configuration to default values.
    Reset,
}

#[derive(Subcommand, Debug)]
pub enum CacheCmd {
    /// Delete the vault's scoped note-summary cache and legacy cache locations. It rebuilds on next launch.
    Reset,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cache_reset() {
        let cli = Cli::try_parse_from(["clin", "cache", "reset"]).unwrap();
        let command = cli.command.unwrap();
        assert!(
            matches!(
                command,
                Command::Cache {
                    action: CacheCmd::Reset
                }
            ),
            "expected Cache::Reset, got {command:?}"
        );
    }
}
