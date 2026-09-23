//! The Entries table's column vocabulary and its config resolution.

use ratatui::prelude::Constraint;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryColumn {
    Date,
    Start,
    End,
    Description,
    Project,
    Tags,
    Duration,
}

impl EntryColumn {
    /// Today's default order. `Project` is in the vocabulary but not here.
    pub(crate) const ALL: [EntryColumn; 6] = [
        EntryColumn::Date,
        EntryColumn::Start,
        EntryColumn::End,
        EntryColumn::Description,
        EntryColumn::Tags,
        EntryColumn::Duration,
    ];

    /// The config spelling, matched case-insensitively by [`resolve`].
    pub(crate) fn name(&self) -> &'static str {
        match self {
            EntryColumn::Date => "date",
            EntryColumn::Start => "start",
            EntryColumn::End => "end",
            EntryColumn::Description => "description",
            EntryColumn::Project => "project",
            EntryColumn::Tags => "tags",
            EntryColumn::Duration => "duration",
        }
    }

    /// The header cell's text.
    pub(crate) fn label(&self) -> &'static str {
        match self {
            EntryColumn::Date => "Date",
            EntryColumn::Start => "Start",
            EntryColumn::End => "End",
            EntryColumn::Description => "Description",
            EntryColumn::Project => "Project",
            EntryColumn::Tags => "Tags",
            EntryColumn::Duration => "Duration",
        }
    }

    /// The column's width. `Project` is `Min`, not `Length`, so it only takes
    /// space the fixed columns leave over.
    pub(crate) fn constraint(&self) -> Constraint {
        match self {
            EntryColumn::Date => Constraint::Length(10),
            EntryColumn::Start => Constraint::Length(5),
            EntryColumn::End => Constraint::Length(5),
            EntryColumn::Description => Constraint::Min(12),
            EntryColumn::Project => Constraint::Min(10),
            EntryColumn::Tags => Constraint::Fill(1),
            EntryColumn::Duration => Constraint::Length(8),
        }
    }

    fn from_name(name: &str) -> Option<EntryColumn> {
        EntryColumn::ALL_WITH_PROJECT
            .into_iter()
            .find(|c| c.name() == name)
    }

    /// [`ALL`](Self::ALL) plus `Project`, for name lookup — `ALL` alone is the
    /// default *order*, not the whole vocabulary.
    pub(crate) const ALL_WITH_PROJECT: [EntryColumn; 7] = [
        EntryColumn::Date,
        EntryColumn::Start,
        EntryColumn::End,
        EntryColumn::Description,
        EntryColumn::Project,
        EntryColumn::Tags,
        EntryColumn::Duration,
    ];

    /// Resolves `[layout].columns` into the render order. An unknown name
    /// warns and is skipped; a repeat warns and is kept once; an absent,
    /// empty, or all-unknown list falls back to [`ALL`](Self::ALL), with a
    /// warning in the all-unknown case. Never errors — this codebase's
    /// config convention is warn-and-continue.
    pub(crate) fn resolve(configured: Option<&[String]>) -> Vec<EntryColumn> {
        let Some(names) = configured.filter(|n| !n.is_empty()) else {
            return EntryColumn::ALL.to_vec();
        };

        let mut columns = Vec::with_capacity(names.len());
        for raw in names {
            let trimmed = raw.trim().to_lowercase();
            let Some(column) = EntryColumn::from_name(&trimmed) else {
                eprintln!("Warning: layout.columns has an unknown column {raw:?}; ignoring it.");
                continue;
            };
            if columns.contains(&column) {
                eprintln!("Warning: layout.columns repeats {raw:?}; keeping only the first.");
                continue;
            }
            columns.push(column);
        }

        if columns.is_empty() {
            eprintln!("Warning: layout.columns has no recognised column; using the default order.");
            return EntryColumn::ALL.to_vec();
        }
        columns
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_list_resolves_to_the_default_order() {
        assert_eq!(EntryColumn::resolve(None), EntryColumn::ALL.to_vec());
    }

    #[test]
    fn an_empty_list_resolves_to_the_default_order() {
        assert_eq!(EntryColumn::resolve(Some(&[])), EntryColumn::ALL.to_vec());
    }

    #[test]
    fn a_reordered_list_resolves_in_the_given_order() {
        let configured = vec!["duration".to_string(), "date".to_string()];
        assert_eq!(
            EntryColumn::resolve(Some(&configured)),
            vec![EntryColumn::Duration, EntryColumn::Date]
        );
    }

    #[test]
    fn a_hidden_column_is_left_out() {
        let configured = vec!["date".to_string(), "duration".to_string()];
        let resolved = EntryColumn::resolve(Some(&configured));
        assert!(!resolved.contains(&EntryColumn::Tags));
    }

    #[test]
    fn an_all_unknown_list_falls_back_to_the_default_order() {
        let configured = vec!["Datum".to_string(), "Dauer".to_string()];
        assert_eq!(
            EntryColumn::resolve(Some(&configured)),
            EntryColumn::ALL.to_vec()
        );
    }

    #[test]
    fn an_unknown_name_warns_and_is_skipped() {
        let configured = vec!["date".to_string(), "bogus".to_string()];
        assert_eq!(
            EntryColumn::resolve(Some(&configured)),
            vec![EntryColumn::Date]
        );
    }

    #[test]
    fn a_duplicate_name_warns_and_is_kept_once() {
        let configured = vec!["date".to_string(), "Date".to_string()];
        assert_eq!(
            EntryColumn::resolve(Some(&configured)),
            vec![EntryColumn::Date]
        );
    }

    #[test]
    fn project_is_in_the_vocabulary_but_not_the_default_order() {
        assert!(!EntryColumn::ALL.contains(&EntryColumn::Project));
        let configured = vec!["project".to_string()];
        assert_eq!(
            EntryColumn::resolve(Some(&configured)),
            vec![EntryColumn::Project]
        );
    }
}
