//! Reader and writer for the agent layer's open phase marks.
//!
//! This module owns *every* fact about the mark-file format, so the coupling to
//! the `tt-safe` shell wrapper lives in exactly one file. That is deliberate
//! fork-local divergence — `tt-safe` does not exist upstream — and keeping it
//! self-contained means an upstream merge can simply drop this file.
//!
//! The format, from `bin/tt-safe`:
//!
//! - One file per phase at `$MARK_DIR/<project>.<issue>.<phase>`, where
//!   `MARK_DIR` is `${TT_MARK_DIR:-$HOME/.cache/tt-safe/marks}`.
//! - The name is sanitised `[^A-Za-z0-9._-]` → `_`, so a *segment* may itself
//!   contain `.` or `_` and the name is **not** losslessly splittable.
//! - The content is a single unix-seconds start timestamp.
//! - A mark may have sibling files alongside it (`<mark>.last` today, and
//!   `<mark>.beats` once the append-only heartbeat lands) which are not marks.
//!
//! Only the start timestamp is read. See [`open_marks_in`] for why the
//! heartbeat siblings are deliberately left alone.

use chrono::{DateTime, Local};
use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Suffixes that turn a mark's name into one of its sibling files.
///
/// `.last` is `tt-safe touch`'s overwritten heartbeat; `.beats` is the
/// append-only replacement that #12 introduces. Listing both now means the
/// reader is already correct when that change lands.
const SIBLING_SUFFIXES: [&str; 2] = [".last", ".beats"];

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
}

/// The directory `tt-safe` keeps its marks in, mirroring `bin/tt-safe`'s
/// `MARK_DIR="${TT_MARK_DIR:-$HOME/.cache/tt-safe/marks}"`.
pub fn mark_dir() -> Option<PathBuf> {
    resolve_mark_dir(std::env::var_os("TT_MARK_DIR"), std::env::var_os("HOME"))
}

/// The env-free half of [`mark_dir`], so the fallback can be tested without
/// repointing this process's `HOME`.
fn resolve_mark_dir(mark_dir: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    match mark_dir {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => Some(PathBuf::from(home?).join(".cache/tt-safe/marks")),
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
/// **Only the start timestamp is read; the heartbeat siblings are not.** A
/// directory-mtime fingerprint cannot detect `tt-safe touch` rewriting
/// `<mark>.last` in place, so anything derived from a heartbeat would appear to
/// work and then silently go stale — and #12 changes that file's shape anyway.
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

    /// A fresh scratch mark directory. The real one at
    /// `~/.cache/tt-safe/marks` has live marks in it and must never be touched.
    fn sandbox(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tt-marks-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, name: &str, contents: &str) {
        fs::write(dir.join(name), contents).unwrap();
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

    #[test]
    fn a_beats_sibling_is_skipped_too() {
        let dir = sandbox("beats");
        write(&dir, "tt.8.impl", "1000200\n");
        write(&dir, "tt.8.impl.beats", "1000210\n1000220\n");
        // No `tt.9.beats` mark exists, so this one is a mark of its own.
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
    fn the_default_directory_falls_back_to_home() {
        assert_eq!(
            resolve_mark_dir(None, Some("/sandbox/home".into())),
            Some(PathBuf::from("/sandbox/home/.cache/tt-safe/marks"))
        );
        // An empty `TT_MARK_DIR` is no setting at all, as in the shell.
        assert_eq!(
            resolve_mark_dir(Some("".into()), Some("/sandbox/home".into())),
            Some(PathBuf::from("/sandbox/home/.cache/tt-safe/marks"))
        );
        assert_eq!(
            resolve_mark_dir(Some("/elsewhere".into()), Some("/sandbox/home".into())),
            Some(PathBuf::from("/elsewhere"))
        );
        assert_eq!(resolve_mark_dir(None, None), None, "no HOME, no default");
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
