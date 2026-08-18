//! The shared harness for the `tt agent` integration tests.
//!
//! `tests/agent_marks.rs` and `tests/agent_end.rs` are separate test binaries, so
//! the sandbox, the fixtures and the assertions live here rather than being
//! duplicated — and a `tests/` **subdirectory** is not compiled as a target of its
//! own, which is why this is `common/mod.rs` and not `common.rs`.
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
//!
//! Each test binary compiles this module separately, so an item only one of them
//! uses is `dead_code` in the other — hence the crate-level allow, which is not a
//! judgement about any particular helper.
#![allow(dead_code)]

use chrono::{DateTime, Local};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

/// One case's sandbox: a `HOME`, a mark directory inside it, and nothing else.
pub struct Case {
    pub home: PathBuf,
    pub marks: PathBuf,
}

/// What one `tt agent` invocation produced.
pub struct Run {
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Case {
    pub fn new(name: &str) -> Self {
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
    pub fn run(&self, args: &[&str]) -> Run {
        let mut argv = vec!["agent"];
        argv.extend_from_slice(args);
        self.run_with(&argv, true)
    }

    /// Run `tt agent <args>` with **no** `TT_MARK_DIR`, so the default location
    /// inside the sandboxed `HOME`'s cache directory is exercised — including the
    /// one-shot migration, which only runs for the default.
    pub fn run_in_cache(&self, args: &[&str]) -> Run {
        let mut argv = vec!["agent"];
        argv.extend_from_slice(args);
        self.run_with(&argv, false)
    }

    /// Run `tt agent <args>` with one extra `KEY=value` in the environment, which
    /// `env_clear` would otherwise strip. The oracle's own `EXTRA_ENV`.
    pub fn run_with_env(&self, args: &[&str], env: &[(&str, &str)]) -> Run {
        let mut argv = vec!["agent"];
        argv.extend_from_slice(args);
        self.run_full(&argv, true, env)
    }

    fn run_with(&self, args: &[&str], mark_dir: bool) -> Run {
        self.run_full(args, mark_dir, &[])
    }

    fn run_full(&self, args: &[&str], mark_dir: bool, env: &[(&str, &str)]) -> Run {
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
        for (key, value) in env {
            command.env(key, value);
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

    pub fn mark_file(&self, key: &str) -> PathBuf {
        self.marks.join(key)
    }

    pub fn beats_file(&self, key: &str) -> PathBuf {
        self.marks.join("beats").join(key)
    }

    /// Fabricate an open mark started at an absolute epoch.
    pub fn write_mark(&self, key: &str, start: i64) {
        fs::write(self.mark_file(key), format!("{start}\n")).unwrap();
    }

    /// Run `tt <args>` with **no** `agent` prefix — the store-reading commands.
    ///
    /// [`Case::run`] prepends `agent` to every argv because that is what the mark
    /// tests need; `tt report` is not an agent subcommand, so it needs the bare
    /// form.
    pub fn run_bare(&self, args: &[&str]) -> Run {
        self.run_with(args, true)
    }

    /// Lay a `data.json` into the sandbox from fabricated rows.
    ///
    /// `schema_version: 1` on purpose: `main`'s migrate preamble early-returns at
    /// `>= 1`, so the fabricated `project` fields survive instead of being
    /// overwritten by the tag-inference migration this Story exists to replace.
    pub fn write_store(&self, rows: &[StoreRow]) {
        let entries: Vec<String> = rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let project = match row.project {
                    Some(name) => format!("\"{name}\""),
                    None => "null".to_string(),
                };
                let tags: Vec<String> =
                    row.tags.iter().map(|tag| format!("\"{tag}\"")).collect();
                let end = match row.end {
                    Some(end) => format!("\"{}\"", stamp(end)),
                    None => "null".to_string(),
                };
                format!(
                    r#"{{"id":{},"description":"{}","project":{},"tags":[{}],"start_time":"{}","end_time":{},"idle":[]}}"#,
                    i as u64 + 1,
                    row.description,
                    project,
                    tags.join(","),
                    stamp(row.start),
                    end
                )
            })
            .collect();

        let dir = self.data_dir();
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("data.json"),
            format!(
                r#"{{"entries":[{}],"next_id":{},"schema_version":1}}"#,
                entries.join(","),
                rows.len() as u64 + 1
            ),
        )
        .unwrap();
    }

    /// The sandbox's own store directory — where `data.json` and `data.lock` land
    /// if anything at all reaches the store.
    pub fn data_dir(&self) -> PathBuf {
        self.home
            .join("Library/Application Support/com.timetracker.tt")
    }

    /// The sandbox's store, parsed.
    ///
    /// An integration test cannot `use` a binary crate's modules, so the store is
    /// read back through its own JSON rather than through `storage::load_data` —
    /// the same file, one deserialiser further out.
    pub fn store(&self) -> Store {
        let path = self.data_dir().join("data.json");
        let body = fs::read_to_string(path).expect("the sandbox's data.json");
        serde_json::from_str(&body).expect("data.json should parse")
    }

    /// Fabricate a heartbeat sequence at absolute epochs — the oracle's
    /// `beats_at`. Written directly rather than beaten out in real time, so a
    /// four-hour phase costs a test nothing.
    pub fn beats_at(&self, key: &str, beats: &[i64]) {
        let file = self.beats_file(key);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let body: String = beats.iter().map(|beat| format!("{beat}\n")).collect();
        fs::write(file, body).unwrap();
    }

    /// Fabricate the legacy pre-`beats/` single heartbeat, `<mark>.last`. Nothing
    /// writes this any more; `end` still reads it (#55) and `cancel` clears it.
    pub fn write_legacy_beat(&self, key: &str, beat: i64) {
        fs::write(self.mark_file(&format!("{key}.last")), format!("{beat}\n")).unwrap();
    }

    /// Regular files directly in the mark directory — the beats subdirectory is
    /// not one, which is the property `list` relies on.
    pub fn mark_count(&self) -> usize {
        count_files(&self.marks)
    }
}

pub fn count_files(dir: &Path) -> usize {
    match fs::read_dir(dir) {
        Ok(entries) => entries
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .count(),
        Err(_) => 0,
    }
}

pub fn count_lines(path: &Path) -> usize {
    fs::read_to_string(path).map_or(0, |body| body.lines().count())
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// The `HH:MM` a fixture's own epoch renders as, so a row expectation is derived
/// from the fixture rather than from a clock read by hand.
pub fn clock(epoch: i64) -> String {
    chrono::DateTime::from_timestamp(epoch, 0)
        .unwrap()
        .with_timezone(&chrono::Local)
        .format("%H:%M")
        .to_string()
}

/// One fabricated store row for [`Case::write_store`].
pub struct StoreRow {
    pub description: &'static str,
    pub project: Option<&'static str>,
    pub tags: &'static [&'static str],
    /// Epoch seconds.
    pub start: i64,
    /// Epoch seconds; `None` leaves the entry running.
    pub end: Option<i64>,
}

/// An epoch second as the store's RFC 3339 local timestamp.
fn stamp(epoch: i64) -> String {
    DateTime::<chrono::Utc>::from_timestamp(epoch, 0)
        .expect("a real epoch")
        .with_timezone(&Local)
        .to_rfc3339()
}

/// The store as `tt` writes it, with only the fields these tests assert on.
#[derive(Debug, serde::Deserialize)]
pub struct Store {
    pub entries: Vec<Entry>,
}

/// One logged entry.
#[derive(Debug, serde::Deserialize)]
pub struct Entry {
    pub description: String,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub start_time: DateTime<Local>,
    pub end_time: Option<DateTime<Local>>,
    #[serde(default)]
    pub idle: Vec<Idle>,
}

impl Entry {
    /// How long this entry covers, in whole seconds.
    pub fn seconds(&self) -> i64 {
        self.end_time
            .expect("a logged entry is closed")
            .signed_duration_since(self.start_time)
            .num_seconds()
    }
}

/// One recorded silent stretch, compared against the fixture's own epochs.
#[derive(Debug, serde::Deserialize)]
pub struct Idle {
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
}

impl Idle {
    /// The epoch pair the wrapper would have passed as `--idle=<from>-<to>`.
    pub fn epochs(&self) -> (i64, i64) {
        (self.start.timestamp(), self.end.timestamp())
    }
}

/// `tt-safe`'s own rounding, mirrored so an expectation can be derived from the
/// fixture's timestamps instead of restating a literal.
pub fn round_quarter(minutes: i64) -> i64 {
    (((minutes + 7) / 15) * 15).max(15)
}

/// The `- Duration: <h>h <m>m` tail `cli::log` prints for `minutes`, which is what
/// replaces the shell oracle's `--time=` argv assertion.
pub fn logged_duration(minutes: i64) -> String {
    let rounded = round_quarter(minutes);
    format!("- Duration: {}h {}m", rounded / 60, rounded % 60)
}

impl Run {
    pub fn assert_status(&self, expected: i32) {
        assert_eq!(
            self.status,
            Some(expected),
            "exit code (stderr: {:?}, stdout: {:?})",
            self.stderr,
            self.stdout
        );
    }

    pub fn assert_stdout_has(&self, needle: &str) {
        assert!(
            self.stdout.contains(needle),
            "stdout missing {needle:?}: {:?}",
            self.stdout
        );
    }

    /// The minutes `cli::log` said it logged, read back off its own output.
    ///
    /// The in-process stand-in for the oracle's `logged_minutes`, which read
    /// `--time=<n>m` off the stub's argv: `log` prints the duration it was given,
    /// **before** any split, which is exactly what the shell's `--time=` was. Lets
    /// two runs be compared without either figure being written down.
    pub fn logged_minutes(&self) -> i64 {
        let tail = self
            .stdout
            .split("- Duration: ")
            .nth(1)
            .unwrap_or_else(|| panic!("no logged duration in {:?}", self.stdout));
        let (hours, rest) = tail.split_once('h').expect("`<h>h <m>m`");
        let (minutes, _) = rest.trim_start().split_once('m').expect("`<h>h <m>m`");
        hours.trim().parse::<i64>().unwrap() * 60 + minutes.trim().parse::<i64>().unwrap()
    }

    pub fn assert_stderr_has(&self, needle: &str) {
        assert!(
            self.stderr.contains(needle),
            "stderr missing {needle:?}: {:?}",
            self.stderr
        );
    }
}
