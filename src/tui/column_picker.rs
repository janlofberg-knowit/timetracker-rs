//! The `c` popup's state: seeds ranks from `entry_columns`, tracks the
//! cursor, and derives the applied order from typed digits.

use super::App;
use super::render::columns::EntryColumn;
use super::types::InputMode;

impl App {
    /// Seeds ranks from the live order and opens the picker. Reseeding on
    /// every open is what makes `Esc` free.
    pub(crate) fn open_column_picker(&mut self) {
        let mut ranks = [None; 7];
        for (position, column) in self.entry_columns.iter().enumerate() {
            if let Some(index) = EntryColumn::ALL_WITH_PROJECT
                .iter()
                .position(|c| c == column)
            {
                ranks[index] = Some(position as u8 + 1);
            }
        }
        self.column_ranks = ranks;
        self.column_cursor = 0;
        self.input_mode = InputMode::ColumnPicker;
    }

    pub(crate) fn column_picker_move(&mut self, delta: isize) {
        let len = EntryColumn::ALL_WITH_PROJECT.len() as isize;
        let next = (self.column_cursor as isize + delta).rem_euclid(len);
        self.column_cursor = next as usize;
    }

    pub(crate) fn column_picker_set_rank(&mut self, digit: u8) {
        self.column_ranks[self.column_cursor] = Some(digit);
    }

    pub(crate) fn column_picker_clear(&mut self) {
        self.column_ranks[self.column_cursor] = None;
    }

    /// The numbered columns, sorted by `(rank, canonical index)`. Ties keep
    /// canonical order; duplicate ranks are legal.
    pub(crate) fn column_picker_order(&self) -> Vec<EntryColumn> {
        let mut numbered: Vec<(u8, usize, EntryColumn)> = EntryColumn::ALL_WITH_PROJECT
            .into_iter()
            .enumerate()
            .filter_map(|(index, column)| {
                self.column_ranks[index].map(|rank| (rank, index, column))
            })
            .collect();
        numbered.sort_by_key(|(rank, index, _)| (*rank, *index));
        numbered.into_iter().map(|(_, _, column)| column).collect()
    }

    /// `Enter`: applies the derived order and persists it. Nothing numbered
    /// changes nothing and stays in the picker.
    pub(crate) fn column_picker_apply(&mut self) {
        let order = self.column_picker_order();
        if order.is_empty() {
            return;
        }
        self.entry_columns = order;
        self.persist_layout();
        self.input_mode = InputMode::Normal;
    }

    pub(crate) fn column_picker_cancel(&mut self) {
        self.input_mode = InputMode::Normal;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::env_guard;
    use crate::storage::env_sandbox as sandbox;
    use crate::tracker::TimeData;

    fn seeded_app() -> App {
        crate::storage::save_data(&TimeData {
            entries: Vec::new(),
            next_id: 0,
            schema_version: 1,
        })
        .unwrap();
        App::new().unwrap()
    }

    #[test]
    fn opening_reproduces_a_non_default_order() {
        let _guard = env_guard();
        sandbox("column-picker-open");
        let mut app = seeded_app();
        app.entry_columns = vec![
            EntryColumn::Duration,
            EntryColumn::Project,
            EntryColumn::Date,
        ];

        app.open_column_picker();

        assert_eq!(app.input_mode, InputMode::ColumnPicker);
        assert_eq!(app.column_cursor, 0);
        assert_eq!(
            app.column_picker_order(),
            vec![
                EntryColumn::Duration,
                EntryColumn::Project,
                EntryColumn::Date
            ]
        );
    }

    #[test]
    fn clearing_a_rank_drops_the_column_from_the_order() {
        let _guard = env_guard();
        sandbox("column-picker-clear");
        let mut app = seeded_app();
        app.entry_columns = EntryColumn::ALL.to_vec();
        app.open_column_picker();

        // Date is index 0 in `ALL_WITH_PROJECT`.
        app.column_cursor = 0;
        app.column_picker_clear();

        assert!(!app.column_picker_order().contains(&EntryColumn::Date));
    }

    #[test]
    fn equal_ranks_come_out_in_canonical_order() {
        let _guard = env_guard();
        sandbox("column-picker-ties");
        let mut app = seeded_app();
        app.open_column_picker();

        // Duration (index 6) and Date (index 0) both ranked 1: Date is
        // canonically first.
        let date_index = EntryColumn::ALL_WITH_PROJECT
            .iter()
            .position(|c| *c == EntryColumn::Date)
            .unwrap();
        let duration_index = EntryColumn::ALL_WITH_PROJECT
            .iter()
            .position(|c| *c == EntryColumn::Duration)
            .unwrap();
        app.column_ranks = [None; 7];
        app.column_ranks[date_index] = Some(1);
        app.column_ranks[duration_index] = Some(1);

        assert_eq!(
            app.column_picker_order(),
            vec![EntryColumn::Date, EntryColumn::Duration]
        );
    }

    #[test]
    fn applying_with_everything_cleared_changes_nothing() {
        let _guard = env_guard();
        sandbox("column-picker-apply-empty");
        let mut app = seeded_app();
        let original = app.entry_columns.clone();
        app.open_column_picker();
        app.column_ranks = [None; 7];

        app.column_picker_apply();

        assert_eq!(app.entry_columns, original);
        assert_eq!(app.input_mode, InputMode::ColumnPicker);
    }

    #[test]
    fn applying_writes_the_new_names_and_keeps_other_layout_keys() {
        let _guard = env_guard();
        let dir = sandbox("column-picker-apply-writes");
        std::fs::write(
            dir.join("config.toml"),
            "[layout]\nshow_projects = true\ncolumns = [\"date\", \"start\", \"end\", \"description\", \"tags\", \"duration\"]\n",
        )
        .unwrap();
        let mut app = seeded_app();
        app.open_column_picker();
        let date_index = EntryColumn::ALL_WITH_PROJECT
            .iter()
            .position(|c| *c == EntryColumn::Date)
            .unwrap();
        let project_index = EntryColumn::ALL_WITH_PROJECT
            .iter()
            .position(|c| *c == EntryColumn::Project)
            .unwrap();
        app.column_ranks = [None; 7];
        app.column_ranks[project_index] = Some(1);
        app.column_ranks[date_index] = Some(2);

        app.column_picker_apply();

        assert_eq!(
            app.entry_columns,
            vec![EntryColumn::Project, EntryColumn::Date]
        );
        assert_eq!(app.input_mode, InputMode::Normal);
        let saved = saved_layout();
        assert_eq!(
            saved["columns"].as_array().map(|a| a
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect::<Vec<_>>()),
            Some(vec!["project".to_string(), "date".to_string()])
        );
        assert_eq!(saved["show_projects"].as_bool(), Some(true));
    }

    /// Reads the config file the picker wrote, off disk rather than through
    /// `config::load`, whose resolved value is cached for the process.
    fn saved_layout() -> toml::Value {
        let path = std::env::var("TT_CONFIG_FILE").unwrap();
        let text = std::fs::read_to_string(path).expect("a written config file");
        let doc: toml::Value = toml::from_str(&text).expect("valid TOML");
        doc["layout"].clone()
    }
}
