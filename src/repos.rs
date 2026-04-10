use std::{env, fs, path::PathBuf};

use color_eyre::{eyre::eyre, Result};

#[derive(Clone, Debug)]
pub struct RepoEntry {
    pub label: String,
    pub normalized: String,
    pub path: PathBuf,
}

pub fn scan_repos() -> Result<Vec<RepoEntry>> {
    let base = repos_dir()?;
    let read_dir =
        fs::read_dir(&base).map_err(|err| eyre!("Failed to read {}: {err}", base.display()))?;

    let mut repos = Vec::new();
    for entry in read_dir {
        let entry = entry.map_err(|err| eyre!("Failed to inspect entry: {err}"))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|os| os.to_str()) else {
            continue;
        };
        let label = name.to_string();
        let normalized = normalize_label(&label);
        repos.push(RepoEntry {
            label,
            normalized,
            path,
        });
    }

    repos.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()));
    Ok(repos)
}

fn repos_dir() -> Result<PathBuf> {
    let dir = env::var("REPOS_DIR").map_err(|_| eyre!("REPOS_DIR is not set"))?;
    let path = PathBuf::from(dir);
    if !path.exists() {
        return Err(eyre!("{} does not exist", path.display()));
    }
    if !path.is_dir() {
        return Err(eyre!("{} is not a directory", path.display()));
    }
    Ok(path)
}

pub fn normalize_label(input: &str) -> String {
    let mut normalized = String::with_capacity(input.len());
    let mut last_dash = false;

    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch.to_ascii_lowercase());
            last_dash = false;
            continue;
        }

        if last_dash {
            continue;
        }

        normalized.push('-');
        last_dash = true;
    }

    while normalized.ends_with('-') {
        normalized.pop();
    }

    while normalized.starts_with('-') {
        normalized.remove(0);
    }

    normalized
}
