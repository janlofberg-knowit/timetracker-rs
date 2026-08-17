use anyhow::{Context, Result};
use std::{
    fs::{self, File},
    path::PathBuf,
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

/// A cheap fingerprint of the store file, for deciding whether an in-memory
/// snapshot is stale without reading or parsing the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoreStamp {
    len: u64,
    modified: Option<SystemTime>,
}

impl StoreStamp {
    /// `None` when the store does not exist yet, or cannot be stat'd.
    pub fn read() -> Option<Self> {
        let meta = fs::metadata(get_data_path().ok()?).ok()?;
        Some(Self {
            len: meta.len(),
            modified: meta.modified().ok(),
        })
    }

    /// Whether this stamp can be trusted to differ once the file is written again.
    ///
    /// mtime granularity is one second on some filesystems, so two writes inside
    /// the same second can leave both mtime and length unchanged — an update that
    /// no later comparison would ever notice. While the recorded mtime is still
    /// inside the current second the stamp is therefore *unsettled*, and callers
    /// should reload regardless of it. That costs a few extra reads in the second
    /// following a write, and nothing at all once the store goes quiet.
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
