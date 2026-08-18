//! Reader and writer for the agent layer's open phase marks.
//!
//! This module owns *every* fact about the mark-file format, so the coupling to
//! the `tt-safe` shell wrapper lives in exactly one file. That is deliberate
//! fork-local divergence — `tt-safe` does not exist upstream — and keeping it
//! self-contained means an upstream merge can simply drop this file.
//!
//! The format, shared with `bin/tt-safe` until the wrapper is retired:
//!
//! - One file per phase at `<mark dir>/<project>.<issue>.<phase>`, where the mark
//!   directory is `$TT_MARK_DIR` if set, else `marks` inside this app's own cache
//!   directory (`$HOME/Library/Caches/com.timetracker.tt` on macOS).
//! - The name is sanitised `[^A-Za-z0-9._-]` → `_`, so a *segment* may itself
//!   contain `.` or `_` and the name is **not** losslessly splittable.
//! - The content is a single unix-seconds start timestamp.
//! - Heartbeats live in a `beats/` **subdirectory** of the mark directory, one
//!   append-only file per mark under the same name — never as a
//!   `<mark>.<suffix>` sibling. A mark can still have one legacy sibling,
//!   `<mark>.last`, from before `beats/` existed; nothing writes it any more and
//!   `cancel` clears it.
//!
//! Only the start timestamp is read. See [`open_marks_in`] for why the heartbeats
//! are deliberately left alone.

use chrono::{DateTime, Local, TimeDelta};
use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::duration;

/// Suffixes that turn a mark's name into one of its sibling files.
///
/// Exactly one: `.last`, the single-value heartbeat `touch` overwrote before
/// `beats/` existed. It is real — migration carries it, `cancel` clears it and
/// `end` still reads it as a one-beat sequence — so the reader has to know it is
/// not a mark.
///
/// A `.beats` sibling was listed here in anticipation of the append-only
/// heartbeat, but that landed as a `beats/` **subdirectory** instead
/// (see [`beats_path`]), so nothing has ever written the sibling form and nothing
/// will. It is dropped rather than left as dead weight that implies a file shape
/// this module does not have.
const SIBLING_SUFFIXES: [&str; 1] = [".last"];

/// One open phase mark.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    pub project: String,
    /// `None` when the mark was made with `tt-safe`'s no-issue sentinel `-`.
    pub issue: Option<String>,
    pub phase: String,
    pub start: DateTime<Local>,
}

impl Mark {
    /// `project/issue phase`, or bare `project phase` for a mark made with
    /// `tt-safe`'s no-issue sentinel `-`.
    ///
    /// This module owns the row format as well as the file format, so the CLI and
    /// the TUI cannot drift into showing the same mark two different ways.
    pub fn label(&self) -> String {
        let subject = match &self.issue {
            Some(issue) => format!("{}/{}", self.project, issue),
            None => self.project.clone(),
        };
        if self.phase.is_empty() {
            // A name with no `.` at all has no phase to show — see `split_key`.
            return subject;
        }
        format!("{} {}", subject, self.phase)
    }

    /// The clock time the mark was made, `HH:MM`, as `tt-safe marks` prints it.
    pub fn started_at(&self) -> String {
        self.start.format("%H:%M").to_string()
    }

    /// How long this mark has been open, as `2m` or `2h 6m`.
    ///
    /// Derived from [`start`](Mark::start) on every call and **never cached as a
    /// string**: the mark list is only re-read when the mark directory changes,
    /// so a cached elapsed would freeze until someone began or dropped a mark.
    /// Deriving it here instead means an open surface counts up every frame at
    /// the cost of no directory read at all.
    pub fn elapsed(&self) -> String {
        self.elapsed_at(Local::now())
    }

    /// The `now`-taking half of [`elapsed`](Mark::elapsed), so the formatting can
    /// be tested without waiting for a clock.
    pub fn elapsed_at(&self, now: DateTime<Local>) -> String {
        // A start in the future (a clock stepping back, a hand-written mark) reads
        // as 0m rather than as a negative age.
        let minutes = (now - self.start).num_minutes().max(0);
        match minutes / 60 {
            0 => format!("{}m", minutes),
            hours => format!("{}h {}m", hours, minutes % 60),
        }
    }

    /// How long this mark has been open, in the **house** duration format —
    /// always `{h}h {m}m`, as [`crate::duration::format`] renders every other
    /// span the CLI prints.
    ///
    /// Not [`elapsed`](Mark::elapsed), which renders `15m` / `2h 6m` for the TUI's
    /// narrow rows. And clamped first: `duration::format` on a negative span
    /// prints `0h -5m`, and a mark started in the future is a real case — a clock
    /// stepping back, or a hand-written mark.
    pub fn age_at(&self, now: DateTime<Local>) -> String {
        duration::format((now - self.start).max(TimeDelta::zero()))
    }
}

/// Narrowest label column `tt agent list` will use, so a single short mark still
/// reads as a column rather than as a sentence. `src/tui/render.rs` keeps its own
/// copy for the surface's rows; whether the two rows should converge is #58's.
const LABEL_WIDTH: usize = 18;

/// The rows `tt agent list` prints, one per mark, without the CLI's indent.
///
/// Assembled here rather than in `crate::agent` because this module owns the row
/// as well as the file: the TUI composes its own styled spans from
/// [`Mark::label`], [`Mark::started_at`] and [`Mark::elapsed`], and a second
/// plain-text row built somewhere else is how the same mark starts reading two
/// ways.
///
/// **This is the port's one deliberate divergence from `tt-safe marks`.** The
/// wrapper prints `%-*s  %s  (%s)`; a `tt` subcommand looks like the rest of `tt`
/// instead — one label column, ` - since HH:MM`, and the house `{h}h {m}m`
/// duration. Owner ruling, 2026-08-18: #58 must not "fix" this back toward the
/// shell.
pub fn rows(marks: &[Mark]) -> Vec<String> {
    rows_at(marks, Local::now())
}

/// The `now`-taking half of [`rows`], so a row can be asserted against a
/// fabricated age instead of a clock.
pub fn rows_at(marks: &[Mark], now: DateTime<Local>) -> Vec<String> {
    // One column for the whole list, as the TUI does it, so the start times and
    // ages read as columns instead of trailing each label at its own indent.
    let width = marks
        .iter()
        .map(|mark| mark.label().chars().count())
        .max()
        .unwrap_or(0)
        .max(LABEL_WIDTH);

    marks
        .iter()
        .map(|mark| {
            let label = mark.label();
            let pad = " ".repeat(width.saturating_sub(label.chars().count()));
            format!(
                "{}{} - since {} ({})",
                label,
                pad,
                mark.started_at(),
                mark.age_at(now)
            )
        })
        .collect()
}

/// The directory the marks live in: `$TT_MARK_DIR` when set, else `marks` inside
/// this app's own cache directory.
///
/// `tt` writes these files itself now, so they belong under the app's own
/// directories rather than in the wrapper's `~/.cache/tt-safe/`. `TT_MARK_DIR`
/// stays as the override — every sandboxed test depends on it, and the wrapper
/// still honours the same variable — and "set but empty" is no setting at all,
/// as in the shell.
///
/// `None` only when there is no home directory to resolve at all, which is a
/// caller's error to report rather than a path to guess at.
pub fn mark_dir() -> Option<PathBuf> {
    resolve_mark_dir(std::env::var_os("TT_MARK_DIR"), cache_dir())
}

/// This app's cache directory — `$HOME/Library/Caches/com.timetracker.tt` on
/// macOS — the same `ProjectDirs` triple `storage::get_data_path` uses.
///
/// It follows `HOME`, so a sandboxed `HOME` redirects the marks exactly as it
/// already redirects the store.
fn cache_dir() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("com", "timetracker", "tt")?;
    Some(dirs.cache_dir().to_path_buf())
}

/// The env-free half of [`mark_dir`], taking the resolved cache root so the
/// fallback can be tested without repointing this process's `HOME`.
fn resolve_mark_dir(mark_dir: Option<OsString>, cache: Option<PathBuf>) -> Option<PathBuf> {
    match mark_dir {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => Some(cache?.join("marks")),
    }
}

/// Every open mark, newest first. A missing or empty mark directory is an empty
/// list rather than an error — nothing running is the normal case.
pub fn open_marks() -> Vec<Mark> {
    match mark_dir() {
        Some(dir) => open_marks_in(&dir),
        None => Vec::new(),
    }
}

/// Every open mark in `dir`, newest first.
///
/// Unreadable and unparseable files are skipped individually, so one bad file
/// never hides its siblings.
///
/// **Only the start timestamp is read; the heartbeats are not.** A directory-mtime
/// fingerprint cannot detect a beat being appended inside `beats/`, so anything
/// derived from a heartbeat would appear to work and then silently go stale. The
/// `beats/` subdirectory is skipped by the file-type filter below, never by its
/// name — see [`beats_path`].
pub fn open_marks_in(dir: &Path) -> Vec<Mark> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let names: Vec<OsString> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.file_name())
        .collect();
    let present: HashSet<&OsString> = names.iter().collect();

    let mut marks: Vec<Mark> = names
        .iter()
        .filter(|name| !is_sibling_of_a_mark(name, &present))
        .filter_map(|name| read_mark(dir, name))
        .collect();

    // Newest first, then by name, so the order never depends on `read_dir`.
    marks.sort_by(|a, b| {
        b.start
            .cmp(&a.start)
            .then_with(|| (&a.project, &a.issue, &a.phase).cmp(&(&b.project, &b.issue, &b.phase)))
    });
    marks
}

/// Whether `name` is a mark's sibling rather than a mark in its own right.
///
/// Decided **structurally**: only a file whose name is some *other* file's name
/// plus a known sibling suffix counts. Do not "simplify" this into a suffix test
/// on the whole filename — that is the shape of `tt-safe`'s own `cmd_marks`
/// (`case "$mark" in *.last) continue ;; esac`) and it is a real bug, filed as
/// #16: a mark whose phase is literally `last` produces `proj.-.last`, which a
/// plain suffix test hides. Aligning this reader with the shell would reproduce
/// that bug in the TUI.
fn is_sibling_of_a_mark(name: &OsString, present: &HashSet<&OsString>) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    SIBLING_SUFFIXES.iter().any(|suffix| {
        name.strip_suffix(suffix)
            .is_some_and(|stem| !stem.is_empty() && present.contains(&OsString::from(stem)))
    })
}

/// The instant a mark file holds, or `None` when it holds anything else.
///
/// The single place the file's one-line body is interpreted, so the reader, the
/// row and `begin`'s "already marked" message all read a mark the same way.
fn read_start(path: &Path) -> Option<DateTime<Local>> {
    let contents = fs::read_to_string(path).ok()?;
    let seconds: i64 = contents.trim().parse().ok()?;
    Some(DateTime::from_timestamp(seconds, 0)?.with_timezone(&Local))
}

/// Parse one mark file, or `None` if it is not one.
fn read_mark(dir: &Path, name: &OsString) -> Option<Mark> {
    let start = read_start(&dir.join(name))?;
    let (project, issue, phase) = split_key(name.to_str()?);
    Some(Mark {
        project,
        issue,
        phase,
        start,
    })
}

/// Split a mark filename back into `<project>.<issue>.<phase>`.
///
/// Lossy by construction: sanitisation keeps `.` and `_`, so any segment may
/// contain a `.` and there is no way to know where the real boundaries were.
/// Splitting on the **first** and **last** `.` recovers the common case exactly
/// and degrades a pathological name into a readable-but-imperfect label — which
/// is the right trade for a display-only reader, and much better than refusing
/// to list a mark that is genuinely open.
fn split_key(name: &str) -> (String, Option<String>, String) {
    let issue = |raw: &str| (raw != "-").then(|| raw.to_string());

    match name.split_once('.') {
        // `a.b.c` → project `a`, issue `b`, phase `c`; extra dots land in the
        // middle field, which is the least misleading place for them.
        Some((project, rest)) => match rest.rsplit_once('.') {
            Some((mid, phase)) => (project.to_string(), issue(mid), phase.to_string()),
            // Only one `.`: read it as an issue-less `<project>.<phase>`, the
            // same shape `tt-safe`'s `-` sentinel produces.
            None => (project.to_string(), None, rest.to_string()),
        },
        // No `.` at all: nothing to split, so show the whole name as the project.
        None => (name.to_string(), None, String::new()),
    }
}

// --- writer ---------------------------------------------------------------
//
// The writer lives beside the reader because this module owns *every* fact about
// the format: a writer in its own file would make that claim false and let the
// two halves drift. Nothing here locks anything — no path below is the store, and
// `create_new` plus `O_APPEND` cover the only two races there are.

/// What [`begin_in`] found: a mark it created, or one that was already open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Begin {
    /// The mark file was created, holding this instant.
    Created(DateTime<Local>),
    /// A mark was already open and is left byte-identical, so the original start
    /// wins. `None` when its contents are not a timestamp — the same
    /// unreadable-start case `bin/tt-safe`'s `fmt_time` prints as `??:??`.
    AlreadyOpen(Option<DateTime<Local>>),
}

/// What [`touch_in`] did: recorded a heartbeat, or refused for want of a mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Touch {
    Recorded,
    /// No mark file for this phase, so no beats file was created either.
    NoMark,
}

/// The sanitised filename for one phase: `<project>.<issue>.<phase>` with every
/// character outside `[A-Za-z0-9._-]` replaced by `_`, mirroring `bin/tt-safe`'s
/// `mark_key`.
///
/// The key is built here and nowhere else, because a mark and its heartbeats are
/// the same name in two directories: a mark path and a beats path that sanitised
/// differently would leave a phase with beats it could never find again. The
/// no-issue sentinel `-` is written literally, so `begin vinge - plan` is
/// `vinge.-.plan`.
pub fn mark_key(project: &str, issue: &str, phase: &str) -> String {
    format!("{project}.{issue}.{phase}")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The mark file for `key` inside `dir`.
pub fn mark_path(dir: &Path, key: &str) -> PathBuf {
    dir.join(key)
}

/// The heartbeat file for `key` inside `dir`.
///
/// A `beats/` **subdirectory**, never a `<mark>.<suffix>` sibling. Because
/// [`mark_key`] always builds `<project>.<issue>.<phase>`, every mark filename
/// contains at least two dots, so no mark can ever be named `beats` — which is
/// what lets [`open_marks_in`] separate marks from heartbeats with a file-type
/// test instead of a name filter (see #16).
pub fn beats_path(dir: &Path, key: &str) -> PathBuf {
    dir.join("beats").join(key)
}

/// The legacy pre-beats heartbeat for `key`: `tt-safe touch` overwrote a single
/// value into it before `beats/` existed. Never written here, only cleared.
fn legacy_beat_path(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("{key}.last"))
}

/// Open a mark for one phase, or report the one already open.
///
/// Created with `create_new`, which makes the file's existence and its creation a
/// single atomic step: the shell's `[ -f "$mark" ] || date +%s > "$mark"` can lose
/// a start to a concurrent `begin` between the test and the write, and this cannot.
/// An existing mark is never rewritten — the original start is the whole point of a
/// mark surviving a compacted context.
pub fn begin_in(dir: &Path, project: &str, issue: &str, phase: &str) -> io::Result<Begin> {
    let path = mark_path(dir, &mark_key(project, issue, phase));
    fs::create_dir_all(dir)?;

    let start = Local::now();
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            writeln!(file, "{}", start.timestamp())?;
            Ok(Begin::Created(start))
        }
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            Ok(Begin::AlreadyOpen(read_start(&path)))
        }
        Err(err) => Err(err),
    }
}

/// Append one heartbeat for a phase that is already marked.
///
/// Appended, never overwritten: the *sequence* of beats is the evidence that
/// tells a long active phase from a long silence. One `O_APPEND` line per call,
/// which needs no lock — concurrent appends of a short line do not interleave.
///
/// A phase nobody began records nothing at all: a beats file without its mark
/// would be heartbeats for work no `end` can ever measure.
pub fn touch_in(dir: &Path, project: &str, issue: &str, phase: &str) -> io::Result<Touch> {
    let key = mark_key(project, issue, phase);
    if !mark_path(dir, &key).is_file() {
        return Ok(Touch::NoMark);
    }

    let beats = beats_path(dir, &key);
    if let Some(parent) = beats.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().append(true).create(true).open(&beats)?;
    writeln!(file, "{}", Local::now().timestamp())?;
    Ok(Touch::Recorded)
}

/// Drop a mark and every heartbeat belonging to it.
///
/// All three paths are cleared — the mark, the legacy `.last` beat and the
/// `beats/` entry — so an upgraded session leaves nothing behind, and each is
/// allowed to be absent: cancelling a phase that was never begun is not an error.
/// The `beats/` directory itself stays, since other phases' beats live in it.
pub fn cancel_in(dir: &Path, project: &str, issue: &str, phase: &str) -> io::Result<()> {
    let key = mark_key(project, issue, phase);
    for path in [
        mark_path(dir, &key),
        legacy_beat_path(dir, &key),
        beats_path(dir, &key),
    ] {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Serialises the one test here that repoints `TT_MARK_DIR`, since env is
    /// process-wide. Every other test passes its directory in explicitly. Shared
    /// with the TUI's `HOME`-repointing tests — see `storage::env_guard`.
    use crate::storage::env_guard;

    /// A fresh scratch mark directory. The real one — under this app's cache
    /// directory, and the wrapper's older one beside it — has live marks in it
    /// and must never be touched.
    fn sandbox(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tt-marks-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, name: &str, contents: &str) {
        fs::write(dir.join(name), contents).unwrap();
    }

    /// A fabricated instant, so an age is derived from the fixture's own epochs
    /// rather than waited for.
    fn at(seconds: i64) -> DateTime<Local> {
        DateTime::from_timestamp(seconds, 0)
            .unwrap()
            .with_timezone(&Local)
    }

    fn labels(marks: &[Mark]) -> Vec<(String, Option<String>, String)> {
        marks
            .iter()
            .map(|m| (m.project.clone(), m.issue.clone(), m.phase.clone()))
            .collect()
    }

    #[test]
    fn marks_come_back_newest_first() {
        let dir = sandbox("newest-first");
        write(&dir, "tt.8.impl", "1000200\n");
        write(&dir, "loremind.64.plan", "1000300\n");
        write(&dir, "vinge.-.plan", "1000100\n");

        let marks = open_marks_in(&dir);
        assert_eq!(
            labels(&marks),
            vec![
                ("loremind".into(), Some("64".into()), "plan".into()),
                ("tt".into(), Some("8".into()), "impl".into()),
                ("vinge".into(), None, "plan".into()),
            ]
        );
        assert_eq!(marks[0].start.timestamp(), 1000300);
    }

    #[test]
    fn a_phase_literally_called_last_is_a_mark_and_a_real_last_sibling_is_not() {
        let dir = sandbox("phase-last");
        // The mark `tt-safe marks` itself cannot see — see #16.
        write(&dir, "proj.-.last", "1000100\n");
        // A genuine heartbeat sibling: `tt.8.impl` exists alongside it.
        write(&dir, "tt.8.impl", "1000200\n");
        write(&dir, "tt.8.impl.last", "1000250\n");

        assert_eq!(
            labels(&open_marks_in(&dir)),
            vec![
                ("tt".into(), Some("8".into()), "impl".into()),
                ("proj".into(), None, "last".into()),
            ]
        );
    }

    /// Heartbeats are a subdirectory, so they are excluded by the file-type
    /// filter and not by their name — which is why a mark whose *phase* is
    /// `beats` is still listed. There is no `<mark>.beats` sibling to skip: that
    /// form was never written and is no longer a sibling suffix.
    #[test]
    fn the_beats_subdirectory_is_not_a_mark_but_a_beats_phase_is() {
        let dir = sandbox("beats");
        write(&dir, "tt.8.impl", "1000200\n");
        fs::create_dir_all(dir.join("beats")).unwrap();
        write(&dir.join("beats"), "tt.8.impl", "1000210\n1000220\n");
        write(&dir, "proj.-.beats", "1000100\n");

        assert_eq!(
            labels(&open_marks_in(&dir)),
            vec![
                ("tt".into(), Some("8".into()), "impl".into()),
                ("proj".into(), None, "beats".into()),
            ]
        );
    }

    #[test]
    fn an_unparseable_file_is_skipped_without_failing_its_siblings() {
        let dir = sandbox("unparseable");
        write(&dir, "tt.8.impl", "1000200\n");
        write(&dir, "broken.1.impl", "not a timestamp\n");
        write(&dir, "empty.1.impl", "");

        assert_eq!(
            labels(&open_marks_in(&dir)),
            vec![("tt".into(), Some("8".into()), "impl".into())]
        );
    }

    #[test]
    fn a_missing_or_empty_directory_yields_no_marks() {
        let dir = sandbox("missing");
        assert_eq!(open_marks_in(&dir), Vec::new(), "empty directory");
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(open_marks_in(&dir), Vec::new(), "missing directory");
    }

    #[test]
    fn tt_mark_dir_is_honoured() {
        let _guard = env_guard();
        let dir = sandbox("env");
        write(&dir, "tt.13.impl", "1000200\n");

        let restore = std::env::var_os("TT_MARK_DIR");
        unsafe { std::env::set_var("TT_MARK_DIR", &dir) };
        assert_eq!(mark_dir().as_deref(), Some(dir.as_path()));
        let marks = open_marks();
        match restore {
            Some(value) => unsafe { std::env::set_var("TT_MARK_DIR", value) },
            None => unsafe { std::env::remove_var("TT_MARK_DIR") },
        }

        assert_eq!(
            labels(&marks),
            vec![("tt".into(), Some("13".into()), "impl".into())]
        );
    }

    #[test]
    fn the_default_directory_is_marks_inside_the_app_cache_dir() {
        let cache = || {
            Some(PathBuf::from(
                "/sandbox/home/Library/Caches/com.timetracker.tt",
            ))
        };
        assert_eq!(
            resolve_mark_dir(None, cache()),
            Some(PathBuf::from(
                "/sandbox/home/Library/Caches/com.timetracker.tt/marks"
            ))
        );
        // An empty `TT_MARK_DIR` is no setting at all, as in the shell.
        assert_eq!(
            resolve_mark_dir(Some("".into()), cache()),
            Some(PathBuf::from(
                "/sandbox/home/Library/Caches/com.timetracker.tt/marks"
            ))
        );
        assert_eq!(
            resolve_mark_dir(Some("/elsewhere".into()), cache()),
            Some(PathBuf::from("/elsewhere"))
        );
        assert_eq!(
            resolve_mark_dir(None, None),
            None,
            "no cache dir, no default"
        );
        // …but an override still stands without one, so `TT_MARK_DIR` alone is
        // enough to run.
        assert_eq!(
            resolve_mark_dir(Some("/elsewhere".into()), None),
            Some(PathBuf::from("/elsewhere"))
        );
    }

    /// The cache directory really does follow `HOME`, which is what makes a
    /// sandboxed `HOME` enough to redirect the marks as well as the store.
    #[test]
    fn the_cache_directory_follows_home() {
        let _guard = env_guard();
        let dir = sandbox("cache-follows-home");

        let restore_home = std::env::var_os("HOME");
        let restore_marks = std::env::var_os("TT_MARK_DIR");
        unsafe { std::env::set_var("HOME", &dir) };
        unsafe { std::env::remove_var("TT_MARK_DIR") };
        let resolved = mark_dir();
        // A mark written through the writer comes back from the default location.
        let begun = resolved.as_deref().map(|d| begin_in(d, "tt", "54", "impl"));
        let listed = open_marks();
        match restore_home {
            Some(value) => unsafe { std::env::set_var("HOME", value) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        if let Some(value) = restore_marks {
            unsafe { std::env::set_var("TT_MARK_DIR", value) };
        }

        let resolved = resolved.expect("a sandboxed HOME resolves a cache dir");
        assert!(
            resolved.starts_with(&dir),
            "the cache dir ignored HOME: {resolved:?}"
        );
        assert_eq!(resolved.file_name().unwrap(), "marks");
        begun.unwrap().unwrap();
        assert_eq!(
            labels(&listed),
            vec![("tt".into(), Some("54".into()), "impl".into())]
        );
    }

    /// The row format `tt-safe marks` prints and the TUI shows, in one place.
    #[test]
    fn a_row_reads_project_slash_issue_phase_and_drops_a_missing_issue() {
        let dir = sandbox("row-format");
        write(&dir, "tt.8.impl", "1000200\n");
        write(&dir, "loremind.64.plan", "1000300\n");
        // `tt-safe begin vinge - plan`: the `-` sentinel collapses away entirely.
        write(&dir, "vinge.-.plan", "1000100\n");
        // A name with nothing to split has no phase to show.
        write(&dir, "bare", "1000000\n");

        let labels: Vec<String> = open_marks_in(&dir).iter().map(Mark::label).collect();
        assert_eq!(
            labels,
            vec!["loremind/64 plan", "tt/8 impl", "vinge plan", "bare"]
        );

        let mark = &open_marks_in(&dir)[1];
        assert_eq!(mark.started_at(), mark.start.format("%H:%M").to_string());
        assert_eq!(mark.started_at().len(), 5, "HH:MM");
    }

    #[test]
    fn elapsed_is_derived_from_the_start_and_counts_up() {
        let mark = |seconds: i64| Mark {
            project: "tt".into(),
            issue: Some("14".into()),
            phase: "impl".into(),
            start: DateTime::from_timestamp(seconds, 0)
                .unwrap()
                .with_timezone(&Local),
        };
        let start = 1_000_000_000;
        let now = |offset: i64| {
            DateTime::from_timestamp(start + offset, 0)
                .unwrap()
                .with_timezone(&Local)
        };

        assert_eq!(mark(start).elapsed_at(now(0)), "0m");
        assert_eq!(mark(start).elapsed_at(now(119)), "1m", "seconds truncate");
        assert_eq!(mark(start).elapsed_at(now(120)), "2m");
        assert_eq!(mark(start).elapsed_at(now(60 * 60)), "1h 0m");
        assert_eq!(mark(start).elapsed_at(now(126 * 60)), "2h 6m");
        // The same mark, read again a minute later: the number moves without the
        // mark file being touched, which is the whole point of deriving it.
        let m = mark(start);
        assert_eq!(m.elapsed_at(now(60)), "1m");
        assert_eq!(m.elapsed_at(now(61 * 60)), "1h 1m");
        // A clock that stepped backwards must not print a negative age.
        assert_eq!(m.elapsed_at(now(-90)), "0m");
    }

    #[test]
    fn a_lossy_name_degrades_to_a_label_instead_of_an_error() {
        let dir = sandbox("lossy");
        // A project containing a `.`: the extra dot lands in the issue field.
        write(&dir, "my.proj.7.impl", "1000400\n");
        // A phase containing a `.` does the same.
        write(&dir, "tt.8.impl.v2", "1000300\n");
        // `_` survives sanitisation and needs no special handling.
        write(&dir, "my_proj.-.code_review", "1000200\n");
        // Nothing to split at all.
        write(&dir, "bare", "1000100\n");

        assert_eq!(
            labels(&open_marks_in(&dir)),
            vec![
                ("my".into(), Some("proj.7".into()), "impl".into()),
                ("tt".into(), Some("8.impl".into()), "v2".into()),
                ("my_proj".into(), None, "code_review".into()),
                ("bare".into(), None, String::new()),
            ]
        );
    }

    /// The house-style row `tt agent list` prints — the port's one ruled
    /// divergence from `tt-safe marks` (owner, 2026-08-18).
    #[test]
    fn a_row_is_one_padded_label_column_then_since_and_the_house_duration() {
        let dir = sandbox("rows");
        let now = 1_000_000_000;
        // Two labels of different lengths, so the column can be seen to align.
        write(
            &dir,
            "timetracker-rs.54.plan",
            &format!("{}\n", now - 15 * 60),
        );
        write(&dir, "vinge.12.impl", &format!("{}\n", now - 45 * 60));

        let marks = open_marks_in(&dir);
        let rows = rows_at(&marks, at(now));
        let since = |offset: i64| at(now - offset).format("%H:%M").to_string();
        assert_eq!(
            rows,
            vec![
                format!("timetracker-rs/54 plan - since {} (0h 15m)", since(15 * 60)),
                format!("vinge/12 impl          - since {} (0h 45m)", since(45 * 60)),
            ]
        );
        // One column for the whole list: the separator lands at the same offset.
        let separator = |row: &String| row.find(" - since").unwrap();
        assert_eq!(separator(&rows[0]), separator(&rows[1]));
    }

    #[test]
    fn a_row_reads_the_names_back_the_way_the_reader_does() {
        let dir = sandbox("rows-names");
        let now = 1_000_000_000;
        // The `-` sentinel collapses away entirely — never `vinge/-`.
        write(&dir, "vinge.-.plan", &format!("{}\n", now));
        // Nothing to split: a bare project.
        write(&dir, "solo", &format!("{}\n", now));
        // A phase literally called `last` is a mark, not a sibling (#16).
        write(&dir, "proj.-.last", &format!("{}\n", now));

        let rows = rows_at(&open_marks_in(&dir), at(now));
        assert!(
            rows.iter().any(|row| row.starts_with("vinge plan ")),
            "the - sentinel leaked or the row is missing: {rows:?}"
        );
        assert!(
            rows.iter().any(|row| row.starts_with("solo ")),
            "a dotless name should list as a bare project: {rows:?}"
        );
        assert!(
            rows.iter().any(|row| row.starts_with("proj last ")),
            "a phase called `last` should still be listed: {rows:?}"
        );
        assert!(
            !rows.iter().any(|row| row.contains("vinge/-")),
            "the - sentinel leaked into a label: {rows:?}"
        );
        // Every label here is shorter than the minimum column, so all three rows
        // put `since` at the same offset — that minimum is what stops one short
        // mark reading as a sentence.
        for row in &rows {
            assert_eq!(row.find(" - since"), Some(18), "{row:?}");
        }
    }

    #[test]
    fn a_rows_duration_is_the_house_format_and_never_negative() {
        let dir = sandbox("rows-age");
        let now = 1_000_000_000;
        write(&dir, "long.1.impl", &format!("{}\n", now - 126 * 60));
        write(&dir, "short.2.impl", &format!("{}\n", now - 2 * 60));
        // A start in the future reads as `0h 0m`, not as `0h -10m`.
        write(&dir, "future.3.impl", &format!("{}\n", now + 600));

        let rows = rows_at(&open_marks_in(&dir), at(now));
        let ages: Vec<&str> = rows
            .iter()
            .map(|row| &row[row.find('(').unwrap()..])
            .collect();
        assert_eq!(ages, vec!["(0h 0m)", "(0h 2m)", "(2h 6m)"], "{rows:?}");
    }

    #[test]
    fn no_marks_have_no_rows() {
        let dir = sandbox("rows-empty");
        assert_eq!(
            rows_at(&open_marks_in(&dir), Local::now()),
            Vec::<String>::new()
        );
    }

    // --- writer -----------------------------------------------------------

    fn read(dir: &Path, name: &str) -> String {
        fs::read_to_string(dir.join(name)).unwrap()
    }

    #[test]
    fn a_key_is_sanitised_once_and_keeps_the_no_issue_sentinel() {
        assert_eq!(mark_key("tt", "8", "impl"), "tt.8.impl");
        // The `-` sentinel is a literal part of the name, not an absent field.
        assert_eq!(mark_key("vinge", "-", "plan"), "vinge.-.plan");
        // `[^A-Za-z0-9._-]` → `_`, exactly as the shell's `${key//…/_}`.
        assert_eq!(
            mark_key("my proj", "7", "code/review"),
            "my_proj.7.code_review"
        );
        // Both paths are built from that one key, so they can never disagree.
        let dir = Path::new("/marks");
        assert_eq!(
            mark_path(dir, &mark_key("my proj", "7", "impl")),
            PathBuf::from("/marks/my_proj.7.impl")
        );
        assert_eq!(
            beats_path(dir, &mark_key("my proj", "7", "impl")),
            PathBuf::from("/marks/beats/my_proj.7.impl")
        );
    }

    #[test]
    fn begin_writes_the_start_the_reader_reads_back() {
        let dir = sandbox("begin");
        let Begin::Created(start) = begin_in(&dir, "tt", "8", "impl").unwrap() else {
            panic!("a fresh directory should have no mark to find");
        };

        // The file holds exactly one `\n`-terminated decimal epoch.
        assert_eq!(read(&dir, "tt.8.impl"), format!("{}\n", start.timestamp()));
        // Writer and reader agree, which is the whole point of the Story.
        assert_eq!(
            labels(&open_marks_in(&dir)),
            vec![("tt".into(), Some("8".into()), "impl".into())]
        );
        assert_eq!(open_marks_in(&dir)[0].start.timestamp(), start.timestamp());
    }

    #[test]
    fn a_second_begin_keeps_the_original_start_byte_for_byte() {
        let dir = sandbox("begin-again");
        write(&dir, "tt.8.impl", "1000200\n");

        let again = begin_in(&dir, "tt", "8", "impl").unwrap();
        assert_eq!(
            read(&dir, "tt.8.impl"),
            "1000200\n",
            "the mark was rewritten"
        );
        match again {
            Begin::AlreadyOpen(Some(start)) => assert_eq!(start.timestamp(), 1000200),
            other => panic!("expected an already-open mark, got {other:?}"),
        }

        // An unreadable start still reports the mark as open, so the caller can say
        // so with `??:??` rather than silently taking a new start.
        write(&dir, "broken.1.impl", "not a timestamp\n");
        assert_eq!(
            begin_in(&dir, "broken", "1", "impl").unwrap(),
            Begin::AlreadyOpen(None)
        );
    }

    #[test]
    fn each_touch_appends_one_beat_and_leaves_the_mark_alone() {
        let dir = sandbox("touch");
        begin_in(&dir, "tt", "8", "impl").unwrap();
        let mark = read(&dir, "tt.8.impl");

        for _ in 0..3 {
            assert_eq!(touch_in(&dir, "tt", "8", "impl").unwrap(), Touch::Recorded);
        }

        let beats = read(&dir, "beats/tt.8.impl");
        assert_eq!(beats.lines().count(), 3, "one line per touch: {beats:?}");
        assert!(
            beats.lines().all(|line| line.parse::<i64>().is_ok()),
            "every beat is a bare epoch: {beats:?}"
        );
        assert_eq!(read(&dir, "tt.8.impl"), mark, "the mark was rewritten");
        // Beats are a subdirectory, never a sibling suffix.
        assert!(!dir.join("tt.8.impl.beats").exists());
        assert!(!dir.join("tt.8.impl.last").exists());
    }

    #[test]
    fn touch_on_an_unbegun_phase_writes_nothing_at_all() {
        let dir = sandbox("touch-unbegun");
        assert_eq!(touch_in(&dir, "tt", "8", "impl").unwrap(), Touch::NoMark);
        assert!(
            !dir.join("beats").exists(),
            "no beats directory was created"
        );
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            0,
            "the directory is untouched"
        );
    }

    #[test]
    fn cancel_clears_the_mark_its_legacy_beat_and_its_beats_but_not_a_sibling_phase() {
        let dir = sandbox("cancel");
        begin_in(&dir, "tt", "8", "impl").unwrap();
        touch_in(&dir, "tt", "8", "impl").unwrap();
        // The pre-beats heartbeat, from a session that predates `beats/`.
        write(&dir, "tt.8.impl.last", "1000250\n");
        begin_in(&dir, "other", "9", "plan").unwrap();
        touch_in(&dir, "other", "9", "plan").unwrap();

        cancel_in(&dir, "tt", "8", "impl").unwrap();

        assert!(!dir.join("tt.8.impl").exists());
        assert!(!dir.join("tt.8.impl.last").exists());
        assert!(!dir.join("beats/tt.8.impl").exists());
        assert!(
            dir.join("beats").is_dir(),
            "the beats directory itself stays"
        );
        assert!(
            dir.join("other.9.plan").is_file(),
            "a sibling mark was cleared"
        );
        assert!(dir.join("beats/other.9.plan").is_file());

        // Cancelling what was never begun is not an error.
        cancel_in(&dir, "tt", "8", "impl").unwrap();
        cancel_in(&dir, "never", "1", "begun").unwrap();
    }
}
