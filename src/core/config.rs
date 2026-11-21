use std::fs;
use std::path::PathBuf;

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub library_path: PathBuf,
    pub dark_mode: bool,
    pub check_updates_on_start: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        let library_path = ProjectDirs::from("com", "weekbox", "wb-r")
            .map(|dirs| dirs.data_local_dir().join("library"))
            .unwrap_or_else(|| PathBuf::from("./library"));

        Self {
            library_path,
            dark_mode: true,
            check_updates_on_start: true,
        }
    }
}

impl AppConfig {
    pub fn config_file_path() -> Option<PathBuf> {
        ProjectDirs::from("com", "weekbox", "wb-r")
            .map(|dirs| dirs.config_dir().join("config.json"))
    }

    pub fn load_or_default() -> anyhow::Result<Self> {
        let Some(path) = Self::config_file_path() else {
            return Ok(Self::default());
        };

        if !path.exists() {
            let cfg = Self::default();
            cfg.save()?;
            return Ok(cfg);
        }

        let raw = fs::read_to_string(&path)?;
        if raw.trim().is_empty() {
            return Ok(Self::default());
        }
        let cfg = serde_json::from_str(&raw)?;
        Ok(cfg)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let Some(path) = Self::config_file_path() else {
            anyhow::bail!("No se pudo resolver la ruta de configuración del sistema");
        };

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string_pretty(self)?;
        let tmp_path = path.with_extension("json.tmp");
        fs::write(&tmp_path, &json)?;
        if fs::rename(&tmp_path, &path).is_err() {
            let _ = fs::remove_file(&tmp_path);
            fs::write(&path, json)?;
        }
        Ok(())
    }
}
