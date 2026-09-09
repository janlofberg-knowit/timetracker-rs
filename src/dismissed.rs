//! Reader and writer for the dismissal ledger: one file per stretch of one
//! session's clock time that was **not** work.
//!
//! A sibling of `marks/` and `activity/` in the cache dir. It exists because no
//! `tt` entry can cover an audit row without billing its minutes, so "this was
//! not work" has nowhere else to live. A dismissal is scoped to one session:
//! project-scoped, it would erase a concurrent agent's row for the same minutes.

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// One dismissed stretch of one session's clock time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dismissal {
    pub project: String,
    pub session_id: String,
    pub start: i64,
    pub end: i64,
    pub reason: Option<String>,
}

/// `$TT_DISMISSED_DIR` when set, else `dismissed` inside the cache dir.
pub fn dismissed_dir() -> Option<PathBuf> {
    resolve_dismissed_dir(
        std::env::var_os("TT_DISMISSED_DIR"),
        crate::paths::cache_dir(),
    )
}

/// The env-free half of [`dismissed_dir`]. The override rule is
/// [`crate::paths::env_or`]; the `dismissed` subdirectory is this module's own.
fn resolve_dismissed_dir(
    dismissed_dir: Option<OsString>,
    cache: Option<PathBuf>,
) -> Option<PathBuf> {
    crate::paths::env_or(dismissed_dir, Some(cache?.join("dismissed")))
}

/// The filename one dismissal is written under, in [`crate::marks::mark_key`]'s
/// segment convention: each segment sanitised on its own, joined with `.`.
fn dismissal_key(dismissal: &Dismissal) -> String {
    let segment = crate::paths::sanitise_key;
    format!(
        "{}.{}.{}-{}",
        crate::marks::project_key(&dismissal.project),
        segment(&dismissal.session_id),
        dismissal.start,
        dismissal.end
    )
}

/// Record one dismissal. Idempotent — the same session, span and project write
/// the same file, and an existing one is left untouched.
pub fn write_in(dir: &Path, dismissal: &Dismissal) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let path = dir.join(dismissal_key(dismissal));
    let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => return Ok(()),
        Err(err) => return Err(err),
    };
    writeln!(file, "project={}", dismissal.project)?;
    writeln!(file, "session={}", dismissal.session_id)?;
    writeln!(file, "start={}", dismissal.start)?;
    writeln!(file, "end={}", dismissal.end)?;
    if let Some(reason) = &dismissal.reason {
        writeln!(file, "reason={reason}")?;
    }
    Ok(())
}

/// Returns every dismissal in dir; skips a file missing any of project, session, start, end.
pub fn read_all_in(dir: &Path) -> Vec<Dismissal> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|e| read_dismissal(&e.path()))
        .collect()
}

/// Every dismissal on this machine; no cache directory reads as none.
pub fn read_all() -> Vec<Dismissal> {
    dismissed_dir()
        .map(|dir| read_all_in(&dir))
        .unwrap_or_default()
}

fn read_dismissal(path: &Path) -> Option<Dismissal> {
    let body = fs::read_to_string(path).ok()?;
    let mut project = None;
    let mut session = None;
    let mut start = None;
    let mut end = None;
    let mut reason = None;

    for line in body.lines() {
        let Some((field, value)) = line.split_once('=') else {
            continue;
        };
        match field {
            "project" => project = Some(value.to_string()),
            "session" => session = Some(value.to_string()),
            "start" => start = value.parse().ok(),
            "end" => end = value.parse().ok(),
            "reason" => reason = Some(value.to_string()),
            _ => {}
        }
    }

    Some(Dismissal {
        project: project?,
        session_id: session?,
        start: start?,
        end: end?,
        reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tt-dismissed-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn dismissal() -> Dismissal {
        Dismissal {
            project: "my proj".to_string(),
            session_id: "sess-1".to_string(),
            start: 1_000_000,
            end: 1_003_600,
            reason: Some("a break".to_string()),
        }
    }

    /// The override rule itself is covered once, in `paths::env_or`. What is
    /// this module's own is the subdirectory it defaults to.
    #[test]
    fn the_default_directory_is_dismissed_inside_the_app_cache_dir() {
        let cache = || Some(PathBuf::from("cache"));
        assert_eq!(
            resolve_dismissed_dir(None, cache()),
            Some(PathBuf::from("cache").join("dismissed"))
        );
        assert_eq!(
            resolve_dismissed_dir(None, None),
            None,
            "no cache, no default"
        );
    }

    #[test]
    fn the_filename_joins_the_project_key_the_session_and_the_span() {
        assert_eq!(
            dismissal_key(&dismissal()),
            "my_proj.sess-1.1000000-1003600"
        );
    }

    #[test]
    fn a_dismissal_reads_back_every_field_it_was_written_with() {
        let dir = sandbox("round-trip");
        write_in(&dir, &dismissal()).unwrap();
        assert_eq!(read_all_in(&dir), vec![dismissal()]);
    }

    #[test]
    fn writing_the_same_dismissal_twice_leaves_one_record() {
        let dir = sandbox("idempotent");
        write_in(&dir, &dismissal()).unwrap();
        let repeat = Dismissal {
            reason: Some("something else".to_string()),
            ..dismissal()
        };
        write_in(&dir, &repeat).unwrap();

        let recorded = read_all_in(&dir);
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].reason.as_deref(), Some("a break"));
    }

    #[test]
    fn a_dismissal_with_no_reason_reads_back_without_one() {
        let dir = sandbox("no-reason");
        write_in(
            &dir,
            &Dismissal {
                reason: None,
                ..dismissal()
            },
        )
        .unwrap();
        assert_eq!(read_all_in(&dir)[0].reason, None);
    }

    #[test]
    fn a_missing_directory_reads_as_no_dismissals() {
        let dir = sandbox("missing");
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(read_all_in(&dir), Vec::new());
    }

    #[test]
    fn a_file_missing_a_required_field_is_not_a_dismissal() {
        let dir = sandbox("incomplete");
        fs::write(dir.join("junk"), "project=tt\nstart=1\n").unwrap();
        assert_eq!(read_all_in(&dir), Vec::new());
    }
}
