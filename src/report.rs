//! Rollups over the store — the `tt report` surface.
//!
//! Ported from `bin/tt-report`, a 229-line Python script that read `data.json`
//! directly. That was the last part of the agent workflow a `cargo install` could
//! not carry, and the last thing parsing the store behind the binary's back.
//!
//! **The project axis changed in the port, deliberately.** The script derived the
//! project from the tags — the first tag with no `/` that was not a phase — and
//! since the tag convention emits `#<project>/<issue> #<phase> #agent`, that tag
//! was always `agent`. On real data it filed the great majority of agent-logged
//! work under a project literally called `agent`. Project is a real field
//! ([`TimeEntry::project`]), so this groups on the field and the numbers change
//! because they were wrong, not because the port drifted.

use std::collections::BTreeMap;

use chrono::{Duration, Local, NaiveDate};
use serde::Serialize;

use crate::agent::PHASES;
use crate::duration as fmt_duration;
use crate::icons;
use crate::tracker::{TimeData, TimeEntry};

/// The bucket an entry with no `project` field lands in.
///
/// The script called this `(untagged)`, which described how it guessed rather than
/// what it found. Nothing has been untagged since project became a field.
pub const NO_PROJECT: &str = "(no project)";

/// One item's totals — an item being a `<project>/<issue>` tag, or the bare
/// description when an entry carries no such tag.
#[derive(Debug, Default)]
pub struct ItemNode {
    pub seconds: i64,
    /// Seconds per phase, for the ` · `-joined breakdown. Empty when no entry
    /// under this item carried a phase tag.
    pub phases: BTreeMap<String, i64>,
    /// The first contributing entry's description, shown when it differs from the
    /// item key — exactly the script's `item["entries"][0]["description"]`.
    pub description: String,
    /// Whether any contributing entry is still running.
    pub active: bool,
}

/// One project's totals and its items.
#[derive(Debug, Default)]
pub struct ProjectNode {
    pub seconds: i64,
    pub items: BTreeMap<String, ItemNode>,
}

/// A whole rollup: the tree, the grand total, and the overlap count.
#[derive(Debug, Default)]
pub struct Rollup {
    pub projects: BTreeMap<String, ProjectNode>,
    pub total_seconds: i64,
    /// Overlapping *pairs*, not entries — see [`count_overlaps`].
    pub overlaps: usize,
}

impl Rollup {
    pub fn is_empty(&self) -> bool {
        self.projects.is_empty()
    }
}

/// Split an entry's tags into its item and phase axes.
///
/// Stored tags carry **no** `#` — `tracker::parse_tags` strips the prefix before
/// saving — so these compare bare words. The item is the first tag containing a
/// `/`; the phase is the first tag in [`PHASES`]; `agent` matches neither and is
/// ignored, which is the whole reason the project axis had to move to the field.
pub fn classify(tags: &[String]) -> (Option<&str>, Option<&str>) {
    let mut item = None;
    let mut phase = None;
    for tag in tags {
        if tag.contains('/') {
            if item.is_none() {
                item = Some(tag.as_str());
            }
        } else if phase.is_none() && PHASES.contains(&tag.as_str()) {
            phase = Some(tag.as_str());
        }
    }
    (item, phase)
}

/// An entry's billable seconds. Never negative: an entry whose end precedes its
/// start is corrupt, and reporting it as negative time would corrupt the totals
/// around it too.
fn entry_seconds(entry: &TimeEntry) -> i64 {
    entry.duration().num_seconds().max(0)
}

/// Count overlapping *pairs* of spans.
///
/// Reproduces `bin/tt-report:117-126`, early `break` included. The break is
/// correct rather than a bug: the list is sorted by start, so once a successor
/// starts at or after `first`'s end, every later successor does too.
///
/// The count matters because `tt log` back-dates from now (`end = now`,
/// `start = now - duration`), so entries logged in a batch claim overlapping
/// evening slots. Daily totals stay right; the timeline does not. This is what
/// keeps that visible.
pub fn count_overlaps(entries: &[&TimeEntry]) -> usize {
    let mut ordered: Vec<&&TimeEntry> = entries.iter().collect();
    ordered.sort_by_key(|entry| entry.start_time);

    let mut pairs = 0;
    for (i, first) in ordered.iter().enumerate() {
        let first_end = first.end_time.unwrap_or_else(Local::now);
        for second in &ordered[i + 1..] {
            if second.start_time >= first_end {
                break;
            }
            pairs += 1;
        }
    }
    pairs
}

/// Build the rollup for a set of entries.
pub fn rollup(entries: &[&TimeEntry]) -> Rollup {
    let mut result = Rollup {
        overlaps: count_overlaps(entries),
        ..Default::default()
    };

    for entry in entries {
        let seconds = entry_seconds(entry);
        let (item, phase) = classify(&entry.tags);
        let project = entry.project.as_deref().unwrap_or(NO_PROJECT);
        // An item tag if there is one, else the description — the same fallback
        // the script used, so a hand-written entry still gets its own row.
        let key = item.unwrap_or(entry.description.as_str());

        result.total_seconds += seconds;
        let project_node = result.projects.entry(project.to_string()).or_default();
        project_node.seconds += seconds;

        let item_node = project_node.items.entry(key.to_string()).or_default();
        if item_node.description.is_empty() {
            item_node.description = entry.description.clone();
        }
        item_node.seconds += seconds;
        item_node.active |= entry.end_time.is_none();
        if let Some(phase) = phase {
            *item_node.phases.entry(phase.to_string()).or_insert(0) += seconds;
        }
    }

    result
}

/// Keys of a map, ordered by descending value with the name as the tie-break.
///
/// The script's tie order was dict-insertion order — first appearance in the store
/// — which is neither stable under editing nor testable. Alphabetical ties are
/// both, at the cost of two equal-length adjacent rows swapping against the
/// script's output.
fn by_seconds_desc<'a, T>(
    map: &'a BTreeMap<String, T>,
    seconds: impl Fn(&T) -> i64,
) -> Vec<&'a str> {
    let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
    keys.sort_by(|a, b| {
        seconds(&map[*b])
            .cmp(&seconds(&map[*a]))
            .then_with(|| a.cmp(b))
    });
    keys
}

fn seconds_to_duration(seconds: i64) -> Duration {
    Duration::seconds(seconds)
}

/// Truncate to `n` **characters**, not bytes — descriptions are free text and
/// byte slicing panics on a multi-byte boundary.
fn truncate(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

/// The human rollup, keeping `bin/tt-report:129-159`'s columns and content.
pub fn render(rollup: &Rollup, label: &str) -> String {
    if rollup.is_empty() {
        return format!("No entries for {label}.\n");
    }

    let mut out = String::new();
    out.push_str(&format!(
        "{} {label} — {}\n\n",
        icons::CALENDAR,
        fmt_duration::format(seconds_to_duration(rollup.total_seconds))
    ));

    for project in by_seconds_desc(&rollup.projects, |node| node.seconds) {
        let node = &rollup.projects[project];
        out.push_str(&format!(
            "{:<28} {:>8}\n",
            project,
            fmt_duration::format(seconds_to_duration(node.seconds))
        ));

        for key in by_seconds_desc(&node.items, |item| item.seconds) {
            let item = &node.items[key];
            let marker = if item.active { " *" } else { "" };
            out.push_str(&format!(
                "  {:<26} {:>8}{}\n",
                truncate(key, 26),
                fmt_duration::format(seconds_to_duration(item.seconds)),
                marker
            ));
            // The description earns its own line only when it says something the
            // key does not.
            if item.description != key && !item.description.is_empty() {
                out.push_str(&format!("      {}\n", truncate(&item.description, 60)));
            }
            if !item.phases.is_empty() {
                let breakdown: Vec<String> = by_seconds_desc(&item.phases, |secs| *secs)
                    .into_iter()
                    .map(|phase| {
                        format!(
                            "{phase} {}",
                            fmt_duration::format(seconds_to_duration(item.phases[phase]))
                        )
                    })
                    .collect();
                out.push_str(&format!("      {}\n", breakdown.join(" · ")));
            }
        }
        out.push('\n');
    }

    if rollup.overlaps > 0 {
        out.push_str(&format!(
            "{} {} overlapping span(s) — retro `tt log` back-dates from now, so\n",
            icons::WARNING,
            rollup.overlaps
        ));
        out.push_str(
            "totals are right but the timeline is not. Log at each commit, not in batches.\n",
        );
    }

    out
}

/// The `--json` payload.
///
/// Keeps `bin/tt-report:162-182`'s key names and nesting, with **three
/// deliberate deltas** — recorded here so a later reader does not take them for
/// accidents:
///
/// 1. `projects` keys are the `project` **field**, and the empty bucket is
///    [`NO_PROJECT`] rather than the script's `(untagged)`.
/// 2. `seconds` and `total_seconds` are whole integers, not the script's floats.
/// 3. `label` is additive, so a consumer can echo back the scope it asked for.
///
/// Safe because nothing reads the old shape programmatically: its only references
/// were prose in the wrapper repo's agent instructions. Anything keying off the
/// old names still keys off these; only the values under `projects` move, and they
/// move because they were wrong.
#[derive(Debug, Serialize)]
pub struct JsonReport {
    pub label: String,
    pub total_seconds: i64,
    pub overlaps: usize,
    pub projects: BTreeMap<String, JsonProject>,
}

#[derive(Debug, Serialize)]
pub struct JsonProject {
    pub seconds: i64,
    pub items: BTreeMap<String, JsonItem>,
}

#[derive(Debug, Serialize)]
pub struct JsonItem {
    pub seconds: i64,
    pub phases: BTreeMap<String, i64>,
    pub description: String,
    pub active: bool,
}

pub fn to_json(rollup: &Rollup, label: &str) -> JsonReport {
    JsonReport {
        label: label.to_string(),
        total_seconds: rollup.total_seconds,
        overlaps: rollup.overlaps,
        projects: rollup
            .projects
            .iter()
            .map(|(name, node)| {
                (
                    name.clone(),
                    JsonProject {
                        seconds: node.seconds,
                        items: node
                            .items
                            .iter()
                            .map(|(key, item)| {
                                (
                                    key.clone(),
                                    JsonItem {
                                        seconds: item.seconds,
                                        phases: item.phases.clone(),
                                        description: item.description.clone(),
                                        active: item.active,
                                    },
                                )
                            })
                            .collect(),
                    },
                )
            })
            .collect(),
    }
}

/// What period a report covers, and what to call it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    /// Inclusive lower bound; `None` means unbounded (`--all`).
    pub from: Option<NaiveDate>,
    /// Inclusive upper bound; `None` means "up to the end of the data".
    pub until: Option<NaiveDate>,
    pub label: String,
}

/// Resolve the scope flags into bounds and a label, reproducing
/// `bin/tt-report:196-211`.
///
/// Note `--week` has a lower bound only — the script sets no upper bound for it,
/// so a week report includes anything dated after today, which a fabricated or
/// mis-clocked entry can be.
pub fn resolve_scope(
    today: NaiveDate,
    all: bool,
    week: bool,
    since: Option<NaiveDate>,
    until: Option<NaiveDate>,
    project: Option<&str>,
) -> Scope {
    let mut scope = if all {
        Scope {
            from: None,
            until: None,
            label: "All entries".to_string(),
        }
    } else if week {
        let start = TimeData::week_start(today);
        Scope {
            from: Some(start),
            until: None,
            label: format!("Week of {start}"),
        }
    } else if let Some(since) = since {
        Scope {
            from: Some(since),
            until: None,
            label: format!("Since {since}"),
        }
    } else {
        Scope {
            from: Some(today),
            until: Some(today),
            label: format!("Today, {today}"),
        }
    };

    // `--until` only ever narrows, and clap requires a scope alongside it, so it
    // can never silently overwrite the default day the way the script's did.
    if let Some(until) = until {
        scope.until = Some(until);
    }
    if let Some(project) = project {
        scope.label.push_str(&format!(" — #{project}"));
    }
    scope
}

/// The entries a scope selects: dated inside its bounds, and matching its project
/// filter on the **field**.
pub fn select<'a>(data: &'a TimeData, scope: &Scope, project: Option<&str>) -> Vec<&'a TimeEntry> {
    data.entries
        .iter()
        .filter(|entry| {
            let date = entry.start_time.date_naive();
            scope.from.is_none_or(|from| date >= from)
                && scope.until.is_none_or(|until| date <= until)
                && project.is_none_or(|wanted| entry.project.as_deref() == Some(wanted))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(hour: u32, minute: u32) -> chrono::DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 8, 18, hour, minute, 0)
            .single()
            .expect("a real local time")
    }

    fn entry(
        id: u64,
        project: Option<&str>,
        tags: &[&str],
        from: (u32, u32),
        minutes: i64,
    ) -> TimeEntry {
        let start = at(from.0, from.1);
        TimeEntry {
            id,
            description: "did the thing".to_string(),
            project: project.map(str::to_string),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            start_time: start,
            end_time: Some(start + Duration::minutes(minutes)),
            idle: Vec::new(),
        }
    }

    #[test]
    fn an_agent_shaped_entry_rolls_up_under_its_project_not_under_agent() {
        // The bug the port exists to fix: `agent` is a tag on every agent-written
        // entry, and the script took it for the project.
        let e = entry(1, Some("vinge"), &["vinge/6", "plan", "agent"], (9, 0), 30);
        let rolled = rollup(&[&e]);

        assert!(
            rolled.projects.contains_key("vinge"),
            "grouped on the project field"
        );
        assert!(
            !rolled.projects.contains_key("agent"),
            "the provenance tag is not a project"
        );
        let node = &rolled.projects["vinge"];
        assert_eq!(node.items.keys().collect::<Vec<_>>(), vec!["vinge/6"]);
        assert_eq!(node.items["vinge/6"].phases["plan"], 30 * 60);
    }

    #[test]
    fn an_entry_with_no_item_tag_is_keyed_on_its_description() {
        let e = entry(1, Some("tt"), &["impl", "agent"], (9, 0), 15);
        let rolled = rollup(&[&e]);
        assert_eq!(
            rolled.projects["tt"].items.keys().collect::<Vec<_>>(),
            vec!["did the thing"]
        );
    }

    #[test]
    fn an_entry_with_no_project_field_buckets_under_no_project() {
        let e = entry(1, None, &["plan"], (9, 0), 15);
        let rolled = rollup(&[&e]);
        assert_eq!(rolled.projects.keys().collect::<Vec<_>>(), vec![NO_PROJECT]);
    }

    #[test]
    fn phase_seconds_sum_across_entries_under_one_item() {
        let a = entry(1, Some("tt"), &["tt/12", "plan"], (9, 0), 30);
        let b = entry(2, Some("tt"), &["tt/12", "plan"], (10, 0), 15);
        let c = entry(3, Some("tt"), &["tt/12", "impl"], (11, 0), 60);
        let rolled = rollup(&[&a, &b, &c]);

        let item = &rolled.projects["tt"].items["tt/12"];
        assert_eq!(item.phases["plan"], 45 * 60, "two plan passes add up");
        assert_eq!(item.phases["impl"], 60 * 60);
        assert_eq!(item.seconds, 105 * 60);
        assert_eq!(rolled.total_seconds, 105 * 60);
    }

    #[test]
    fn an_active_entry_is_marked_and_measured_to_now() {
        let mut e = entry(1, Some("tt"), &["tt/12", "impl"], (9, 0), 30);
        e.start_time = Local::now() - Duration::minutes(20);
        e.end_time = None;

        let rolled = rollup(&[&e]);
        let item = &rolled.projects["tt"].items["tt/12"];
        assert!(item.active, "a running entry is flagged");
        // Measured against now, so it is about twenty minutes rather than zero.
        assert!(
            item.seconds >= 19 * 60 && item.seconds <= 21 * 60,
            "measured to now, got {}s",
            item.seconds
        );
    }

    #[test]
    fn overlaps_count_pairs_and_the_early_break_does_not_undercount() {
        // Three spans: the first two overlap each other, the third starts after
        // the first ends but before the second does. One pair with the first, one
        // with the second — the `break` must not hide the second.
        let a = entry(1, Some("tt"), &["tt/1"], (9, 0), 60); // 09:00–10:00
        let b = entry(2, Some("tt"), &["tt/2"], (9, 30), 60); // 09:30–10:30
        let c = entry(3, Some("tt"), &["tt/3"], (10, 15), 30); // 10:15–10:45

        assert_eq!(count_overlaps(&[&a, &b, &c]), 2, "a×b and b×c, not a×c");
        assert_eq!(count_overlaps(&[&a]), 0, "one span overlaps nothing");
    }

    #[test]
    fn a_corrupt_backwards_entry_contributes_no_negative_time() {
        let mut e = entry(1, Some("tt"), &["tt/12", "impl"], (9, 0), 30);
        e.end_time = Some(at(8, 0));
        assert_eq!(entry_seconds(&e), 0, "clamped, not negative");
    }

    #[test]
    fn classify_ignores_the_provenance_tag_and_takes_the_first_of_each_axis() {
        let tags: Vec<String> = ["agent", "vinge/6", "vinge/9", "plan", "impl"]
            .iter()
            .map(|t| t.to_string())
            .collect();
        assert_eq!(classify(&tags), (Some("vinge/6"), Some("plan")));
        assert_eq!(classify(&[]), (None, None));
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("a real date")
    }

    #[test]
    fn the_default_scope_is_today_bounded_at_both_ends() {
        let today = date(2026, 8, 18);
        let scope = resolve_scope(today, false, false, None, None, None);
        assert_eq!(scope.from, Some(today));
        assert_eq!(scope.until, Some(today));
        assert_eq!(scope.label, "Today, 2026-08-18");
    }

    #[test]
    fn all_is_unbounded_and_week_starts_on_monday_with_no_upper_bound() {
        // 2026-08-18 is a Tuesday, so the week starts the day before.
        let today = date(2026, 8, 18);

        let all = resolve_scope(today, true, false, None, None, None);
        assert_eq!((all.from, all.until), (None, None));
        assert_eq!(all.label, "All entries");

        let week = resolve_scope(today, false, true, None, None, None);
        assert_eq!(week.from, Some(date(2026, 8, 17)), "Monday");
        assert_eq!(
            week.until, None,
            "the script sets no upper bound for a week"
        );
        assert_eq!(week.label, "Week of 2026-08-17");
    }

    #[test]
    fn since_and_until_bound_the_range_and_project_suffixes_any_label() {
        let today = date(2026, 8, 18);
        let scope = resolve_scope(
            today,
            false,
            false,
            Some(date(2026, 8, 1)),
            Some(date(2026, 8, 5)),
            Some("vinge"),
        );
        assert_eq!(scope.from, Some(date(2026, 8, 1)));
        assert_eq!(scope.until, Some(date(2026, 8, 5)));
        assert_eq!(scope.label, "Since 2026-08-01 — #vinge");
    }

    #[test]
    fn an_empty_rollup_says_so_and_renders_nothing_else() {
        let rendered = render(&Rollup::default(), "Today, 2026-08-18");
        assert_eq!(rendered, "No entries for Today, 2026-08-18.\n");
    }

    #[test]
    fn the_human_form_carries_the_header_the_rows_and_the_phase_breakdown() {
        let a = entry(1, Some("tt"), &["tt/12", "plan", "agent"], (9, 0), 30);
        let b = entry(2, Some("tt"), &["tt/12", "impl", "agent"], (11, 0), 60);
        let rendered = render(&rollup(&[&a, &b]), "Today, 2026-08-18");

        assert!(
            rendered.starts_with(&format!(
                "{} Today, 2026-08-18 — 1h 30m\n\n",
                icons::CALENDAR
            )),
            "header carries the icon, the label and the total: {rendered:?}"
        );
        assert!(
            rendered.contains("tt                             1h 30m\n"),
            "{rendered:?}"
        );
        assert!(
            rendered.contains("  tt/12                        1h 30m\n"),
            "{rendered:?}"
        );
        // Longest phase first, joined with the middot.
        assert!(
            rendered.contains("      impl 1h 0m · plan 0h 30m\n"),
            "{rendered:?}"
        );
        // The description line appears because it differs from the item key.
        assert!(rendered.contains("      did the thing\n"), "{rendered:?}");
        assert!(
            !rendered.contains("overlapping"),
            "two disjoint spans warn about nothing"
        );
    }

    #[test]
    fn the_overlap_warning_appears_only_when_spans_collide() {
        let a = entry(1, Some("tt"), &["tt/1", "impl"], (9, 0), 60);
        let b = entry(2, Some("tt"), &["tt/2", "impl"], (9, 30), 60);
        let rendered = render(&rollup(&[&a, &b]), "Today, 2026-08-18");
        assert!(
            rendered.contains(&format!("{} 1 overlapping span(s)", icons::WARNING)),
            "{rendered:?}"
        );
        assert!(rendered.contains("Log at each commit, not in batches."));
    }

    #[test]
    fn an_active_entry_is_starred_in_the_item_row() {
        let mut e = entry(1, Some("tt"), &["tt/12", "impl"], (9, 0), 30);
        e.end_time = None;
        let rendered = render(&rollup(&[&e]), "Today");
        assert!(
            rendered
                .lines()
                .any(|l| l.starts_with("  tt/12") && l.ends_with(" *")),
            "{rendered:?}"
        );
    }

    #[test]
    fn a_long_key_and_description_are_truncated_by_characters_not_bytes() {
        // Multi-byte throughout: byte slicing would panic mid-character.
        let mut e = entry(1, Some("tt"), &[], (9, 0), 30);
        e.description = "å".repeat(80);
        let rendered = render(&rollup(&[&e]), "Today");
        // The key is the description here, so it is cut at 26 and the separate
        // description line does not appear (they are equal before truncation).
        assert!(rendered.contains(&"å".repeat(26)), "{rendered:?}");
        assert!(!rendered.contains(&"å".repeat(27)), "cut at 26 characters");
    }

    #[test]
    fn select_filters_on_the_date_bounds_and_on_the_project_field() {
        let inside = entry(1, Some("vinge"), &["vinge/6", "plan"], (9, 0), 30);
        let mut earlier = entry(2, Some("vinge"), &["vinge/7", "plan"], (9, 0), 30);
        earlier.start_time = at(9, 0) - Duration::days(3);
        earlier.end_time = Some(earlier.start_time + Duration::minutes(30));
        let other_project = entry(3, Some("tt"), &["tt/12", "plan"], (10, 0), 30);

        let data = TimeData {
            entries: vec![inside.clone(), earlier.clone(), other_project.clone()],
            ..Default::default()
        };

        let today = date(2026, 8, 18);
        let day = resolve_scope(today, false, false, None, None, None);
        assert_eq!(select(&data, &day, None).len(), 2, "the two dated today");

        let all = resolve_scope(today, true, false, None, None, None);
        assert_eq!(select(&data, &all, None).len(), 3);
        assert_eq!(
            select(&data, &all, Some("vinge"))
                .iter()
                .map(|e| e.id)
                .collect::<Vec<_>>(),
            vec![1, 2],
            "filtered on the field, not on a tag"
        );
    }

    #[test]
    fn the_json_payload_keeps_the_scripts_keys_with_integer_seconds() {
        let agent_shaped = entry(1, Some("vinge"), &["vinge/6", "plan", "agent"], (9, 0), 30);
        let fieldless = entry(2, None, &["plan"], (9, 15), 15);
        let rolled = rollup(&[&agent_shaped, &fieldless]);

        let json = serde_json::to_value(to_json(&rolled, "Today, 2026-08-18"))
            .expect("the payload serialises");

        // The script's own keys, unchanged.
        assert_eq!(json["total_seconds"], 45 * 60);
        assert_eq!(json["overlaps"], 1, "the two spans collide");
        // Additive, so a consumer can echo back what it asked for.
        assert_eq!(json["label"], "Today, 2026-08-18");

        // Project keys are the field, and the empty bucket is not `(untagged)`.
        let projects = json["projects"].as_object().expect("an object");
        let mut names: Vec<&str> = projects.keys().map(String::as_str).collect();
        names.sort();
        assert_eq!(names, vec![NO_PROJECT, "vinge"]);
        assert!(
            !projects.contains_key("agent"),
            "the provenance tag is not a project"
        );

        let item = &json["projects"]["vinge"]["items"]["vinge/6"];
        assert_eq!(item["seconds"], 30 * 60);
        assert_eq!(item["phases"]["plan"], 30 * 60);
        assert_eq!(item["description"], "did the thing");
        assert_eq!(item["active"], false);

        // Integers throughout, not the script's floats — no `.0` anywhere.
        assert!(json["total_seconds"].is_i64());
        assert!(item["seconds"].is_i64());
    }
}
