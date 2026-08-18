use anyhow::{Context, Result};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use crate::tracker;

pub fn get_data_path() -> Result<PathBuf> {
    let proj_dirs = directories::ProjectDirs::from("com", "timetracker", "tt")
        .context("Could not determine config directory")?;
    let data_dir = proj_dirs.data_dir();
    fs::create_dir_all(data_dir)?;
    Ok(data_dir.join("data.json"))
}

/// A cheap fingerprint of a path — a file or a directory — for deciding whether
/// an in-memory snapshot of it is stale without reading or parsing anything.
///
/// One type for both because the reasoning about mtime granularity below is the
/// hard part and must exist in exactly one place; see [`store_stamp`] for the
/// store's own stamp and `App::marks_stamp` for the mark directory's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathStamp {
    len: u64,
    modified: Option<SystemTime>,
}

impl PathStamp {
    /// `None` when `path` does not exist yet, or cannot be stat'd.
    pub fn read(path: &Path) -> Option<Self> {
        let meta = fs::metadata(path).ok()?;
        Some(Self {
            len: meta.len(),
            modified: meta.modified().ok(),
        })
    }

    /// Whether `current` can be trusted to mean "nothing has changed since
    /// `previous`", i.e. whether a caller may skip re-reading the path.
    ///
    /// The settledness test below only applies to a path that exists: a missing
    /// path has no mtime to be inside the current second, so two `None`s are as
    /// equal as they look.
    pub fn unchanged(previous: Option<Self>, current: Option<Self>) -> bool {
        match current {
            Some(stamp) => current == previous && stamp.is_settled(),
            None => current == previous,
        }
    }

    /// Whether this stamp can be trusted to differ once the path is written again.
    ///
    /// mtime granularity is one second on some filesystems, so two writes inside
    /// the same second can leave both mtime and length unchanged — an update that
    /// no later comparison would ever notice. While the recorded mtime is still
    /// inside the current second the stamp is therefore *unsettled*, and callers
    /// should reload regardless of it. That costs a few extra reads in the second
    /// following a write, and nothing at all once the path goes quiet.
    pub fn is_settled(&self) -> bool {
        let Some(modified) = self.modified else {
            return false;
        };
        match SystemTime::now().duration_since(modified) {
            Ok(age) => age >= Duration::from_secs(1),
            // mtime in the future (clock skew): fall back to comparing stamps,
            // rather than reloading on every tick until the clock catches up.
            Err(_) => true,
        }
    }
}

/// The store file's stamp. `None` when the store does not exist yet.
pub fn store_stamp() -> Option<PathStamp> {
    PathStamp::read(&get_data_path().ok()?)
}

fn get_lock_path() -> Result<PathBuf> {
    Ok(get_data_path()?.with_extension("lock"))
}

/// Take an exclusive lock on the store, held until the returned file is dropped
fn lock_data() -> Result<File> {
    let path = get_lock_path()?;
    let lock = File::create(&path).context("Could not open lock file")?;
    lock.lock().context("Could not lock data file")?;
    Ok(lock)
}

pub fn load_data() -> Result<tracker::TimeData> {
    let path = get_data_path()?;
    if path.exists() {
        let content = fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&content)?)
    } else {
        Ok(tracker::TimeData::default())
    }
}

pub fn save_data(data: &tracker::TimeData) -> Result<()> {
    let path = get_data_path()?;
    let content = serde_json::to_string_pretty(data)?;

    // Write to a temp file and rename, so a reader never sees a half-written
    // store and an interrupted write cannot truncate it
    let temp_path = path.with_extension("json.tmp");
    fs::write(&temp_path, content)?;
    fs::rename(&temp_path, &path)?;
    Ok(())
}

/// Load the data, apply `edit`, and save the result under one exclusive lock
///
/// Every mutation goes through here. Loading and saving as separate steps lets two
/// concurrent `tt` calls read the same snapshot, and the second save then silently
/// discards the entry the first one added.
pub fn with_data<T>(edit: impl FnOnce(&mut tracker::TimeData) -> Result<T>) -> Result<T> {
    let _lock = lock_data()?;
    let mut data = load_data()?;
    let result = edit(&mut data)?;
    save_data(&data)?;
    Ok(result)
}

/// One lock for every test that repoints an environment variable — `HOME` for
/// the store, `TT_MARK_DIR` for the marks. Env is process-wide and the test
/// binary is threaded, so *all* of them have to serialise against the same
/// mutex: two modules each with their own lock is the same race with more steps.
#[cfg(test)]
pub(crate) fn env_guard() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, OnceLock};
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch path, never the real store or the real mark directory — both
    /// are written continuously by live agent sessions.
    fn sandbox(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tt-stamp-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_stamp_taken_within_the_same_second_is_unsettled() {
        let dir = sandbox("settled");
        let file = dir.join("data.json");
        fs::write(&file, "{}").unwrap();

        // Freshly written: mtime is inside the current second, so the next write
        // could leave this stamp looking identical. It must not be trusted.
        let fresh = PathStamp::read(&file).unwrap();
        assert!(!fresh.is_settled(), "a stamp of a just-written path");
        assert!(
            !PathStamp::unchanged(Some(fresh), Some(fresh)),
            "two identical unsettled stamps still mean 'reload'"
        );

        // The same stamp, once the second it was taken in has passed.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let settled = PathStamp::read(&file).unwrap();
        assert_eq!(fresh, settled, "the path did not change");
        assert!(settled.is_settled(), "a stamp older than a second");
        assert!(PathStamp::unchanged(Some(fresh), Some(settled)));
    }

    #[test]
    fn a_missing_path_stamps_as_none_and_compares_equal_to_itself() {
        let dir = sandbox("missing");
        let file = dir.join("nope.json");
        assert_eq!(PathStamp::read(&file), None);
        assert!(
            PathStamp::unchanged(None, None),
            "still missing is still unchanged"
        );

        fs::write(&file, "{}").unwrap();
        assert!(
            !PathStamp::unchanged(None, PathStamp::read(&file)),
            "appearing is a change"
        );
    }
}
