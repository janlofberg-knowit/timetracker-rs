//! Candidate providers for dynamic shell completion. Every provider runs on each
//! Tab press: read through the lock-free `storage::load_data`, never `with_data`,
//! and on any failure return nothing rather than an error.

use std::collections::BTreeSet;

use clap_complete::CompletionCandidate;

use crate::agent::PHASES;
use crate::marks::{Mark, open_marks};
use crate::report::classify;
use crate::storage::load_data;
use crate::tracker::TimeData;

/// Every project named by a store entry or an open mark.
pub fn projects() -> Vec<CompletionCandidate> {
    let data = load_data().unwrap_or_default();
    to_candidates(project_names(&data, &open_marks()))
}

/// Issues known for the project typed earlier on the line, or every issue when
/// no project can be read off it. Always includes the no-issue sentinel `-`.
pub fn issues() -> Vec<CompletionCandidate> {
    let data = load_data().unwrap_or_default();
    let project = typed_project(std::env::args());
    to_candidates(issue_names(&data, &open_marks(), project.as_deref()))
}

pub fn phases() -> Vec<CompletionCandidate> {
    PHASES.iter().map(CompletionCandidate::new).collect()
}

fn to_candidates(names: BTreeSet<String>) -> Vec<CompletionCandidate> {
    names.into_iter().map(CompletionCandidate::new).collect()
}

fn project_names(data: &TimeData, marks: &[Mark]) -> BTreeSet<String> {
    data.entries
        .iter()
        .filter_map(|e| e.project.clone())
        .chain(marks.iter().map(|m| m.project.clone()))
        .filter(|p| !p.is_empty())
        .collect()
}

fn issue_names(data: &TimeData, marks: &[Mark], project: Option<&str>) -> BTreeSet<String> {
    let wanted = |p: &str| project.is_none_or(|w| w == p);
    let from_entries = data.entries.iter().filter_map(|e| {
        let (item, _) = classify(&e.tags);
        let (p, issue) = item?.split_once('/')?;
        (wanted(p) && !issue.is_empty()).then(|| issue.to_string())
    });
    let from_marks = marks
        .iter()
        .filter(|m| wanted(&m.project))
        .filter_map(|m| m.issue.clone());
    from_entries
        .chain(from_marks)
        .chain(std::iter::once("-".to_string()))
        .collect()
}

/// The project positional already typed on an `agent` command line, read from
/// the completer's own argv: `<bin> -- tt agent <sub> <project> ...`. `None`
/// whenever that shape is not found, so the caller falls back to every issue.
fn typed_project<I: Iterator<Item = String>>(args: I) -> Option<String> {
    let words: Vec<String> = args.skip_while(|a| a != "--").skip(1).collect();
    let agent = words.iter().position(|w| w == "agent")?;
    let project = words.get(agent + 2)?;
    (!project.is_empty()).then(|| project.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::TimeEntry;
    use chrono::Local;

    fn entry(project: Option<&str>, tags: &[&str]) -> TimeEntry {
        TimeEntry {
            id: 0,
            description: String::new(),
            project: project.map(String::from),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            start_time: Local::now(),
            end_time: None,
            idle: Vec::new(),
        }
    }

    fn mark(project: &str, issue: Option<&str>) -> Mark {
        Mark {
            project: project.to_string(),
            issue: issue.map(String::from),
            phase: "impl".to_string(),
            start: Local::now(),
        }
    }

    fn data(entries: Vec<TimeEntry>) -> TimeData {
        TimeData {
            entries,
            next_id: 1,
            schema_version: 1,
        }
    }

    fn argv(words: &[&str]) -> impl Iterator<Item = String> {
        words
            .iter()
            .map(|w| w.to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn projects_union_store_and_marks_deduplicated_and_sorted() {
        let d = data(vec![
            entry(Some("zeta"), &[]),
            entry(Some("alpha"), &[]),
            entry(None, &[]),
            entry(Some(""), &[]),
        ]);
        let names = project_names(&d, &[mark("alpha", None), mark("mid", None)]);
        assert_eq!(names.into_iter().collect::<Vec<_>>(), ["alpha", "mid", "zeta"]);
    }

    #[test]
    fn issues_scoped_to_project_with_sentinel() {
        let d = data(vec![
            entry(Some("a"), &["a/10", "impl"]),
            entry(Some("b"), &["b/20"]),
            entry(Some("a"), &["a/"]),
        ]);
        let marks = [mark("a", Some("11")), mark("b", Some("21")), mark("a", None)];
        let scoped = issue_names(&d, &marks, Some("a"));
        assert_eq!(scoped.into_iter().collect::<Vec<_>>(), ["-", "10", "11"]);
    }

    #[test]
    fn issues_unfiltered_without_project() {
        let d = data(vec![entry(Some("a"), &["a/10"]), entry(Some("b"), &["b/20"])]);
        let all = issue_names(&d, &[mark("b", Some("21"))], None);
        assert_eq!(all.into_iter().collect::<Vec<_>>(), ["-", "10", "20", "21"]);
    }

    #[test]
    fn phases_are_the_canonical_list() {
        let got: Vec<String> = phases().iter().map(|c| c.get_value().to_string_lossy().into_owned()).collect();
        assert_eq!(got, PHASES);
    }

    #[test]
    fn typed_project_reads_the_agent_positional() {
        let a = argv(&["/bin/tt", "--", "tt", "agent", "begin", "proj", ""]);
        assert_eq!(typed_project(a), Some("proj".to_string()));
    }

    #[test]
    fn typed_project_is_none_on_odd_shapes() {
        assert_eq!(typed_project(argv(&["/bin/tt"])), None);
        assert_eq!(typed_project(argv(&["/bin/tt", "tt", "agent", "begin", "proj"])), None);
        assert_eq!(typed_project(argv(&["/bin/tt", "--", "tt", "agent", "begin"])), None);
        assert_eq!(typed_project(argv(&["/bin/tt", "--", "tt", "agent", "begin", ""])), None);
        assert_eq!(typed_project(argv(&["/bin/tt", "--", "tt", "start", "--project", ""])), None);
    }
}
