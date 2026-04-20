use std::env;
use std::path::PathBuf;

use color_eyre::{eyre::eyre, Result};

use crate::apis::jira::JiraConfig;

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub jira: JiraConfig,
    pub repos_dir: PathBuf,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        let jira = JiraConfig::from_env()?;
        let repos_dir =
            PathBuf::from(env::var("REPOS_DIR").map_err(|_| eyre!("REPOS_DIR is not set"))?);
        if !repos_dir.is_dir() {
            return Err(eyre!(
                "REPOS_DIR ({}) is not a directory",
                repos_dir.display()
            ));
        }
        Ok(Self { jira, repos_dir })
    }
}
