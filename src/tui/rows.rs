//! The entries table's row model: the filtered entries with every issue's
//! entries folded into one group header, expanded or not.

use super::App;
use super::cache::RowKey;
use super::types::ViewMode;
use chrono::{Duration, NaiveDate};
use std::collections::HashMap;

/// One line of the entries table. `Entry` holds an index into `data.entries`,
/// never a reference, so the row cache does not borrow `data`.
#[derive(Clone, PartialEq)]
pub(crate) enum VisibleRow {
    DayHeader { date: NaiveDate, total: Duration },
    GroupHeader(GroupHeader),
    Entry(usize),
}

/// One issue's entries, summarised for the collapsed row.
#[derive(Clone, PartialEq)]
pub(crate) struct GroupHeader {
    /// The item tag as stored, carrying no `#`.
    pub(crate) tag: String,
    /// Indices into `data.entries`, in the order `filtered_entries` gave them.
    pub(crate) members: Vec<usize>,
    pub(crate) total: Duration,
}

impl App {
    /// The rows the entries table draws, in sort order. Cached against
    /// [`RowKey`], so the many calls a single frame makes cost one walk.
    pub(crate) fn rows(&self) -> Vec<VisibleRow> {
        self.with_rows(<[VisibleRow]>::to_vec)
    }

    /// Run `f` over the cached rows, recomputing them first if any input the
    /// [`RowKey`] covers has moved since they were built.
    fn with_rows<T>(&self, f: impl FnOnce(&[VisibleRow]) -> T) -> T {
        let key = RowKey::of(self);
        let stale = match &*self.rows_cache.borrow() {
            Some((cached, _)) => *cached != key,
            None => true,
        };
        if stale {
            let rows = self.compute_rows();
            *self.rows_cache.borrow_mut() = Some((key, rows));
        }
        let cache = self.rows_cache.borrow();
        let (_, rows) = cache.as_ref().expect("filled just above");
        f(rows)
    }

    /// The uncached walk: day partitions in Week view, each partitioned again
    /// into issues, so a group header exists per issue *per day*.
    fn compute_rows(&self) -> Vec<VisibleRow> {
        let indices = self.filtered_indices();
        let mut rows = Vec::with_capacity(indices.len());

        if self.view_mode != ViewMode::Week {
            self.push_segment(&indices, &mut rows);
            return rows;
        }

        let mut day_totals: HashMap<NaiveDate, Duration> = HashMap::new();
        for entry in indices.iter().filter_map(|i| self.data.entries.get(*i)) {
            *day_totals
                .entry(entry.start_time.date_naive())
                .or_insert_with(Duration::zero) += entry.duration();
        }

        // Sorting is by start time, so a date's entries are one contiguous run.
        let mut segment: Vec<usize> = Vec::new();
        let mut current: Option<NaiveDate> = None;
        for index in indices {
            let Some(entry) = self.data.entries.get(index) else {
                continue;
            };
            let date = entry.start_time.date_naive();
            if current != Some(date) {
                self.push_segment(&segment, &mut rows);
                segment.clear();
                current = Some(date);
                let total = day_totals
                    .get(&date)
                    .copied()
                    .unwrap_or_else(Duration::zero);
                rows.push(VisibleRow::DayHeader { date, total });
            }
            segment.push(index);
        }
        self.push_segment(&segment, &mut rows);
        rows
    }

    /// Append one day partition's rows: an issue with two or more entries as a
    /// group header, anything else as the plain entry row it is today.
    fn push_segment(&self, segment: &[usize], rows: &mut Vec<VisibleRow>) {
        enum Slot {
            Entry(usize),
            Group(usize),
        }
        let mut slots: Vec<Slot> = Vec::with_capacity(segment.len());
        let mut groups: Vec<(&str, Vec<usize>)> = Vec::new();
        let mut seen: HashMap<&str, usize> = HashMap::new();

        for index in segment {
            let Some(entry) = self.data.entries.get(*index) else {
                continue;
            };
            let Some(tag) = crate::report::classify(&entry.tags).0 else {
                slots.push(Slot::Entry(*index));
                continue;
            };
            match seen.get(tag) {
                Some(group) => groups[*group].1.push(*index),
                None => {
                    seen.insert(tag, groups.len());
                    slots.push(Slot::Group(groups.len()));
                    groups.push((tag, vec![*index]));
                }
            }
        }

        for slot in slots {
            match slot {
                Slot::Entry(index) => rows.push(VisibleRow::Entry(index)),
                Slot::Group(group) => {
                    let (tag, members) = &groups[group];
                    if members.len() < 2 {
                        rows.push(VisibleRow::Entry(members[0]));
                        continue;
                    }
                    rows.push(VisibleRow::GroupHeader(self.summarise(tag, members)));
                    if self.expanded_issues.contains(*tag) {
                        rows.extend(members.iter().map(|index| VisibleRow::Entry(*index)));
                    }
                }
            }
        }
    }

    /// The collapsed row's figures for `members`, which are never empty.
    fn summarise(&self, tag: &str, members: &[usize]) -> GroupHeader {
        let entries = || members.iter().filter_map(|i| self.data.entries.get(*i));
        GroupHeader {
            tag: tag.to_string(),
            members: members.to_vec(),
            total: entries().fold(Duration::zero(), |acc, e| acc + e.duration()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::VisibleRow;
    use crate::storage;
    use crate::storage::env_guard;
    use crate::storage::env_sandbox as sandbox;
    use crate::tracker::{TimeData, TimeEntry};
    use crate::tui::App;
    use crate::tui::types::ViewMode;
    use chrono::{Local, NaiveDate};

    fn seed(entries: Vec<TimeEntry>) {
        let next_id = entries.len() as u64;
        storage::save_data(&TimeData {
            entries,
            next_id,
            schema_version: 1,
        })
        .unwrap();
    }

    /// A half-hour entry starting at `hour` on `date`.
    fn at(id: u64, description: &str, tags: &[&str], date: NaiveDate, hour: u32) -> TimeEntry {
        let start = date
            .and_hms_opt(hour, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .unwrap();
        TimeEntry {
            id,
            description: description.to_string(),
            project: Some("tt".to_string()),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            start_time: start,
            end_time: Some(start + chrono::Duration::minutes(30)),
            idle: Vec::new(),
            data: None,
        }
    }

    /// Each row as one line: `day <date> <total>`, `group <tag> <members>
    /// <total>`, or `entry <description>`.
    fn shape(app: &App) -> Vec<String> {
        app.rows()
            .iter()
            .map(|row| match row {
                VisibleRow::DayHeader { date, total } => {
                    format!("day {date} {}", crate::duration::format(*total))
                }
                VisibleRow::GroupHeader(header) => format!(
                    "group {} {} {}",
                    header.tag,
                    header.members.len(),
                    crate::duration::format(header.total)
                ),
                VisibleRow::Entry(index) => {
                    format!("entry {}", app.data.entries[*index].description)
                }
            })
            .collect()
    }

    fn today() -> NaiveDate {
        Local::now().date_naive()
    }

    #[test]
    fn untagged_and_single_member_issues_stay_one_row_each() {
        let _guard = env_guard();
        sandbox("rows-singletons");
        seed(vec![
            at(0, "loose", &[], today(), 9),
            at(1, "only member", &["impl", "tt/8"], today(), 10),
            at(2, "other issue", &["tt/9"], today(), 11),
        ]);
        let mut app = App::new().unwrap();
        app.view_mode = ViewMode::Day;

        assert_eq!(
            shape(&app),
            vec!["entry other issue", "entry only member", "entry loose"]
        );
        assert_eq!(app.rows().len(), app.filtered_len());
    }

    #[test]
    fn three_entries_on_one_issue_collapse_to_a_header_carrying_their_total() {
        let _guard = env_guard();
        sandbox("rows-collapse");
        seed(vec![
            at(0, "round one", &["tt/8"], today(), 9),
            at(1, "round two", &["tt/8"], today(), 10),
            at(2, "round three", &["tt/8"], today(), 11),
            at(3, "loose", &[], today(), 12),
        ]);
        let mut app = App::new().unwrap();
        app.view_mode = ViewMode::Day;

        assert_eq!(shape(&app), vec!["entry loose", "group tt/8 3 1h 30m"]);
    }

    #[test]
    fn an_expanded_issue_shows_its_header_and_then_its_members() {
        let _guard = env_guard();
        sandbox("rows-expand");
        seed(vec![
            at(0, "round one", &["tt/8"], today(), 9),
            at(1, "round two", &["tt/8"], today(), 10),
            at(2, "round three", &["tt/8"], today(), 11),
        ]);
        let mut app = App::new().unwrap();
        app.view_mode = ViewMode::Day;
        app.expanded_issues.insert("tt/8".to_string());

        assert_eq!(
            shape(&app),
            vec![
                "group tt/8 3 1h 30m",
                "entry round three",
                "entry round two",
                "entry round one",
            ]
        );
    }

    /// One issue over two days: a header per day, each under its day header.
    #[test]
    fn week_view_nests_a_group_header_inside_every_day_partition() {
        let _guard = env_guard();
        sandbox("rows-week");
        let week_start = TimeData::week_start(today());
        let day_two = week_start + chrono::Duration::days(1);
        seed(vec![
            at(0, "mon one", &["tt/8"], week_start, 9),
            at(1, "mon two", &["tt/8"], week_start, 10),
            at(2, "tue one", &["tt/8"], day_two, 9),
            at(3, "tue two", &["tt/8"], day_two, 10),
        ]);
        let mut app = App::new().unwrap();
        app.selected_date = week_start;
        app.view_mode = ViewMode::Week;

        assert_eq!(
            shape(&app),
            vec![
                format!("day {day_two} 1h 0m"),
                "group tt/8 2 1h 0m".to_string(),
                format!("day {week_start} 1h 0m"),
                "group tt/8 2 1h 0m".to_string(),
            ]
        );
    }

    #[test]
    fn the_rows_are_rebuilt_when_the_expanded_set_moves() {
        let _guard = env_guard();
        sandbox("rows-cache-key");
        seed(vec![
            at(0, "round one", &["tt/8"], today(), 9),
            at(1, "round two", &["tt/8"], today(), 10),
        ]);
        let mut app = App::new().unwrap();
        app.view_mode = ViewMode::Day;

        assert_eq!(shape(&app), vec!["group tt/8 2 1h 0m"]);
        app.expanded_issues.insert("tt/8".to_string());
        assert_eq!(
            shape(&app),
            vec!["group tt/8 2 1h 0m", "entry round two", "entry round one"]
        );
        app.expanded_issues.remove("tt/8");
        assert_eq!(shape(&app), vec!["group tt/8 2 1h 0m"]);
    }
}
