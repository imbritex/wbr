use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{Local, Utc};
use serde::{Deserialize, Serialize};
use sysinfo::System;

use crate::core::host;
use crate::core::storage::{
    LibraryPaths, read_json_or_default, sanitize_folder_name, write_json_atomic,
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone)]
pub struct EngineDefinition {
    pub id: &'static str,
    pub category_id: u64,
    pub name: &'static str,
    pub repo: &'static str,
    pub description: &'static str,
    pub accent: &'static str,
    pub default_exe_name: &'static str,
    pub has_banner: bool,
    pub docs_markdown: &'static str,
}

impl EngineDefinition {
    pub fn repo_owner_and_name(&self) -> Option<(&'static str, &'static str)> {
        self.repo.split_once('/')
    }

    pub fn github_url(&self) -> String {
        format!("https://github.com/{}", self.repo)
    }

    pub fn accent_rgb(&self) -> (u8, u8, u8) {
        let hex = self.accent.trim_start_matches('#');
        if hex.len() == 6 {
            if let (Ok(r), Ok(g), Ok(b)) = (
                u8::from_str_radix(&hex[0..2], 16),
                u8::from_str_radix(&hex[2..4], 16),
                u8::from_str_radix(&hex[4..6], 16),
            ) {
                return (r, g, b);
            }
        }
        (255, 255, 255)
    }
}

pub const ENGINE_DEFINITIONS: [EngineDefinition; 6] = [
    EngineDefinition {
        id: "psych",
        category_id: 28367,
        name: "Psych Engine",
        repo: "ShadowMario/FNF-PsychEngine",
        description: "The premier modular engine for Friday Night Funkin' mods, featuring Lua script support, customizable weeks, and high performance.",
        accent: "#ff4d6d",
        default_exe_name: "PsychEngine.exe",
        has_banner: true,
        docs_markdown: include_str!("descriptions/psych.md"),
    },
    EngineDefinition {
        id: "codename",
        category_id: 34764,
        name: "Codename Engine",
        repo: "CodenameCrew/CodenameEngine",
        description: "Next-generation FNF modding engine built from the ground up for extreme softcoding, Haxe scripting, modularity, and smooth gameplay.",
        accent: "#8b5cf6",
        default_exe_name: "CodenameEngine.exe",
        has_banner: false,
        docs_markdown: include_str!("descriptions/codename.md"),
    },
    EngineDefinition {
        id: "pslice",
        category_id: 43798,
        name: "P-Slice Engine",
        repo: "Psych-Slice/P-Slice",
        description: "A community fork porting official V-Slice mechanics, state management, and visual polish to the Psych Engine base.",
        accent: "#fb7185",
        default_exe_name: "P-Slice.exe",
        has_banner: false,
        docs_markdown: include_str!("descriptions/pslice.md"),
    },
    EngineDefinition {
        id: "vslice",
        category_id: 29202,
        name: "V-Slice Engine",
        repo: "FunkinCrew/Funkin",
        description: "The official Friday Night Funkin' base engine featuring polymod system, charts, and base game weeks.",
        accent: "#38bdf8",
        default_exe_name: "Funkin.exe",
        has_banner: true,
        docs_markdown: include_str!("descriptions/vslice.md"),
    },
    EngineDefinition {
        id: "psychonline",
        category_id: 43788,
        name: "Psych Online",
        repo: "Snirozu/Funkin-Psych-Online",
        description: "Online multiplayer modification for Friday Night Funkin' powered by Psych Engine with custom matchmaking.",
        accent: "#34d399",
        default_exe_name: "PsychOnline.exe",
        has_banner: true,
        docs_markdown: include_str!("descriptions/psychonline.md"),
    },
    EngineDefinition {
        id: "fpsplus",
        category_id: 43850,
        name: "FPS Plus",
        repo: "ThatRozebudDude/FPS-Plus-Public",
        description: "The classic FPS Plus engine by Rozebud with improved input accuracy, custom keybinds, and uncapped framerate.",
        accent: "#fbbf24",
        default_exe_name: "FPSPlus.exe",
        has_banner: false,
        docs_markdown: include_str!("descriptions/fpsplus.md"),
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EnginePlatform {
    Windows,
    Linux,
    MacOs,
    Android,
    Other,
}

impl EnginePlatform {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Linux => "linux",
            Self::MacOs => "macos",
            Self::Android => "android",
            Self::Other => "other",
        }
    }

    pub fn display_label(&self) -> &'static str {
        match self {
            Self::Windows => "Windows",
            Self::Linux => "Linux",
            Self::MacOs => "macOS",
            Self::Android => "Android",
            Self::Other => "Other",
        }
    }
}

pub fn detect_platform() -> EnginePlatform {
    #[cfg(target_os = "windows")]
    {
        EnginePlatform::Windows
    }
    #[cfg(target_os = "linux")]
    {
        EnginePlatform::Linux
    }
    #[cfg(target_os = "macos")]
    {
        EnginePlatform::MacOs
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        EnginePlatform::Other
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineAssetCandidate {
    pub name: String,
    pub size: u64,
    pub download_url: String,
}

fn asset_extension(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") {
        ".tar.gz"
    } else if lower.ends_with(".tar.xz") {
        ".tar.xz"
    } else if lower.ends_with(".tgz") {
        ".tgz"
    } else if lower.ends_with(".zip") {
        ".zip"
    } else if lower.ends_with(".7z") {
        ".7z"
    } else if lower.ends_with(".rar") {
        ".rar"
    } else if lower.ends_with(".tar") {
        ".tar"
    } else if lower.ends_with(".appimage") {
        ".appimage"
    } else if lower.ends_with(".exe") {
        ".exe"
    } else if lower.ends_with(".msi") {
        ".msi"
    } else if lower.ends_with(".dmg") {
        ".dmg"
    } else if lower.ends_with(".pkg") {
        ".pkg"
    } else if lower.ends_with(".deb") {
        ".deb"
    } else if lower.ends_with(".rpm") {
        ".rpm"
    } else if lower.ends_with(".apk") {
        ".apk"
    } else {
        ""
    }
}

fn is_supported_archive_or_package(name: &str) -> bool {
    matches!(
        asset_extension(name),
        ".zip" | ".appimage" | ".tar.gz" | ".tgz" | ".7z" | ".tar.xz" | ".tar" | ".rar"
    )
}

fn format_preference_rank(name: &str) -> u8 {
    match asset_extension(name) {
        ".zip" => 4,
        ".appimage" => 3,
        ".tar.gz" | ".tgz" => 2,
        ".7z" | ".tar.xz" | ".tar" | ".rar" => 1,
        _ => 0,
    }
}

fn asset_platforms(name: &str) -> HashSet<EnginePlatform> {
    let mut platforms = HashSet::new();
    let lower = name.to_ascii_lowercase();

    for token in lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
    {
        if matches!(token, "win" | "win32" | "win64") || token.starts_with("windows") {
            platforms.insert(EnginePlatform::Windows);
        }
        if token.starts_with("linux") {
            platforms.insert(EnginePlatform::Linux);
        }
        if matches!(token, "mac" | "osx" | "darwin") || token.starts_with("macos") {
            platforms.insert(EnginePlatform::MacOs);
        }
        if token.starts_with("android") {
            platforms.insert(EnginePlatform::Android);
        }
    }

    match asset_extension(name) {
        ".exe" | ".msi" => {
            platforms.insert(EnginePlatform::Windows);
        }
        ".appimage" | ".deb" | ".rpm" => {
            platforms.insert(EnginePlatform::Linux);
        }
        ".dmg" | ".pkg" => {
            platforms.insert(EnginePlatform::MacOs);
        }
        ".apk" => {
            platforms.insert(EnginePlatform::Android);
        }
        _ => {}
    }

    platforms
}

fn has_delimited_suffix(lower: &str, suffix: &str) -> bool {
    let bytes = lower.as_bytes();
    let needle = suffix.as_bytes();
    if bytes.len() < needle.len() {
        return false;
    }
    for (idx, window) in bytes.windows(needle.len()).enumerate() {
        if window == needle {
            let after = idx + needle.len();
            if after == bytes.len() || !bytes[after].is_ascii_alphanumeric() {
                return true;
            }
        }
    }
    false
}

fn asset_architecture_rank(name: &str) -> u8 {
    let lower = name.to_ascii_lowercase();
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();

    let is_64 = lower.contains("x86_64")
        || lower.contains("x86-64")
        || lower.contains("windows64")
        || lower.contains("win64")
        || lower.contains("64bit")
        || tokens
            .iter()
            .any(|t| matches!(*t, "x64" | "amd64" | "win64" | "windows64" | "64bit"))
        || has_delimited_suffix(&lower, "-64")
        || has_delimited_suffix(&lower, "_64")
        || has_delimited_suffix(&lower, ".64");

    if is_64 {
        return 3;
    }

    let is_32 = lower.contains("windows32")
        || lower.contains("win32")
        || lower.contains("32bit")
        || tokens.iter().any(|t| {
            matches!(
                *t,
                "x86" | "ia32" | "i386" | "i686" | "win32" | "windows32" | "32bit"
            )
        })
        || has_delimited_suffix(&lower, "-32")
        || has_delimited_suffix(&lower, "_32")
        || has_delimited_suffix(&lower, ".32");

    if is_32 {
        return 1;
    }

    2
}

pub fn get_compatible_asset<'a>(
    assets: &'a [EngineAssetCandidate],
) -> Option<&'a EngineAssetCandidate> {
    get_compatible_asset_for_platform(assets, detect_platform())
}

pub fn get_compatible_asset_for_platform<'a>(
    assets: &'a [EngineAssetCandidate],
    platform: EnginePlatform,
) -> Option<&'a EngineAssetCandidate> {
    let candidates: Vec<&'a EngineAssetCandidate> = assets
        .iter()
        .filter(|a| is_supported_archive_or_package(&a.name))
        .collect();
    if candidates.is_empty() {
        return None;
    }

    if platform == EnginePlatform::Other {
        return candidates
            .iter()
            .copied()
            .max_by_key(|a| format_preference_rank(&a.name));
    }

    let mut platform_matches: Vec<(usize, &'a EngineAssetCandidate)> = candidates
        .into_iter()
        .enumerate()
        .filter(|(_, asset)| asset_platforms(&asset.name).contains(&platform))
        .collect();

    if platform_matches.is_empty() {
        return None;
    }

    platform_matches.sort_by(|(idx_a, a), (idx_b, b)| {
        let arch_cmp = asset_architecture_rank(&b.name).cmp(&asset_architecture_rank(&a.name));
        if arch_cmp != std::cmp::Ordering::Equal {
            return arch_cmp;
        }
        let fmt_cmp = format_preference_rank(&b.name).cmp(&format_preference_rank(&a.name));
        if fmt_cmp != std::cmp::Ordering::Equal {
            return fmt_cmp;
        }
        idx_a.cmp(idx_b)
    });

    platform_matches.first().map(|(_, asset)| *asset)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct InstalledEngineVersion {
    pub version: String,
    pub tag: String,
    pub name: String,
    pub release_id: Option<u64>,
    pub asset_name: Option<String>,
    pub asset_size: Option<u64>,
    pub download_url: Option<String>,
    pub install_path: String,
    pub executable_path: Option<String>,
    pub installed_at: i64,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct EngineRecord {
    pub engine_id: String,
    pub active_version: Option<String>,
    pub installed_versions: Vec<InstalledEngineVersion>,
    pub last_checked: Option<i64>,
    pub latest_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct EngineUpdateState {
    pub last_checked: Option<i64>,
    pub engines: HashMap<String, EngineRecord>,
}

impl EngineUpdateState {
    pub fn load(paths: &LibraryPaths) -> anyhow::Result<Self> {
        Ok(read_json_or_default(&paths.engine_update_state_json))
    }

    pub fn save(&self, paths: &LibraryPaths) -> anyhow::Result<()> {
        write_json_atomic(&paths.engine_update_state_json, self)
    }

    pub fn versions_for(&self, engine_id: &str) -> &[InstalledEngineVersion] {
        self.engines
            .get(engine_id)
            .map(|r| r.installed_versions.as_slice())
            .unwrap_or(&[])
    }

    pub fn active_version_for(&self, engine_id: &str) -> Option<&str> {
        let rec = self.engines.get(engine_id)?;
        if let Some(active) = rec.active_version.as_deref() {
            if !active.is_empty() {
                return Some(active);
            }
        }
        rec.installed_versions
            .iter()
            .find(|v| v.is_default)
            .or_else(|| rec.installed_versions.first())
            .map(|v| v.version.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct DayEnginePlaytimeStat {
    pub seconds: u64,
    pub launches: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct DayPlaytimeEntry {
    pub date: String,
    pub total_seconds: u64,
    pub total_launches: u64,
    pub engines: HashMap<String, DayEnginePlaytimeStat>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct EnginePlaytimeEntry {
    pub engine_id: String,
    pub total_seconds: u64,
    pub total_launches: u64,
    pub first_played: i64,
    pub last_played: i64,
    pub days: HashMap<String, DayEnginePlaytimeStat>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct PlaytimeSessionEntry {
    pub id: String,
    pub engine_id: String,
    pub version: Option<String>,
    pub date: String,
    pub start_time: i64,
    pub end_time: i64,
    pub duration_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct PlaytimeDb {
    pub schema_version: u32,
    pub last_updated: i64,
    pub total_seconds_all_engines: u64,
    pub engines: HashMap<String, EnginePlaytimeEntry>,
    pub days: HashMap<String, DayPlaytimeEntry>,
    pub sessions: Vec<PlaytimeSessionEntry>,
}

impl Default for PlaytimeDb {
    fn default() -> Self {
        Self {
            schema_version: 1,
            last_updated: Utc::now().timestamp_millis(),
            total_seconds_all_engines: 0,
            engines: HashMap::new(),
            days: HashMap::new(),
            sessions: Vec::new(),
        }
    }
}

impl PlaytimeDb {
    pub fn load(paths: &LibraryPaths) -> anyhow::Result<Self> {
        Ok(read_json_or_default(&paths.playtime_json))
    }

    pub fn save(&self, paths: &LibraryPaths) -> anyhow::Result<()> {
        write_json_atomic(&paths.playtime_json, self)
    }
}

#[derive(Debug, Clone)]
pub struct RunningEngine {
    pub engine_id: String,
    pub pid: u32,
    pub process_name: String,
}

pub struct EngineManager;

impl Default for EngineManager {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineManager {
    pub fn new() -> Self {
        Self
    }

    pub fn definitions(&self) -> &[EngineDefinition] {
        &ENGINE_DEFINITIONS
    }

    pub fn definition(&self, engine_id: &str) -> Option<&EngineDefinition> {
        ENGINE_DEFINITIONS.iter().find(|def| def.id == engine_id)
    }

    pub fn scan_local_engines(&self, paths: &LibraryPaths) -> anyhow::Result<EngineUpdateState> {
        if !paths.engines_dir.exists() {
            fs::create_dir_all(&paths.engines_dir)?;
        }

        let mut state = EngineUpdateState::load(paths)?;

        // Prune missing install directories from existing state
        for record in state.engines.values_mut() {
            record.installed_versions.retain(|v| {
                !v.install_path.is_empty() && Path::new(&v.install_path).exists()
            });
        }

        if let Ok(engine_dirs) = fs::read_dir(&paths.engines_dir) {
            for engine_entry in engine_dirs.flatten() {
                let Ok(ft) = engine_entry.file_type() else {
                    continue;
                };
                if !ft.is_dir() {
                    continue;
                }
                let engine_id = engine_entry.file_name().to_string_lossy().to_string();
                let Some(def) = self.definition(&engine_id) else {
                    continue;
                };

                let record = state
                    .engines
                    .entry(engine_id.clone())
                    .or_insert_with(|| EngineRecord {
                        engine_id: engine_id.clone(),
                        ..Default::default()
                    });

                let Ok(version_dirs) = fs::read_dir(engine_entry.path()) else {
                    continue;
                };
                for ver_entry in version_dirs.flatten() {
                    let Ok(vft) = ver_entry.file_type() else {
                        continue;
                    };
                    if !vft.is_dir() {
                        continue;
                    }
                    let version_name = ver_entry.file_name().to_string_lossy().to_string();
                    if version_name.starts_with('.') {
                        continue;
                    }

                    let ver_path = ver_entry.path();
                    let detected_exe = Self::find_engine_executable(&ver_path, def.default_exe_name)
                        .map(|p| p.to_string_lossy().to_string());

                    if let Some(existing) = record
                        .installed_versions
                        .iter_mut()
                        .find(|v| v.version == version_name)
                    {
                        let exe_missing = existing
                            .executable_path
                            .as_deref()
                            .is_none_or(|p| !Path::new(p).exists());
                        if exe_missing && detected_exe.is_some() {
                            existing.executable_path = detected_exe;
                        }
                    } else {
                        let is_first = record.installed_versions.is_empty();
                        record.installed_versions.push(InstalledEngineVersion {
                            version: version_name.clone(),
                            tag: version_name.clone(),
                            name: format!("{} {}", def.name, version_name),
                            install_path: ver_path.to_string_lossy().to_string(),
                            executable_path: detected_exe,
                            installed_at: Utc::now().timestamp_millis(),
                            is_default: is_first,
                            ..Default::default()
                        });
                    }
                }
            }
        }

        // Reconcile active_version & is_default for every engine record
        for record in state.engines.values_mut() {
            if record.installed_versions.is_empty() {
                record.active_version = None;
                continue;
            }
            let active_valid = record.active_version.as_deref().is_some_and(|act| {
                record.installed_versions.iter().any(|v| v.version == act)
            });
            if !active_valid {
                let default_ver = record
                    .installed_versions
                    .iter()
                    .find(|v| v.is_default)
                    .or_else(|| record.installed_versions.first())
                    .map(|v| v.version.clone());
                record.active_version = default_ver;
            }
            if let Some(active) = record.active_version.as_deref() {
                for v in &mut record.installed_versions {
                    v.is_default = v.version == active;
                }
            }
        }

        state.save(paths)?;
        Ok(state)
    }

    pub fn find_engine_executable(dir: &Path, preferred_exe: &str) -> Option<PathBuf> {
        if !dir.exists() {
            return None;
        }

        let preferred_lower = preferred_exe.trim().to_ascii_lowercase();
        let preferred_stem = preferred_lower
            .strip_suffix(".exe")
            .unwrap_or(&preferred_lower)
            .to_string();

        let mut queue = std::collections::VecDeque::new();
        queue.push_back((dir.to_path_buf(), 0usize));

        let mut fallback: Option<PathBuf> = None;

        while let Some((current, depth)) = queue.pop_front() {
            let Ok(entries) = fs::read_dir(&current) else {
                continue;
            };

            let mut subdirs = Vec::new();
            for entry in entries.flatten() {
                let Ok(ft) = entry.file_type() else {
                    continue;
                };
                let name = entry.file_name().to_string_lossy().to_string();
                let lower = name.to_ascii_lowercase();
                let path = entry.path();

                if ft.is_dir() {
                    if depth < 2 && lower != "mods" && !lower.starts_with('.') {
                        subdirs.push(path);
                    }
                    continue;
                }

                if !ft.is_file() {
                    continue;
                }

                if lower.contains("unins") || lower.contains("crash") || lower.contains("unitycrash") {
                    continue;
                }

                if is_preferred_exe_match(&lower, &preferred_lower, &preferred_stem)
                    && is_platform_executable_candidate(&path, &lower)
                {
                    return Some(path);
                }

                if fallback.is_none() && is_platform_executable_candidate(&path, &lower) {
                    fallback = Some(path);
                }
            }

            for sub in subdirs {
                queue.push_back((sub, depth + 1));
            }
        }

        fallback
    }

    pub async fn install_release<F>(
        &self,
        paths: &LibraryPaths,
        client: &reqwest::Client,
        engine_id: &str,
        tag: &str,
        release_name: &str,
        asset: &EngineAssetCandidate,
        cancel: &AtomicBool,
        mut on_progress: F,
    ) -> anyhow::Result<InstalledEngineVersion>
    where
        F: FnMut(f32, &str) + Send,
    {
        if cancel.load(Ordering::Relaxed) {
            anyhow::bail!("Instalación cancelada");
        }

        let def = self
            .definition(engine_id)
            .ok_or_else(|| anyhow::anyhow!("Engine desconocido: {engine_id}"))?;

        let safe_tag = sanitize_folder_name(tag);
        let target_dir = paths.engines_dir.join(engine_id).join(&safe_tag);
        let dir_created = !target_dir.exists();
        tokio::fs::create_dir_all(&target_dir).await?;

        let safe_asset_name = sanitize_folder_name(&asset.name);
        let temp_archive = target_dir.join(format!(".temp_download_{safe_asset_name}"));

        let dl_res = host::download_file_with_progress(
            client,
            &asset.download_url,
            &temp_archive,
            cancel,
            |pct| {
                on_progress((pct * 0.85).clamp(0.0, 0.85), "Descargando archivos...");
            },
        )
        .await;

        if let Err(err) = dl_res {
            let _ = tokio::fs::remove_file(&temp_archive).await;
            if dir_created {
                let _ = tokio::fs::remove_dir_all(&target_dir).await;
            }
            return Err(err);
        }

        if cancel.load(Ordering::Relaxed) {
            let _ = tokio::fs::remove_file(&temp_archive).await;
            if dir_created {
                let _ = tokio::fs::remove_dir_all(&target_dir).await;
            }
            anyhow::bail!("Instalación cancelada");
        }

        on_progress(0.90, "Extrayendo archivos...");
        let ext = asset_extension(&asset.name);
        let archive_clone = temp_archive.clone();
        let target_clone = target_dir.clone();
        let asset_name_clone = asset.name.clone();

        let extract_res = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            match ext {
                ".zip" | "" => {
                    host::extract_zip_to_dir(&archive_clone, &target_clone, true)?;
                    let _ = fs::remove_file(&archive_clone);
                    Ok(())
                }
                ".appimage" => {
                    let dest_file = target_clone.join(&asset_name_clone);
                    if dest_file.exists() {
                        let _ = fs::remove_file(&dest_file);
                    }
                    fs::rename(&archive_clone, &dest_file)?;
                    #[cfg(not(target_os = "windows"))]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        if let Ok(meta) = fs::metadata(&dest_file) {
                            let mut perms = meta.permissions();
                            perms.set_mode(perms.mode() | 0o755);
                            let _ = fs::set_permissions(&dest_file, perms);
                        }
                    }
                    Ok(())
                }
                ".tar.gz" | ".tgz" => {
                    extract_tar_gz(&archive_clone, &target_clone)?;
                    let _ = fs::remove_file(&archive_clone);
                    host::flatten_single_root_dir(&target_clone)?;
                    Ok(())
                }
                other => {
                    let _ = fs::remove_file(&archive_clone);
                    anyhow::bail!(
                        "Formato de archivo no soportado ({other}). Selecciona una versión con archivo .zip."
                    );
                }
            }
        })
        .await?;

        if let Err(err) = extract_res {
            let _ = tokio::fs::remove_file(&temp_archive).await;
            if dir_created {
                let _ = tokio::fs::remove_dir_all(&target_dir).await;
            }
            return Err(err);
        }

        if cancel.load(Ordering::Relaxed) {
            if dir_created {
                let _ = tokio::fs::remove_dir_all(&target_dir).await;
            }
            anyhow::bail!("Instalación cancelada");
        }

        let detected_exe = Self::find_engine_executable(&target_dir, def.default_exe_name)
            .map(|p| p.to_string_lossy().to_string());

        let mut state = EngineUpdateState::load(paths)?;
        let record = state
            .engines
            .entry(engine_id.to_string())
            .or_insert_with(|| EngineRecord {
                engine_id: engine_id.to_string(),
                ..Default::default()
            });

        let already_default = record
            .installed_versions
            .iter()
            .find(|v| v.version == safe_tag)
            .is_some_and(|v| v.is_default);
        let is_default = record.installed_versions.is_empty() || already_default;

        let display_name = if release_name.trim().is_empty() {
            format!("{} {}", def.name, tag)
        } else {
            release_name.to_string()
        };

        let installed = InstalledEngineVersion {
            version: safe_tag.clone(),
            tag: tag.to_string(),
            name: display_name,
            release_id: None,
            asset_name: Some(asset.name.clone()),
            asset_size: Some(asset.size),
            download_url: Some(asset.download_url.clone()),
            install_path: target_dir.to_string_lossy().to_string(),
            executable_path: detected_exe,
            installed_at: Utc::now().timestamp_millis(),
            is_default,
        };

        if let Some(idx) = record
            .installed_versions
            .iter()
            .position(|v| v.version == safe_tag || v.tag == tag)
        {
            record.installed_versions[idx] = installed.clone();
        } else {
            record.installed_versions.push(installed.clone());
        }

        if is_default || record.active_version.is_none() {
            record.active_version = Some(installed.version.clone());
        }
        if let Some(active) = record.active_version.as_deref() {
            for v in &mut record.installed_versions {
                v.is_default = v.version == active;
            }
        }

        state.save(paths)?;
        on_progress(1.0, "Instalación completada");
        Ok(installed)
    }

    pub fn set_default_version(
        &self,
        paths: &LibraryPaths,
        engine_id: &str,
        version: &str,
    ) -> anyhow::Result<()> {
        let mut state = EngineUpdateState::load(paths)?;
        let record = state
            .engines
            .get_mut(engine_id)
            .ok_or_else(|| anyhow::anyhow!("El engine '{engine_id}' no tiene versiones instaladas"))?;

        let found = record
            .installed_versions
            .iter()
            .find(|v| v.version == version || v.tag == version)
            .map(|v| v.version.clone())
            .ok_or_else(|| {
                anyhow::anyhow!("Versión '{version}' no encontrada para {engine_id}")
            })?;

        record.active_version = Some(found.clone());
        for v in &mut record.installed_versions {
            v.is_default = v.version == found;
        }
        state.save(paths)?;
        Ok(())
    }

    pub fn uninstall_version(
        &self,
        paths: &LibraryPaths,
        engine_id: &str,
        version: &str,
    ) -> anyhow::Result<()> {
        let mut state = EngineUpdateState::load(paths)?;
        let safe_ver = sanitize_folder_name(version);
        let fallback_dir = paths.engines_dir.join(engine_id).join(&safe_ver);

        if let Some(record) = state.engines.get_mut(engine_id) {
            let mut dirs_to_remove = vec![fallback_dir.clone()];
            for v in &record.installed_versions {
                if v.version == version || v.tag == version {
                    if !v.install_path.is_empty() {
                        dirs_to_remove.push(PathBuf::from(&v.install_path));
                    }
                }
            }
            for dir in dirs_to_remove {
                if dir.exists() {
                    let _ = fs::remove_dir_all(&dir);
                }
            }

            record
                .installed_versions
                .retain(|v| v.version != version && v.tag != version);

            if record.active_version.as_deref() == Some(version)
                || record.active_version.as_deref() == Some(safe_ver.as_str())
                || record.active_version.is_none()
            {
                record.active_version = record
                    .installed_versions
                    .first()
                    .map(|v| v.version.clone());
            }

            if let Some(active) = record.active_version.as_deref() {
                for v in &mut record.installed_versions {
                    v.is_default = v.version == active;
                }
            }
        } else if fallback_dir.exists() {
            fs::remove_dir_all(&fallback_dir)?;
        }

        state.save(paths)?;
        Ok(())
    }

    pub fn launch_version(
        &self,
        paths: &LibraryPaths,
        engine_id: &str,
        version: Option<&str>,
    ) -> anyhow::Result<u32> {
        let mut state = EngineUpdateState::load(paths)?;
        if state
            .engines
            .get(engine_id)
            .is_none_or(|r| r.installed_versions.is_empty())
        {
            state = self.scan_local_engines(paths)?;
        }

        let def = self.definition(engine_id);
        let preferred_exe = def.map(|d| d.default_exe_name).unwrap_or("");

        let record = state
            .engines
            .get(engine_id)
            .ok_or_else(|| anyhow::anyhow!("El engine '{engine_id}' no está instalado"))?;

        let chosen = if let Some(target_ver) = version.filter(|v| !v.is_empty()) {
            record
                .installed_versions
                .iter()
                .find(|v| v.version == target_ver || v.tag == target_ver)
        } else {
            let active = record.active_version.as_deref();
            record
                .installed_versions
                .iter()
                .find(|v| Some(v.version.as_str()) == active)
                .or_else(|| record.installed_versions.iter().find(|v| v.is_default))
                .or_else(|| record.installed_versions.first())
        }
        .ok_or_else(|| anyhow::anyhow!("No hay una versión instalada para '{engine_id}'"))?;

        let install_dir = PathBuf::from(&chosen.install_path);
        let exe_path = chosen
            .executable_path
            .as_deref()
            .map(PathBuf::from)
            .filter(|p| p.exists())
            .or_else(|| Self::find_engine_executable(&install_dir, preferred_exe))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No se encontró el ejecutable de {} ({})",
                    engine_id,
                    chosen.version
                )
            })?;

        let work_dir = exe_path.parent().unwrap_or(&install_dir);
        let pid = host::launch_executable(&exe_path, Some(work_dir))?;
        let _ = self.record_launch(paths, engine_id);
        Ok(pid)
    }

    pub fn detect_running_engines(&self) -> Vec<RunningEngine> {
        self.detect_running_engines_with_state(None)
    }

    pub fn detect_running_engines_with_state(
        &self,
        state: Option<&EngineUpdateState>,
    ) -> Vec<RunningEngine> {
        let mut sys = System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

        let mut running = Vec::new();
        for process in sys.processes().values() {
            let proc_name = process.name().to_string_lossy().to_string();
            let proc_lower = proc_name.to_ascii_lowercase();
            let proc_stem = strip_exe_suffix(&proc_lower);

            for def in &ENGINE_DEFINITIONS {
                let mut candidates = engine_candidate_names(def.id);
                if let Some(st) = state {
                    if let Some(rec) = st.engines.get(def.id) {
                        for v in &rec.installed_versions {
                            if let Some(exe) = v.executable_path.as_deref() {
                                if let Some(fname) = Path::new(exe).file_name() {
                                    candidates.push(fname.to_string_lossy().to_ascii_lowercase());
                                }
                            }
                        }
                    }
                }

                let matched = candidates.iter().any(|cand| {
                    let cand_stem = strip_exe_suffix(cand);
                    proc_lower == *cand || proc_stem == cand_stem
                });

                if matched {
                    running.push(RunningEngine {
                        engine_id: def.id.to_string(),
                        pid: process.pid().as_u32(),
                        process_name: proc_name.clone(),
                    });
                    break;
                }
            }
        }
        running
    }

    pub fn stop_engine(&self, engine_id: &str) -> anyhow::Result<bool> {
        let mut sys = System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

        let candidates = engine_candidate_names(engine_id);
        let mut killed_any = false;

        for process in sys.processes().values() {
            let proc_lower = process.name().to_string_lossy().to_ascii_lowercase();
            let proc_stem = strip_exe_suffix(&proc_lower);
            let matched = candidates.iter().any(|cand| {
                let cand_stem = strip_exe_suffix(cand);
                proc_lower == *cand || proc_stem == cand_stem
            });

            if matched && process.kill() {
                killed_any = true;
            }
        }

        Ok(killed_any)
    }

    pub fn record_launch(&self, paths: &LibraryPaths, engine_id: &str) -> anyhow::Result<()> {
        let mut db = PlaytimeDb::load(paths)?;
        let now = Utc::now().timestamp_millis();
        let today = Local::now().format("%Y-%m-%d").to_string();

        let entry = db
            .engines
            .entry(engine_id.to_string())
            .or_insert_with(|| EnginePlaytimeEntry {
                engine_id: engine_id.to_string(),
                first_played: now,
                last_played: now,
                ..Default::default()
            });
        entry.total_launches = entry.total_launches.saturating_add(1);
        entry.last_played = now;
        let engine_day = entry.days.entry(today.clone()).or_default();
        engine_day.launches = engine_day.launches.saturating_add(1);

        let day_entry = db.days.entry(today.clone()).or_insert_with(|| DayPlaytimeEntry {
            date: today,
            ..Default::default()
        });
        day_entry.total_launches = day_entry.total_launches.saturating_add(1);
        let day_eng = day_entry.engines.entry(engine_id.to_string()).or_default();
        day_eng.launches = day_eng.launches.saturating_add(1);

        db.last_updated = now;
        db.save(paths)?;
        Ok(())
    }

    pub fn record_playtime(
        &self,
        paths: &LibraryPaths,
        engine_id: &str,
        seconds: u64,
    ) -> anyhow::Result<()> {
        if seconds == 0 {
            return Ok(());
        }

        let mut db = PlaytimeDb::load(paths)?;
        let now = Utc::now().timestamp_millis();
        let today = Local::now().format("%Y-%m-%d").to_string();

        let entry = db
            .engines
            .entry(engine_id.to_string())
            .or_insert_with(|| EnginePlaytimeEntry {
                engine_id: engine_id.to_string(),
                total_launches: 1,
                first_played: now,
                last_played: now,
                ..Default::default()
            });
        entry.total_seconds = entry.total_seconds.saturating_add(seconds);
        entry.last_played = now;
        let engine_day = entry.days.entry(today.clone()).or_default();
        engine_day.seconds = engine_day.seconds.saturating_add(seconds);

        let day_entry = db.days.entry(today.clone()).or_insert_with(|| DayPlaytimeEntry {
            date: today,
            ..Default::default()
        });
        day_entry.total_seconds = day_entry.total_seconds.saturating_add(seconds);
        let day_eng = day_entry.engines.entry(engine_id.to_string()).or_default();
        day_eng.seconds = day_eng.seconds.saturating_add(seconds);

        db.total_seconds_all_engines = db.total_seconds_all_engines.saturating_add(seconds);
        db.last_updated = now;
        db.save(paths)?;
        Ok(())
    }

    pub fn get_playtime_seconds(&self, paths: &LibraryPaths, engine_id: &str) -> u64 {
        PlaytimeDb::load(paths)
            .ok()
            .and_then(|db| db.engines.get(engine_id).map(|e| e.total_seconds))
            .unwrap_or(0)
    }

    pub fn get_playtime_minutes(&self, paths: &LibraryPaths, engine_id: &str) -> u64 {
        self.get_playtime_seconds(paths, engine_id) / 60
    }
}

pub fn format_playtime_minutes(minutes: u64) -> String {
    if minutes < 60 {
        format!("{minutes} min")
    } else {
        let h = minutes / 60;
        let m = minutes % 60;
        if m == 0 {
            format!("{h}h")
        } else {
            format!("{h}h {m}m")
        }
    }
}

fn engine_candidate_names(engine_id: &str) -> Vec<String> {
    match engine_id {
        "psych" => vec![
            "psychengine.exe".into(),
            "psychengine".into(),
            "psych.exe".into(),
            "psych".into(),
        ],
        "vslice" => vec![
            "funkin.exe".into(),
            "funkin".into(),
            "fnf.exe".into(),
            "fnf".into(),
        ],
        "codename" => vec![
            "codenameengine.exe".into(),
            "codenameengine".into(),
            "codename.exe".into(),
            "codename".into(),
        ],
        "pslice" => vec![
            "p-slice.exe".into(),
            "p-slice".into(),
            "pslice.exe".into(),
            "pslice".into(),
        ],
        "psychonline" => vec![
            "psychonline.exe".into(),
            "psychonline".into(),
        ],
        "fpsplus" => vec![
            "fpsplus.exe".into(),
            "fpsplus".into(),
        ],
        other => {
            let lower = other.to_ascii_lowercase();
            vec![format!("{lower}.exe"), lower]
        }
    }
}

fn strip_exe_suffix(name: &str) -> &str {
    name.strip_suffix(".exe")
        .or_else(|| name.strip_suffix(".x86_64"))
        .or_else(|| name.strip_suffix(".appimage"))
        .unwrap_or(name)
}

fn is_preferred_exe_match(lower: &str, preferred_lower: &str, preferred_stem: &str) -> bool {
    if preferred_lower.is_empty() {
        return false;
    }
    if lower == preferred_lower {
        return true;
    }
    #[cfg(not(target_os = "windows"))]
    {
        if !preferred_stem.is_empty() && strip_exe_suffix(lower) == preferred_stem {
            return true;
        }
    }
    #[cfg(target_os = "windows")]
    let _ = preferred_stem;

    false
}

fn is_platform_executable_candidate(path: &Path, lower_name: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        let _ = path;
        lower_name.ends_with(".exe")
    }

    #[cfg(not(target_os = "windows"))]
    {
        if lower_name.ends_with(".so")
            || lower_name.contains(".so.")
            || lower_name.ends_with(".dll")
            || lower_name.ends_with(".exe")
        {
            return false;
        }

        if lower_name.ends_with(".appimage")
            || lower_name.ends_with(".x86_64")
            || lower_name.ends_with(".bin")
        {
            return true;
        }

        if path.extension().is_none() {
            use std::os::unix::fs::PermissionsExt;
            let has_exec_bit = fs::metadata(path)
                .map(|m| m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false);
            return has_exec_bit || is_elf_binary(path);
        }

        false
    }
}

#[cfg(not(target_os = "windows"))]
fn is_elf_binary(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut f) = fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 4];
    f.read_exact(&mut magic).is_ok() && magic == [0x7F, b'E', b'L', b'F']
}

pub(crate) fn extract_tar_gz(archive: &Path, dest_dir: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dest_dir)?;
    let mut cmd = Command::new("tar");
    cmd.arg("-xzf").arg(archive).arg("-C").arg(dest_dir);

    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let out = cmd.output()?;
    if !out.status.success() {
        anyhow::bail!(
            "Error extrayendo tar.gz: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}
