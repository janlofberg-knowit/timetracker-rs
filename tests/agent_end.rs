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

use common::{Case, clock, logged_duration, now};

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

// --- end (tt-safe-gaps.sh:244-291) -----------------------------------------

/// Ported from "end derives minutes from the mark".
#[test]
fn end_derives_minutes_from_the_mark() {
    let case = Case::new("end-derives");
    let elapsed = 1800;
    case.write_mark("proj.7.impl", now() - elapsed);

    let run = case.run(&["end", "proj", "7", "impl", "did the thing"]);
    run.assert_status(0);
    // The wrapper asserted `--time=`, `--project=` and `--description=` on a stub's
    // argv; in process the one observable is the line `cli::log` prints.
    run.assert_stdout_has(&format!(
        "\"did the thing\" (proj) [#proj/7 #impl #agent] {}",
        logged_duration(elapsed / 60)
    ));
    assert!(!case.mark_file("proj.7.impl").exists());
}

/// Ported from "a successful end clears the mark and its beats".
#[test]
fn a_successful_end_clears_the_mark_and_its_beats() {
    let case = Case::new("end-clears");
    case.write_mark("proj.7.impl", now());
    case.run(&["touch", "proj", "7", "impl"]).assert_status(0);
    assert!(case.beats_file("proj.7.impl").is_file());

    case.run(&["end", "proj", "7", "impl", "did the thing"])
        .assert_status(0);
    assert!(!case.mark_file("proj.7.impl").exists());
    assert!(!case.beats_file("proj.7.impl").exists());
}

/// Ported from "an explicit trailing minutes argument overrides the mark".
#[test]
fn an_explicit_trailing_minutes_argument_overrides_the_mark() {
    let case = Case::new("end-explicit");
    case.write_mark("proj.7.impl", now() - 1800);

    let run = case.run(&["end", "proj", "7", "impl", "did the thing", "90"]);
    run.assert_status(0);
    // 90, not the mark's 30: the argument skips the timestamps entirely.
    run.assert_stdout_has(&logged_duration(90));
}

/// Ported from "end without a mark exits 64".
#[test]
fn end_without_a_mark_exits_64() {
    let case = Case::new("end-unmarked");
    let run = case.run(&["end", "proj", "7", "impl", "did the thing"]);
    run.assert_status(64);
    run.assert_stderr_has("no mark for proj/7 impl");
    assert!(case.store().entries.is_empty(), "nothing was logged");
}

/// Ported from "end without a summary exits 64".
///
/// 64 and not clap's 2, which is why `summary` is an `Option<String>` checked by
/// hand rather than a required positional.
#[test]
fn end_without_a_summary_exits_64() {
    let case = Case::new("end-no-summary");
    case.write_mark("proj.7.impl", now());

    let run = case.run(&["end", "proj", "7", "impl"]);
    run.assert_status(64);
    run.assert_stderr_has("need a summary");
    assert!(case.store().entries.is_empty(), "nothing was logged");
    assert!(case.mark_file("proj.7.impl").is_file(), "the mark survived");
}

// --- gaps (tt-safe-gaps.sh:400-514) ----------------------------------------

/// Ported from "steady beats log the full span however long it ran".
#[test]
fn steady_beats_log_the_full_span_however_long_it_ran() {
    let case = Case::new("gaps-steady");
    let step = 600;
    // Four hours, beaten every ten minutes.
    let start = now() - step * 24;
    let beats: Vec<i64> = (1..=24).map(|i| start + i * step).collect();
    case.write_mark("proj.7.impl", start);
    case.beats_at("proj.7.impl", &beats);

    let run = case.run(&["end", "proj", "7", "impl", "long active session"]);
    run.assert_status(0);
    // Measured to the last beat, which the fixture puts exactly at the span's end.
    let last_beat = *beats.last().unwrap();
    run.assert_stdout_has(&logged_duration((last_beat - start) / 60));
}

/// The oracle's `gap_fixture`: a 110-minute phase with an 80-minute hole in the
/// middle of it, reused by the cases below so none of them restates a number.
struct GapFixture {
    hole: (i64, i64),
    /// The measured span, unrounded.
    minutes: i64,
    /// The hole, unrounded.
    hole_minutes: i64,
}

impl GapFixture {
    /// The measured span as it is actually logged, through the same floor-15
    /// rounding `end` applies.
    fn minutes_rounded(&self) -> i64 {
        common::round_quarter(self.minutes)
    }
}

fn gap_fixture(case: &Case) -> GapFixture {
    let start = now() - 110 * 60;
    let beats = [
        start + 10 * 60,
        // The hole starts here…
        start + 30 * 60,
        // …and ends here, 80 minutes later.
        start + 110 * 60,
    ];
    case.write_mark("proj.12.plan", start);
    case.beats_at("proj.12.plan", &beats);
    GapFixture {
        hole: (beats[1], beats[2]),
        minutes: (beats[2] - start) / 60,
        hole_minutes: (beats[2] - beats[1]) / 60,
    }
}

/// Ported from "a single over-threshold hole is refused and named".
#[test]
fn a_single_over_threshold_hole_is_refused_and_named() {
    let case = Case::new("gaps-refused");
    let fixture = gap_fixture(&case);

    let run = case.run(&["end", "proj", "12", "plan", "planned the thing"]);
    run.assert_status(65);
    run.assert_stderr_has(&format!(
        "proj/12 plan has an {}m gap",
        fixture.hole_minutes
    ));
    run.assert_stderr_has(&format!(
        "({}-{})",
        clock(fixture.hole.0),
        clock(fixture.hole.1)
    ));
    // Both answers on one line so neither is privileged, both **unrounded**, and
    // `--trim`'s figure the span minus the hole rather than a number restated here.
    run.assert_stderr_has(&format!(
        "--full logs {}m, --trim logs {}m",
        fixture.minutes,
        fixture.minutes - fixture.hole_minutes
    ));
    run.assert_stderr_has("or pass the real minutes instead.");
    assert!(case.store().entries.is_empty(), "nothing was logged");
    // A refusal must leave the mark and its beats intact so the phase can be
    // closed once the human call is made.
    assert!(case.mark_file("proj.12.plan").is_file());
    assert!(case.beats_file("proj.12.plan").is_file());
}

/// Ported from "--full accepts a flagged span".
#[test]
fn full_accepts_a_flagged_span() {
    let case = Case::new("gaps-full");
    let fixture = gap_fixture(&case);

    let run = case.run(&["end", "proj", "12", "plan", "planned the thing", "--full"]);
    run.assert_status(0);
    run.assert_stdout_has(&logged_duration(fixture.minutes));
    assert!(!case.mark_file("proj.12.plan").exists());
    assert!(!case.beats_file("proj.12.plan").exists());
}

/// Ported from "explicit minutes still win over a flagged span".
#[test]
fn explicit_minutes_still_win_over_a_flagged_span() {
    let case = Case::new("gaps-explicit");
    gap_fixture(&case);

    let run = case.run(&["end", "proj", "12", "plan", "planned the thing", "30"]);
    run.assert_status(0);
    run.assert_stdout_has(&logged_duration(30));
}

/// Ported from "silence before the first beat is a gap too".
#[test]
fn silence_before_the_first_beat_is_a_gap_too() {
    let case = Case::new("gaps-leading");
    let start = now() - 120 * 60;
    let first = start + 110 * 60;
    case.write_mark("proj.7.impl", start);
    case.beats_at("proj.7.impl", &[first, first + 300]);

    let run = case.run(&["end", "proj", "7", "impl", "late first beat"]);
    run.assert_status(65);
    run.assert_stderr_has(&format!("gap ({}-{})", clock(start), clock(first)));
}

/// Ported from "the threshold is configurable".
#[test]
fn the_threshold_is_configurable() {
    let case = Case::new("gaps-threshold");
    let span = 30 * 60;
    let start = now() - span;
    case.write_mark("proj.7.impl", start);
    case.beats_at("proj.7.impl", &[start + span]);

    // Passed explicitly, since the harness clears the environment.
    let run = case.run_with_env(
        &[
            "end",
            "proj",
            "7",
            "impl",
            "half an hour, no beats inside it",
        ],
        &[("TT_MAX_GAP_MINUTES", "10")],
    );
    run.assert_status(65);
    run.assert_stderr_has(&format!("{}m gap", span / 60));
}

/// Ported from "a legacy .last heartbeat is read as a single beat".
#[test]
fn a_legacy_last_heartbeat_is_read_as_a_single_beat() {
    let case = Case::new("gaps-legacy");
    let start = now() - 40 * 60;
    let last = start + 10 * 60;
    case.write_mark("proj.7.impl", start);
    case.write_legacy_beat("proj.7.impl", last);

    let run = case.run(&["end", "proj", "7", "impl", "upgraded mid-phase"]);
    run.assert_status(0);
    // Measured start → the legacy heartbeat, not start → now.
    run.assert_stdout_has(&logged_duration((last - start) / 60));
    assert!(!case.mark_file("proj.7.impl.last").exists());
}

/// Ported from "a beats file supersedes a stale legacy .last".
#[test]
fn a_beats_file_supersedes_a_stale_legacy_last() {
    let case = Case::new("gaps-beats-win");
    let start = now() - 40 * 60;
    case.write_mark("proj.7.impl", start);
    case.write_legacy_beat("proj.7.impl", start + 5 * 60);
    let last = start + 35 * 60;
    case.beats_at("proj.7.impl", &[start + 20 * 60, last]);

    let run = case.run(&["end", "proj", "7", "impl", "beats win"]);
    run.assert_status(0);
    run.assert_stdout_has(&logged_duration((last - start) / 60));
}

/// Ported from "an unvouched span over the threshold is flagged".
///
/// A mark with no heartbeats at all is judged as one silence across its whole
/// span, which is the honest reading of a phase that produced no evidence.
#[test]
fn an_unvouched_span_over_the_threshold_is_flagged() {
    let case = Case::new("gaps-unvouched");
    let start = now() - 90 * 60;
    case.write_mark("proj.7.impl", start);

    let run = case.run(&["end", "proj", "7", "impl", "no evidence either way"]);
    run.assert_status(65);
    run.assert_stderr_has(&format!("gap ({}-", clock(start)));
    assert!(case.store().entries.is_empty(), "nothing was logged");
}

// --- idle and trim (tt-safe-gaps.sh:534-627) -------------------------------
//
// These are the cases the shell asserted on `--idle=` / `--trim` argv, which has
// no in-process counterpart: `cli::log` takes the intervals as values and does the
// split itself, inside the same store transaction. The epoch pair is what must
// match, so these read the sandbox's `data.json` instead.

/// The oracle's `two_gap_fixture`: two holes in one phase, so the *order* of the
/// recorded intervals has something to prove.
struct TwoGapFixture {
    holes: [(i64, i64); 2],
}

fn two_gap_fixture(case: &Case) -> TwoGapFixture {
    let start = now() - 150 * 60;
    let beats = [
        // The first hole starts here…
        start + 10 * 60,
        // …and ends here, 60 minutes later.
        start + 70 * 60,
        // The second starts here…
        start + 80 * 60,
        // …and ends here, another 60 later.
        start + 140 * 60,
    ];
    case.write_mark("proj.12.plan", start);
    case.beats_at("proj.12.plan", &beats);
    TwoGapFixture {
        holes: [(beats[0], beats[1]), (beats[2], beats[3])],
    }
}

/// Ported from "every flagged gap becomes an --idle argument, in chronological
/// order".
#[test]
fn every_flagged_gap_becomes_an_idle_interval_in_chronological_order() {
    let case = Case::new("idle-order");
    let fixture = two_gap_fixture(&case);

    case.run(&["end", "proj", "12", "plan", "planned the thing", "--full"])
        .assert_status(0);

    let entries = case.store().entries;
    assert_eq!(entries.len(), 1, "--full never splits");
    // One interval per fabricated hole — the count comes from the fixture, not
    // from a number written here.
    let recorded: Vec<(i64, i64)> = entries[0].idle.iter().map(|gap| gap.epochs()).collect();
    assert_eq!(recorded, fixture.holes, "the fixture's holes, in order");
}

/// Ported from "a phase with no flagged gap passes no --idle at all".
#[test]
fn a_phase_with_no_flagged_gap_records_none_and_no_trim() {
    let case = Case::new("idle-none");
    let step = 600;
    let start = now() - step * 24;
    let beats: Vec<i64> = (1..=24).map(|i| start + i * step).collect();
    case.write_mark("proj.7.impl", start);
    case.beats_at("proj.7.impl", &beats);

    case.run(&["end", "proj", "7", "impl", "long active session"])
        .assert_status(0);

    let entries = case.store().entries;
    assert_eq!(entries.len(), 1, "nothing was split");
    assert!(entries[0].idle.is_empty(), "no silence to record");
}

/// Ported from "--trim adds the flag and --full does not".
///
/// The shell asserted the absence of a `--trim` argument; in process the split is
/// a call, so what proves it did not happen is the entry still standing whole with
/// its interval on it.
#[test]
fn trim_adds_the_split_and_full_does_not() {
    let case = Case::new("idle-full-no-trim");
    let fixture = gap_fixture(&case);

    let run = case.run(&["end", "proj", "12", "plan", "planned the thing", "--full"]);
    run.assert_status(0);

    let entries = case.store().entries;
    assert_eq!(entries.len(), 1, "--full asks for no split");
    assert_eq!(
        entries[0]
            .idle
            .iter()
            .map(|gap| gap.epochs())
            .collect::<Vec<_>>(),
        vec![fixture.hole],
        "the silence is recorded, not removed"
    );
    assert_eq!(entries[0].seconds(), fixture.minutes_rounded() * 60);
}

/// Ported from "--trim asks tt to trim, and still records the interval".
///
/// The interval *is* passed — `trim: true` never travels with an empty idle
/// vector — and `split_at_idle` then consumes it: the logged span is already the
/// span minus every flagged gap, so the recorded interval covers what is left and
/// the split leaves nothing of it behind. That is exactly what the wrapper's
/// `--time=<trimmed> --idle=… --trim` argv produces against the real `tt`, so it
/// is parity rather than a port artefact; see the report on #55.
#[test]
fn trim_trims_and_still_records_the_interval() {
    let case = Case::new("idle-trim");
    let fixture = gap_fixture(&case);

    let run = case.run(&["end", "proj", "12", "plan", "planned the thing", "--trim"]);
    run.assert_status(0);
    // The span minus the hole, where `--full` on the same fixture logs the span.
    assert_eq!(
        run.logged_minutes(),
        common::round_quarter(fixture.minutes - fixture.hole_minutes)
    );
    // The trim was asked for and acted on, which is what tells this run apart from
    // the `--full` one above: no interval is left standing on the entry.
    let entries = case.store().entries;
    assert!(
        entries.iter().all(|entry| entry.idle.is_empty()),
        "the split did not consume the interval: {entries:?}"
    );
    assert!(!case.mark_file("proj.12.plan").exists());
    assert!(!case.beats_file("proj.12.plan").exists());
}

/// Ported from "explicit minutes beat --trim as well, and record nothing".
///
/// Stronger than the shell's form, which could only pass one positional: here the
/// flag and the override are given *together*, and the override still wins.
#[test]
fn explicit_minutes_beat_trim_as_well_and_record_nothing() {
    let case = Case::new("idle-explicit");
    gap_fixture(&case);

    let run = case.run(&[
        "end",
        "proj",
        "12",
        "plan",
        "planned the thing",
        "30",
        "--trim",
    ]);
    run.assert_status(0);
    assert_eq!(run.logged_minutes(), common::round_quarter(30));

    let entries = case.store().entries;
    assert_eq!(entries.len(), 1, "nothing was split");
    // The mark's timestamps were skipped entirely, so there was no silence to find.
    assert!(entries[0].idle.is_empty(), "an idle interval was recorded");
}

/// The oracle's `aligned_gap_fixture`: quarter-aligned on purpose, so the `--full`
/// and `--trim` runs below differ by the hole and not by a rounding artefact.
struct AlignedGapFixture {
    hole_minutes: i64,
}

fn aligned_gap_fixture(case: &Case) -> AlignedGapFixture {
    let start = now() - 120 * 60;
    let beats = [start + 30 * 60, start + 120 * 60];
    case.write_mark("proj.12.plan", start);
    case.beats_at("proj.12.plan", &beats);
    AlignedGapFixture {
        hole_minutes: (beats[1] - beats[0]) / 60,
    }
}

/// Ported from "--full logs the whole measured span".
#[test]
fn full_logs_the_whole_measured_span() {
    let case = Case::new("aligned-full");
    aligned_gap_fixture(&case);

    let run = case.run(&["end", "proj", "12", "plan", "planned the thing", "--full"]);
    run.assert_status(0);
    assert!(run.logged_minutes() > 0, "no duration was logged");
}

/// Ported from "--trim logs the span minus every flagged gap".
///
/// Two runs of the same fixture in two sandboxes, compared as a **delta**, so
/// neither figure is written down anywhere — the shell carried `full_logged`
/// across two cases to do the same thing.
#[test]
fn trim_logs_the_span_minus_every_flagged_gap() {
    let full_case = Case::new("aligned-full-delta");
    let fixture = aligned_gap_fixture(&full_case);
    let full = full_case.run(&["end", "proj", "12", "plan", "planned the thing", "--full"]);
    full.assert_status(0);

    let trim_case = Case::new("aligned-trim-delta");
    aligned_gap_fixture(&trim_case);
    let trim = trim_case.run(&["end", "proj", "12", "plan", "planned the thing", "--trim"]);
    trim.assert_status(0);

    assert_eq!(
        full.logged_minutes() - trim.logged_minutes(),
        fixture.hole_minutes,
        "minutes removed by --trim"
    );
}

/// Ported from "--trim on a phase with nothing flagged is a no-op, not a usage
/// error".
#[test]
fn trim_on_a_phase_with_nothing_flagged_is_a_no_op() {
    let case = Case::new("idle-trim-noop");
    let span = 30 * 60;
    let start = now() - span;
    case.write_mark("proj.7.impl", start);
    case.beats_at("proj.7.impl", &[start + 10 * 60, start + span]);

    let run = case.run(&["end", "proj", "7", "impl", "nothing to trim", "--trim"]);
    run.assert_status(0);
    // No silence means no split: `cli::log`'s clap `requires` makes `--trim` with
    // nothing to trim a usage error, so `trim: true` must never be passed alone.
    run.assert_stdout_has(&logged_duration(span / 60));

    let entries = case.store().entries;
    assert_eq!(entries.len(), 1, "nothing was split");
    assert!(entries[0].idle.is_empty());
}
