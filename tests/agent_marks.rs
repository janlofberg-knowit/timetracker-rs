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

mod common;

use common::{Case, clock, count_lines, now};
use std::fs;

// --- the store is never touched -------------------------------------------

/// The load-bearing assertion of the whole namespace: `main` dispatches the
/// mark-only commands ahead of its `storage::with_data(migrate)` preamble, so
/// none of them creates the store or takes its lock. An agent heartbeats
/// constantly; a store lock per beat is exactly what `bin/tt-safe` skips its own
/// lock to avoid.
///
/// Asserted on the two *files* rather than on the data directory, because
/// `get_data_path` itself does a `create_dir_all` — so a directory proves nothing
/// while `data.json` and `data.lock` prove everything.
///
/// The complement is asserted in the same sandbox and in the same test, because
/// each half is what makes the other mean anything: `tt agent item` **does**
/// create both files, so the absence above is a decision about where these
/// commands dispatch and not an accident of the sandbox.
#[test]
fn only_the_mark_only_agent_commands_leave_the_store_untouched() {
    let case = Case::new("store-untouched");
    let data_dir = case.data_dir();

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

    // …and the absence above means something: an *entry-logging* agent command in
    // the same sandbox creates both files right there, so this `HOME` really is
    // where the store would have landed.
    case.run(&["item", "proj", "7", "impl", "did the thing", "30"])
        .assert_status(0);
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

/// `cancel` clears the mark and its `beats/` entry together.
#[test]
fn cancel_clears_the_mark_and_its_beats_file() {
    let case = Case::new("cancel-clears-beats");
    case.write_mark("proj.7.impl", now());
    case.run(&["touch", "proj", "7", "impl"]).assert_status(0);
    assert!(case.beats_file("proj.7.impl").is_file());

    case.run(&["cancel", "proj", "7", "impl"]).assert_status(0);
    assert!(!case.mark_file("proj.7.impl").exists());
    assert!(!case.beats_file("proj.7.impl").exists());
}

// --- list (tt-safe-gaps.sh:321-398, `marks`) -------------------------------
//
// Three of these read the **house-style** row from #64 rather than the wrapper's
// terse `%-*s  %s  (%s)`: the port's one ruled divergence (owner, 2026-08-18).
// "nothing when the directory is empty" needs no change at all, because
// `No open marks.` happens to be identical in both.

/// Ported from "marks reports nothing when the directory is empty".
#[test]
fn list_reports_nothing_when_the_directory_is_empty() {
    let case = Case::new("list-empty");
    let run = case.run(&["list"]);
    run.assert_status(0);
    run.assert_stdout_has("No open marks.");
    // The bare line, with no header and no emoji — as `tt list` says
    // `No entries yet.`
    assert_eq!(run.stdout, "No open marks.\n");
}

/// Ported from "marks lists an open mark in the TUI's row format", reading the
/// house-style row instead.
#[test]
fn list_shows_an_open_mark_as_a_house_style_row() {
    let case = Case::new("list-row");
    let start = now() - 600;
    case.write_mark("proj.23.impl", start);

    let run = case.run(&["list"]);
    run.assert_status(0);
    assert_eq!(
        run.stdout,
        format!(
            "\u{1F916} Open marks:\n\n  proj/23 impl       - since {} (0h 10m)\n",
            clock(start)
        ),
        "the header, the blank line and one padded row"
    );
}

/// Ported from "marks drops the issue for the - sentinel".
#[test]
fn list_drops_the_issue_for_the_sentinel() {
    let case = Case::new("list-sentinel");
    case.write_mark("proj.-.plan", now());

    let run = case.run(&["list"]);
    run.assert_status(0);
    run.assert_stdout_has("proj plan");
    assert!(
        !run.stdout.contains("proj/-"),
        "the - sentinel leaked into the label: {:?}",
        run.stdout
    );
}

/// Ported from "marks renders elapsed as the TUI does", reading the house
/// duration instead: always `{h}h {m}m`, and never negative.
#[test]
fn list_renders_an_age_in_the_house_duration_format() {
    let case = Case::new("list-ages");
    case.write_mark("long.1.impl", now() - 126 * 60);
    case.write_mark("short.2.impl", now() - 2 * 60);
    case.write_mark("future.3.impl", now() + 600);

    let run = case.run(&["list"]);
    run.assert_status(0);
    run.assert_stdout_has("(2h 6m)");
    run.assert_stdout_has("(0h 2m)");
    // A start in the future reads as `0h 0m`, not as `0h -10m`.
    run.assert_stdout_has("(0h 0m)");
    // No age is ever negative — `duration::format` on an unclamped negative span
    // would print `0h -10m`. (` - since ` is the row separator, so the check is on
    // the parenthesised age itself.)
    for line in run.stdout.lines().filter(|line| line.contains('(')) {
        let age = &line[line.find('(').unwrap()..];
        assert!(!age.contains('-'), "a negative age reached {age:?}");
    }
}

/// Ported from "marks still lists a mark whose phase contains a dot".
#[test]
fn list_still_shows_a_mark_whose_phase_contains_a_dot() {
    let case = Case::new("list-dotted-phase");
    case.write_mark("proj.23.impl.v2", now());

    let run = case.run(&["list"]);
    run.assert_status(0);
    // The first/last-dot split is lossy by design: extra dots land in the middle
    // field. Listing an imperfect label beats hiding an open mark.
    run.assert_stdout_has("proj/23.impl v2");
}

/// Ported from "marks lists a dotless mark name as a bare project".
#[test]
fn list_shows_a_dotless_name_as_a_bare_project() {
    let case = Case::new("list-dotless");
    case.write_mark("solo", now());

    let run = case.run(&["list"]);
    run.assert_status(0);
    run.assert_stdout_has("solo");
}

/// Ported from "marks lists a mark whose phase is literally 'last' (issue #16)".
#[test]
fn list_shows_a_mark_whose_phase_is_literally_last() {
    let case = Case::new("list-phase-last");
    case.run(&["begin", "proj", "-", "last"]).assert_status(0);
    assert!(case.mark_file("proj.-.last").is_file());

    let run = case.run(&["list"]);
    run.assert_status(0);
    // A name filter would hide this; the reader has none.
    run.assert_stdout_has("proj last");
}

/// Ported from "marks ignores the beats subdirectory".
#[test]
fn list_ignores_the_beats_subdirectory() {
    let case = Case::new("list-ignores-beats");
    case.write_mark("proj.7.impl", now());
    case.run(&["touch", "proj", "7", "impl"]).assert_status(0);

    let run = case.run(&["list"]);
    run.assert_status(0);
    run.assert_stdout_has("proj/7 impl");
    assert!(
        !run.stdout.contains("beats"),
        "the beats directory was listed as a mark: {:?}",
        run.stdout
    );
    // One row per regular file in the mark directory, and the directory is not
    // one — a file-type test, never a name filter (#16).
    let rows = run
        .stdout
        .lines()
        .filter(|line| line.starts_with("  "))
        .count();
    assert_eq!(rows, case.mark_count(), "one row per mark file");
}
