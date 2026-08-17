use anyhow::{Context, Result};
use std::{
    fs::{self, File},
    path::PathBuf,
};

use crate::tracker;

pub fn get_data_path() -> Result<PathBuf> {
    let proj_dirs = directories::ProjectDirs::from("com", "timetracker", "tt")
        .context("Could not determine config directory")?;
    let data_dir = proj_dirs.data_dir();
    fs::create_dir_all(data_dir)?;
    Ok(data_dir.join("data.json"))
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
