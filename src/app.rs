use anyhow::{Context, Result};
use directories::ProjectDirs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct AppContext {
    pub scheduler_db: PathBuf,
    pub whatsapp_db: PathBuf,
}

impl AppContext {
    pub fn new(data_dir: Option<PathBuf>) -> Result<Self> {
        let dir = match data_dir {
            Some(path) => path,
            None => ProjectDirs::from("dev", "whatschedule", "whatschedule")
                .context("could not resolve an application data directory")?
                .data_dir()
                .to_path_buf(),
        };

        std::fs::create_dir_all(&dir)
            .with_context(|| format!("could not create {}", dir.display()))?;

        Ok(Self {
            scheduler_db: dir.join("scheduler.db"),
            whatsapp_db: dir.join("whatsapp-session.db"),
        })
    }
}
