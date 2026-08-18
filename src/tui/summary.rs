//! Per-project totals for the current view scope, and the state behind the
//! collapsible `Summary` surface that shows them.
//!
//! The surface is **display-only**: not on the `Tab` focus ring, no cursor, no
//! `Enter` behaviour, because the Projects pane already owns project filtering
//! and a second entry point for it would be one gesture too many. `render.rs`
//! draws it; everything it needs to draw — height, row set, title marker — is
//! decided here.
//!
//! **This folds the scope, not the view.** [`App::project_summary`] reads
//! `scope_entries()` and never `filtered_entries()`, because the question the
//! surface answers is "how did this day / week actually split across projects",
//! which stays worth answering while a filter is on. Folding whatever the table
//! is showing instead is the tempting one-word change and it is the behaviour the
//! owner rejected: filter by a project and the summary collapses to a single
//! 100% row that tells you nothing. The accepted cost is that the summary and a
//! filtered footer total disagree — #23 says so on the surface's title bar rather
//! than fixing it here.

use std::collections::HashMap;

use chrono::Duration;

use super::App;
use super::panes::surface_count;

/// Most project rows the surface shows before the rest live in the border count.
/// Six is the panes' cap (`MAX_VISIBLE_VALUES`), and a summary that grows past
/// the entries table it sits under has stopped being a footnote about the day.
const MAX_VISIBLE_PROJECTS: usize = 6;

/// The standing statement on the surface's title bar, after the scope word.
///
/// It is here rather than inlined in the renderer because it is the cue that
/// makes the whole surface honest: the summary is unfiltered while the footer
/// total is filtered, so the two disagree whenever a filter is on, and these
/// three words are what say that is intended rather than broken.
const ALL_PROJECTS: &str = "all projects";

/// The label a row carries when its entries have no project at all.
///
/// The Projects *pane* omits absence, because "no project" is not a value you can
/// filter on (`pane_values`). A summary has the opposite obligation: its parts
/// must sum to the scope total, and the live store really does hold unprojected
/// rows, so dropping them would quietly under-report the day.
pub(crate) const NO_PROJECT: &str = "(no project)";

/// One row of the summary: a project, its time in the scope, how many entries
/// made it up, and its share of the scope total.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectTotal {
    /// The project name as stored, or [`NO_PROJECT`] for the collapsed
    /// no-project row.
    pub(crate) project: String,
    /// Raw and unformatted on purpose: the caller applies `duration::format`, so
    /// these totals read identically to the footer and the CLI.
    pub(crate) total: Duration,
    pub(crate) entries: usize,
    /// Percent of the scope total, rounded to the nearest whole point.
    ///
    /// Deliberately **not** fudged to make the column sum to exactly 100 — a
    /// scope split three ways can honestly read 33/33/33 or 34/33/34, and
    /// nudging a row to close the gap would misstate that row's actual share.
    pub(crate) share: u16,
}

impl App {
    /// Per-project totals for the current view scope, largest first.
    ///
    /// Folds [`scope_entries`](Self::scope_entries) — the day / week / all set
    /// *before* the pane filter and the search term narrow it — so the answer is
    /// about the period, not about the rows currently on screen. See the module
    /// docs: swapping in `filtered_entries()` here is the whole bug.
    ///
    /// An empty scope gives an empty list, and a zero scope total gives 0% rows
    /// rather than a division by zero.
    pub(crate) fn project_summary(&self) -> Vec<ProjectTotal> {
        let entries = self.scope_entries();
        let mut totals: HashMap<&str, (Duration, usize)> = HashMap::new();
        for entry in &entries {
            // Trimmed, and empty-after-trim counted as absent, matching how the
            // form stores a blank project and how `pane_values` reads one.
            let project = entry.project.as_deref().map(str::trim).unwrap_or("");
            let key = if project.is_empty() {
                NO_PROJECT
            } else {
                project
            };
            let row = totals.entry(key).or_insert((Duration::zero(), 0));
            row.0 += entry.duration();
            row.1 += 1;
        }

        let scope_total: i64 = totals.values().map(|(d, _)| d.num_seconds()).sum();
        let mut rows: Vec<ProjectTotal> = totals
            .into_iter()
            .map(|(project, (total, entries))| ProjectTotal {
                project: project.to_string(),
                total,
                entries,
                share: share_of(total, scope_total),
            })
            .collect();
        // Largest total first, ties broken by name — `pane_values`' convention, so
        // the order is stable frame to frame instead of following the HashMap.
        rows.sort_by(|a, b| {
            b.total
                .cmp(&a.total)
                .then_with(|| a.project.cmp(&b.project))
        });
        rows
    }

    /// Height of the summary surface including borders, or 0 while it is hidden —
    /// the layout then drops the row entirely rather than reserving a zero-height
    /// one, which is exactly what makes the collapsed screen identical to the one
    /// before this surface existed.
    ///
    /// The shape is `pane_surface_height`': a box sized to its content up to the
    /// cap, and one empty row when the scope has no entries at all so the box can
    /// say so rather than collapsing to two touching borders.
    pub(crate) fn summary_surface_height(&self) -> u16 {
        if !self.show_summary {
            return 0;
        }
        2 + self.project_summary().len().clamp(1, MAX_VISIBLE_PROJECTS) as u16
    }

    /// The rows that fit in `visible_rows` of inner height, largest first.
    ///
    /// Driven off the height the frame really has rather than the cap, so a
    /// terminal too short for the full box still shows a truthful count of what
    /// it left out.
    pub(crate) fn visible_project_summary(&self, visible_rows: usize) -> Vec<ProjectTotal> {
        let mut rows = self.project_summary();
        rows.truncate(visible_rows);
        rows
    }

    /// `shown/total` once more projects exist than fit, and `None` while they all
    /// fit — the panes' and the marks box's own [`surface_count`], in its
    /// cursorless "there are more of these than you can see" reading.
    ///
    /// The one difference from the marks box is that the all-fit case stays
    /// silent instead of printing a bare total: this count is appended to a title
    /// that already reads `day · all projects`, and a `· 3` after that says
    /// nothing the three visible rows do not.
    pub(crate) fn summary_count(&self, visible_rows: usize) -> Option<String> {
        let total = self.project_summary().len();
        if total <= visible_rows {
            return None;
        }
        surface_count(None, total, visible_rows)
    }

    /// The right-aligned half of the title bar: `day · all projects`, plus
    /// `· 6/9` when rows are off screen.
    ///
    /// Present in **both** filter states — it describes what the surface covers,
    /// which is equally true when nothing is filtered — and the renderer changes
    /// only its colour, never its words, when
    /// [`total_is_filtered`](Self::total_is_filtered) makes it load-bearing.
    pub(crate) fn summary_marker(&self, visible_rows: usize) -> String {
        let mut marker = format!("{} · {}", self.view_mode.label(), ALL_PROJECTS);
        if let Some(count) = self.summary_count(visible_rows) {
            marker.push_str(" · ");
            marker.push_str(&count);
        }
        marker
    }

    /// `Shift-S`: show or hide the surface.
    ///
    /// Touches no focus, like `toggle_marks` and unlike `toggle_pane`: the
    /// surface never holds any, so there is nothing to repair when it closes.
    pub(crate) fn toggle_summary(&mut self) {
        self.show_summary = !self.show_summary;
    }
}

/// `part` as a whole-percent share of `whole`, both in seconds.
///
/// A zero `whole` is 0%: an empty scope never reaches here, but a scope of
/// zero-length entries does, and it must not divide by zero.
fn share_of(part: Duration, whole: i64) -> u16 {
    if whole <= 0 {
        return 0;
    }
    let part = part.num_seconds().max(0);
    // Round half up in integers rather than via f64, so the same input always
    // gives the same point.
    (((part * 200) / whole + 1) / 2) as u16
}
