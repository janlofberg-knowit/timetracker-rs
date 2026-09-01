//! Reconciles the hook-only activity ledger against marks and logged
//! entries — the `tt agent audit` command. See
//! `docs/decisions/0001-agent-activity-tracking.md`.
//!
//! An activity window counts as accounted for once every part of it falls
//! inside some same-project mark's lease or a same-project `#agent`- or
//! `#auto`-tagged entry. Both sources subtract, so they compose (see
//! `docs/decisions/0002-auto-logging-unaccounted-activity.md` for the
//! latter). Neither is **unaccounted agent activity**: real work that never
//! got tracked at all.

use chrono::{DateTime, Local};

use crate::activity::Session;
use crate::marks::{self, Lease, Thresholds};
use crate::time::instant;
use crate::tracker::{IdleInterval, TimeEntry};

/// One activity window with no evidence it was tracked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unaccounted {
    pub project: String,
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
    pub subagents: usize,
    /// Idle stretches between subagent-dispatch heartbeats, over
    /// [`max_gap_minutes`] — see [`write_auto_log`](crate::agent) and
    /// docs/decisions/0003-auto-log-on-stop.md ("3b. Idle time must be
    /// subtracted before a window is auto-logged"). `describe()` still
    /// reports the raw wall-clock span; only an auto-logged entry's
    /// duration is adjusted by this.
    pub idle: Vec<IdleInterval>,
}

impl Unaccounted {
    /// `<project> - since HH:MM (Xh Ym)`, with a trailing subagent-dispatch
    /// count when there were any. Shared by the CLI (`tt agent audit`,
    /// `tt agent activity check`) and the TUI's Agents panel, so the two
    /// cannot disagree on how this reads.
    pub fn describe(&self) -> String {
        let subagents = match self.subagents {
            0 => String::new(),
            1 => ", 1 subagent dispatch".to_string(),
            n => format!(", {n} subagent dispatches"),
        };
        format!(
            "{} - since {} ({}{})",
            self.project,
            self.start.format("%H:%M"),
            crate::duration::format(self.end.signed_duration_since(self.start)),
            subagents
        )
    }
}

/// How long an activity window may run with no covering mark or entry before
/// it counts as unaccounted, in minutes. Shared with the same setting
/// `tt agent end` judges an unvouched phase against: `TT_MAX_UNVOUCHED_MINUTES`,
/// else `agent.max_unvouched_minutes`, else 120.
pub fn max_unvouched_minutes() -> i64 {
    crate::config::resolve_minutes(
        "TT_MAX_UNVOUCHED_MINUTES",
        crate::config::load().agent.max_unvouched_minutes,
        120,
    )
}

/// How long a silence *between subagent-dispatch heartbeats* has to be to
/// count as idle, in minutes — the same knob `tt agent end` judges interior
/// mark silence against. `TT_MAX_GAP_MINUTES`, else `agent.max_gap_minutes`,
/// else 45.
pub fn max_gap_minutes() -> i64 {
    crate::config::resolve_minutes(
        "TT_MAX_GAP_MINUTES",
        crate::config::load().agent.max_gap_minutes,
        45,
    )
}

/// Whether the `Stop` hook should auto-log a session's own unaccounted
/// window, per `agent.auto_log_on_stop` — see
/// docs/decisions/0003-auto-log-on-stop.md. `config::load` already resets
/// this to `None` if `auto_log_after_minutes` is not also set, so a bare
/// read here is enough; no need to re-check the precondition.
pub fn auto_log_on_stop_enabled() -> bool {
    crate::config::load().agent.auto_log_on_stop == Some(true)
}

/// How long a window must stay unaccounted for before `tt agent audit
/// --auto-log` (see docs/decisions/0002-auto-logging-unaccounted-activity.md)
/// writes a fallback `#auto` entry for it, in minutes.
/// `TT_AUTO_LOG_AFTER_MINUTES`, else `agent.auto_log_after_minutes`.
///
/// `None` disables auto-logging outright — both when neither is set (the
/// default) and when the configured value does not exceed
/// [`max_unvouched_minutes`]. The latter is a misconfiguration, and it must
/// fail toward "off" rather than toward auto-logging a window the audit
/// surfaces never had a chance to warn about first.
pub fn auto_log_after_minutes() -> Option<i64> {
    let configured = std::env::var("TT_AUTO_LOG_AFTER_MINUTES")
        .ok()
        .and_then(|value| value.parse().ok())
        .or(crate::config::load().agent.auto_log_after_minutes)?;
    (configured > max_unvouched_minutes()).then_some(configured)
}

/// The thresholds every judgement below an entry point is made by, read from
/// the environment and config once. Resolve at `run_audit`, `check_session`,
/// `list`, `end` and the TUI's sync, and pass the value down; nothing below
/// them reads config, so a caller and a test judge by the same numbers.
pub fn thresholds() -> Thresholds {
    Thresholds {
        gap: max_gap_minutes(),
        unvouched: max_unvouched_minutes(),
    }
}

/// Half-open interval overlap: touching endpoints do not count.
fn overlaps(a_start: i64, a_end: i64, b_start: i64, b_end: i64) -> bool {
    a_start < b_end && b_start < a_end
}

/// Every session below `thresholds.unvouched` is not flagged — a one-line
/// question should not trip the warning. A session with no project cannot be
/// reconciled against anything, so it is skipped rather than assumed
/// unaccounted.
pub fn unaccounted(
    sessions: &[Session],
    leases: &[Lease],
    entries: &[TimeEntry],
    now: DateTime<Local>,
    thresholds: Thresholds,
) -> Vec<Unaccounted> {
    let now_epoch = now.timestamp();
    let floor_minutes = thresholds.unvouched;

    let mut found: Vec<Unaccounted> = sessions
        .iter()
        .flat_map(|session| {
            let Some(project) = session.project.as_deref() else {
                return Vec::new();
            };
            let end_epoch = session.end.unwrap_or(now_epoch);
            if (end_epoch - session.start) / 60 < floor_minutes {
                return Vec::new();
            }

            let stretches = uncovered_by_marks(
                project,
                session.start,
                end_epoch,
                leases,
                now_epoch,
                thresholds,
            );
            uncovered_by_entries(project, stretches, entries, now_epoch)
                .into_iter()
                // The floor applies to what is left as much as to the window: the
                // short head before a live mark was opened is not worth flagging.
                .filter(|(from, to)| (to - from) / 60 >= floor_minutes)
                .filter_map(|(from, to)| {
                    // Unlike a mark's beats, no subagent dispatches is not evidence
                    // of silence — most sessions never dispatch one at all. Only
                    // treat gaps *between* dispatches (and before the first / after
                    // the last) as idle when there is at least one to anchor on.
                    // Computed over the *uncovered* stretch, or an auto-logged
                    // entry would subtract idle time from outside what it reports.
                    let idle = if session.subagent_at.is_empty() {
                        Vec::new()
                    } else {
                        marks::gaps_over(from, to, &session.subagent_at, thresholds.gap)
                            .into_iter()
                            .filter_map(|(a, b)| Some(IdleInterval::new(instant(a)?, instant(b)?)))
                            .collect()
                    };

                    // Each fragment reports only the dispatches inside it, so the
                    // rows sum to the session's. With no timestamps to split by
                    // there is nothing to attribute, so the session's own count
                    // stands.
                    let subagents = if session.subagent_at.is_empty() {
                        session.subagents
                    } else {
                        session
                            .subagent_at
                            .iter()
                            .filter(|&&at| at >= from && at < to)
                            .count()
                    };

                    Some(Unaccounted {
                        project: project.to_string(),
                        start: instant(from)?,
                        end: instant(to)?,
                        subagents,
                        idle,
                    })
                })
                .collect()
        })
        .collect();

    found.sort_by_key(|u| std::cmp::Reverse(u.start));
    found
}

/// What is left of `start → end` after removing every same-project lease's
/// covered interval — a lease covers `mark.start` up to whichever comes first,
/// `now` or its expiry. Subtraction, not overlap: a mark that vouched only for
/// the head of a still-open session leaves its tail flagged, which is the
/// weekend incident's own shape.
fn uncovered_by_marks(
    project: &str,
    start: i64,
    end: i64,
    leases: &[Lease],
    now: i64,
    thresholds: Thresholds,
) -> Vec<(i64, i64)> {
    let mut remaining = vec![(start, end)];
    // The same segment rule the beat uses, so a lossy name still joins.
    for lease in leases
        .iter()
        .filter(|lease| marks::owned_by(&lease.mark, project))
    {
        let from = lease.mark.start.timestamp();
        let until = lease.expires_at(thresholds).timestamp().min(now);
        if until <= from {
            continue;
        }
        remaining = remaining
            .into_iter()
            .flat_map(|stretch| subtract(stretch, (from, until)))
            .collect();
    }
    remaining
}

/// `stretch` minus `cut`, as the zero, one or two stretches that survive.
fn subtract(stretch: (i64, i64), cut: (i64, i64)) -> Vec<(i64, i64)> {
    let (start, end) = stretch;
    let (cut_start, cut_end) = cut;
    if !overlaps(start, end, cut_start, cut_end) {
        return vec![stretch];
    }
    let mut kept = Vec::new();
    if start < cut_start {
        kept.push((start, cut_start));
    }
    if cut_end < end {
        kept.push((cut_end, end));
    }
    kept
}

/// What is left of `stretches` after removing every covering entry's span. An
/// entry covers when it is tagged `#agent` (an agent's own self-report) or
/// `#auto` (a prior `--auto-log` run); the two provenances are deliberately
/// distinct tags, checked together only here. One still open covers up to `now`.
///
/// Subtraction, not overlap, and applied to the stretches the leases left: the
/// two coverage sources compose, so auto-logging one fragment cannot hide the
/// rest of its session.
fn uncovered_by_entries(
    project: &str,
    stretches: Vec<(i64, i64)>,
    entries: &[TimeEntry],
    now: i64,
) -> Vec<(i64, i64)> {
    let mut remaining = stretches;
    for entry in entries
        .iter()
        .filter(|entry| entry.has_tag("agent") || entry.has_tag("auto"))
        .filter(|entry| {
            entry
                .project
                .as_deref()
                .is_some_and(|p| p.eq_ignore_ascii_case(project))
        })
    {
        let from = entry.start_time.timestamp();
        let until = entry.end_time.map(|t| t.timestamp()).unwrap_or(now);
        if until <= from {
            continue;
        }
        remaining = remaining
            .into_iter()
            .flat_map(|stretch| subtract(stretch, (from, until)))
            .collect();
    }
    remaining
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn session(project: Option<&str>, start: i64, end: Option<i64>, subagents: usize) -> Session {
        Session {
            project: project.map(str::to_string),
            start,
            end,
            subagents,
            subagent_at: Vec::new(),
        }
    }

    /// An open mark that has never beaten, so it leans on the unvouched grace.
    fn mark(project: &str, start: i64) -> Lease {
        Lease {
            mark: crate::marks::Mark {
                project: project.to_string(),
                issue: None,
                phase: "impl".to_string(),
                start: at(start),
            },
            last_seen: None,
        }
    }

    /// The same mark, last beaten at `last_seen`.
    fn beaten(project: &str, start: i64, last_seen: i64) -> Lease {
        Lease {
            last_seen: Some(at(last_seen)),
            ..mark(project, start)
        }
    }

    fn entry(project: &str, start: i64, end: Option<i64>, tags: &[&str]) -> TimeEntry {
        TimeEntry {
            id: 1,
            description: "x".to_string(),
            project: Some(project.to_string()),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            start_time: at(start),
            end_time: end.map(at),
            idle: Vec::new(),
        }
    }

    fn at(epoch: i64) -> DateTime<Local> {
        Local.timestamp_opt(epoch, 0).unwrap()
    }

    const FLOOR: i64 = 120;
    const HOUR: i64 = 3600;

    /// The thresholds are stated here, never read from the developer's config:
    /// this shadows [`super::unaccounted`] for every test below.
    fn unaccounted(
        sessions: &[Session],
        leases: &[Lease],
        entries: &[TimeEntry],
        now: DateTime<Local>,
        floor_minutes: i64,
    ) -> Vec<Unaccounted> {
        super::unaccounted(
            sessions,
            leases,
            entries,
            now,
            Thresholds {
                gap: 45,
                unvouched: floor_minutes,
            },
        )
    }

    #[test]
    fn a_session_with_no_project_is_never_flagged() {
        let sessions = vec![session(None, 0, Some(3 * HOUR), 0)];
        assert!(unaccounted(&sessions, &[], &[], at(3 * HOUR), FLOOR).is_empty());
    }

    #[test]
    fn a_session_under_the_floor_is_never_flagged() {
        let sessions = vec![session(Some("tt"), 0, Some(60 * 60), 0)]; // 1h, floor 2h
        assert!(unaccounted(&sessions, &[], &[], at(HOUR), FLOOR).is_empty());
    }

    #[test]
    fn a_session_over_the_floor_with_no_coverage_is_flagged() {
        let sessions = vec![session(Some("tt"), 0, Some(3 * HOUR), 2)];
        let flagged = unaccounted(&sessions, &[], &[], at(3 * HOUR), FLOOR);
        assert_eq!(flagged.len(), 1);
        assert_eq!(flagged[0].project, "tt");
        assert_eq!(flagged[0].subagents, 2);
    }

    #[test]
    fn a_session_covered_by_an_overlapping_open_mark_is_not_flagged() {
        let sessions = vec![session(Some("tt"), 0, Some(3 * HOUR), 0)];
        let marks = vec![mark("tt", HOUR)];
        assert!(unaccounted(&sessions, &marks, &[], at(3 * HOUR), FLOOR).is_empty());
    }

    /// The weekend incident: four marks sat open for days with no heartbeat and
    /// silenced the warning for their own projects.
    #[test]
    fn a_mark_open_for_days_with_no_beats_no_longer_covers() {
        let sessions = vec![session(Some("tt"), 110 * HOUR, Some(113 * HOUR), 0)];
        let abandoned = vec![mark("tt", 0)];
        assert_eq!(
            unaccounted(&sessions, &abandoned, &[], at(114 * HOUR), FLOOR).len(),
            1
        );

        // The same mark, beaten ten minutes ago, still vouches.
        let live = vec![beaten("tt", 0, 114 * HOUR - 600)];
        assert!(unaccounted(&sessions, &live, &[], at(114 * HOUR), FLOOR).is_empty());
    }

    /// The incident as it happened: the session was still open, its mark was
    /// opened at the same instant and never beat, and the weekend passed.
    #[test]
    fn the_remainder_of_a_still_open_session_past_its_marks_expiry_is_flagged() {
        let sessions = vec![session(Some("tt"), 0, None, 0)];
        let abandoned = vec![mark("tt", 0)];
        let flagged = unaccounted(&sessions, &abandoned, &[], at(114 * HOUR), FLOOR);
        assert_eq!(flagged.len(), 1);
        assert_eq!(flagged[0].start, at(2 * HOUR));
        assert_eq!(flagged[0].end, at(114 * HOUR));
    }

    #[test]
    fn two_consecutive_leases_covering_a_window_between_them_leave_nothing_flagged() {
        let sessions = vec![session(Some("tt"), 0, Some(4 * HOUR), 0)];
        // The first expires 2h in (unvouched), where the second picks up.
        let leases = vec![mark("tt", 0), beaten("tt", 2 * HOUR, 4 * HOUR)];
        assert!(unaccounted(&sessions, &leases, &[], at(4 * HOUR), FLOOR).is_empty());
    }

    #[test]
    fn a_mark_for_a_different_project_does_not_cover() {
        let sessions = vec![session(Some("tt"), 0, Some(3 * HOUR), 0)];
        let marks = vec![mark("other", HOUR)];
        assert_eq!(
            unaccounted(&sessions, &marks, &[], at(3 * HOUR), FLOOR).len(),
            1
        );
    }

    /// The lease's project is parsed out of a sanitised filename, so both sides
    /// have to be normalised or a lossy name never joins.
    #[test]
    fn a_lease_for_a_lossy_project_name_still_covers_its_session() {
        let sessions = vec![session(Some("my proj"), 0, Some(3 * HOUR), 0)];
        let leases = vec![beaten("my_proj", 0, 3 * HOUR)];
        assert!(unaccounted(&sessions, &leases, &[], at(3 * HOUR), FLOOR).is_empty());
    }

    #[test]
    fn a_mark_of_a_dot_related_project_does_not_cover() {
        let sessions = vec![session(Some("app"), 0, Some(3 * HOUR), 0)];
        let leases = vec![Lease {
            mark: crate::marks::Mark {
                project: "app".to_string(),
                issue: Some("web.7".to_string()),
                phase: "impl".to_string(),
                start: at(0),
            },
            last_seen: Some(at(3 * HOUR)),
        }];
        assert_eq!(
            unaccounted(&sessions, &leases, &[], at(3 * HOUR), FLOOR).len(),
            1,
            "an app.web mark covered an app session"
        );
    }

    #[test]
    fn a_mark_that_started_after_the_window_ended_does_not_cover() {
        let sessions = vec![session(Some("tt"), 0, Some(3 * HOUR), 0)];
        let marks = vec![mark("tt", 4 * HOUR)];
        assert_eq!(
            unaccounted(&sessions, &marks, &[], at(5 * HOUR), FLOOR).len(),
            1
        );
    }

    #[test]
    fn a_session_covered_by_a_closed_agent_tagged_entry_is_not_flagged() {
        let sessions = vec![session(Some("tt"), 0, Some(3 * HOUR), 0)];
        let entries = vec![entry("tt", 0, Some(3 * HOUR), &["tt", "impl", "agent"])];
        assert!(unaccounted(&sessions, &[], &entries, at(3 * HOUR), FLOOR).is_empty());
    }

    #[test]
    fn a_logged_entry_without_the_agent_tag_does_not_cover() {
        let sessions = vec![session(Some("tt"), 0, Some(3 * HOUR), 0)];
        let entries = vec![entry("tt", 0, Some(3 * HOUR), &["tt", "impl"])];
        assert_eq!(
            unaccounted(&sessions, &[], &entries, at(3 * HOUR), FLOOR).len(),
            1
        );
    }

    #[test]
    fn an_auto_tagged_entry_covers_the_same_as_an_agent_tagged_one() {
        let sessions = vec![session(Some("tt"), 0, Some(3 * HOUR), 0)];
        let entries = vec![entry("tt", 0, Some(3 * HOUR), &["tt", "auto", "auto"])];
        assert!(
            unaccounted(&sessions, &[], &entries, at(3 * HOUR), FLOOR).is_empty(),
            "a prior --auto-log entry must stop the window from re-flagging"
        );
    }

    #[test]
    fn an_entry_covering_the_tail_leaves_the_head_flagged() {
        let sessions = vec![session(Some("tt"), 0, Some(3 * HOUR), 0)];
        let entries = vec![entry("tt", 2 * HOUR, Some(4 * HOUR), &["tt", "agent"])];
        let flagged = unaccounted(&sessions, &[], &entries, at(3 * HOUR), FLOOR);
        assert_eq!(flagged.len(), 1);
        assert_eq!((flagged[0].start, flagged[0].end), (at(0), at(2 * HOUR)));
    }

    /// Auto-logging one fragment must not hide the rest of the session.
    #[test]
    fn an_entry_covering_the_middle_leaves_the_head_and_the_tail_flagged() {
        let sessions = vec![session(Some("tt"), 0, Some(6 * HOUR), 0)];
        let entries = vec![entry("tt", 2 * HOUR, Some(4 * HOUR), &["tt", "auto"])];
        let flagged = unaccounted(&sessions, &[], &entries, at(6 * HOUR), FLOOR);
        assert_eq!(
            flagged.iter().map(|u| (u.start, u.end)).collect::<Vec<_>>(),
            vec![(at(4 * HOUR), at(6 * HOUR)), (at(0), at(2 * HOUR))]
        );
    }

    /// A session split into fragments reports each fragment's own dispatches.
    #[test]
    fn each_fragment_counts_only_the_dispatches_inside_it() {
        let dispatches = vec![
            30 * 60,
            60 * 60,
            90 * 60,
            4 * HOUR + 30 * 60,
            5 * HOUR,
            5 * HOUR + 30 * 60,
        ];
        let sessions = vec![session_with_subagents(
            "tt",
            0,
            Some(6 * HOUR),
            dispatches.clone(),
        )];
        let entries = vec![entry("tt", 2 * HOUR, Some(4 * HOUR), &["tt", "agent"])];
        let flagged = unaccounted(&sessions, &[], &entries, at(6 * HOUR), FLOOR);
        assert_eq!(flagged.len(), 2);
        assert_eq!(flagged.iter().map(|u| u.subagents).sum::<usize>(), 6);
        for item in &flagged {
            assert!(
                item.describe().contains("3 subagent dispatches"),
                "{}",
                item.describe()
            );
        }
    }

    #[test]
    fn a_still_open_session_is_measured_to_now() {
        let sessions = vec![session(Some("tt"), 0, None, 0)];
        let flagged = unaccounted(&sessions, &[], &[], at(3 * HOUR), FLOOR);
        assert_eq!(flagged.len(), 1);
        assert_eq!(flagged[0].end, at(3 * HOUR));
    }

    #[test]
    fn a_still_open_entry_is_measured_to_now_when_checking_coverage() {
        let sessions = vec![session(Some("tt"), 0, None, 0)];
        let entries = vec![entry("tt", 0, None, &["tt", "agent"])];
        assert!(unaccounted(&sessions, &[], &entries, at(3 * HOUR), FLOOR).is_empty());
    }

    #[test]
    fn results_come_back_newest_first() {
        let sessions = vec![
            session(Some("older"), 0, Some(3 * HOUR), 0),
            session(Some("newer"), HOUR, Some(4 * HOUR), 0),
        ];
        let flagged = unaccounted(&sessions, &[], &[], at(4 * HOUR), FLOOR);
        assert_eq!(
            flagged
                .iter()
                .map(|u| u.project.as_str())
                .collect::<Vec<_>>(),
            vec!["newer", "older"]
        );
    }

    // --- auto_log_after_minutes --------------------------------------
    //
    // Serialised via `crate::storage::env_guard` against every other test
    // that touches env; it is process-wide.
    use crate::storage::env_guard;

    fn set(var: &str, value: &str) {
        unsafe { std::env::set_var(var, value) };
    }

    fn unset(var: &str) {
        unsafe { std::env::remove_var(var) };
    }

    #[test]
    fn unset_leaves_auto_logging_disabled() {
        let _guard = env_guard();
        unset("TT_AUTO_LOG_AFTER_MINUTES");
        unset("TT_MAX_UNVOUCHED_MINUTES");
        crate::storage::env_sandbox("audit-auto-log-unset");

        assert_eq!(auto_log_after_minutes(), None);
    }

    #[test]
    fn a_value_over_the_unvouched_floor_is_honoured() {
        let _guard = env_guard();
        crate::storage::env_sandbox("audit-auto-log-honoured");
        unset("TT_MAX_UNVOUCHED_MINUTES"); // default floor: 120
        set("TT_AUTO_LOG_AFTER_MINUTES", "480");

        assert_eq!(auto_log_after_minutes(), Some(480));
        unset("TT_AUTO_LOG_AFTER_MINUTES");
    }

    // --- idle-gap subtraction (issue #26) -----------------------------

    fn session_with_subagents(
        project: &str,
        start: i64,
        end: Option<i64>,
        subagent_at: Vec<i64>,
    ) -> Session {
        Session {
            project: Some(project.to_string()),
            start,
            end,
            subagents: subagent_at.len(),
            subagent_at,
        }
    }

    #[test]
    fn a_window_with_no_subagent_dispatches_carries_no_idle() {
        let sessions = vec![session(Some("tt"), 0, Some(3 * HOUR), 0)];
        let flagged = unaccounted(&sessions, &[], &[], at(3 * HOUR), FLOOR);
        assert_eq!(flagged.len(), 1);
        assert!(flagged[0].idle.is_empty());
    }

    #[test]
    fn a_long_silence_between_dispatches_is_recorded_as_idle() {
        let _guard = env_guard();
        crate::storage::env_sandbox("audit-idle-gap");
        unset("TT_MAX_GAP_MINUTES"); // default: 45

        // A dispatch 10 minutes in, then nothing until the window's tail.
        let sessions = vec![session_with_subagents(
            "tt",
            0,
            Some(3 * HOUR),
            vec![10 * 60],
        )];
        let flagged = unaccounted(&sessions, &[], &[], at(3 * HOUR), FLOOR);
        assert_eq!(flagged.len(), 1);
        // The lead-in before the dispatch is under the 45m threshold and
        // does not count; the long silence after it does.
        assert_eq!(flagged[0].idle.len(), 1);
        assert_eq!(flagged[0].idle[0].start, at(10 * 60));
        assert_eq!(flagged[0].idle[0].end, at(3 * HOUR));
    }

    #[test]
    fn a_value_at_or_under_the_unvouched_floor_disables_auto_logging() {
        let _guard = env_guard();
        crate::storage::env_sandbox("audit-auto-log-misconfigured");
        set("TT_MAX_UNVOUCHED_MINUTES", "120");
        set("TT_AUTO_LOG_AFTER_MINUTES", "120");
        assert_eq!(
            auto_log_after_minutes(),
            None,
            "equal to the floor must not enable auto-logging"
        );

        set("TT_AUTO_LOG_AFTER_MINUTES", "60");
        assert_eq!(
            auto_log_after_minutes(),
            None,
            "under the floor must not enable auto-logging"
        );

        unset("TT_MAX_UNVOUCHED_MINUTES");
        unset("TT_AUTO_LOG_AFTER_MINUTES");
    }
}
