//! Per-project totals for the current view scope — the computation half of the
//! Summary surface (#21). The rendering, the `Shift-S` toggle and the help line
//! are #23; nothing consumes this module yet.
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
