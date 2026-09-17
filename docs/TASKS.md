# Tasks

CLI for querying checklist items (`- [ ] ...`) across the vault, compatible
with a scoped subset of the [Obsidian Tasks plugin](https://publish.obsidian.md/tasks)'s
own signifiers and query syntax: due date, completion (done) date, status,
tags, priority, filtering, and sorting.

**Source:** `src/tasks.rs` (parsing, `Filter`/`Sort` engine), `src/tasks_query.rs`
(text query grammar), `src/cli.rs` + `src/lib.rs` (`clin tasks list`)

---

## Overview

`clin tasks list` scans every note in the vault for checklist items, parses
Tasks-plugin-style emoji signifiers out of each one, and lets you filter and
sort the results. It's read-only: nothing here edits a task line, toggles a
status, or writes a date back to a note.

```markdown
- [ ] Ship the release 📅 2026-09-20 ⏫ #work/release
- [x] Write the changelog ✅ 2026-09-10 #work
```

```console
$ clin tasks list --status open --priority high
[ ] Ship the release #work/release (due 2026-09-20) [High]
```

---

## Signifiers

| Field | Marker | Example | Notes |
|---|---|---|---|
| Due date | `📅 YYYY-MM-DD` | `📅 2026-09-20` | |
| Done (completion) date | `✅ YYYY-MM-DD` | `✅ 2026-09-10` | |
| Scheduled date | `⏳ YYYY-MM-DD` | `⏳ 2026-09-18` | Parsed, not exposed in filters/sort (see [Scope](#scope)) |
| Start date | `🛫 YYYY-MM-DD` | `🛫 2026-09-15` | Parsed, not exposed |
| Created date | `➕ YYYY-MM-DD` | `➕ 2026-09-01` | Parsed, not exposed |
| Cancelled date | `❌ YYYY-MM-DD` | `❌ 2026-09-12` | Parsed, not exposed |
| Priority | one of `🔺` `⏫` `🔼` `🔽` `⏬` | `⏫` | No signifier = `None`. Ordinal order (most → least urgent): `Highest, High, Medium, None, Low, Lowest` — `Low`/`Lowest` sort *below* `None`, matching the plugin |
| Tags | `#tag`, `#nested/tag` | `#work/release` | Stay in the description (only dates/priority are stripped); `#` optional when filtering |
| Status | the character in `[ ]` | `[x]`, `[-]`, `[/]` | See [Statuses](#statuses) |

## Statuses

Every task's status resolves to a `(symbol, name, type)` triple. `type` is
one of the plugin's six groups: `TODO`, `IN_PROGRESS`, `ON_HOLD`, `DONE`,
`CANCELLED`, `NON_TASK`. `done`/`--status done` matches `DONE`, `CANCELLED`,
and `NON_TASK`; `not done`/`--status open` matches the other three.

Built-in defaults:

| Symbol | Name | Type |
|---|---|---|
| ` ` (space) | Todo | `TODO` |
| `x` | Done | `DONE` |
| `X` | Done | `DONE` |
| `-` | Cancelled | `CANCELLED` |
| `/` | In Progress | `IN_PROGRESS` |

A symbol not in this list falls back to an `"Unknown"`/`TODO` status,
matching the plugin's own documented behaviour. Override or extend the list
via `[tasks]` in `config.toml` — see [CONFIG_REFERENCE.md](CONFIG_REFERENCE.md#tasks).

---

## CLI: `clin tasks list`

```console
clin tasks list [OPTIONS]

  --status <STATUS>      `open` (not done), `done`, or `all` (default: no status filter)
  --due <DUE>             Due-date filter, e.g. `before 2025-01-01`, `today`, `2025-01-01`
  --done <DONE>           Done-date filter, same syntax as --due
  --priority <PRIORITY>   Priority filter, e.g. `high`, `above none`, `not none`
  --tag <TAG>             Require a tag (repeatable; AND'd). `#` is optional
  --path <PATH>           Only tasks whose note path contains this text
  --sort <SORT>           Sort key (repeatable), e.g. `due`, `priority reverse`
  --query <QUERY>         Raw multi-line query (see below); ignores the flags above when set
  -v, --verbose           Also print each task's note path and line number
```

Every flag is sugar for one line of the same [query grammar](#query-grammar)
`--query` accepts directly — `--tag work` is exactly `tags include work`,
`--sort "priority reverse"` is exactly `sort by priority reverse`. Flags are
AND'd together; `--query` is the full escape hatch for anything the flags
don't cover (boolean combinations, `status.type`, `description`, etc.) and
is mutually exclusive with them (when `--query` is given, the other flags
are ignored, so there's exactly one filter/sort source per invocation).

With no filters at all, `clin tasks list` lists everything in the vault.

### Examples

```console
$ clin tasks list --status open --tag work --sort due
$ clin tasks list --due "before 2025-01-01" --priority high
$ clin tasks list --path Projects --verbose
$ clin tasks list --query "(no due date) OR (due after 2021-04-04)
sort by due
sort by priority reverse"
```

---

## Query grammar

Filter lines are AND'd together (one filter per line, like the plugin's own
`tasks` code-block query). `sort by <key>` lines set the sort order.

### Filters

```text
done
not done
has due date / no due date
due (on|before|after|on or before|on or after) <date>
has done date / no done date
done (on|before|after|on or before|on or after) <date>
priority is (above|below|not)? (lowest|low|none|medium|high|highest)
has tags / no tags
tags include <tag>      (or: tag includes <tag>)
tags do not include <tag>
path includes <text>
path does not include <text>
description includes <text>
description does not include <text>
status.name includes <text>
status.type is <TODO|IN_PROGRESS|ON_HOLD|DONE|CANCELLED|NON_TASK>
```

`<date>` is either `YYYY-MM-DD` or one of: `today`, `tomorrow`, `yesterday`,
`in N days`, `N days ago`, `next <weekday>`, `last <weekday>` — resolved
once, at query-parse time, against today's date. `next`/`last <weekday>`
never resolve to today itself, even if today is that weekday.

One level of boolean combination is supported by parenthesizing each side:

```text
(no due date) OR (due after 2021-04-04)
(path includes Projects) AND NOT (tags include #done)
NOT (done)
```

`AND`, `OR`, `AND NOT`, `OR NOT`, and `XOR` are all supported connectives.
Nesting parenthesized groups inside each other is not — the plugin's own
docs describe this same one-level scope as its "Matching multiple filters"
feature.

### Sorting

```text
sort by due
sort by done
sort by priority
sort by status.type      (also: status)
sort by status.name
sort by path
sort by description
sort by tags
sort by <key> reverse
```

Multiple `sort by` lines apply in order, each breaking ties from the one
before. With no `sort by` lines, the default order is
`status.type, due, priority, path` (ascending) — the plugin's own default
minus `urgency`, a derived multi-factor score that's out of scope here (see
below).

Blank lines and lines starting with `#` are ignored (comments), matching
the plugin's query blocks.

---

## Configuration

`[tasks]` in `config.toml` customizes the checkbox-symbol-to-status
mapping. See [CONFIG_REFERENCE.md](CONFIG_REFERENCE.md#tasks) for the full
option and its default value.

---

## Scope

Ported deliberately as a subset, not a clone, of the Tasks plugin. Not
implemented:

- **Date ranges** — `due 2023-11-25 2023-11-30`, `next week`,
  `2022-W14`/`2023-10`/`2021-Q4` numbered ranges. Only single-date
  comparisons (`before`/`after`/`on`/`on or before`/`on or after`) are
  supported.
- **Scheduled/start/created/cancelled dates as filter/sort keys** — parsed
  from every task, but only `due`/`done` are exposed, per the requested
  scope. Extending this is a small, low-risk follow-up: the parsing already
  captures all six dates uniformly.
- **Recurrence** (`🔁 every week`), **task dependencies** (`🆔`/`⛔`,
  `is blocking`/`is blocked`), and **on-completion actions** (`🏁 delete`) —
  each a substantial feature in its own right in the source plugin.
- **Grouping** (`group by ...`) — a pure post-processing step over sorted
  results; addable later without touching the query engine.
- **Urgency** — a derived score from several task properties combined; not
  computed, and therefore not part of the default sort order (the plugin's
  own default is `status.type, urgency, due, priority, path`; this CLI's is
  `status.type, due, priority, path`).
- **`filter by function` / `sort by function`** — the plugin's JavaScript
  scripting layer. Not applicable outside a JS-hosted plugin runtime.
- **Editing** — no toggling a task's status, no writing a date back to a
  note. `clin tasks list` only ever reads.
- **Full custom status collections** — statuses are configured via clin's
  own `[tasks]` config section (see above), not by reading the Tasks
  plugin's actual settings file
  (`.obsidian/plugins/obsidian-tasks-plugin/data.json`).

---

## Encrypted vaults

`.clin` (encrypted) notes are included in the scan. For plain `.md` notes,
`--verbose` line numbers match the real on-disk file (frontmatter's line
count is added back after parsing the frontmatter-stripped body). For
`.clin` notes, the reported line number is relative to the decrypted
content instead — the on-disk file is ciphertext and has no other
meaningful line numbering of its own.
