//! Integration tests for `tt agent`, driving the real binary.
//!
//! The mark-lifecycle half of `tt-safe`'s own oracle
//! (`/Users/jnao/Code/tt/tests/tt-safe-gaps.sh`) ported onto `tt agent`, plus the
//! assertions that only the real process can make — that a mark command takes no
//! store lock and creates no `data.json`.
//!
//! **Sandboxing is not optional and is asserted, not assumed.** The live store
//! (`~/Library/Application Support/com.timetracker.tt/data.json`) and the live
//! mark directory are written continuously by concurrent agent sessions, so every
//! case gets a throwaway `HOME` *and* `TT_MARK_DIR` inside it, and [`Case::run`]
//! refuses to run a command whose paths are not inside the sandbox.
//!
//! Time is fabricated, never waited for: marks and heartbeats are written
//! directly at synthetic epochs and every expectation is derived from those
//! fixtures rather than from a clock read by hand.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

/// One case's sandbox: a `HOME`, a mark directory inside it, and nothing else.
struct Case {
    home: PathBuf,
    marks: PathBuf,
}

/// What one `tt agent` invocation produced.
struct Run {
    status: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Case {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("tt-agent-test-{name}"));
        let _ = fs::remove_dir_all(&root);
        let home = root.join("home");
        let marks = root.join("marks");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&marks).unwrap();
        Self { home, marks }
    }

    /// Run `tt agent <args>` with the sandbox in force.
    ///
    /// `env_clear` rather than a couple of overrides: an inherited `TT_MARK_DIR`
    /// from the developer's own shell would silently point a case at the live
    /// marks, which is the one failure mode this harness exists to prevent.
    fn run(&self, args: &[&str]) -> Run {
        let mut argv = vec!["agent"];
        argv.extend_from_slice(args);
        self.run_with(&argv, true)
    }

    /// Run `tt agent <args>` with **no** `TT_MARK_DIR`, so the default location
    /// inside the sandboxed `HOME`'s cache directory is exercised — including the
    /// one-shot migration, which only runs for the default.
    fn run_in_cache(&self, args: &[&str]) -> Run {
        let mut argv = vec!["agent"];
        argv.extend_from_slice(args);
        self.run_with(&argv, false)
    }

    /// Run `tt <args>` with no `agent` prefix, for the one case that needs a
    /// store-taking command to prove the sandbox is where the store would land.
    fn run_bare(&self, args: &[&str]) -> Run {
        self.run_with(args, true)
    }

    fn run_with(&self, args: &[&str], mark_dir: bool) -> Run {
        assert!(
            self.home.starts_with(std::env::temp_dir())
                && self.marks.starts_with(self.home.parent().unwrap()),
            "sandbox paths escaped the scratch directory: {:?}",
            self.home
        );

        let mut command = Command::new(env!("CARGO_BIN_EXE_tt"));
        command.env_clear();
        command.env("HOME", &self.home);
        if mark_dir {
            command.env("TT_MARK_DIR", &self.marks);
        }
        command.args(args);

        let Output {
            status,
            stdout,
            stderr,
        } = command.output().expect("the tt binary should run");
        Run {
            status: status.code(),
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        }
    }

    fn mark_file(&self, key: &str) -> PathBuf {
        self.marks.join(key)
    }

    fn beats_file(&self, key: &str) -> PathBuf {
        self.marks.join("beats").join(key)
    }

    /// Fabricate an open mark started at an absolute epoch.
    fn write_mark(&self, key: &str, start: i64) {
        fs::write(self.mark_file(key), format!("{start}\n")).unwrap();
    }

    /// Regular files directly in the mark directory — the beats subdirectory is
    /// not one, which is the property `list` relies on.
    fn mark_count(&self) -> usize {
        count_files(&self.marks)
    }
}

fn count_files(dir: &Path) -> usize {
    match fs::read_dir(dir) {
        Ok(entries) => entries
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .count(),
        Err(_) => 0,
    }
}

fn count_lines(path: &Path) -> usize {
    fs::read_to_string(path).map_or(0, |body| body.lines().count())
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

impl Run {
    fn assert_status(&self, expected: i32) {
        assert_eq!(
            self.status,
            Some(expected),
            "exit code (stderr: {:?}, stdout: {:?})",
            self.stderr,
            self.stdout
        );
    }

    fn assert_stdout_has(&self, needle: &str) {
        assert!(
            self.stdout.contains(needle),
            "stdout missing {needle:?}: {:?}",
            self.stdout
        );
    }

    fn assert_stderr_has(&self, needle: &str) {
        assert!(
            self.stderr.contains(needle),
            "stderr missing {needle:?}: {:?}",
            self.stderr
        );
    }
}

// --- the store is never touched -------------------------------------------

/// The load-bearing assertion of the whole namespace: `main` dispatches these
/// commands ahead of its `storage::with_data(migrate)` preamble, so a mark
/// command neither creates the store nor takes its lock. An agent heartbeats
/// constantly; a store lock per beat is exactly what `bin/tt-safe` skips its own
/// lock to avoid.
///
/// Asserted on the two *files* rather than on the data directory, because
/// `get_data_path` itself does a `create_dir_all` — so a directory proves nothing
/// while `data.json` and `data.lock` prove everything.
#[test]
fn no_agent_command_creates_the_store_or_takes_its_lock() {
    let case = Case::new("store-untouched");
    let data_dir = case
        .home
        .join("Library/Application Support/com.timetracker.tt");

    case.run(&["begin", "proj", "7", "impl"]).assert_status(0);
    case.run(&["touch", "proj", "7", "impl"]).assert_status(0);
    case.run(&["list"]).assert_status(0);
    case.run(&["cancel", "proj", "7", "impl"]).assert_status(0);

    for name in ["data.json", "data.lock"] {
        let path = data_dir.join(name);
        assert!(
            !path.exists(),
            "{name} was created by a mark-only command: {path:?}"
        );
    }

    // …and the absence above means something: a *store* command in the same
    // sandbox creates both files right there, so this `HOME` really is where the
    // store would have landed.
    case.run_bare(&["list"]).assert_status(0);
    for name in ["data.json", "data.lock"] {
        assert!(
            data_dir.join(name).is_file(),
            "the sandbox is not in effect — {name} landed somewhere else"
        );
    }
}

// --- begin (tt-safe-gaps.sh:187-205) --------------------------------------

/// Ported from "begin creates a mark".
#[test]
fn begin_creates_a_mark() {
    let case = Case::new("begin-creates");
    let run = case.run(&["begin", "proj", "7", "impl"]);
    run.assert_status(0);
    run.assert_stdout_has("marked proj/7 impl");

    let body = fs::read_to_string(case.mark_file("proj.7.impl")).expect("the mark file");
    assert!(
        body.trim().parse::<i64>().is_ok(),
        "a mark holds a unix timestamp, got {body:?}"
    );
}

/// Ported from "begin is idempotent and keeps the original start".
#[test]
fn begin_is_idempotent_and_keeps_the_original_start() {
    let case = Case::new("begin-idempotent");
    let before = now() - 600;
    case.write_mark("proj.7.impl", before);

    let run = case.run(&["begin", "proj", "7", "impl"]);
    run.assert_status(0);
    run.assert_stderr_has("already marked");
    assert_eq!(
        fs::read_to_string(case.mark_file("proj.7.impl")).unwrap(),
        format!("{before}\n"),
        "the original start"
    );
}

// --- touch (tt-safe-gaps.sh:211-238) --------------------------------------

/// Ported from "touch without a mark exits 64".
#[test]
fn touch_without_a_mark_exits_64() {
    let case = Case::new("touch-unmarked");
    let run = case.run(&["touch", "proj", "7", "impl"]);
    run.assert_status(64);
    run.assert_stderr_has("nothing to touch");
    assert_eq!(case.mark_count(), 0, "nothing was written");
}

/// Ported from "touch appends one beat per call".
#[test]
fn touch_appends_one_beat_per_call() {
    let case = Case::new("touch-appends");
    case.write_mark("proj.7.impl", now());
    let before = count_lines(&case.beats_file("proj.7.impl"));

    for _ in 0..3 {
        case.run(&["touch", "proj", "7", "impl"]).assert_status(0);
    }

    assert_eq!(
        count_lines(&case.beats_file("proj.7.impl")),
        before + 3,
        "beat count"
    );
    // The mark itself is untouched — beats are a separate file, not a rewrite.
    assert!(case.mark_file("proj.7.impl").is_file());
}

/// Ported from "beats live in a subdirectory, not as a mark sibling".
#[test]
fn beats_live_in_a_subdirectory_not_as_a_mark_sibling() {
    let case = Case::new("touch-subdirectory");
    case.write_mark("proj.7.impl", now());
    case.run(&["touch", "proj", "7", "impl"]).assert_status(0);

    assert!(!case.mark_file("proj.7.impl.last").exists());
    assert!(!case.mark_file("proj.7.impl.beats").exists());
    assert!(case.beats_file("proj.7.impl").is_file());
}

// --- cancel (tt-safe-gaps.sh:293-319) -------------------------------------

/// Ported from "cancel removes the mark and logs nothing".
#[test]
fn cancel_removes_the_mark_and_leaves_the_others() {
    let case = Case::new("cancel-removes");
    case.write_mark("proj.7.impl", now());
    case.write_mark("other.9.plan", now());
    let before = case.mark_count();

    let run = case.run(&["cancel", "proj", "7", "impl"]);
    run.assert_status(0);
    run.assert_stdout_has("dropped mark for proj/7 impl");
    assert_eq!(case.mark_count(), before - 1, "mark count");
    assert!(!case.mark_file("proj.7.impl").exists());
    assert!(case.mark_file("other.9.plan").is_file());
}

/// Ported from "cancel clears the beats file and any legacy heartbeat".
#[test]
fn cancel_clears_the_beats_file_and_any_legacy_heartbeat() {
    let case = Case::new("cancel-clears-beats");
    case.write_mark("proj.7.impl", now());
    fs::write(case.mark_file("proj.7.impl.last"), format!("{}\n", now())).unwrap();
    case.run(&["touch", "proj", "7", "impl"]).assert_status(0);
    assert!(case.beats_file("proj.7.impl").is_file());

    case.run(&["cancel", "proj", "7", "impl"]).assert_status(0);
    assert!(!case.mark_file("proj.7.impl").exists());
    assert!(!case.mark_file("proj.7.impl.last").exists());
    assert!(!case.beats_file("proj.7.impl").exists());
}

// --- the one-shot migration, end to end -----------------------------------

/// The wrapper's directory is carried across once, by the real binary, at the
/// real default location — with `HOME` sandboxed, so the old directory read here
/// is the sandbox's own and never the live one.
#[test]
fn the_wrappers_marks_are_carried_over_once_and_the_old_directory_stays() {
    let case = Case::new("migration");
    let old = case.home.join(".cache/tt-safe/marks");
    fs::create_dir_all(old.join("beats")).unwrap();
    let start = now() - 15 * 60;
    fs::write(old.join("proj.7.impl"), format!("{start}\n")).unwrap();
    fs::write(old.join("beats/proj.7.impl"), format!("{}\n", now())).unwrap();

    let first = case.run_in_cache(&["list"]);
    first.assert_status(0);
    first.assert_stderr_has("carried 2 mark files over");
    // The migrated mark is open, with its original start.
    first.assert_stdout_has("proj/7 impl");
    first.assert_stdout_has("(0h 15m)");

    let cache = case.home.join("Library/Caches/com.timetracker.tt");
    assert!(cache.join(".marks-migrated").is_file(), "the sentinel");
    // A sibling of the mark directory, so `list` can never read it as a mark.
    assert!(!cache.join("marks/.marks-migrated").exists());
    // The old directory is left completely in place.
    assert!(old.join("proj.7.impl").is_file());

    // Second run: nothing is carried, and a mark cancelled in between is not
    // resurrected from the wrapper's lingering copy.
    case.run_in_cache(&["cancel", "proj", "7", "impl"])
        .assert_status(0);
    let second = case.run_in_cache(&["list"]);
    second.assert_status(0);
    assert!(
        !second.stderr.contains("carried"),
        "migration ran twice: {:?}",
        second.stderr
    );
    second.assert_stdout_has("No open marks.");
}
