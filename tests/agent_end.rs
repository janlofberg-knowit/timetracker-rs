//! Integration tests for the entry-logging half of `tt agent` — `item` and `end`.
//!
//! `tt-safe`'s own oracle (`/Users/jnao/Code/tt/tests/tt-safe-gaps.sh`) ported
//! onto `tt agent end`, case by case and under each case's own name, plus the
//! `item` cases the oracle never had (nothing in it exercises `item` directly).
//!
//! The assertion medium is the whole adaptation. Where the shell inspected a
//! recording `tt` stub's argv (`--time=`, `--description=`, `--idle=`, `--trim`),
//! there is no argv in process, so the equivalent observables are `cli::log`'s own
//! stdout line and the sandbox's `data.json`. Every expectation is still derived
//! from the fixture rather than written down.
//!
//! Sandboxing is [`common`]'s and is asserted before the binary runs: a throwaway
//! `HOME` *and* `TT_MARK_DIR`, so the live store, the live lock and the wrapper's
//! `~/.cache/tt-safe/marks` are never touched.

mod common;

use common::{Case, logged_duration, now};

// --- item ------------------------------------------------------------------

/// The convention's three tags reach the entry, and the project is a field.
#[test]
fn item_logs_the_three_convention_tags() {
    let case = Case::new("item-tags");
    let run = case.run(&[
        "item",
        "loremind",
        "77",
        "impl",
        "store/links boundary",
        "43",
    ]);
    run.assert_status(0);

    let store = case.store();
    assert_eq!(store.entries.len(), 1, "one entry");
    let entry = &store.entries[0];
    assert_eq!(entry.description, "store/links boundary");
    assert_eq!(entry.project.as_deref(), Some("loremind"));
    assert_eq!(entry.tags, ["loremind/77", "impl", "agent"]);
}

/// The `-` sentinel drops the item tag, and no bare project tag appears.
#[test]
fn item_drops_the_item_tag_for_the_sentinel_issue() {
    let case = Case::new("item-sentinel");
    case.run(&["item", "loremind", "-", "plan", "sketched the shape", "20"])
        .assert_status(0);

    let entry = &case.store().entries[0];
    assert_eq!(entry.tags, ["plan", "agent"]);
    // The project is a real field with its own axis — never a tag.
    assert!(
        !entry.tags.iter().any(|tag| tag == "loremind"),
        "a bare project tag was emitted: {:?}",
        entry.tags
    );
}

/// 15-minute rounding, and the floor under it.
#[test]
fn item_rounds_the_minutes_to_a_quarter_hour() {
    let case = Case::new("item-rounding");
    let run = case.run(&["item", "proj", "7", "impl", "did the thing", "43"]);
    run.assert_status(0);
    run.assert_stdout_has(&logged_duration(43));

    let floored = Case::new("item-rounding-floor");
    let run = floored.run(&["item", "proj", "7", "impl", "a quick errand", "2"]);
    run.assert_status(0);
    run.assert_stdout_has(&logged_duration(2));
}

/// A summary that merely mentions an issue number does not become a tag (#11).
#[test]
fn item_strips_a_stray_hash_from_the_summary() {
    let case = Case::new("item-stray-hash");
    case.run(&[
        "item",
        "proj",
        "7",
        "impl",
        "closed #12 with the C# bridge",
        "30",
    ])
    .assert_status(0);

    let entry = &case.store().entries[0];
    assert_eq!(entry.description, "closed 12 with the C# bridge");
    assert_eq!(entry.tags, ["proj/7", "impl", "agent"]);
}

/// `item` is a one-shot log: it reads and writes no mark file at all.
#[test]
fn item_creates_no_mark_file() {
    let case = Case::new("item-no-mark");
    case.run(&["item", "proj", "7", "impl", "did the thing", "30"])
        .assert_status(0);
    assert_eq!(case.mark_count(), 0, "no mark was written");
    assert!(!case.beats_file("proj.7.impl").exists());
}

/// The wrapper's message and its exit code, not clap's 2 and its usage block.
#[test]
fn item_with_non_numeric_minutes_exits_64() {
    let case = Case::new("item-bad-minutes");
    let run = case.run(&["item", "proj", "7", "impl", "did the thing", "half an hour"]);
    run.assert_status(64);
    run.assert_stderr_has("minutes must be a whole number, got 'half an hour'");
    // The migration preamble has already created the store by the time the
    // handler rejects the argument, so the assertion is on the entries.
    assert!(
        case.store().entries.is_empty(),
        "a rejected item logged something"
    );
}

/// A missing minutes argument is the wrapper's usage error, also 64.
#[test]
fn item_without_minutes_exits_64() {
    let case = Case::new("item-no-minutes");
    let run = case.run(&["item", "proj", "7", "impl", "did the thing"]);
    run.assert_status(64);
    run.assert_stderr_has("usage: tt agent item");
}

/// The mark commands are unaffected by `item` living beside them: a phase marked
/// before an `item` call is still open afterwards.
#[test]
fn item_leaves_an_open_mark_alone() {
    let case = Case::new("item-leaves-marks");
    case.write_mark("proj.7.impl", now() - 600);
    case.run(&["item", "proj", "7", "impl", "something else entirely", "30"])
        .assert_status(0);
    assert!(case.mark_file("proj.7.impl").is_file(), "the mark survived");
}
