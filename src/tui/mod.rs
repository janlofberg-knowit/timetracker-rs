use anyhow::Result;
use chrono::{Local, NaiveDate};
use std::io::{self, Stdout};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend, widgets::TableState};
use crate::storage::{StoreStamp, load_data};
use crate::tracker::TimeData;

pub mod theme;
pub mod types;
mod search;
mod navigation;
mod entry_form;
mod render;

pub use types::{InputField, InputMode, SortOrder, ViewMode};

pub(crate) struct App {
    pub(crate) data: TimeData,
    pub(crate) table_state: TableState,
    pub(crate) should_quit: bool,
    pub(crate) view_mode: ViewMode,
    pub(crate) selected_date: NaiveDate,
    pub(crate) input_mode: InputMode,
    pub(crate) input_field: InputField,
    pub(crate) input_description: String,
    pub(crate) input_project: String,
    pub(crate) input_tags: String,
    pub(crate) input_start_time: String,
    pub(crate) input_end_time: String,
    pub(crate) input_duration: String,
    pub(crate) search_term: String,
    pub(crate) tag_filter: Vec<String>,
    pub(crate) editing_entry_id: Option<u64>,
    pub(crate) sort_order: SortOrder,
    /// Cursor position within the currently active input field (char index, not byte index).
    pub(crate) cursor_pos: usize,
    /// Fingerprint of the store as of the last load, so the event loop can spot
    /// writes made outside the TUI without reading the file every tick.
    pub(crate) store_stamp: Option<StoreStamp>,
}

impl App {
    fn new() -> Result<Self> {
        // Stamp before loading — see `App::reload`.
        let store_stamp = StoreStamp::read();
        Ok(Self {
            data: load_data()?,
            store_stamp,
            table_state: TableState::default().with_selected(Some(0)),
            should_quit: false,
            view_mode: ViewMode::Day,
            selected_date: Local::now().date_naive(),
            input_mode: InputMode::Normal,
            input_field: InputField::Description,
            input_description: String::new(),
            input_project: String::new(),
            input_tags: String::new(),
            input_start_time: String::new(),
            input_end_time: String::new(),
            input_duration: String::new(),
            search_term: String::new(),
            tag_filter: Vec::new(),
            editing_entry_id: None,
            sort_order: SortOrder::NewestFirst,
            cursor_pos: 0,
        })
    }

    /// Apply `edit` to the store under its exclusive lock, then refresh the
    /// in-memory view from what actually landed.
    ///
    /// `App.data` is loaded once at startup, so mutating that snapshot and saving
    /// it back would rewrite the whole file and silently drop anything written
    /// since — and reuse a stale `next_id`. Every TUI mutation goes through here
    /// instead: the intent is computed from the view, but applied to the freshly
    /// loaded store.
    pub(crate) fn mutate_store<T>(&mut self, edit: impl FnOnce(&mut TimeData) -> T) -> Result<T> {
        let (result, fresh) = crate::storage::with_data(|data| {
            let result = edit(data);
            Ok((result, data.clone()))
        })?;
        self.data = fresh;
        // Our own write moved the file on; stamping it here keeps the next tick
        // from reloading what we already hold.
        self.store_stamp = StoreStamp::read();
        Ok(result)
    }
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

pub fn run_tui() -> Result<()> {
    let mut terminal = setup_terminal()?;
    let mut app = App::new()?;

    loop {
        terminal.draw(|f| render::ui(f, &mut app))?;

        if event::poll(std::time::Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match app.input_mode {
                        InputMode::Normal => match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => {
                                if app.is_searching() {
                                    app.clear_search();
                                } else if app.is_tag_filtering() {
                                    app.clear_tag_filter();
                                } else {
                                    app.should_quit = true;
                                }
                            }
                            KeyCode::Char('j') | KeyCode::Down => app.next(),
                            KeyCode::Char('k') | KeyCode::Up => app.previous(),
                            KeyCode::Char('d') => app.delete_selected()?,
                            KeyCode::Char('s') => app.stop_active()?,
                            KeyCode::Char('r') => app.reload()?,
                            KeyCode::Char('a') => app.start_adding(),
                            KeyCode::Char('e') => app.start_editing(),
                            KeyCode::Char('f') => app.filter_by_selected_tags(),
                            KeyCode::Char('/') => app.start_search(),
                            KeyCode::Char('1') => app.set_view_mode(ViewMode::Day),
                            KeyCode::Char('2') => app.set_view_mode(ViewMode::Week),
                            KeyCode::Char('3') => app.set_view_mode(ViewMode::All),
                            KeyCode::Char('h') | KeyCode::Left => app.previous_period(),
                            KeyCode::Char('l') | KeyCode::Right => app.next_period(),
                            KeyCode::Char('t') => app.go_to_today(),
                            KeyCode::Char('o') => app.toggle_sort_order(),
                            KeyCode::Char('?') => app.input_mode = InputMode::Help,
                            _ => {}
                        },
                        InputMode::AddingEntry => match key.code {
                            KeyCode::Esc => app.cancel_adding(),
                            KeyCode::Enter => app.submit_entry()?,
                            KeyCode::Tab => app.next_input_field(),
                            KeyCode::BackTab => app.prev_input_field(),
                            KeyCode::Backspace => app.handle_input_backspace(),
                            KeyCode::Left => {
                                if key.modifiers.contains(KeyModifiers::CONTROL) {
                                    app.move_cursor_word_left();
                                } else {
                                    app.move_cursor_left();
                                }
                            }
                            KeyCode::Right => {
                                if key.modifiers.contains(KeyModifiers::CONTROL) {
                                    app.move_cursor_word_right();
                                } else {
                                    app.move_cursor_right();
                                }
                            }
                            KeyCode::Char(c) => app.handle_input_char(c),
                            _ => {}
                        },
                        InputMode::EditingEntry => match key.code {
                            KeyCode::Esc => app.cancel_adding(),
                            KeyCode::Enter => app.submit_edit()?,
                            KeyCode::Tab => app.next_input_field(),
                            KeyCode::BackTab => app.prev_input_field(),
                            KeyCode::Backspace => app.handle_input_backspace(),
                            KeyCode::Left => {
                                if key.modifiers.contains(KeyModifiers::CONTROL) {
                                    app.move_cursor_word_left();
                                } else {
                                    app.move_cursor_left();
                                }
                            }
                            KeyCode::Right => {
                                if key.modifiers.contains(KeyModifiers::CONTROL) {
                                    app.move_cursor_word_right();
                                } else {
                                    app.move_cursor_right();
                                }
                            }
                            KeyCode::Char(c) => app.handle_input_char(c),
                            _ => {}
                        },
                        InputMode::Searching => match key.code {
                            KeyCode::Esc => app.clear_search(),
                            KeyCode::Enter => app.confirm_search(),
                            KeyCode::Backspace => app.handle_search_backspace(),
                            KeyCode::Left => {
                                if key.modifiers.contains(KeyModifiers::CONTROL) {
                                    app.move_cursor_word_left();
                                } else {
                                    app.move_cursor_left();
                                }
                            }
                            KeyCode::Right => {
                                if key.modifiers.contains(KeyModifiers::CONTROL) {
                                    app.move_cursor_word_right();
                                } else {
                                    app.move_cursor_right();
                                }
                            }
                            KeyCode::Char(c) => app.handle_search_char(c),
                            _ => {}
                        },
                        InputMode::Help => match key.code {
                            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                                app.input_mode = InputMode::Normal;
                            }
                            _ => {}
                        },
                    }
                }
            }
        }

        // The 250 ms poll above is the loop's clock: whether it returned a key or
        // timed out, this is where we notice a store written from outside.
        app.sync_from_store()?;

        if app.should_quit {
            break;
        }
    }

    restore_terminal(&mut terminal)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage;
    use crate::tracker::TimeEntry;
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    /// Serialises the tests that repoint `HOME`, since it is process-wide.
    fn env_guard() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Point `HOME` at a fresh scratch dir so `ProjectDirs` resolves the store
    /// inside it. Tests must never touch the user's real store.
    fn sandbox(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tt-store-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("HOME", &dir) };
        let path = storage::get_data_path().unwrap();
        assert!(
            path.starts_with(&dir),
            "sandbox HOME not in effect: {path:?}"
        );
        dir
    }

    fn entry(id: u64, description: &str) -> TimeEntry {
        TimeEntry {
            id,
            description: description.to_string(),
            project: None,
            tags: Vec::new(),
            start_time: Local::now(),
            end_time: None,
        }
    }

    fn seed(entries: Vec<TimeEntry>, next_id: u64) {
        storage::save_data(&TimeData {
            entries,
            next_id,
            schema_version: 1,
        })
        .unwrap();
    }

    /// A write from outside the TUI, through the same `with_data` path `tt log`
    /// uses — i.e. what an agent session does while the TUI sits on its snapshot.
    fn agent_write(description: &str) -> u64 {
        storage::with_data(|data| {
            Ok(data
                .add_entry(
                    description.to_string(),
                    Some("probe".to_string()),
                    vec!["probe".to_string()],
                    Local::now(),
                    Some(Local::now()),
                )
                .id)
        })
        .unwrap()
    }

    fn on_disk() -> TimeData {
        storage::load_data().unwrap()
    }

    fn descriptions(data: &TimeData) -> Vec<&str> {
        data.entries
            .iter()
            .map(|e| e.description.as_str())
            .collect()
    }

    fn select(app: &mut App, description: &str) {
        let idx = app
            .filtered_entries()
            .iter()
            .position(|e| e.description == description)
            .expect("entry not in view");
        app.table_state.select(Some(idx));
    }

    #[test]
    fn delete_keeps_a_concurrent_agent_write() {
        let _guard = env_guard();
        sandbox("delete");
        seed(vec![entry(0, "keep"), entry(1, "doomed")], 2);

        let mut app = App::new().unwrap();
        agent_write("probe");
        select(&mut app, "doomed");
        app.delete_selected().unwrap();

        let data = on_disk();
        assert_eq!(descriptions(&data), vec!["keep", "probe"]);
        assert_eq!(descriptions(&app.data), vec!["keep", "probe"]);
    }

    #[test]
    fn deleting_an_already_removed_id_is_a_no_op() {
        let _guard = env_guard();
        sandbox("delete-gone");
        seed(vec![entry(0, "keep"), entry(1, "doomed")], 2);

        let mut app = App::new().unwrap();
        select(&mut app, "doomed");
        // Someone else removed it first, then wrote an entry of their own
        storage::with_data(|data| {
            data.entries.retain(|e| e.id != 1);
            Ok(())
        })
        .unwrap();
        agent_write("probe");

        app.delete_selected().unwrap();

        let data = on_disk();
        assert_eq!(descriptions(&data), vec!["keep", "probe"]);
    }

    #[test]
    fn stop_active_keeps_a_concurrent_agent_write() {
        let _guard = env_guard();
        sandbox("stop");
        seed(vec![entry(0, "running")], 1);

        let mut app = App::new().unwrap();
        agent_write("probe");
        app.stop_active().unwrap();

        let data = on_disk();
        assert_eq!(descriptions(&data), vec!["running", "probe"]);
        assert!(
            data.entries[0].end_time.is_some(),
            "the active entry should have been stopped"
        );
    }

    #[test]
    fn add_keeps_a_concurrent_agent_write_and_takes_a_fresh_id() {
        let _guard = env_guard();
        sandbox("add");
        seed(vec![entry(0, "existing")], 1);

        let mut app = App::new().unwrap();
        // The agent claims id 1, which the TUI's snapshot still thinks is free
        let agent_id = agent_write("probe");
        assert_eq!(agent_id, 1);

        app.start_adding();
        app.input_description = "from the tui".to_string();
        app.input_duration = "15m".to_string();
        app.submit_entry().unwrap();

        let data = on_disk();
        assert_eq!(
            descriptions(&data),
            vec!["existing", "probe", "from the tui"]
        );
        let tui_id = data
            .entries
            .iter()
            .find(|e| e.description == "from the tui")
            .unwrap()
            .id;
        assert_ne!(tui_id, agent_id, "the TUI entry reused the agent's id");
        assert_eq!(tui_id, 2);
        let mut ids: Vec<u64> = data.entries.iter().map(|e| e.id).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "duplicate ids in the store");
    }

    fn selected_description(app: &App) -> String {
        let idx = app.table_state.selected().expect("nothing selected");
        app.filtered_entries()[idx].description.clone()
    }

    #[test]
    fn sync_picks_up_an_outside_write_and_keeps_the_selection() {
        let _guard = env_guard();
        sandbox("sync");
        seed(vec![entry(0, "first"), entry(1, "second")], 2);

        let mut app = App::new().unwrap();
        select(&mut app, "second");
        agent_write("probe");

        app.sync_from_store().unwrap();

        assert!(descriptions(&app.data).contains(&"probe"));
        assert_eq!(selected_description(&app), "second");
    }

    #[test]
    fn sync_is_skipped_while_a_form_is_open() {
        let _guard = env_guard();
        sandbox("sync-form");
        seed(vec![entry(0, "first")], 1);

        let mut app = App::new().unwrap();
        agent_write("probe");
        for mode in [
            InputMode::AddingEntry,
            InputMode::EditingEntry,
            InputMode::Searching,
        ] {
            app.input_mode = mode;
            app.input_description = "half typed".to_string();
            app.sync_from_store().unwrap();
            assert_eq!(descriptions(&app.data), vec!["first"]);
            assert_eq!(app.input_description, "half typed");
        }

        // …and the change is picked up once the mode is Normal again.
        app.input_mode = InputMode::Normal;
        app.sync_from_store().unwrap();
        assert!(descriptions(&app.data).contains(&"probe"));
    }

    #[test]
    fn sync_falls_back_to_a_nearby_row_when_the_selection_is_gone() {
        let _guard = env_guard();
        sandbox("sync-gone");
        seed(vec![entry(0, "a"), entry(1, "b"), entry(2, "c")], 3);

        let mut app = App::new().unwrap();
        let last = app.filtered_entries().len() - 1;
        app.table_state.select(Some(last));
        let doomed = app.filtered_entries()[last].id;
        storage::with_data(|data| {
            data.entries.retain(|e| e.id != doomed);
            Ok(())
        })
        .unwrap();

        app.sync_from_store().unwrap();

        let len = app.filtered_entries().len();
        assert_eq!(len, 2);
        assert_eq!(app.table_state.selected(), Some(len - 1));
    }

    #[test]
    fn an_untouched_store_reports_no_change_but_an_unsettled_mtime_does() {
        let _guard = env_guard();
        sandbox("sync-quiet");
        seed(vec![entry(0, "first")], 1);

        let mut app = App::new().unwrap();
        // Let the mtime fall out of the current second, past the granularity guard.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert!(
            app.store_is_unchanged(),
            "a quiet store should not trigger a reload"
        );

        agent_write("probe");
        assert!(!app.store_is_unchanged(), "an outside write was missed");

        // A stamp identical to the one on disk still counts as changed while the
        // mtime sits inside the current second, since a second write in that
        // second could leave both mtime and length untouched.
        app.store_stamp = storage::StoreStamp::read();
        assert!(
            !app.store_is_unchanged(),
            "an unsettled stamp should not be trusted"
        );
    }

    #[test]
    fn edit_keeps_a_concurrent_agent_write() {
        let _guard = env_guard();
        sandbox("edit");
        seed(vec![entry(0, "before")], 1);

        let mut app = App::new().unwrap();
        agent_write("probe");
        select(&mut app, "before");
        app.start_editing();
        app.input_description = "after".to_string();
        app.submit_edit().unwrap();

        let data = on_disk();
        assert_eq!(descriptions(&data), vec!["after", "probe"]);
        assert_eq!(descriptions(&app.data), vec!["after", "probe"]);
    }

    /// The form's Project field is optional: whitespace-only means "no project",
    /// which must land as JSON `null` rather than an empty string.
    #[test]
    fn the_form_writes_the_project_and_leaves_a_blank_one_null() {
        let _guard = env_guard();
        sandbox("project-form");
        seed(Vec::new(), 0);

        let mut app = App::new().unwrap();
        app.start_adding();
        app.input_description = "with a project".to_string();
        app.input_project = "  acme  ".to_string();
        app.input_duration = "15m".to_string();
        app.submit_entry().unwrap();

        app.start_adding();
        app.input_description = "without one".to_string();
        app.input_project = "   ".to_string();
        app.input_duration = "15m".to_string();
        app.submit_entry().unwrap();

        let data = on_disk();
        let project = |desc: &str| {
            data.entries
                .iter()
                .find(|e| e.description == desc)
                .unwrap()
                .project
                .clone()
        };
        assert_eq!(project("with a project"), Some("acme".to_string()));
        assert_eq!(project("without one"), None);

        let raw = std::fs::read_to_string(storage::get_data_path().unwrap()).unwrap();
        assert!(
            raw.contains("\"project\": null"),
            "blank project not null: {raw}"
        );
        assert!(
            !raw.contains("\"project\": \"\""),
            "blank project stored as \"\": {raw}"
        );
    }

    /// `e` pre-fills the Project field, and clearing it drops the project.
    #[test]
    fn editing_round_trips_the_project_field() {
        let _guard = env_guard();
        sandbox("project-edit");
        let mut seeded = entry(0, "has a project");
        seeded.project = Some("acme".to_string());
        seed(vec![seeded], 1);

        let mut app = App::new().unwrap();
        select(&mut app, "has a project");
        app.start_editing();
        assert_eq!(app.input_project, "acme");

        app.input_project = "beta".to_string();
        app.submit_edit().unwrap();
        assert_eq!(on_disk().entries[0].project, Some("beta".to_string()));

        select(&mut app, "has a project");
        app.start_editing();
        app.input_project.clear();
        app.submit_edit().unwrap();
        assert_eq!(on_disk().entries[0].project, None);
    }
}
