use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Full cache file contents.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Cache {
    /// Historical CI check durations in seconds, keyed by "repo_slug/check_name".
    /// Stores the most recent completed duration for ETA estimation.
    #[serde(default)]
    pub check_durations: HashMap<String, u64>,
    /// Collapsed story keys with their section context.
    #[serde(default)]
    pub collapsed_stories: HashSet<(String, Option<bool>)>,
}

fn cache_path() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("work-tui").join("snapshot.json"))
}

pub fn load() -> Cache {
    let Some(path) = cache_path() else {
        return Cache::default();
    };
    let Ok(data) = fs::read_to_string(&path) else {
        return Cache::default();
    };
    serde_json::from_str(&data).unwrap_or_default()
}

pub fn save(cache: &Cache) {
    let Some(path) = cache_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let Ok(data) = serde_json::to_string_pretty(cache) else {
        return;
    };
    let _ = fs::write(&path, data);
}
