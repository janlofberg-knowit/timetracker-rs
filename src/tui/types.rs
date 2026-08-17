#[derive(Clone, Copy, PartialEq)]
pub enum ViewMode {
    All,
    Day,
    Week,
}

impl ViewMode {
    pub fn title(&self) -> &'static str {
        match self {
            ViewMode::All => "All Entries",
            ViewMode::Day => "Daily View",
            ViewMode::Week => "Weekly View",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum InputMode {
    Normal,
    AddingEntry,
    EditingEntry,
    Searching,
    Help,
}

#[derive(Clone, Copy, PartialEq)]
pub enum InputField {
    Description,
    Project,
    Tags,
    StartTime,
    EndTime,
    Duration,
}

/// One of the two collapsible value pickers on the surface between the Status
/// panel and the tabs row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pane {
    Projects,
    Tags,
}

impl Pane {
    /// Block title, with the toggle key so the surface documents itself.
    pub fn title(self) -> &'static str {
        match self {
            Pane::Projects => " Projects (⇧P) ",
            Pane::Tags => " Tags (⇧T) ",
        }
    }
}

/// What `j`/`k` currently move in `InputMode::Normal`: the entries table, or the
/// cursor inside one of the panes. `Tab` cycles through it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Table,
    Pane(Pane),
}

#[derive(Clone, Copy, PartialEq)]
pub enum SortOrder {
    NewestFirst,
    OldestFirst,
}

impl SortOrder {
    pub fn toggle(self) -> Self {
        match self {
            SortOrder::NewestFirst => SortOrder::OldestFirst,
            SortOrder::OldestFirst => SortOrder::NewestFirst,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SortOrder::NewestFirst => "newest first",
            SortOrder::OldestFirst => "oldest first",
        }
    }
}
