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

    /// `position/total` for a pane whose values do not all fit in `visible_rows`,
    /// or `None` when they do.
    ///
    /// The cap at [`MAX_VISIBLE_VALUES`] means a long list scrolls, and a list that
    /// scrolls with nothing on screen to say so hides values outright — the top
    /// value slides away under `j` and looks like it never existed. This is the
    /// "there is more, and here is where you are" the box would otherwise lack.
    pub(crate) fn pane_scroll_indicator(&self, pane: Pane, visible_rows: usize) -> Option<String> {
        let total = self.pane_values(pane).len();
        if total <= visible_rows {
            return None;
        }
        Some(format!("{}/{}", self.pane_cursor(pane) + 1, total))
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
        self.shift_focus(1);
    }

    /// `Shift-Tab`: the exact inverse of [`cycle_focus`](Self::cycle_focus).
    pub(crate) fn cycle_focus_back(&mut self) {
        self.shift_focus(-1);
    }

    /// Walk the focus ring — the table followed by the visible panes, left to
    /// right — by `delta` steps, wrapping. A focus pointing at a pane that is not
    /// on the ring (a pane hidden while focused) reads as the table, so both
    /// directions recover to the same place.
    fn shift_focus(&mut self, delta: isize) {
        let mut order = vec![Focus::Table];
        order.extend(self.visible_panes().into_iter().map(Focus::Pane));
        let current = order.iter().position(|f| *f == self.focus).unwrap_or(0) as isize;
        let len = order.len() as isize;
        self.focus = order[(current + delta).rem_euclid(len) as usize];
    }

    /// The values selected in a pane, i.e. what it contributes to the filter.
    pub(crate) fn pane_selection(&self, pane: Pane) -> &[String] {
        match pane {
            Pane::Projects => &self.selected_projects,
            Pane::Tags => &self.selected_tags,
        }
    }

    /// Whether a value a pane is offering is in that pane's selection. Both sides
    /// come from [`pane_values`](Self::pane_values), so this is an exact match; the
    /// filter predicates stay case-insensitive because the *store* is not
    /// normalised.
    pub(crate) fn pane_value_is_selected(&self, pane: Pane, value: &str) -> bool {
        self.pane_selection(pane).iter().any(|v| v == value)
    }

    /// `Enter` inside the focused pane: toggle the value under that pane's cursor
    /// into or out of its selection set.
    ///
    /// Returns false when no pane has focus, so `Enter` on the entries table falls
    /// through to whatever that arm does — `App.focus` is what disambiguates the
    /// one key, and the table's meaning is the detail popover.
    pub(crate) fn toggle_pane_value(&mut self) -> bool {
        let Some(pane) = self.focused_pane() else {
            return false;
        };
        let cursor = self.pane_cursor(pane);
        let Some((value, _)) = self.pane_values(pane).into_iter().nth(cursor) else {
            return true;
        };
        let selection = match pane {
            Pane::Projects => &mut self.selected_projects,
            Pane::Tags => &mut self.selected_tags,
        };
        match selection.iter().position(|v| v == &value) {
            Some(pos) => {
                selection.remove(pos);
            }
            None => selection.push(value),
        }
        // The row that was selected is very unlikely to still be the same row.
        self.table_state.select(Some(0));
        true
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
