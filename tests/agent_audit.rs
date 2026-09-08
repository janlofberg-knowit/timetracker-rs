//! `tt agent audit` and `tt agent audit --auto-log`, end to end through the
//! real binary and a sandboxed store/marks/activity directory. See
//! docs/decisions/0001-agent-activity-tracking.md and
//! docs/decisions/0002-auto-logging-unaccounted-activity.md.

mod common;
use common::{Case, clock, now};

const HOUR: i64 = 3600;

#[test]
fn a_clean_sandbox_reports_nothing_unaccounted() {
    let case = Case::new("audit-clean");
    let run = case.run(&["audit"]);
    run.assert_status(0);
    run.assert_stdout_has("No unaccounted agent activity.");
}

#[test]
fn a_session_past_the_floor_with_no_coverage_is_reported() {
    let case = Case::new("audit-flagged");
    let start = now() - 3 * HOUR;
    case.write_session("sess-1", "smoke", start, None);

    let run = case.run(&["audit"]);
    run.assert_status(0);
    run.assert_stdout_has("Unaccounted agent activity");
    run.assert_stdout_has("smoke");
}

#[test]
fn a_covering_mark_removes_it_from_the_report() {
    let case = Case::new("audit-covered-by-mark");
    let start = now() - 3 * HOUR;
    case.write_session("sess-1", "smoke", start, None);
    case.write_mark("smoke.-.impl", start - 60);

    let run = case.run(&["audit"]);
    run.assert_status(0);
    run.assert_stdout_has("No unaccounted agent activity.");
}

#[test]
fn auto_log_is_a_no_op_when_the_setting_is_unset() {
    let case = Case::new("audit-auto-log-unset");
    let start = now() - 3 * HOUR;
    case.write_session("sess-1", "smoke", start, None);

    let run = case.run(&["audit", "--auto-log"]);
    run.assert_status(0);
    // Same report as a plain audit: nothing got logged.
    run.assert_stdout_has("Unaccounted agent activity");
    run.assert_stdout_has("smoke");
    assert!(
        case.store().entries.is_empty(),
        "no entry should have been written"
    );
}

#[test]
fn auto_log_writes_a_fixed_phase_auto_entry_over_the_threshold() {
    let case = Case::new("audit-auto-log-writes");
    case.write_config("[agent]\nauto_log_after_minutes = 180\n"); // 3h, floor stays 120
    let start = now() - 4 * HOUR; // 240m, over both the 120m floor and the 180m auto-log threshold
    case.write_session("sess-1", "smoke", start, None);

    let run = case.run(&["audit", "--auto-log"]);
    run.assert_status(0);
    run.assert_stdout_has("No unaccounted agent activity.");

    let store = case.store();
    assert_eq!(store.entries.len(), 1);
    let entry = &store.entries[0];
    assert_eq!(entry.description, "unattended activity");
    assert_eq!(entry.project.as_deref(), Some("smoke"));
    assert_eq!(entry.tags, vec!["auto".to_string()]);
    assert!(
        !entry.tags.contains(&"agent".to_string()),
        "an auto-logged entry must never carry #agent"
    );
}

/// An auto-logged entry pins `ended_at`, so rounding its span would push the
/// start back into the idle gap the audit just judged.
#[test]
fn auto_log_never_rounds_even_with_round_minutes_set() {
    let case = Case::new("audit-auto-log-unrounded");
    case.write_config(
        "[agent]\nmax_unvouched_minutes = 20\nauto_log_after_minutes = 21\nround_minutes = 5\n",
    );
    let start = now() - 47 * 60;
    case.write_session("sess-1", "smoke", start, None);

    let run = case.run(&["audit", "--auto-log"]);
    run.assert_status(0);

    let store = case.store();
    assert_eq!(store.entries.len(), 1);
    assert_eq!(
        store.entries[0].seconds(),
        47 * 60,
        "the unaccounted window is logged as measured"
    );
}

#[test]
fn a_window_under_the_auto_log_threshold_is_reported_but_not_logged() {
    let case = Case::new("audit-auto-log-under-threshold");
    case.write_config("[agent]\nauto_log_after_minutes = 300\n"); // 5h
    let start = now() - 3 * HOUR; // over the 120m floor, under the 300m auto-log threshold
    case.write_session("sess-1", "smoke", start, None);

    let run = case.run(&["audit", "--auto-log"]);
    run.assert_status(0);
    run.assert_stdout_has("Unaccounted agent activity");
    run.assert_stdout_has("smoke");
    assert!(
        case.store().entries.is_empty(),
        "under the auto-log threshold: reported, never logged"
    );
}

#[test]
fn a_misconfigured_threshold_at_or_under_the_floor_disables_auto_log() {
    let case = Case::new("audit-auto-log-misconfigured");
    // Not strictly greater than the (default 120m) floor: must disable, not clamp.
    case.write_config("[agent]\nauto_log_after_minutes = 120\n");
    let start = now() - 4 * HOUR;
    case.write_session("sess-1", "smoke", start, None);

    let run = case.run(&["audit", "--auto-log"]);
    run.assert_status(0);
    run.assert_stdout_has("smoke");
    assert!(
        case.store().entries.is_empty(),
        "a misconfigured threshold must fail toward off, not auto-log anyway"
    );
}

#[test]
fn running_auto_log_twice_logs_the_window_once() {
    let case = Case::new("audit-auto-log-idempotent");
    case.write_config("[agent]\nauto_log_after_minutes = 180\n");
    let start = now() - 4 * HOUR;
    case.write_session("sess-1", "smoke", start, None);

    case.run(&["audit", "--auto-log"]).assert_status(0);
    assert_eq!(case.store().entries.len(), 1, "first run logs one entry");

    let second = case.run(&["audit", "--auto-log"]);
    second.assert_status(0);
    second.assert_stdout_has("No unaccounted agent activity.");
    assert_eq!(
        case.store().entries.len(),
        1,
        "the second run must not log a duplicate"
    );
}

/// The minutes an 8h session with two 60-minute idle holes reports, as three
/// contiguous active stretches. Counted against the dispatches below it: a
/// removed dispatch leaves a 60m hole between its neighbours.
const ROWS: [(i64, i64); 3] = [(0, 120), (180, 270), (330, 480)];

/// Every 30 minutes over the 8h window, minus the one at 150m and the one at
/// 300m.
fn dispatches(start: i64) -> Vec<i64> {
    (1..16)
        .map(|i| i * 30)
        .filter(|m| ![150, 300].contains(m))
        .map(|m| start + m * 60)
        .collect()
}

/// `Lease::close_command` prints the mark's sanitised project, so the entry an
/// operator logs by following a `[stale]` row has to be one the audit's entry
/// join accepts.
#[test]
fn an_entry_logged_with_the_printed_project_silences_the_next_check() {
    let case = Case::new("audit-close-line-covers");
    let start = now() - 5 * HOUR;
    case.write_session("sess-1", "my proj", start, None);
    case.write_mark("my_proj.-.impl", start);

    let flagged = case.run(&["activity", "check", "sess-1"]);
    flagged.assert_status(0);
    flagged.assert_stdout_has("my proj");

    // The close line the stale row prints, run verbatim.
    let listed = case.run(&["list", "my_proj"]);
    listed.assert_stdout_has("tt agent end my_proj - impl \"<summary>\" <minutes>");
    case.run(&["end", "my_proj", "-", "impl", "did the thing", "300"])
        .assert_status(0);

    let silenced = case.run(&["activity", "check", "sess-1"]);
    silenced.assert_status(0);
    assert_eq!(
        silenced.stdout, "",
        "the entry the printed close line logs must cover the fragment it was printed for"
    );
}

/// The floor gates the session's total, so fragments under it are reported —
/// and `auto_log_after_minutes`, which must exceed the floor, still gates each
/// fragment on its own.
#[test]
fn fragments_under_the_floor_are_reported_but_never_auto_logged() {
    let case = Case::new("audit-fragment-floor");
    case.write_config("[agent]\nauto_log_after_minutes = 180\n");
    let start = now() - 6 * HOUR;
    case.write_session("sess-1", "smoke", start, None);
    // Two unvouched marks, each covering the two hours after it was opened,
    // leaving a 50m head and an 80m middle.
    case.write_mark("smoke.1.impl", start + 50 * 60);
    case.write_mark("smoke.2.impl", start + 4 * HOUR + 10 * 60);

    let run = case.run(&["audit", "--auto-log"]);
    run.assert_status(0);
    assert_eq!(
        run.stdout.lines().filter(|l| l.contains("smoke")).count(),
        2,
        "both fragments are under the floor and both belong in the report: {:?}",
        run.stdout
    );
    assert!(
        case.store().entries.is_empty(),
        "a fragment under auto_log_after_minutes must never be written"
    );
}

/// An idle hole is never reported: the rows are the active stretches around
/// it, and an `#auto` entry spans exactly the row it was written for, so no
/// later audit can flag the hole.
#[test]
fn a_session_with_two_idle_holes_reports_three_active_rows() {
    let case = Case::new("audit-idle-holes");
    // The floor and the auto-log threshold both under the shortest row, so
    // every row clears them.
    case.write_config("[agent]\nmax_unvouched_minutes = 60\nauto_log_after_minutes = 61\n");
    let start = now() - 8 * HOUR;
    case.write_session_with_dispatches(
        "sess-1",
        "repro",
        start,
        Some(start + 8 * HOUR),
        &dispatches(start),
    );

    let run = case.run(&["audit"]);
    run.assert_status(0);
    let rows: Vec<&str> = run
        .stdout
        .lines()
        .filter(|line| line.contains("repro"))
        .collect();
    assert_eq!(rows.len(), 3, "{:?}", run.stdout);
    for (row, (from, to)) in rows.iter().rev().zip(ROWS) {
        assert!(
            row.contains(&format!("since {}", clock(start + from * 60))),
            "{row} is not the row starting at {from}m"
        );
        let minutes = to - from;
        assert!(
            row.contains(&format!("({}h {}m", minutes / 60, minutes % 60)),
            "{row} is not {minutes}m long"
        );
    }

    let logged = case.run(&["audit", "--auto-log"]);
    logged.assert_status(0);
    logged.assert_stdout_has("No unaccounted agent activity.");
    let mut spans: Vec<(i64, i64)> = case
        .store()
        .entries
        .iter()
        .map(|entry| {
            (
                entry.start_time.timestamp() - start,
                entry.end_time.unwrap().timestamp() - start,
            )
        })
        .collect();
    spans.sort();
    assert_eq!(
        spans,
        ROWS.map(|(from, to)| (from * 60, to * 60)).to_vec(),
        "each entry spans exactly the row it was written for"
    );

    let again = case.run(&["audit", "--auto-log"]);
    again.assert_status(0);
    again.assert_stdout_has("No unaccounted agent activity.");
    assert_eq!(
        case.store().entries.len(),
        3,
        "a hole left between the rows must never be re-reported or re-logged"
    );
}
