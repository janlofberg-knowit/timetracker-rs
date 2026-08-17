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
    /// The selected entry's detail popover. Modal like `Help` in that nothing else
    /// can be open beside it, but the list stays live underneath: j/k and the
    /// arrows move the table cursor and the popover re-reads the new selection,
    /// and `e`/`d` act on whichever entry is being shown.
    Detail,
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
    /// Block title, with the toggle key so the surface documents itself. The key
    /// is a bare capital: the case of the letter already says "shift", so a shift
    /// glyph in front of it only spends columns the footer legend cannot spare.
    pub fn title(self) -> &'static str {
        match self {
            Pane::Projects => " Projects (P) ",
            Pane::Tags => " Tags (T) ",
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
