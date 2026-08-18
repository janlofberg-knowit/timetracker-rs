use chrono::{DateTime, Datelike, Duration, Local, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::duration;
use crate::icons;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TimeEntry {
    pub id: u64,
    pub description: String,
    /// Project this entry belongs to, if known
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub start_time: DateTime<Local>,
    pub end_time: Option<DateTime<Local>>,
    /// Silent stretches inside this entry's span, as `tt log --idle` recorded them.
    ///
    /// `#[serde(default)]` and no schema bump: absent means empty means the
    /// duration is unchanged, so there is nothing to backfill.
    #[serde(default)]
    pub idle: Vec<IdleInterval>,
}

/// One silent stretch inside an entry — a heartbeat gap `tt-safe` already judged
/// too long (#12), recorded so it can be shown and trimmed away later.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct IdleInterval {
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
}

impl IdleInterval {
    pub fn new(start: DateTime<Local>, end: DateTime<Local>) -> Self {
        Self { start, end }
    }

    pub fn duration(&self) -> Duration {
        self.end.signed_duration_since(self.start)
    }

    /// `09:50 – 10:20 (30m)`, as the detail popover lists it — clock times, since
    /// the epoch seconds the interval was recorded from are a wire format.
    pub fn format_span(&self) -> String {
        format!(
            "{} – {} ({})",
            self.start.format("%H:%M"),
            self.end.format("%H:%M"),
            duration::format(self.duration())
        )
    }
}

/// Parse tags (words starting with #) from text and return (clean_text, tags)
pub fn parse_tags(text: &str) -> (String, Vec<String>) {
    let mut tags = Vec::new();
    let mut clean_parts = Vec::new();
    
    for word in text.split_whitespace() {
        if word.starts_with('#') && word.len() > 1 {
            // Remove the # prefix and add to tags
            tags.push(word[1..].to_string());
        } else {
            clean_parts.push(word);
        }
    }
    
    (clean_parts.join(" "), tags)
}

/// Infer the project for an entry from its tags.
///
/// `parse_tags` harvests `#…` words from anywhere in the description, so the first
/// tag is not reliably the project. The project is the tag `X` for which some other
/// tag starts with `X/` (the `#project` / `#project/issue` pair the wrapper emits);
/// failing that, the first tag; with no tags, `None`.
pub fn infer_project(tags: &[String]) -> Option<String> {
    let with_child = tags.iter().enumerate().find(|(i, candidate)| {
        let prefix = format!("{}/", candidate);
        tags.iter()
            .enumerate()
            .any(|(j, other)| j != *i && other.starts_with(&prefix))
    });

    with_child
        .map(|(_, candidate)| candidate)
        .or_else(|| tags.first())
        .cloned()
}

/// One-time migration: infer `project` for entries that lack one, then stamp the
/// schema version. A no-op once the store is at version 1 or above.
pub fn migrate(data: &mut TimeData) {
    if data.schema_version >= 1 {
        return;
    }
    for entry in &mut data.entries {
        if entry.project.is_none() {
            entry.project = infer_project(&entry.tags);
        }
    }
    data.schema_version = 1;
}

impl TimeEntry {
    pub fn duration(&self) -> Duration {
        let end = self.end_time.unwrap_or_else(Local::now);
        end.signed_duration_since(self.start_time)
    }

    pub fn format_duration(&self) -> String {
        duration::format(self.duration())
    }

    /// The date column, as the entries table prints it.
    pub fn format_date(&self) -> String {
        self.start_time.format("%Y-%m-%d").to_string()
    }

    pub fn format_start_time(&self) -> String {
        self.start_time.format("%H:%M").to_string()
    }

    /// The end-time column: a clock time, or an em dash while the entry runs.
    pub fn format_end_time(&self) -> String {
        self.end_time
            .map(|t| t.format("%H:%M").to_string())
            .unwrap_or_else(|| "—".to_string())
    }

    pub fn is_active(&self) -> bool {
        self.end_time.is_none()
    }

    /// Returns the status icon for this entry (active or empty)
    pub fn status_icon(&self) -> &'static str {
        if self.is_active() {
            icons::ACTIVE
        } else {
            ""
        }
    }

    /// Format tags for display (with # prefix)
    pub fn format_tags(&self) -> String {
        self.tags.iter().map(|t| format!("#{}", t)).collect::<Vec<_>>().join(" ")
    }

    /// Check if entry has a specific tag (case-insensitive)
    pub fn has_tag(&self, tag: &str) -> bool {
        let tag_lower = tag.to_lowercase();
        self.tags.iter().any(|t| t.to_lowercase() == tag_lower)
    }

    /// Check if entry matches any of the given tags (case-insensitive)
    pub fn has_any_tag(&self, tags: &[String]) -> bool {
        tags.iter().any(|t| self.has_tag(t))
    }

    /// Check if the entry's project is any of the given ones (case-insensitive).
    ///
    /// An entry with no project matches nothing: absence is not a value anyone can
    /// select, so it can never be one of `projects`.
    pub fn has_any_project(&self, projects: &[String]) -> bool {
        let Some(project) = self
            .project
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        else {
            return false;
        };
        let project = project.to_lowercase();
        projects.iter().any(|p| p.trim().to_lowercase() == project)
    }

    /// Everything a row or the detail popover can show, lower-cased and joined —
    /// the haystack `/` searches.
    ///
    /// The owner asked to "search on anything", so this is built from the same
    /// formatters the table renders with: a field the UI can show is a field the
    /// search reaches, and the two cannot drift apart.
    pub fn search_haystack(&self) -> String {
        [
            self.id.to_string(),
            self.description.clone(),
            self.project.clone().unwrap_or_default(),
            self.format_tags(),
            self.format_date(),
            self.format_start_time(),
            self.format_end_time(),
            self.format_duration(),
        ]
        .join(" ")
        .to_lowercase()
    }

    /// Whether `needle` (already lower-cased) appears anywhere in this entry.
    pub fn matches_search(&self, needle_lower: &str) -> bool {
        self.search_haystack().contains(needle_lower)
    }
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct TimeData {
    pub entries: Vec<TimeEntry>,
    pub next_id: u64,
    /// Store schema version; legacy files read as 0 and are migrated to 1
    #[serde(default)]
    pub schema_version: u32,
}

impl TimeData {
    pub fn active_entry(&self) -> Option<&TimeEntry> {
        self.entries.iter().find(|e| e.is_active())
    }

    pub fn active_entry_mut(&mut self) -> Option<&mut TimeEntry> {
        self.entries.iter_mut().find(|e| e.is_active())
    }

    /// Stop the currently active entry. Returns true if an entry was stopped.
    pub fn stop_active(&mut self) -> bool {
        if let Some(entry) = self.active_entry_mut() {
            entry.end_time = Some(Local::now());
            true
        } else {
            false
        }
    }

    pub fn today_entries(&self) -> Vec<&TimeEntry> {
        let today = Local::now().date_naive();
        self.entries
            .iter()
            .filter(|e| e.start_time.date_naive() == today)
            .collect()
    }

    pub fn today_total(&self) -> Duration {
        self.today_entries()
            .iter()
            .fold(Duration::zero(), |acc, e| acc + e.duration())
    }

    pub fn add_entry(
        &mut self,
        description: String,
        project: Option<String>,
        tags: Vec<String>,
        start_time: DateTime<Local>,
        end_time: Option<DateTime<Local>>,
    ) -> &TimeEntry {
        let entry = TimeEntry {
            id: self.next_id,
            description,
            project,
            tags,
            start_time,
            end_time,
            idle: Vec::new(),
        };
        self.next_id += 1;
        self.entries.push(entry);
        self.entries.last().unwrap()
    }

    /// Get entries for a specific date
    pub fn entries_for_date(&self, date: NaiveDate) -> Vec<&TimeEntry> {
        self.entries
            .iter()
            .filter(|e| e.start_time.date_naive() == date)
            .collect()
    }

    /// Get total duration for a specific date
    pub fn total_for_date(&self, date: NaiveDate) -> Duration {
        self.entries_for_date(date)
            .iter()
            .fold(Duration::zero(), |acc, e| acc + e.duration())
    }

    /// Get the start of the week (Monday) for a given date
    pub fn week_start(date: NaiveDate) -> NaiveDate {
        let days_from_monday = date.weekday().num_days_from_monday();
        date - Duration::days(days_from_monday as i64)
    }

    /// Get entries for a specific week (starting Monday)
    pub fn entries_for_week(&self, week_start: NaiveDate) -> Vec<&TimeEntry> {
        let week_end = week_start + Duration::days(7);
        self.entries
            .iter()
            .filter(|e| {
                let date = e.start_time.date_naive();
                date >= week_start && date < week_end
            })
            .collect()
    }

    /// Get total duration for a specific week
    pub fn total_for_week(&self, week_start: NaiveDate) -> Duration {
        self.entries_for_week(week_start)
            .iter()
            .fold(Duration::zero(), |acc, e| acc + e.duration())
    }

    /// Get daily breakdown for a week (returns Vec of (date, total_duration))
    pub fn daily_breakdown(&self, week_start: NaiveDate) -> Vec<(NaiveDate, Duration)> {
        (0..7)
            .map(|i| {
                let date = week_start + Duration::days(i);
                (date, self.total_for_date(date))
            })
            .collect()
    }

    /// Update an existing entry by ID
    pub fn update_entry(
        &mut self,
        id: u64,
        description: String,
        project: Option<String>,
        tags: Vec<String>,
        start_time: DateTime<Local>,
        end_time: Option<DateTime<Local>>,
    ) -> bool {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
            entry.description = description;
            entry.project = project;
            entry.tags = tags;
            entry.start_time = start_time;
            entry.end_time = end_time;
            true
        } else {
            false
        }
    }

    /// Get an entry by ID
    pub fn get_entry(&self, id: u64) -> Option<&TimeEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Split the entry with `id` on **every** idle interval it carries, so the
    /// silent stretches drop out of the timeline.
    ///
    /// Takes no intervals — it reads the entry's stored `idle`, which is the reason
    /// that field exists. It splits on all of them because threshold policy lives
    /// entirely in `tt-safe`, which only ever records gaps it has already judged too
    /// long (#12); `tt` holds no opinion about what counts as a gap.
    ///
    /// The store mechanic is a **split** while the user-facing verb is **trim**
    /// (`tt log --trim`, the popover's `[t]`). That is a deliberate distinction, not
    /// an inconsistency: one entry becomes two or more and every endpoint stays
    /// true, which "trim" would misdescribe.
    ///
    /// The first (earliest) piece keeps the original id via an in-place update and
    /// later pieces take fresh ids from `next_id`, so a caller holding the id still
    /// has something to point at afterwards. Pieces inherit description, project and
    /// tags, and keep only the idle intervals falling inside them — after a split on
    /// every interval, none. Zero-length pieces (a gap touching an endpoint) are
    /// dropped, and if that would leave no pieces at all the entry is left untouched
    /// rather than deleted.
    ///
    /// Pure over `&mut self` with no I/O: the caller owns the store transaction.
    /// Returns the resulting pieces' ids, earliest first, or an empty vec when
    /// nothing changed.
    pub fn split_at_idle(&mut self, id: u64) -> Vec<u64> {
        let Some(entry) = self.get_entry(id) else {
            return Vec::new();
        };
        // A running entry has no end to split against, and only `tt log` records
        // idle, so this is a guard rather than a case.
        let Some(span_end) = entry.end_time else {
            return Vec::new();
        };
        let span_start = entry.start_time;
        let template = entry.clone();

        // Clamp to the span and merge, so an interval reaching past an endpoint or
        // overlapping its neighbour cuts one hole instead of leaving a zero-length
        // piece wedged between two of them.
        let mut gaps: Vec<IdleInterval> = template
            .idle
            .iter()
            .map(|gap| IdleInterval {
                start: gap.start.clamp(span_start, span_end),
                end: gap.end.clamp(span_start, span_end),
            })
            .filter(|gap| gap.end > gap.start)
            .collect();
        gaps.sort_by_key(|gap| gap.start);
        let mut holes: Vec<IdleInterval> = Vec::new();
        for gap in gaps {
            match holes.last_mut() {
                Some(last) if gap.start <= last.end => last.end = last.end.max(gap.end),
                _ => holes.push(gap),
            }
        }
        if holes.is_empty() {
            return Vec::new();
        }

        // The pieces are the complement of the holes within the span. A hole
        // touching an endpoint contributes nothing, which is how zero-length pieces
        // are dropped rather than special-cased.
        let mut pieces: Vec<(DateTime<Local>, DateTime<Local>)> = Vec::new();
        let mut cursor = span_start;
        for hole in &holes {
            if hole.start > cursor {
                pieces.push((cursor, hole.start));
            }
            cursor = hole.end;
        }
        if span_end > cursor {
            pieces.push((cursor, span_end));
        }
        // Idle covering the whole span would leave nothing: keep the entry as it is,
        // since deleting what the owner logged is not a trim.
        if pieces.is_empty() {
            return Vec::new();
        }

        let mut ids = Vec::with_capacity(pieces.len());
        for (i, (piece_start, piece_end)) in pieces.into_iter().enumerate() {
            let inside: Vec<IdleInterval> = template
                .idle
                .iter()
                .copied()
                .filter(|gap| gap.start >= piece_start && gap.end <= piece_end)
                .collect();
            if i == 0 {
                // In place, so the earliest piece keeps the original id.
                let first = self
                    .entries
                    .iter_mut()
                    .find(|e| e.id == id)
                    .expect("the entry resolved above");
                first.start_time = piece_start;
                first.end_time = Some(piece_end);
                first.idle = inside;
                ids.push(id);
            } else {
                let new_id = self.next_id;
                self.next_id += 1;
                self.entries.push(TimeEntry {
                    id: new_id,
                    description: template.description.clone(),
                    project: template.project.clone(),
                    tags: template.tags.clone(),
                    start_time: piece_start,
                    end_time: Some(piece_end),
                    idle: inside,
                });
                ids.push(new_id);
            }
        }
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(values: &[&str]) -> Vec<String> {
        values.iter().map(|t| t.to_string()).collect()
    }

    fn entry(id: u64, project: Option<&str>, entry_tags: &[&str]) -> TimeEntry {
        TimeEntry {
            id,
            description: format!("entry {}", id),
            project: project.map(|p| p.to_string()),
            tags: tags(entry_tags),
            start_time: Local::now(),
            end_time: None,
            idle: Vec::new(),
        }
    }

    #[test]
    fn infers_project_from_sibling_tag_not_first_tag() {
        assert_eq!(
            infer_project(&tags(&[
                "62",
                "77",
                "90/#91/#94/#95",
                "loremind",
                "loremind/62",
                "ops"
            ])),
            Some("loremind".to_string())
        );
    }

    #[test]
    fn infers_project_from_sibling_tag_when_summary_token_is_first() {
        assert_eq!(
            infer_project(&tags(&["63:", "loremind", "loremind/63", "plan"])),
            Some("loremind".to_string())
        );
    }

    #[test]
    fn falls_back_to_first_tag_when_no_sibling_pair() {
        assert_eq!(
            infer_project(&tags(&["tt", "impl"])),
            Some("tt".to_string())
        );
    }

    #[test]
    fn infers_no_project_without_tags() {
        assert_eq!(infer_project(&[]), None);
    }

    #[test]
    fn migrate_fills_missing_projects_and_sets_schema_version() {
        let mut data = TimeData {
            entries: vec![
                entry(1, None, &["tt", "impl"]),
                entry(2, None, &["63:", "loremind", "loremind/63", "plan"]),
                entry(3, Some("explicit"), &["tt", "impl"]),
                entry(4, None, &[]),
            ],
            next_id: 5,
            schema_version: 0,
        };

        migrate(&mut data);

        assert_eq!(data.schema_version, 1);
        assert_eq!(data.entries[0].project.as_deref(), Some("tt"));
        assert_eq!(data.entries[1].project.as_deref(), Some("loremind"));
        assert_eq!(data.entries[2].project.as_deref(), Some("explicit"));
        assert_eq!(data.entries[3].project, None);
    }

    /// A fixed wall clock, so every assertion below can be derived from the
    /// fixture's own offsets rather than from a literal.
    fn base() -> DateTime<Local> {
        NaiveDate::from_ymd_opt(2026, 8, 18)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .unwrap()
    }

    /// One logged entry of `span_minutes`, carrying idle intervals given as
    /// minute offsets from its start.
    fn logged_with_idle(span_minutes: i64, gaps: &[(i64, i64)]) -> TimeData {
        let start = base();
        TimeData {
            entries: vec![TimeEntry {
                id: 7,
                description: "logged span".to_string(),
                project: Some("tt".to_string()),
                tags: tags(&["tt", "tt/36"]),
                start_time: start,
                end_time: Some(start + Duration::minutes(span_minutes)),
                idle: gaps
                    .iter()
                    .map(|(from, to)| {
                        IdleInterval::new(
                            start + Duration::minutes(*from),
                            start + Duration::minutes(*to),
                        )
                    })
                    .collect(),
            }],
            next_id: 8,
            schema_version: 1,
        }
    }

    /// The entry spans, as (minutes from `base`, minutes from `base`) pairs.
    fn spans(data: &TimeData) -> Vec<(i64, i64)> {
        data.entries
            .iter()
            .map(|e| {
                (
                    e.start_time.signed_duration_since(base()).num_minutes(),
                    e.end_time
                        .unwrap()
                        .signed_duration_since(base())
                        .num_minutes(),
                )
            })
            .collect()
    }

    #[test]
    fn splitting_on_one_idle_interval_gives_two_pieces_excluding_it() {
        let mut data = logged_with_idle(120, &[(50, 80)]);

        let ids = data.split_at_idle(7);

        assert_eq!(ids.len(), 2, "one hole in the middle cuts the span in two");
        assert_eq!(ids[0], 7, "the earliest piece keeps the original id");
        assert_eq!(ids[1], 8, "the later piece takes the next id");
        assert_eq!(data.next_id, 9);
        assert_eq!(spans(&data), vec![(0, 50), (50 + 30, 120)]);
        // Every piece inherits the entry's identity and carries no idle of its own.
        for entry in &data.entries {
            assert_eq!(entry.description, "logged span");
            assert_eq!(entry.project.as_deref(), Some("tt"));
            assert_eq!(entry.tags, tags(&["tt", "tt/36"]));
            assert!(entry.idle.is_empty(), "a piece kept a hole it excludes");
        }
    }

    #[test]
    fn splitting_on_two_idle_intervals_gives_three_pieces() {
        let mut data = logged_with_idle(180, &[(30, 45), (100, 130)]);

        let ids = data.split_at_idle(7);

        assert_eq!(ids, vec![7, 8, 9]);
        assert_eq!(spans(&data), vec![(0, 30), (45, 100), (130, 180)]);
    }

    #[test]
    fn an_idle_interval_touching_an_endpoint_gives_one_piece_not_an_empty_one() {
        let mut leading = logged_with_idle(90, &[(0, 20)]);
        assert_eq!(leading.split_at_idle(7), vec![7]);
        assert_eq!(spans(&leading), vec![(20, 90)]);
        assert_eq!(
            leading.next_id, 8,
            "no fresh id was spent on an empty piece"
        );

        let mut trailing = logged_with_idle(90, &[(70, 90)]);
        assert_eq!(trailing.split_at_idle(7), vec![7]);
        assert_eq!(spans(&trailing), vec![(0, 70)]);
        assert_eq!(trailing.next_id, 8);
    }

    #[test]
    fn idle_covering_the_whole_span_leaves_the_entry_untouched() {
        let mut data = logged_with_idle(90, &[(0, 90)]);
        let before = serde_json::to_string(&data).unwrap();

        assert!(data.split_at_idle(7).is_empty());

        assert_eq!(serde_json::to_string(&data).unwrap(), before);
        assert_eq!(spans(&data), vec![(0, 90)], "the logged entry survived");
    }

    #[test]
    fn an_entry_with_no_idle_is_not_split() {
        let mut data = logged_with_idle(90, &[]);
        let before = serde_json::to_string(&data).unwrap();

        assert!(data.split_at_idle(7).is_empty());
        assert!(data.split_at_idle(999).is_empty(), "an unknown id");

        assert_eq!(serde_json::to_string(&data).unwrap(), before);
    }

    #[test]
    fn the_pieces_sum_to_the_span_minus_the_idle_intervals() {
        let gaps = [(20, 35), (60, 61), (100, 140)];
        let mut data = logged_with_idle(180, &gaps);
        let original = data.entries[0].duration();
        let idle_total = data.entries[0]
            .idle
            .iter()
            .fold(Duration::zero(), |acc, gap| acc + gap.duration());

        data.split_at_idle(7);

        let after = data
            .entries
            .iter()
            .fold(Duration::zero(), |acc, e| acc + e.duration());
        assert_eq!(after, original - idle_total);
    }

    #[test]
    fn a_store_written_before_idle_existed_loads_and_stays_at_schema_version_1() {
        let json = r#"{
            "entries": [
                {
                    "id": 1,
                    "description": "legacy entry",
                    "project": "tt",
                    "tags": ["tt", "impl"],
                    "start_time": "2026-08-18T09:00:00+02:00",
                    "end_time": "2026-08-18T10:00:00+02:00"
                }
            ],
            "next_id": 2,
            "schema_version": 1
        }"#;

        let mut data: TimeData = serde_json::from_str(json).unwrap();

        assert!(data.entries[0].idle.is_empty(), "absent means empty");
        assert_eq!(data.entries[0].duration(), Duration::hours(1));
        migrate(&mut data);
        assert_eq!(data.schema_version, 1, "no version bump, no migration");

        // …and round-trips: the new field is written, and reading it back changes
        // nothing about the entry.
        let round_tripped: TimeData =
            serde_json::from_str(&serde_json::to_string(&data).unwrap()).unwrap();
        assert!(round_tripped.entries[0].idle.is_empty());
        assert_eq!(round_tripped.schema_version, 1);
    }

    #[test]
    fn migrate_is_idempotent() {
        let mut data = TimeData {
            entries: vec![entry(1, None, &["tt", "impl"])],
            next_id: 2,
            schema_version: 0,
        };

        migrate(&mut data);
        let after_first = serde_json::to_string(&data).unwrap();

        // A tag vector that would now infer differently must not be re-applied
        data.entries[0].tags = tags(&["other", "other/1"]);
        migrate(&mut data);

        data.entries[0].tags = tags(&["tt", "impl"]);
        assert_eq!(serde_json::to_string(&data).unwrap(), after_first);
        assert_eq!(data.entries[0].project.as_deref(), Some("tt"));
        assert_eq!(data.schema_version, 1);
    }
}
