//! The collapsible Projects / Tags pane surface.
//!
//! Each pane offers the distinct values found in the entries of the current view
//! scope (day / week / all) *before* any filter is applied, with the number of
//! entries each one matches. Listing the unfiltered scope is what keeps a pane
//! from ever offering a value that matches nothing, and from hiding the value the
//! user has just filtered on.

use std::collections::HashMap;

use super::App;
use super::types::{Focus, Pane};
use crate::tracker::TimeEntry;

/// Most value rows a pane shows before it starts scrolling. The surface is sized
/// to its content up to this, so a short list does not leave the box half empty.
const MAX_VISIBLE_VALUES: usize = 6;

impl App {
    /// The entries of the current view scope, before the tag filter and the
    /// search term narrow them down. `filtered_entries` starts from this.
    pub(crate) fn scope_entries(&self) -> Vec<&TimeEntry> {
        use super::types::ViewMode;
        use crate::tracker::TimeData;
        match self.view_mode {
            ViewMode::All => self.data.entries.iter().collect(),
            ViewMode::Day => self.data.entries_for_date(self.selected_date),
            ViewMode::Week => {
                let week_start = TimeData::week_start(self.selected_date);
                self.data.entries_for_week(week_start)
            }
        }
    }

    pub(crate) fn pane_is_visible(&self, pane: Pane) -> bool {
        match pane {
            Pane::Projects => self.show_projects,
            Pane::Tags => self.show_tags,
        }
    }

    /// The open panes, left to right: Projects then Tags.
    pub(crate) fn visible_panes(&self) -> Vec<Pane> {
        [Pane::Projects, Pane::Tags]
            .into_iter()
            .filter(|pane| self.pane_is_visible(*pane))
            .collect()
    }

    /// Distinct values in the current scope with their match counts, most used
    /// first and ties broken by name so the order is stable.
    ///
    /// An entry with no project contributes no Projects row — "no project" is an
    /// absence, not a value to filter on — and a tag repeated within one entry is
    /// still only one match.
    pub(crate) fn pane_values(&self, pane: Pane) -> Vec<(String, usize)> {
        let entries = self.scope_entries();
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for entry in &entries {
            match pane {
                Pane::Projects => {
                    let project = entry.project.as_deref().map(str::trim).unwrap_or("");
                    if !project.is_empty() {
                        *counts.entry(project).or_default() += 1;
                    }
                }
                Pane::Tags => {
                    let mut seen: Vec<&str> = Vec::new();
                    for tag in &entry.tags {
                        let tag = tag.trim();
                        if tag.is_empty() || seen.contains(&tag) {
                            continue;
                        }
                        seen.push(tag);
                        *counts.entry(tag).or_default() += 1;
                    }
                }
            }
        }

        let mut values: Vec<(String, usize)> = counts
            .into_iter()
            .map(|(value, count)| (value.to_string(), count))
            .collect();
        values.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        values
    }

    /// Height of the pane surface including borders, or 0 when both panes are
    /// hidden — the layout then drops the row entirely rather than reserving a
    /// zero-height one, so the surface really costs nothing.
    pub(crate) fn pane_surface_height(&self) -> u16 {
        let panes = self.visible_panes();
        if panes.is_empty() {
            return 0;
        }
        let longest = panes
            .iter()
            .map(|pane| self.pane_values(*pane).len())
            .max()
            .unwrap_or(0);
        2 + longest.clamp(1, MAX_VISIBLE_VALUES) as u16
    }

    pub(crate) fn pane_cursor(&self, pane: Pane) -> usize {
        let cursor = match pane {
            Pane::Projects => self.project_cursor,
            Pane::Tags => self.tag_cursor,
        };
        let len = self.pane_values(pane).len();
        cursor.min(len.saturating_sub(1))
    }

    pub(crate) fn focused_pane(&self) -> Option<Pane> {
        match self.focus {
            Focus::Pane(pane) if self.pane_is_visible(pane) => Some(pane),
            _ => None,
        }
    }

    /// Show or hide a pane. Hiding the focused one hands focus back to the table,
    /// so `j`/`k` never move a cursor that is no longer on screen.
    pub(crate) fn toggle_pane(&mut self, pane: Pane) {
        match pane {
            Pane::Projects => self.show_projects = !self.show_projects,
            Pane::Tags => self.show_tags = !self.show_tags,
        }
        if !self.pane_is_visible(pane) && self.focus == Focus::Pane(pane) {
            self.focus = Focus::Table;
        }
    }

    /// `Tab`: entries table → each visible pane, left to right → back.
    pub(crate) fn cycle_focus(&mut self) {
        let mut order = vec![Focus::Table];
        order.extend(self.visible_panes().into_iter().map(Focus::Pane));
        let current = order.iter().position(|f| *f == self.focus).unwrap_or(0);
        self.focus = order[(current + 1) % order.len()];
    }

    /// `j` inside the focused pane. Returns false when nothing is focused, so the
    /// caller can fall through to moving the table selection.
    pub(crate) fn pane_next(&mut self) -> bool {
        self.move_pane_cursor(1)
    }

    /// `k` inside the focused pane.
    pub(crate) fn pane_previous(&mut self) -> bool {
        self.move_pane_cursor(-1)
    }

    fn move_pane_cursor(&mut self, delta: isize) -> bool {
        let Some(pane) = self.focused_pane() else {
            return false;
        };
        let len = self.pane_values(pane).len();
        if len == 0 {
            return true;
        }
        let current = self.pane_cursor(pane) as isize;
        let next = (current + delta).rem_euclid(len as isize) as usize;
        match pane {
            Pane::Projects => self.project_cursor = next,
            Pane::Tags => self.tag_cursor = next,
        }
        true
    }
}
