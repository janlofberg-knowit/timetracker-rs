use crossterm::event::KeyCode;
use anyhow::Result;
use chrono::{Duration, Local};
use crate::storage::PathStamp;
use super::App;
use super::types::{ConfirmAction, InputMode, PendingConfirm, ViewMode};

impl App {
    /// Record the store's fingerprint, then load it.
    ///
    /// Stamping *before* the read is deliberate: a write landing between the two
    /// leaves the stamp older than the data we hold, which costs one redundant
    /// reload. Stamping afterwards would record a fingerprint newer than the data,
    /// and the update would be lost for good.
    pub(crate) fn reload(&mut self) -> Result<()> {
        self.store_stamp = crate::storage::store_stamp();
        self.data = crate::storage::load_data()?;
        Ok(())
    }

    /// Whether the store on disk still matches the snapshot in memory.
    ///
    /// The "could this stamp change again without looking different" reasoning
    /// lives in `PathStamp::unchanged`, so the mark directory's own check next
    /// door cannot drift from it.
    pub(crate) fn store_is_unchanged(&self) -> bool {
        PathStamp::unchanged(self.store_stamp, crate::storage::store_stamp())
    }

    /// Pick up marks begun, ended or cancelled outside the TUI. Called once per
    /// event-loop tick, right beside [`sync_from_store`](Self::sync_from_store)
    /// and off the same 250 ms `event::poll`.
    ///
    /// Unguarded by `input_mode`, unlike the store: the mark list is display-only
    /// and replacing it cannot clobber text being typed. Unguarded by
    /// `show_marks` too, so `App.marks` is current the instant the surface opens
    /// rather than one tick later — while the surface is closed this costs one
    /// `stat` per tick and no directory read at all.
    pub(crate) fn sync_from_marks(&mut self) {
        let Some(dir) = crate::marks::mark_dir() else {
            self.marks.clear();
            return;
        };
        // Stamp before reading, for the reason `reload` stamps before loading: a
        // write landing between the two costs one redundant read, where stamping
        // afterwards would lose the change for good.
        let current = PathStamp::read(&dir);
        if PathStamp::unchanged(self.marks_stamp, current) {
            return;
        }
        self.marks_stamp = current;
        self.marks = crate::marks::open_marks_in(&dir);
    }

    /// Pick up writes made outside the TUI — an agent's `tt log`, another shell —
    /// without disturbing what the user is doing. Called once per event-loop tick.
    pub(crate) fn sync_from_store(&mut self) -> Result<()> {
        // Replacing `data` mid-form would clobber the text being typed. Skipping is
        // safe: a later tick picks the change up once the mode is Normal again.
        if matches!(
            self.input_mode,
            InputMode::AddingEntry | InputMode::EditingEntry | InputMode::Searching
        ) {
            return Ok(());
        }
        if self.store_is_unchanged() {
            return Ok(());
        }

        // `table_state` holds an index into `filtered_entries()`, which re-sorts on
        // every call, so the index means nothing once the data changes underneath.
        // Anchor on the selected entry's id instead.
        let previous_idx = self.table_state.selected();
        let anchor_id =
            previous_idx.and_then(|idx| self.filtered_entries().get(idx).map(|entry| entry.id));

        self.reload()?;

        let (anchored_idx, len) = {
            let entries = self.filtered_entries();
            let anchored = anchor_id.and_then(|id| entries.iter().position(|e| e.id == id));
            (anchored, entries.len())
        };
        self.table_state.select(match (anchored_idx, previous_idx) {
            (Some(idx), _) => Some(idx),
            // The anchor is gone: stay as near the old position as the new list allows.
            (None, Some(idx)) => Some(idx.min(len.saturating_sub(1))),
            (None, None) => None,
        });

        // `Confirm` is deliberately absent from the guard above (#40 decision 5), so
        // the reload can carry away the entry a prompt is currently describing. Drop
        // the prompt when that happens, rather than leaving a modal on screen that
        // asks about something no longer there. `confirm_pending` re-checks anyway —
        // this is about the frame the user is looking at, not about correctness.
        let target_vanished = self
            .pending_confirm
            .is_some_and(|pending| self.data.get_entry(pending.entry_id).is_none());
        if target_vanished {
            self.cancel_confirm();
        }
        Ok(())
    }

    pub(crate) fn next(&mut self) {
        let len = self.filtered_entries().len();
        if len == 0 {
            return;
        }
        let i = match self.table_state.selected() {
            Some(i) => (i + 1) % len,
            None => 0,
        };
        self.table_state.select(Some(i));
    }

    pub(crate) fn previous(&mut self) {
        let len = self.filtered_entries().len();
        if len == 0 {
            return;
        }
        let i = match self.table_state.selected() {
            Some(i) => {
                if i == 0 { len - 1 } else { i - 1 }
            }
            None => 0,
        };
        self.table_state.select(Some(i));
    }

    /// The entry the table cursor is on, as the current view orders it.
    pub(crate) fn selected_entry(&self) -> Option<&crate::tracker::TimeEntry> {
        let idx = self.table_state.selected()?;
        self.filtered_entries().into_iter().nth(idx)
    }

    /// `Enter` with the table focused: show the selected entry in full.
    ///
    /// Nothing selected means nothing to show — an empty view must not open an
    /// empty modal the user then has to escape from.
    pub(crate) fn open_detail(&mut self) {
        if self.selected_entry().is_some() {
            self.input_mode = InputMode::Detail;
        }
    }

    /// Remove one entry by id, then keep the table cursor inside the shorter list.
    ///
    /// By id and never by selection: `d` only reaches here through a confirmation
    /// that captured its target before the prompt went up, and the cursor may have
    /// moved since — see [`PendingConfirm`].
    pub(crate) fn delete_entry(&mut self, entry_id: u64) -> Result<()> {
        // An id that is already gone simply matches nothing — not an error.
        self.mutate_store(|data| data.entries.retain(|e| e.id != entry_id))?;

        let new_len = self.filtered_entries().len();
        let past_the_end = self
            .table_state
            .selected()
            .is_some_and(|idx| idx >= new_len);
        if past_the_end && new_len > 0 {
            self.table_state.select(Some(new_len - 1));
        }
        Ok(())
    }

    /// `[t]` in the detail popover, once confirmed: trim the entry's idle stretches
    /// away, which splits it into the pieces between them.
    ///
    /// By id for the reason [`delete_entry`](Self::delete_entry) is. Afterwards the
    /// popover stays open on the piece that kept the original id — the earliest one —
    /// **selected by id**: `selected_entry` is positional (`nth` into the re-sorted
    /// list), so inserting the later pieces would otherwise slide the overlay onto a
    /// different entry under the reader. The view can never empty here, since one
    /// entry becomes two or more, so there is no "closed it instead" case.
    pub(crate) fn trim_entry(&mut self, entry_id: u64) -> Result<()> {
        // Nothing to trim comes back as no pieces, which is why the footer only
        // advertises `[t]` when there are intervals: the key is never a surprise
        // no-op the user has to interpret.
        let pieces = self.mutate_store(|data| data.split_at_idle(entry_id))?;
        if pieces.is_empty() {
            return Ok(());
        }
        self.select_by_id(entry_id);
        Ok(())
    }

    /// Raise the confirmation prompt for a destructive action on the selected entry.
    ///
    /// **The target id is captured here, now.** `delete_entry` and `trim_entry` are
    /// given that id rather than re-reading the selection, because `sync_from_store`
    /// is deliberately *not* guarded for `Confirm` (#40 decision 5): the 250 ms poll
    /// keeps running while the prompt is up, so an agent's write can re-anchor the
    /// table cursor underneath it. Guarding the poll instead would freeze the prompt
    /// on data the store no longer has, and the user would be answering about a
    /// stale screen — so the poll stays live and the answer is pinned to an id.
    ///
    /// Nothing selected means nothing to confirm. A trim with nothing to trim is
    /// also nothing to confirm: the popover only advertises `[t]` when the entry has
    /// idle (#39), and a prompt offering to trim an entry into one unchanged piece
    /// would be a prompt whose yes does nothing.
    pub(crate) fn request_confirm(&mut self, action: ConfirmAction) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        let entry_id = entry.id;
        if action == ConfirmAction::Trim && entry.trim_spans().is_empty() {
            return;
        }
        self.pending_confirm = Some(PendingConfirm {
            action,
            entry_id,
            from: self.input_mode,
        });
        self.input_mode = InputMode::Confirm;
    }

    /// Answer the prompt with a keypress — the whole body of the `Confirm` arm, so
    /// that the key contract is testable instead of living in a `match` no test can
    /// reach.
    ///
    /// Every escape route is a no: `n`, `Esc`, **and `Enter`**. `Enter` already means
    /// "close this" in the popover, so leaving it as cancel teaches no second meaning
    /// — and an unlucky `Enter` can then never destroy anything. Anything else is
    /// ignored and the prompt stays up; the destructive answer always takes a
    /// deliberate key.
    pub(crate) fn answer_confirm(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Char(c) if self.confirms_pending(c) => self.confirm_pending()?,
            KeyCode::Char('n') | KeyCode::Esc | KeyCode::Enter => self.cancel_confirm(),
            _ => {}
        }
        Ok(())
    }

    /// Whether `c` is a yes to the prompt that is up: `y`, or the very key that
    /// raised it.
    ///
    /// The originating key and not just any repeat, so a `d` pressed on a trim
    /// prompt and a `t` pressed on a delete prompt are both inert (#40 decision 2).
    /// It lives here rather than in the event loop's `match` guard because it is a
    /// fact about the pending action, and because the guard has to be testable.
    pub(crate) fn confirms_pending(&self, c: char) -> bool {
        self.pending_confirm
            .is_some_and(|pending| c == 'y' || c == pending.action.key())
    }

    /// The yes: perform the pending action on **the captured id and nothing else**.
    ///
    /// A captured id that is no longer in the store self-cancels — the prompt
    /// described an entry that has since been written away, so the honest answer is
    /// to do nothing rather than to act on whatever took its place.
    pub(crate) fn confirm_pending(&mut self) -> Result<()> {
        let Some(pending) = self.pending_confirm.take() else {
            return Ok(());
        };
        if self.data.get_entry(pending.entry_id).is_some() {
            match pending.action {
                ConfirmAction::Delete => self.delete_entry(pending.entry_id)?,
                ConfirmAction::Trim => self.trim_entry(pending.entry_id)?,
            }
        }
        self.close_confirm(pending.from);
        Ok(())
    }

    /// The no — and every escape route: `n`, `Esc` and `Enter` all land here, so the
    /// store is untouched and the screen goes back where it was.
    pub(crate) fn cancel_confirm(&mut self) {
        let from = self
            .pending_confirm
            .take()
            .map_or(InputMode::Normal, |pending| pending.from);
        self.close_confirm(from);
    }

    /// Leave the prompt for the mode that raised it, downgrading to `Normal` when
    /// that was the popover and the view it read has since emptied — an empty modal
    /// is something the user then has to escape from for no reason.
    fn close_confirm(&mut self, from: InputMode) {
        self.pending_confirm = None;
        self.input_mode = if from == InputMode::Detail && self.selected_entry().is_none() {
            InputMode::Normal
        } else {
            from
        };
    }

    /// Put the table cursor on the row with this id, leaving it alone when the
    /// current view does not hold that entry.
    pub(crate) fn select_by_id(&mut self, id: u64) {
        if let Some(idx) = self.filtered_entries().iter().position(|e| e.id == id) {
            self.table_state.select(Some(idx));
        }
    }

    /// The detail popover's footer hints, in render order.
    ///
    /// `[t]` appears only when the selected entry actually has idle intervals, so
    /// the footer never advertises a key that would do nothing. Lives here rather
    /// than in `render` because it is a fact about the selection, and it is what
    /// the tests assert against.
    ///
    /// Both destructive keys carry a trailing `…`, the usual sign that a key opens
    /// something rather than acts: `d` and `t` now raise a confirmation, and a
    /// footer promising a bare "delete" would over-report what one keypress does.
    pub(crate) fn detail_hints(&self) -> Vec<(&'static str, &'static str)> {
        let mut hints = vec![("j/k", "move"), ("e", "edit"), ("d", "delete…")];
        if self.selected_entry().is_some_and(|e| !e.idle.is_empty()) {
            // "trim" is the user-facing verb on every surface, even though the store
            // operation it calls is `split_at_idle`.
            hints.push(("t", "trim…"));
        }
        hints.push(("esc", "close"));
        hints
    }

    pub(crate) fn stop_active(&mut self) -> Result<()> {
        self.mutate_store(|data| data.stop_active())?;
        Ok(())
    }

    pub(crate) fn next_period(&mut self) {
        match self.view_mode {
            ViewMode::All => {}
            ViewMode::Day => self.selected_date += Duration::days(1),
            ViewMode::Week => self.selected_date += Duration::days(7),
        }
        self.table_state.select(Some(0));
    }

    pub(crate) fn previous_period(&mut self) {
        match self.view_mode {
            ViewMode::All => {}
            ViewMode::Day => self.selected_date -= Duration::days(1),
            ViewMode::Week => self.selected_date -= Duration::days(7),
        }
        self.table_state.select(Some(0));
    }

    pub(crate) fn set_view_mode(&mut self, mode: ViewMode) {
        self.view_mode = mode;
        self.table_state.select(Some(0));
    }

    pub(crate) fn go_to_today(&mut self) {
        self.selected_date = Local::now().date_naive();
        self.table_state.select(Some(0));
    }

    pub(crate) fn toggle_sort_order(&mut self) {
        self.sort_order = self.sort_order.toggle();
        self.table_state.select(Some(0));
    }
}
