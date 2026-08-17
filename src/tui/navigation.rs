use anyhow::Result;
use chrono::{Duration, Local};
use super::App;
use super::types::ViewMode;

impl App {
    pub(crate) fn reload(&mut self) -> Result<()> {
        self.data = crate::storage::load_data()?;
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

    pub(crate) fn delete_selected(&mut self) -> Result<()> {
        // Resolve the id from the view, then drop the borrow: the removal itself
        // happens against the freshly loaded store, not this snapshot.
        let Some(idx) = self.table_state.selected() else {
            return Ok(());
        };
        let Some(entry_id) = self.filtered_entries().get(idx).map(|e| e.id) else {
            return Ok(());
        };

        // An id that is already gone simply matches nothing — not an error.
        self.mutate_store(|data| data.entries.retain(|e| e.id != entry_id))?;

        let new_len = self.filtered_entries().len();
        if idx >= new_len && new_len > 0 {
            self.table_state.select(Some(new_len - 1));
        }
        Ok(())
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
