//! Per-project totals and the state behind the collapsible `Summary`
//! surface. Display-only; `render/surfaces.rs` draws it.
//!
//! **Two modes, picked by `summary_follows_filters`:** scope-only, the
//! default, folds `scope_entries()`, so a filter leaves it alone. Follow
//! mode folds `filtered_entries()` — both panes and the `/` search term.

use std::collections::HashMap;

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Timelike};

use super::App;
use super::panes::surface_count;
use super::types::{Focus, ViewMode};
use crate::tracker::{TimeData, TimeEntry};

/// Most project rows the surface shows before the rest live in the border count.
const MAX_VISIBLE_PROJECTS: usize = 6;

/// The rule and the total row under the project rows.
pub(crate) const SUMMARY_TOTAL_LINES: u16 = 2;

/// The scope-only statement on the title bar, after the scope word.
const ALL_PROJECTS: &str = "all projects";

/// The follow-mode statement in its place.
const FILTERED: &str = "filtered";

/// The label for entries with no project; counted so the rows sum to the scope.
pub(crate) const NO_PROJECT: &str = "(no project)";

/// The period one heat-strip cell covers, chosen by the view mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Grain {
    Hour,
    Day,
    Week,
    Month,
}

impl Grain {
    pub(crate) fn for_view(view: ViewMode) -> Self {
        match view {
            ViewMode::Day => Grain::Hour,
            ViewMode::Week => Grain::Day,
            ViewMode::All => Grain::Week,
            ViewMode::Overview => Grain::Month,
        }
    }

    /// The one-word period name, for the column head over the strip.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Grain::Hour => "hour",
            Grain::Day => "day",
            Grain::Week => "week",
            Grain::Month => "month",
        }
    }
}

/// One row: a project, its time in the scope, its entry count and its share.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectTotal {
    /// The name as stored, or [`NO_PROJECT`].
    pub(crate) project: String,
    /// Raw; the caller applies `duration::format`.
    pub(crate) total: Duration,
    /// `human + agent` is always `total`, so the columns cannot disagree.
    pub(crate) human: Duration,
    pub(crate) agent: Duration,
    pub(crate) entries: usize,
    /// Percent of the folded total, rounded, and **not** fudged to sum to 100.
    pub(crate) share: u16,
}

impl App {
    /// The entry set both the rows and the heat strips fold.
    fn summary_entries(&self) -> Vec<&TimeEntry> {
        if self.summary_follows_filters {
            self.filtered_entries()
        } else {
            self.scope_entries()
        }
    }

    /// Per-project totals, largest first. Folds
    /// [`filtered_entries`](Self::filtered_entries) while
    /// `summary_follows_filters` is on — the panes and the `/` search term —
    /// else [`scope_entries`](Self::scope_entries). Nothing folded gives an
    /// empty list.
    pub(crate) fn project_summary(&self) -> Vec<ProjectTotal> {
        let entries = self.summary_entries();
        // (total, human, agent, entries)
        let mut totals: HashMap<&str, (Duration, Duration, Duration, usize)> = HashMap::new();
        for entry in &entries {
            let row = totals.entry(project_key(entry)).or_insert((
                Duration::zero(),
                Duration::zero(),
                Duration::zero(),
                0,
            ));
            let duration = entry.duration();
            row.0 += duration;
            if entry.is_agent() {
                row.2 += duration;
            } else {
                row.1 += duration;
            }
            row.3 += 1;
        }

        let folded_total: i64 = totals.values().map(|row| row.0.num_seconds()).sum();
        let mut rows: Vec<ProjectTotal> = totals
            .into_iter()
            .map(|(project, (total, human, agent, entries))| ProjectTotal {
                project: project.to_string(),
                total,
                human,
                agent,
                entries,
                share: share_of(total, folded_total),
            })
            .collect();
        // Ties broken by name, so the order is stable rather than the HashMap's.
        rows.sort_by(|a, b| {
            b.total
                .cmp(&a.total)
                .then_with(|| a.project.cmp(&b.project))
        });
        rows
    }

    /// The grain of the current view plus one bucket list per row of `rows`,
    /// index for index, oldest bucket first. Every entry's whole duration goes
    /// to the bucket holding its `start_time`, as `year_breakdown` does, so a
    /// row's buckets sum to its [`ProjectTotal::total`]. Sheds nothing: the
    /// renderer owns the width and must take its maximum over what it draws.
    pub(crate) fn project_buckets(&self, rows: &[ProjectTotal]) -> (Grain, Vec<Vec<Duration>>) {
        let grain = Grain::for_view(self.view_mode);
        let entries = self.summary_entries();
        let Some((anchor, count)) = bucket_range(grain, &entries, self.selected_date) else {
            return (grain, vec![Vec::new(); rows.len()]);
        };

        let mut buckets = vec![vec![Duration::zero(); count]; rows.len()];
        let row_of: HashMap<&str, usize> = rows
            .iter()
            .enumerate()
            .map(|(index, row)| (row.project.as_str(), index))
            .collect();
        for entry in &entries {
            let Some(&row) = row_of.get(project_key(entry)) else {
                continue;
            };
            let slot = bucket_index(grain, anchor, entry.start_time.naive_local());
            if let Ok(slot) = usize::try_from(slot)
                && let Some(cell) = buckets[row].get_mut(slot)
            {
                *cell += entry.duration();
            }
        }
        (grain, buckets)
    }

    /// Height including borders. Collapsed, the box is the total row alone;
    /// expanded, the header and the capped rows sit over the rule and the total.
    pub(crate) fn summary_surface_height(&self) -> u16 {
        if !self.show_summary {
            return 3;
        }
        let rows = self.project_summary().len();
        // The empty box says why it is empty, heads no columns and sums nothing.
        let header = u16::from(rows > 0);
        let total = if rows > 0 { SUMMARY_TOTAL_LINES } else { 0 };
        2 + header + total + rows.clamp(1, MAX_VISIBLE_PROJECTS) as u16
    }

    /// The title bar's right half: `day · all projects`, or `day · filtered`
    /// while following, plus `· 6/9` when rows are off screen and `· split`
    /// while the split is on. The word says the mode; only its colour follows
    /// the filter state.
    pub(crate) fn summary_marker(&self, rows: &[ProjectTotal], visible_rows: usize) -> String {
        let mode = if self.summary_follows_filters {
            FILTERED
        } else {
            ALL_PROJECTS
        };
        let mut marker = format!("{} · {mode}", self.view_mode.label());
        if let Some(count) = summary_count(rows, visible_rows) {
            marker.push_str(" · ");
            marker.push_str(&count);
        }
        if self.summary_split {
            marker.push_str(" · split");
        }
        marker
    }

    /// Focus never reads as resting on a hidden Summary.
    pub(crate) fn summary_is_focused(&self) -> bool {
        self.focus == Focus::Summary && self.show_summary
    }

    /// `v`: show or hide the human / agent columns.
    pub(crate) fn toggle_summary_split(&mut self) {
        self.summary_split = !self.summary_split;
        self.persist_layout();
    }

    /// `f`: fold the filtered entries or the whole scope.
    pub(crate) fn toggle_summary_follows_filters(&mut self) {
        self.summary_follows_filters = !self.summary_follows_filters;
        self.persist_layout();
    }

    /// `Shift-S`: show or hide the surface; opening it focuses it.
    pub(crate) fn toggle_summary(&mut self) {
        self.show_summary = !self.show_summary;
        if self.show_summary {
            self.focus = Focus::Summary;
        } else if self.focus == Focus::Summary {
            self.focus = self.focus_after_hiding();
        }
        self.persist_layout();
    }
}

/// One row over **every** project row, on screen or not; `share` is unused.
pub(crate) fn summary_total(rows: &[ProjectTotal]) -> ProjectTotal {
    rows.iter().fold(
        ProjectTotal {
            project: "total".to_string(),
            total: Duration::zero(),
            human: Duration::zero(),
            agent: Duration::zero(),
            entries: 0,
            share: 0,
        },
        |mut acc, row| {
            acc.total += row.total;
            acc.human += row.human;
            acc.agent += row.agent;
            acc.entries += row.entries;
            acc
        },
    )
}

/// The leading rows the box has room for. Takes what `project_summary` folded,
/// so one frame folds the scope once.
pub(crate) fn visible_project_summary(
    rows: &[ProjectTotal],
    visible_rows: usize,
) -> &[ProjectTotal] {
    &rows[..visible_rows.min(rows.len())]
}

/// `shown/total` once more projects exist than fit, else `None`.
pub(crate) fn summary_count(rows: &[ProjectTotal], visible_rows: usize) -> Option<String> {
    if rows.len() <= visible_rows {
        return None;
    }
    surface_count(None, rows.len(), visible_rows)
}

/// The summary row an entry belongs to: its trimmed project, or [`NO_PROJECT`].
/// Empty-after-trim counts as absent, as the form and `pane_values` do.
fn project_key(entry: &TimeEntry) -> &str {
    let project = entry.project.as_deref().map(str::trim).unwrap_or("");
    if project.is_empty() {
        NO_PROJECT
    } else {
        project
    }
}

/// The first bucket's start and how many buckets follow it, or `None` when
/// nothing is folded. `Day` is the seven days of the selected week and `Month`
/// the twelve months of the selected year; `Hour` and `Week` span the entries.
fn bucket_range(
    grain: Grain,
    entries: &[&TimeEntry],
    selected: NaiveDate,
) -> Option<(NaiveDateTime, usize)> {
    let earliest = entries.iter().map(|e| e.start_time.naive_local()).min()?;
    let latest = entries.iter().map(|e| e.start_time.naive_local()).max()?;
    let midnight = |date: NaiveDate| date.and_hms_opt(0, 0, 0).unwrap();
    let anchor = match grain {
        Grain::Hour => floor_hour(earliest),
        Grain::Day => midnight(TimeData::week_start(selected)),
        Grain::Week => midnight(TimeData::week_start(earliest.date())),
        Grain::Month => midnight(NaiveDate::from_ymd_opt(selected.year(), 1, 1)?),
    };
    let count = match grain {
        Grain::Day => 7,
        Grain::Month => 12,
        _ => (bucket_index(grain, anchor, latest) + 1).max(1) as usize,
    };
    Some((anchor, count))
}

/// How many grains `start` sits past `anchor`. Reads the start alone, never the
/// span, so an entry is never shared between the buckets it runs through.
fn bucket_index(grain: Grain, anchor: NaiveDateTime, start: NaiveDateTime) -> i64 {
    match grain {
        Grain::Hour => (floor_hour(start) - anchor).num_hours(),
        Grain::Day => (start.date() - anchor.date()).num_days(),
        Grain::Week => (start.date() - anchor.date()).num_days().div_euclid(7),
        Grain::Month => {
            (i64::from(start.year()) - i64::from(anchor.year())) * 12 + i64::from(start.month())
                - i64::from(anchor.month())
        }
    }
}

fn floor_hour(at: NaiveDateTime) -> NaiveDateTime {
    at.date().and_hms_opt(at.hour(), 0, 0).unwrap()
}

/// `part` as a whole-percent share of `whole`, in seconds; a zero `whole` is 0%.
fn share_of(part: Duration, whole: i64) -> u16 {
    if whole <= 0 {
        return 0;
    }
    let part = part.num_seconds().max(0);
    // Round half up in integers, so the same input always gives the same point.
    (((part * 200) / whole + 1) / 2) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(year, month, day)
            .unwrap()
            .and_hms_opt(hour, minute, 0)
            .unwrap()
    }

    #[test]
    fn every_view_mode_maps_to_its_own_grain() {
        assert_eq!(Grain::for_view(ViewMode::Day), Grain::Hour);
        assert_eq!(Grain::for_view(ViewMode::Week), Grain::Day);
        assert_eq!(Grain::for_view(ViewMode::All), Grain::Week);
        assert_eq!(Grain::for_view(ViewMode::Overview), Grain::Month);
    }

    /// The index keys on the start alone, so a span never reaches the next bucket.
    #[test]
    fn a_bucket_index_counts_grains_from_the_anchor_to_the_start() {
        let anchor = at(2026, 1, 1, 0, 0);
        assert_eq!(
            bucket_index(Grain::Hour, anchor, at(2026, 1, 1, 23, 30)),
            23
        );
        assert_eq!(bucket_index(Grain::Hour, anchor, at(2026, 1, 2, 0, 30)), 24);
        assert_eq!(bucket_index(Grain::Day, anchor, at(2026, 1, 2, 0, 30)), 1);
        assert_eq!(bucket_index(Grain::Week, anchor, at(2026, 1, 15, 9, 0)), 2);
        assert_eq!(
            bucket_index(Grain::Month, anchor, at(2026, 12, 31, 9, 0)),
            11
        );
        assert_eq!(bucket_index(Grain::Month, anchor, at(2027, 2, 1, 9, 0)), 13);
    }

    #[test]
    fn a_grain_names_the_period_one_cell_covers() {
        assert_eq!(Grain::Hour.label(), "hour");
        assert_eq!(Grain::Month.label(), "month");
    }
}
