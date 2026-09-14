//! Per-project totals and the state behind the collapsible `Summary`
//! surface. Display-only; `render.rs` draws it.
//!
//! **Two modes, picked by `summary_follows_filters`:** scope-only, the
//! default, folds `scope_entries()`, so a filter leaves it alone. Follow
//! mode folds `filtered_entries()` — both panes and the `/` search term.

use std::collections::HashMap;

use chrono::Duration;

use super::App;
use super::panes::surface_count;
use super::types::Focus;

/// Most project rows the surface shows before the rest live in the border count.
const MAX_VISIBLE_PROJECTS: usize = 6;

/// The scope-only statement on the title bar, after the scope word.
const ALL_PROJECTS: &str = "all projects";

/// The follow-mode statement in its place.
const FILTERED: &str = "filtered";

/// The label for entries with no project; counted so the rows sum to the scope.
pub(crate) const NO_PROJECT: &str = "(no project)";

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
    /// Per-project totals, largest first. Folds
    /// [`filtered_entries`](Self::filtered_entries) while
    /// `summary_follows_filters` is on — the panes and the `/` search term —
    /// else [`scope_entries`](Self::scope_entries). Nothing folded gives an
    /// empty list.
    pub(crate) fn project_summary(&self) -> Vec<ProjectTotal> {
        let entries = if self.summary_follows_filters {
            self.filtered_entries()
        } else {
            self.scope_entries()
        };
        // (total, human, agent, entries) in one fold, so the parts cannot drift.
        let mut totals: HashMap<&str, (Duration, Duration, Duration, usize)> = HashMap::new();
        for entry in &entries {
            // Empty-after-trim counts as absent, as the form and `pane_values` do.
            let project = entry.project.as_deref().map(str::trim).unwrap_or("");
            let key = if project.is_empty() {
                NO_PROJECT
            } else {
                project
            };
            let row = totals.entry(key).or_insert((
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

    /// Height including borders, or 0 while hidden so the layout drops the row.
    /// The header row is counted here and subtracted from the renderer's row
    /// budget; the two must stay in step or the marker outruns the rows drawn.
    pub(crate) fn summary_surface_height(&self) -> u16 {
        if !self.show_summary {
            return 0;
        }
        let rows = self.project_summary().len();
        // The empty box says why it is empty and heads no columns.
        let header = u16::from(rows > 0);
        2 + header + rows.clamp(1, MAX_VISIBLE_PROJECTS) as u16
    }

    pub(crate) fn visible_project_summary(&self, visible_rows: usize) -> Vec<ProjectTotal> {
        let mut rows = self.project_summary();
        rows.truncate(visible_rows);
        rows
    }

    /// `shown/total` once more projects exist than fit, else `None`.
    pub(crate) fn summary_count(&self, visible_rows: usize) -> Option<String> {
        let total = self.project_summary().len();
        if total <= visible_rows {
            return None;
        }
        surface_count(None, total, visible_rows)
    }

    /// The title bar's right half: `day · all projects`, or `day · filtered`
    /// while following, plus `· 6/9` when rows are off screen and `· split`
    /// while the split is on. The word says the mode, not the filter state;
    /// only its colour follows the filter.
    pub(crate) fn summary_marker(&self, visible_rows: usize) -> String {
        let mode = if self.summary_follows_filters {
            FILTERED
        } else {
            ALL_PROJECTS
        };
        let mut marker = format!("{} · {mode}", self.view_mode.label());
        if let Some(count) = self.summary_count(visible_rows) {
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

    /// `v`: show or hide the human / agent columns. Runtime state only, so this
    /// writes nothing to disk.
    pub(crate) fn toggle_summary_split(&mut self) {
        self.summary_split = !self.summary_split;
    }

    /// `f`: fold the filtered entries or the whole scope. Runtime state only,
    /// so this writes nothing to disk.
    pub(crate) fn toggle_summary_follows_filters(&mut self) {
        self.summary_follows_filters = !self.summary_follows_filters;
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

/// `part` as a whole-percent share of `whole`, in seconds; a zero `whole` is 0%.
fn share_of(part: Duration, whole: i64) -> u16 {
    if whole <= 0 {
        return 0;
    }
    let part = part.num_seconds().max(0);
    // Round half up in integers, so the same input always gives the same point.
    (((part * 200) / whole + 1) / 2) as u16
}
