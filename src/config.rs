use std::path::PathBuf;

use color_eyre::{eyre::eyre, Result};
use serde::Deserialize;

use crate::apis::jira::JiraConfig;

#[derive(Clone, Debug, Deserialize)]
pub struct AppConfig {
    #[serde(flatten)]
    pub jira: JiraConfig,
    pub repos_dir: PathBuf,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        let mut config = envy::from_env::<Self>()?;
        config.jira.jira_url = config.jira.jira_url.trim_end_matches('/').to_string();
        if !config.repos_dir.is_dir() {
            return Err(eyre!(
                "REPOS_DIR ({}) is not a directory",
                config.repos_dir.display()
            ));
        }
        Ok(config)
    }
}
