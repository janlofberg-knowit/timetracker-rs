//! `tt log` end to end, driving the real binary in a sandbox.
//!
//! `cli::log`'s own unit tests cover the store's shape after a split. What only an
//! end-to-end run can check is the **message**, which for a long time reported the
//! span it was asked for rather than the span it stored (#58, wart 1). The store
//! and stdout are therefore asserted against each other here, never separately.

mod common;

use common::Case;

/// The `- Duration:` tail of a successful log, in whole minutes.
fn printed_minutes(stdout: &str) -> i64 {
    let tail = stdout
        .split("- Duration: ")
        .nth(1)
        .unwrap_or_else(|| panic!("no logged duration in {stdout:?}"));
    let (hours, rest) = tail.split_once('h').expect("`<h>h <m>m`");
    let (minutes, _) = rest.trim_start().split_once('m').expect("`<h>h <m>m`");
    hours.trim().parse::<i64>().unwrap() * 60 + minutes.trim().parse::<i64>().unwrap()
}

#[test]
fn trim_reports_the_span_it_stored_and_not_the_span_it_was_asked_for() {
    let case = Case::new("log-trim-message");
    // A one-hour entry with a 20-minute hole strictly inside it, so the trim leaves
    // two pieces and 40 minutes. Anchored on the process's own clock: `tt log`
    // back-dates from now, so the interval has to be expressed relative to now too.
    let now = common::now();
    let hole = (now - 40 * 60, now - 20 * 60);

    let run = case.run_bare(&[
        "log",
        "-d",
        "an hour with a hole in it",
        "-t",
        "60m",
        &format!("--idle={}-{}", hole.0, hole.1),
        "--trim",
    ]);
    run.assert_status(0);

    let entries = case.store().entries;
    assert_eq!(entries.len(), 2, "the hole is interior, so it cuts in two");
    let stored: i64 = entries.iter().map(|entry| entry.seconds()).sum();
    // One second of slack per piece: the entry is back-dated from the process's own
    // `now`, a moment after this test read the clock, so each piece carries a
    // fraction of a second that `num_seconds` truncates away. The pieces' *sum* is
    // exact — it is the read-back that rounds.
    assert!(
        (40 * 60 - entries.len() as i64..=40 * 60).contains(&stored),
        "60 minutes less the 20-minute hole, got {stored}s: {entries:?}"
    );
    assert_eq!(
        printed_minutes(&run.stdout),
        40,
        "the message states what the store holds, not the 60m it was asked for: {:?}",
        run.stdout
    );
}

#[test]
fn a_plain_log_still_reports_the_span_it_was_asked_for() {
    // The other half of the same behaviour: with nothing to cut, "what was stored"
    // and "what was asked for" are the same number, and the common path is
    // unchanged.
    let case = Case::new("log-plain-message");

    let run = case.run_bare(&["log", "-d", "no holes here", "-t", "45m"]);
    run.assert_status(0);
    assert_eq!(printed_minutes(&run.stdout), 45);
    assert_eq!(case.store().entries[0].seconds(), 45 * 60);
}

#[test]
fn idle_covering_the_whole_span_leaves_the_entry_and_reports_it_whole() {
    // `trim_spans` declines rather than deleting what the owner logged, so there is
    // nothing to subtract and the reported figure must not drift to zero.
    let case = Case::new("log-trim-declines");
    let now = common::now();

    let run = case.run_bare(&[
        "log",
        "-d",
        "swallowed whole",
        "-t",
        "30m",
        &format!("--idle={}-{}", now - 90 * 60, now + 90 * 60),
        "--trim",
    ]);
    run.assert_status(0);

    let entries = case.store().entries;
    assert_eq!(entries.len(), 1, "nothing was split");
    assert_eq!(entries[0].seconds(), 30 * 60);
    assert_eq!(printed_minutes(&run.stdout), 30);
}
