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
