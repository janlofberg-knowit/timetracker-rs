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
mod panes;
mod render;

pub use types::{Focus, InputField, InputMode, Pane, SortOrder, ViewMode};

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
    /// Whether the Projects / Tags panes are open. Both default to off, so the
    /// pane surface has zero height and first-run layout is unchanged.
    pub(crate) show_projects: bool,
    pub(crate) show_tags: bool,
    /// What `Tab` has given focus to, and where each pane's cursor rests.
    pub(crate) focus: Focus,
    pub(crate) project_cursor: usize,
    pub(crate) tag_cursor: usize,
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
            show_projects: false,
            show_tags: false,
            focus: Focus::Table,
            project_cursor: 0,
            tag_cursor: 0,
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
                            // j/k move inside the focused pane when there is one,
                            // and fall through to the table otherwise.
                            KeyCode::Char('j') | KeyCode::Down => {
                                if !app.pane_next() {
                                    app.next();
                                }
                            }
                            KeyCode::Char('k') | KeyCode::Up => {
                                if !app.pane_previous() {
                                    app.previous();
                                }
                            }
                            KeyCode::Char('P') => app.toggle_pane(Pane::Projects),
                            KeyCode::Char('T') => app.toggle_pane(Pane::Tags),
                            KeyCode::Tab => app.cycle_focus(),
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

    fn dated(
        id: u64,
        description: &str,
        project: &str,
        tags: &[&str],
        date: NaiveDate,
    ) -> TimeEntry {
        let start = date
            .and_hms_opt(9, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .unwrap();
        TimeEntry {
            id,
            description: description.to_string(),
            project: (!project.is_empty()).then(|| project.to_string()),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            start_time: start,
            end_time: Some(start + chrono::Duration::hours(1)),
        }
    }

    /// A store spanning three scopes: two days inside the current week plus one
    /// entry a week back, so day / week / all each see a different set.
    fn seed_panes() -> App {
        let today = Local::now().date_naive();
        let week_start = TimeData::week_start(today);
        let day_one = week_start;
        let day_two = week_start + chrono::Duration::days(1);
        let last_week = week_start - chrono::Duration::days(7);
        seed(
            vec![
                dated(0, "a", "tt", &["impl", "tt/8"], day_one),
                dated(1, "b", "tt", &["plan"], day_one),
                dated(2, "c", "loremind", &["impl", "ops"], day_one),
                dated(3, "d", "vinge", &["ops"], day_two),
                dated(4, "e", "vinge", &["impl"], last_week),
                dated(5, "f", "", &[], day_one),
            ],
            6,
        );
        let mut app = App::new().unwrap();
        app.selected_date = day_one;
        app
    }

    /// A pane's rows as `value=count`, in the order they are listed.
    fn values(app: &App, pane: Pane) -> String {
        app.pane_values(pane)
            .iter()
            .map(|(value, count)| format!("{value}={count}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Each pane offers the distinct values of the current view scope with their
    /// match counts — and an entry with no project contributes no Projects row.
    #[test]
    fn pane_values_follow_the_view_scope() {
        let _guard = env_guard();
        sandbox("pane-scope");
        let mut app = seed_panes();

        app.view_mode = ViewMode::Day;
        assert_eq!(values(&app, Pane::Projects), "tt=2 loremind=1");
        assert_eq!(values(&app, Pane::Tags), "impl=2 ops=1 plan=1 tt/8=1");

        app.view_mode = ViewMode::Week;
        assert_eq!(values(&app, Pane::Projects), "tt=2 loremind=1 vinge=1");
        assert_eq!(values(&app, Pane::Tags), "impl=2 ops=2 plan=1 tt/8=1");

        app.view_mode = ViewMode::All;
        assert_eq!(values(&app, Pane::Projects), "tt=2 vinge=2 loremind=1");
        assert_eq!(values(&app, Pane::Tags), "impl=3 ops=2 plan=1 tt/8=1");
    }

    /// The panes read the scope *before* the filter, so a pane never hides the
    /// value the user has just filtered on.
    #[test]
    fn pane_values_ignore_the_active_filter_and_search() {
        let _guard = env_guard();
        sandbox("pane-prefilter");
        let mut app = seed_panes();
        app.view_mode = ViewMode::Day;
        let before = app.pane_values(Pane::Tags);

        app.tag_filter = vec!["plan".to_string()];
        app.search_term = "nothing matches this".to_string();
        assert!(app.filtered_entries().is_empty(), "filter did not bite");
        assert_eq!(app.pane_values(Pane::Tags), before);
        assert_eq!(app.pane_values(Pane::Projects).len(), 2);
    }

    #[test]
    fn the_surface_has_no_height_until_a_pane_is_opened() {
        let _guard = env_guard();
        sandbox("pane-height");
        let mut app = seed_panes();
        assert!(!app.show_projects && !app.show_tags);
        assert_eq!(app.pane_surface_height(), 0);

        app.toggle_pane(Pane::Projects);
        assert!(app.pane_surface_height() > 0);
        app.toggle_pane(Pane::Projects);
        assert_eq!(app.pane_surface_height(), 0);
    }

    #[test]
    fn tab_cycles_focus_through_the_visible_panes_only() {
        let _guard = env_guard();
        sandbox("pane-focus");
        let mut app = seed_panes();

        // No pane open: Tab is a no-op.
        app.cycle_focus();
        assert_eq!(app.focus, Focus::Table);

        app.toggle_pane(Pane::Tags);
        app.cycle_focus();
        assert_eq!(app.focus, Focus::Pane(Pane::Tags));
        app.cycle_focus();
        assert_eq!(app.focus, Focus::Table);

        app.toggle_pane(Pane::Projects);
        app.cycle_focus();
        assert_eq!(app.focus, Focus::Pane(Pane::Projects));
        app.cycle_focus();
        assert_eq!(app.focus, Focus::Pane(Pane::Tags));
        app.cycle_focus();
        assert_eq!(app.focus, Focus::Table);
    }

    #[test]
    fn hiding_the_focused_pane_hands_focus_back_to_the_table() {
        let _guard = env_guard();
        sandbox("pane-focus-drop");
        let mut app = seed_panes();
        app.toggle_pane(Pane::Projects);
        app.cycle_focus();
        assert_eq!(app.focus, Focus::Pane(Pane::Projects));

        app.toggle_pane(Pane::Projects);
        assert_eq!(app.focus, Focus::Table);
        assert!(app.focused_pane().is_none());
    }

    /// `j`/`k` wrap inside the focused pane and leave the table alone; with no
    /// pane focused they report "not handled" so the table moves instead.
    #[test]
    fn pane_cursor_moves_only_while_a_pane_has_focus() {
        let _guard = env_guard();
        sandbox("pane-cursor");
        let mut app = seed_panes();
        app.view_mode = ViewMode::Day;
        app.table_state.select(Some(0));

        assert!(!app.pane_next(), "no pane focused, yet j was swallowed");
        assert_eq!(app.pane_cursor(Pane::Tags), 0);

        app.toggle_pane(Pane::Tags);
        app.cycle_focus();
        let len = app.pane_values(Pane::Tags).len();
        assert_eq!(len, 4);
        assert!(app.pane_next());
        assert_eq!(app.pane_cursor(Pane::Tags), 1);
        assert!(app.pane_previous());
        assert_eq!(app.pane_cursor(Pane::Tags), 0);
        // k at the top wraps to the last value
        assert!(app.pane_previous());
        assert_eq!(app.pane_cursor(Pane::Tags), len - 1);
        // …and j at the bottom wraps back
        assert!(app.pane_next());
        assert_eq!(app.pane_cursor(Pane::Tags), 0);
        assert_eq!(app.table_state.selected(), Some(0), "the table moved too");
    }

    /// A cursor left past the end by a scope change is clamped, not panicking.
    #[test]
    fn a_stale_pane_cursor_is_clamped_to_the_new_value_list() {
        let _guard = env_guard();
        sandbox("pane-cursor-stale");
        let mut app = seed_panes();
        app.view_mode = ViewMode::All;
        app.project_cursor = 2;
        assert_eq!(app.pane_cursor(Pane::Projects), 2);

        // Day scope has fewer projects than All
        app.view_mode = ViewMode::Day;
        assert_eq!(app.pane_values(Pane::Projects).len(), 2);
        assert_eq!(app.pane_cursor(Pane::Projects), 1);
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
