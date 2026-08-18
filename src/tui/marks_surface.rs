//! The collapsible `Marks` surface: the open `tt-safe` phase marks, so Status
//! stops being the only answer to "is anything running?".
//!
//! It exists because the agent layer never calls `tt start` — `tt-safe begin`
//! writes a mark file and the work becomes a `tt` entry only at `tt-safe end` —
//! so `data.active_entry()` is blind to any number of phases in flight. The two
//! coexist: this surface is separate from the Status panel and neither
//! suppresses the other.
//!
//! **Display-only, deliberately.** It is not on the `Tab` focus ring, has no
//! cursor and no `Enter` behaviour, because there is nothing here to select — a
//! surface that takes focus and then ignores `Enter` has already produced one
//! "pressing enter does nothing" report on the panes. Consequently nothing
//! scrolls, and the count on its border reports how many marks exist rather than
//! where a cursor sits.
//!
//! The contents come from `App.marks`, refreshed on the event loop's existing
//! tick (`App::sync_from_marks`); a frame never reads the mark directory.

use super::App;
use super::panes::surface_count;
use crate::marks::Mark;

/// Most marks the surface lists. Three is the owner-approved shape, and the
/// newest three are the ones a person is likely to still be inside of; the
/// border count reports the rest.
const MAX_VISIBLE_MARKS: usize = 3;

impl App {
    /// The marks the surface shows: the newest [`MAX_VISIBLE_MARKS`], in
    /// `App.marks`' own newest-first order.
    pub(crate) fn visible_marks(&self) -> &[Mark] {
        let shown = self.marks.len().min(MAX_VISIBLE_MARKS);
        &self.marks[..shown]
    }

    /// Height of the marks surface including borders, or 0 while it is hidden —
    /// the layout then drops the row entirely rather than reserving a
    /// zero-height one, which is what makes the collapsed surface cost nothing.
    ///
    /// The same shape as `pane_surface_height`: a box sized to its content, so a
    /// single mark does not sit in a half-empty frame, and one empty row when
    /// there are no marks at all so the box can say so.
    pub(crate) fn marks_surface_height(&self) -> u16 {
        if !self.show_marks {
            return 0;
        }
        2 + self.marks.len().clamp(1, MAX_VISIBLE_MARKS) as u16
    }

    /// The count for the surface's top border: `N` while every open mark is on
    /// screen, `3/N` once more are open than fit. Cursorless, so it reports
    /// existence rather than position — see [`surface_count`].
    pub(crate) fn marks_count(&self, visible_rows: usize) -> Option<String> {
        surface_count(None, self.marks.len(), visible_rows)
    }

    /// `Shift-M`: show or hide the surface.
    ///
    /// Unlike `toggle_pane` this touches no focus, because the surface never has
    /// any: there is nothing to repair when it closes.
    pub(crate) fn toggle_marks(&mut self) {
        self.show_marks = !self.show_marks;
    }
}
