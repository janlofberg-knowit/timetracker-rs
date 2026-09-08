//! `tt agent`'s mark lifecycle — `begin`, `touch`, `cancel` and `list` — driving the
//! real binary, plus the assertion only a real process can make: that a mark command
//! takes no store lock and creates no `data.json`.
//!
//! Every case runs in the throwaway `HOME` and `TT_MARK_DIR` [`common`] sets up.

mod common;

use common::{Case, clock, count_lines, now};
use std::fs;

// --- the store is never touched -------------------------------------------

/// `main` dispatches the mark-only commands ahead of its `storage::with_data`
/// preamble, so none of them creates the store or takes its lock. Asserted on the
/// two *files*, not the directory, because `get_data_path` does a `create_dir_all`;
/// the `item` complement in the same test proves the sandbox was in effect.
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

    for label in ["code", "style"] {
        case.run(&["begin", "proj", "7", "review", "--agent", label])
            .assert_status(0);
        case.run(&["touch", "proj", "7", "review", "--agent", label])
            .assert_status(0);
        case.run(&["cancel", "proj", "7", "review", "--agent", label])
            .assert_status(0);
    }

    for name in ["data.json", "data.lock"] {
        let path = data_dir.join(name);
        assert!(
            !path.exists(),
            "{name} was created by a labelled mark-only command: {path:?}"
        );
    }

    case.run(&["item", "proj", "7", "impl", "did the thing", "30"])
        .assert_status(0);
    for name in ["data.json", "data.lock"] {
        assert!(
            data_dir.join(name).is_file(),
            "the sandbox is not in effect — {name} landed somewhere else"
        );
    }
}

// --- the automatic beat ----------------------------------------------------

/// `UserPromptSubmit` renews the beating session's own marks and nothing else.
#[test]
fn activity_prompt_beats_only_that_projects_marks() {
    let case = Case::new("activity-prompt");
    case.write_mark("a.7.impl", now());
    case.write_mark("b.9.impl", now());

    case.run(&["activity", "prompt", "a"]).assert_status(0);

    assert_eq!(count_lines(&case.beats_file("a.7.impl")), 1);
    assert!(
        !case.beats_file("b.9.impl").exists(),
        "another project's mark was beaten"
    );
}

/// An unattributable beat is what let an unrelated session keep an abandoned
/// mark alive, so no project means no beat.
#[test]
fn activity_prompt_with_no_project_beats_nothing() {
    let case = Case::new("activity-prompt-bare");
    case.write_mark("a.7.impl", now());

    case.run(&["activity", "prompt"]).assert_status(0);

    assert!(!case.beats_file("a.7.impl").exists());
}

// --- begin -----------------------------------------------------------------

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

// --- touch -----------------------------------------------------------------

#[test]
fn touch_without_a_mark_exits_64() {
    let case = Case::new("touch-unmarked");
    let run = case.run(&["touch", "proj", "7", "impl"]);
    run.assert_status(64);
    run.assert_stderr_has("nothing to touch");
    assert_eq!(case.mark_count(), 0, "nothing was written");
}

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

#[test]
fn beats_live_in_a_subdirectory_not_as_a_mark_sibling() {
    let case = Case::new("touch-subdirectory");
    case.write_mark("proj.7.impl", now());
    case.run(&["touch", "proj", "7", "impl"]).assert_status(0);

    assert!(!case.mark_file("proj.7.impl.last").exists());
    assert!(!case.mark_file("proj.7.impl.beats").exists());
    assert!(case.beats_file("proj.7.impl").is_file());
}

// --- cancel ----------------------------------------------------------------

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

// --- list ------------------------------------------------------------------

#[test]
fn list_reports_nothing_when_the_directory_is_empty() {
    let case = Case::new("list-empty");
    let run = case.run(&["list"]);
    run.assert_status(0);
    run.assert_stdout_has("No open marks.");
    // The bare line, with no header and no emoji.
    assert_eq!(run.stdout, "No open marks.\n");
}

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
            "\u{1F916} Open marks:\n\n  proj/23 impl       - since {} (0h 10m) last seen never\n",
            clock(start)
        ),
        "the header, the blank line and one padded row"
    );
}

/// The incident's shape: a mark days old with no heartbeat is flagged, and the
/// line under it is the one that logs the work and clears it.
#[test]
fn list_flags_a_stale_mark_with_the_command_that_clears_it() {
    let case = Case::new("list-stale");
    case.write_mark("proj.23.impl", now() - 114 * 3600);

    let run = case.run(&["list"]);
    run.assert_status(0);
    run.assert_stdout_has("last seen never [stale]");
    run.assert_stdout_has("tt agent end proj 23 impl \"<summary>\" <minutes>");
}

/// A stale mark the model vouched for gets `--trim`, and it has to remove the
/// hole between the beats rather than log the span `--full` would.
#[test]
fn the_trim_a_stale_vouched_row_prints_changes_the_bill() {
    let case = Case::new("list-stale-trim");
    let start = now() - 160 * 60;
    case.write_mark("proj.23.impl", start);
    case.beats_at("proj.23.impl", &[start + 5 * 60, start + 100 * 60]);

    let listed = case.run(&["list"]);
    listed.assert_status(0);
    listed.assert_stdout_has("[stale]");
    listed.assert_stdout_has("tt agent end proj 23 impl \"<summary>\" --trim");

    // A bare close is refused for the 95m hole, so the printed line is the one
    // that both logs and clears.
    case.run(&["end", "proj", "23", "impl", "<summary>"])
        .assert_status(65);
    let trimmed = case.run(&["end", "proj", "23", "impl", "<summary>", "--trim"]);
    trimmed.assert_status(0);
    trimmed.assert_stdout_has("- Duration: 0h 5m");
}

/// A stale mark only the hooks beat is judged whole-span, so its row asks for
/// the minutes.
#[test]
fn a_stale_hook_beaten_row_asks_for_the_minutes() {
    let case = Case::new("list-stale-hooked");
    let start = now() - 160 * 60;
    case.write_mark("proj.23.impl", start);
    case.hook_beats_at("proj.23.impl", &[start + 100 * 60]);

    let listed = case.run(&["list"]);
    listed.assert_status(0);
    listed.assert_stdout_has("[stale]");
    listed.assert_stdout_has("tt agent end proj 23 impl \"<summary>\" <minutes>");
}

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

/// The house duration format: always `{h}h {m}m`, and never negative.
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
    // No age is ever negative; the check is on the parenthesised age, not the row.
    for line in run.stdout.lines().filter(|line| line.contains('(')) {
        let age = &line[line.find('(').unwrap()..];
        assert!(!age.contains('-'), "a negative age reached {age:?}");
    }
}

#[test]
fn list_still_shows_a_mark_with_more_segments_than_a_key_has() {
    let case = Case::new("list-dotted-phase");
    case.write_mark("proj.23.impl.v2.x", now());

    let run = case.run(&["list"]);
    run.assert_status(0);
    // The dot split is lossy by design; an imperfect label beats hiding an open mark.
    run.assert_stdout_has("proj/23.impl.v2 x");
}

#[test]
fn list_shows_a_dotless_name_as_a_bare_project() {
    let case = Case::new("list-dotless");
    case.write_mark("solo", now());

    let run = case.run(&["list"]);
    run.assert_status(0);
    run.assert_stdout_has("solo");
}

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
    // One row per regular file — a file-type test, never a name filter.
    let rows = run
        .stdout
        .lines()
        .filter(|line| line.starts_with("  "))
        .count();
    assert_eq!(rows, case.mark_count(), "one row per mark file");
}

// --- list <project> --------------------------------------------------------

#[test]
fn list_narrows_to_the_named_project() {
    let case = Case::new("list-filter");
    case.write_mark("app.7.impl", now());
    case.write_mark("other.9.plan", now());

    let run = case.run(&["list", "app"]);
    run.assert_status(0);
    run.assert_stdout_has("app/7 impl");
    assert!(
        !run.stdout.contains("other"),
        "another project's mark was listed: {:?}",
        run.stdout
    );
}

/// A lossy name is found by either spelling: the mark file only ever holds the
/// sanitised one.
#[test]
fn list_finds_a_lossy_project_name_by_either_spelling() {
    let case = Case::new("list-filter-lossy");
    case.run(&["begin", "my proj", "7", "impl"])
        .assert_status(0);
    assert!(case.mark_file("my_proj.7.impl").is_file());

    for spelling in ["my proj", "my_proj"] {
        let run = case.run(&["list", spelling]);
        run.assert_status(0);
        run.assert_stdout_has("my_proj/7 impl");
    }
}

/// The boundary is a whole sanitised segment, in both directions.
#[test]
fn list_does_not_match_a_dot_related_project() {
    let case = Case::new("list-filter-segment");
    case.run(&["begin", "app.web", "7", "impl"])
        .assert_status(0);
    assert!(case.mark_file("app-web.7.impl").is_file());

    let narrow = case.run(&["list", "app"]);
    narrow.assert_status(0);
    assert_eq!(narrow.stdout, "No open marks.\n");

    case.run(&["cancel", "app.web", "7", "impl"])
        .assert_status(0);
    case.run(&["begin", "app", "7", "impl"]).assert_status(0);
    let wide = case.run(&["list", "app.web"]);
    wide.assert_status(0);
    assert_eq!(wide.stdout, "No open marks.\n");
}

/// The filter is `owned_by`, not a substring of the row: a project named after
/// a word in a close line matches only its own marks.
#[test]
fn list_for_a_project_named_tt_lists_its_own_mark_only() {
    let case = Case::new("list-filter-tt");
    case.write_mark("tt.1.impl", now());
    case.write_mark("other.23.impl", now() - 114 * 3600);

    let run = case.run(&["list", "tt"]);
    run.assert_status(0);
    run.assert_stdout_has("tt/1 impl");
    assert!(
        !run.stdout.contains("other"),
        "a stale row of another project was listed: {:?}",
        run.stdout
    );
}

/// A stale row's own close line still rides along under the filter.
#[test]
fn list_for_a_project_keeps_its_stale_rows_close_line() {
    let case = Case::new("list-filter-stale");
    case.write_mark("app.23.impl", now() - 114 * 3600);
    case.write_mark("other.9.plan", now() - 114 * 3600);

    let run = case.run(&["list", "app"]);
    run.assert_status(0);
    run.assert_stdout_has("last seen never [stale]");
    run.assert_stdout_has("tt agent end app 23 impl \"<summary>\" <minutes>");
    assert!(
        !run.stdout.contains("other"),
        "another project's stale row was listed: {:?}",
        run.stdout
    );
}

#[test]
fn list_for_a_project_with_no_open_marks_reports_the_bare_line() {
    let case = Case::new("list-filter-none");
    case.write_mark("other.9.plan", now());

    let run = case.run(&["list", "quiet"]);
    run.assert_status(0);
    assert_eq!(run.stdout, "No open marks.\n");
}

/// A mark file written before `.` mapped to `-` names segments `end` and
/// `cancel` cannot address, so its row offers no runnable close line.
#[test]
fn a_legacy_dotted_mark_is_listed_with_no_close_line() {
    let case = Case::new("list-legacy-dotted");
    let start = now() - 5 * 3600;
    case.write_mark("app.web.7.impl.v2", start);

    let run = case.run(&["list"]);
    run.assert_status(0);
    run.assert_stdout_has(&format!("since {}", clock(start)));
    run.assert_stdout_has("app.web.7.impl.v2 cannot be closed");
    run.assert_stdout_has("tt agent item app web.7.impl v2");
    assert!(
        !run.stdout.contains("tt agent end"),
        "following this row must not log an entry that leaves the mark open: {:?}",
        run.stdout
    );
}

/// The close line of a mark that does round-trip, in both of its forms.
#[test]
fn a_healthy_marks_close_line_is_unchanged() {
    let case = Case::new("list-legacy-healthy");
    let start = now() - 5 * 3600;
    case.write_mark("app.7.impl", start);
    case.write_mark("app.-.plan", start);
    case.beats_at("app.7.impl", &[start + 60]);

    let run = case.run(&["list"]);
    run.assert_status(0);
    run.assert_stdout_has("tt agent end app 7 impl \"<summary>\" --trim");
    run.assert_stdout_has("tt agent end app - plan \"<summary>\" <minutes>");
}

// --- the agent label -------------------------------------------------------

/// Two subagents on one phase hold one mark each, and the phase's own unlabelled
/// mark is a third; each is begun, beaten and dropped on its own.
#[test]
fn a_label_addresses_a_mark_of_its_own() {
    let case = Case::new("agent-label");
    for args in [
        &["begin", "proj", "7", "review", "--agent", "code"][..],
        &["begin", "proj", "7", "review", "--agent", "style"][..],
        &["begin", "proj", "7", "review"][..],
    ] {
        case.run(args).assert_status(0);
    }
    assert_eq!(case.mark_count(), 3);

    case.run(&["touch", "proj", "7", "review", "--agent", "code"])
        .assert_status(0);
    assert_eq!(count_lines(&case.beats_file("proj.7.review.code")), 1);
    assert!(
        !case.beats_file("proj.7.review.style").exists(),
        "the other label's mark was beaten"
    );

    let run = case.run(&["list"]);
    run.assert_stdout_has("proj/7 review:code");
    run.assert_stdout_has("proj/7 review:style");

    case.run(&["cancel", "proj", "7", "review", "--agent", "code"])
        .assert_status(0);
    assert!(!case.mark_file("proj.7.review.code").exists());
    assert!(case.mark_file("proj.7.review.style").is_file());
    assert!(case.mark_file("proj.7.review").is_file());
}

#[test]
fn the_messages_name_the_label_they_addressed() {
    let case = Case::new("agent-label-messages");
    let run = case.run(&["begin", "proj", "7", "review", "--agent", "code"]);
    run.assert_status(0);
    run.assert_stdout_has("marked proj/7 review:code at");

    let again = case.run(&["begin", "proj", "7", "review", "--agent", "code"]);
    again.assert_status(0);
    again.assert_stderr_has("already marked proj/7 review:code");

    let touch = case.run(&["touch", "proj", "7", "review", "--agent", "style"]);
    touch.assert_status(64);
    touch.assert_stderr_has("no mark for proj/7 review:style");

    let cancel = case.run(&["cancel", "proj", "7", "review", "--agent", "code"]);
    cancel.assert_status(0);
    cancel.assert_stdout_has("dropped mark for proj/7 review:code");
}
