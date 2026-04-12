use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::github::CheckStatus;

/// Per-issue snapshot persisted to disk, used to detect changes across restarts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IssueSnapshot {
    pub status: String,
    pub pr_checks: Option<CheckStatus>,
}

/// Full cache file contents.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Cache {
    pub snapshots: HashMap<String, IssueSnapshot>,
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
