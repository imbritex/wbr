use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryPaths {
    pub root: PathBuf,
    pub mods_dir: PathBuf,
    pub engines_dir: PathBuf,
    pub data_dir: PathBuf,
    pub mod_covers_dir: PathBuf,
    pub settings_json: PathBuf,
    pub storage_manifest_json: PathBuf,
    pub bookmarks_json: PathBuf,
    pub favorites_json: PathBuf,
    pub profiles_json: PathBuf,
    pub engine_update_state_json: PathBuf,
    pub playtime_json: PathBuf,
    pub installed_mods_json: PathBuf,
}

impl LibraryPaths {
    pub fn from_root(root: &Path) -> Self {
        let data_dir = root.join("data");
        Self {
            root: root.to_path_buf(),
            mods_dir: root.join("mods"),
            engines_dir: root.join("engines"),
            mod_covers_dir: data_dir.join("mod-covers"),
            settings_json: root.join("settings.json"),
            storage_manifest_json: root.join("storage-manifest.json"),
            bookmarks_json: data_dir.join("bookmarks.json"),
            favorites_json: data_dir.join("favorites.json"),
            profiles_json: data_dir.join("profiles.json"),
            engine_update_state_json: data_dir.join("engineupdatestate.json"),
            playtime_json: data_dir.join("playtime.json"),
            installed_mods_json: data_dir.join("installedmods.json"),
            data_dir,
        }
    }
}

pub fn ensure_weekbox_library(root: &Path) -> anyhow::Result<LibraryPaths> {
    let paths = LibraryPaths::from_root(root);

    for dir in [
        &paths.root,
        &paths.mods_dir,
        &paths.engines_dir,
        &paths.data_dir,
        &paths.mod_covers_dir,
    ] {
        if !dir.exists() {
            fs::create_dir_all(dir)?;
        }
    }

    let now_iso = Utc::now().to_rfc3339();
    let now_ms = Utc::now().timestamp_millis();

    let defaults: [(&Path, serde_json::Value); 8] = [
        (
            &paths.settings_json,
            json!({
                "version": "1.0.0",
                "theme": "dark",
                "language": "en",
                "autoUpdate": true
            }),
        ),
        (
            &paths.storage_manifest_json,
            json!({
                "version": "1.0.0",
                "name": "WeekBoxLibrary",
                "createdAt": now_iso,
                "directories": ["mods", "engines", "data"]
            }),
        ),
        (&paths.bookmarks_json, json!([])),
        (&paths.favorites_json, json!([])),
        (&paths.profiles_json, json!([])),
        (
            &paths.engine_update_state_json,
            json!({
                "lastChecked": null,
                "engines": {}
            }),
        ),
        (
            &paths.playtime_json,
            json!({
                "schemaVersion": 1,
                "lastUpdated": now_ms,
                "totalSecondsAllEngines": 0,
                "engines": {},
                "days": {},
                "sessions": []
            }),
        ),
        (&paths.installed_mods_json, json!([])),
    ];

    for (file_path, default_value) in defaults {
        if !file_path.exists() {
            write_json_atomic(file_path, &default_value)?;
        }
    }

    Ok(paths)
}

pub fn write_json_atomic<T: Serialize + ?Sized>(path: &Path, value: &T) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }
    let json = serde_json::to_string_pretty(value)?;
    let tmp_name = format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("weekbox")
    );
    let tmp_path = path.with_file_name(tmp_name);
    fs::write(&tmp_path, &json)?;
    if fs::rename(&tmp_path, path).is_err() {
        let _ = fs::remove_file(&tmp_path);
        fs::write(path, json)?;
    }
    Ok(())
}

pub fn read_json_or_default<T: DeserializeOwned + Default>(path: &Path) -> T {
    let Ok(raw) = fs::read_to_string(path) else {
        return T::default();
    };
    if raw.trim().is_empty() {
        return T::default();
    }
    serde_json::from_str(&raw).unwrap_or_default()
}

pub fn sanitize_folder_name(raw: &str) -> String {
    let sanitized: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = sanitized.trim_matches(|c| c == '.' || c == '_');
    if trimmed.is_empty() {
        "item".to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn format_file_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;

    let b = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if b < MB {
        format!("{:.1} KB", b / KB)
    } else if b < GB {
        format!("{:.1} MB", b / MB)
    } else {
        format!("{:.2} GB", b / GB)
    }
}

pub fn format_compact_number(n: i64) -> String {
    let sign = if n < 0 { "-" } else { "" };
    let abs = n.unsigned_abs();
    if abs < 1_000 {
        return format!("{n}");
    }

    let (divisor, suffix) = if abs >= 1_000_000_000 {
        (1_000_000_000.0, 'B')
    } else if abs >= 1_000_000 {
        (1_000_000.0, 'M')
    } else {
        (1_000.0, 'K')
    };

    let formatted = format!("{:.1}", abs as f64 / divisor);
    let clean = formatted.strip_suffix(".0").unwrap_or(&formatted);
    format!("{sign}{clean}{suffix}")
}

pub fn dir_size_bytes(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.file_type().is_symlink() {
        return 0;
    }
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }

    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            // Do not traverse symlinks/junctions to avoid loops or double counting
            if ft.is_symlink() {
                continue;
            }
            if ft.is_file() {
                if let Ok(m) = entry.metadata() {
                    total = total.saturating_add(m.len());
                }
            } else if ft.is_dir() {
                stack.push(entry.path());
            }
        }
    }
    total
}
