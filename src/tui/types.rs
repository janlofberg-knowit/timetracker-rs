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

    /// The one-word scope name, for places that name the period inline rather
    /// than heading a panel with it — the Summary surface's `day · all projects`.
    ///
    /// A real mapping rather than a derived `Debug`: the words are user-facing
    /// copy that happens to coincide with the variant names today, and deriving
    /// `Debug` here would quietly tie the two together, so renaming a variant
    /// would change what the screen says.
    pub fn label(&self) -> &'static str {
        match self {
            ViewMode::All => "all",
            ViewMode::Day => "day",
            ViewMode::Week => "week",
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
    /// A destructive action is waiting for a yes. Modal like `Help`, and the only
    /// mode whose *subject* lives outside it: `App.pending_confirm` says which
    /// action and which entry.
    ///
    /// A unit variant on purpose. `InputMode` is `Copy + PartialEq` and compared
    /// with `==` all over `render.rs`, so a payload here would make every one of
    /// those comparisons depend on the payload's value; the separate field follows
    /// the `editing_entry_id` form pattern instead.
    Confirm,
}

/// Which destructive action a confirmation prompt is standing in front of.
///
/// The variants are the actions, not the keys — but each one knows the key that
/// raised it, because that same key is what confirms it, and a `d` pressed on a
/// trim prompt must not count as a yes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConfirmAction {
    Delete,
    Trim,
}

impl ConfirmAction {
    /// The key that opens this prompt, and therefore also confirms it.
    pub fn key(self) -> char {
        match self {
            ConfirmAction::Delete => 'd',
            ConfirmAction::Trim => 't',
        }
    }

    /// The prompt's verb, capitalised for the title. **`Trim`, never "split"** —
    /// the store mechanic is a split (`split_at_idle`), the user-facing verb is
    /// trim on every surface (#35 decision 0).
    pub fn verb(self) -> &'static str {
        match self {
            ConfirmAction::Delete => "Delete",
            ConfirmAction::Trim => "Trim",
        }
    }
}

/// A destructive action that has been asked for and not yet answered.
///
/// **`entry_id` is captured when the prompt is raised, not read back when it is
/// answered.** `sync_from_store` is deliberately not guarded for `Confirm`, so the
/// 250 ms poll can move the table cursor while the prompt is on screen; resolving
/// the target from the selection at confirm time would destroy an entry the prompt
/// never named.
#[derive(Clone, Copy, PartialEq)]
pub struct PendingConfirm {
    pub action: ConfirmAction,
    pub entry_id: u64,
    /// The mode the prompt was raised from, so cancelling puts the screen back —
    /// the popover reopens on its entry, the bare table keeps its selection.
    pub from: InputMode,
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
