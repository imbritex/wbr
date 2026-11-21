use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::Utc;
use serde::{Deserialize, Deserializer, Serialize};

use crate::core::engines::{ENGINE_DEFINITIONS, EngineManager, EngineUpdateState, extract_tar_gz};
use crate::core::host;
use crate::core::storage::{
    LibraryPaths, dir_size_bytes, read_json_or_default, sanitize_folder_name, write_json_atomic,
};

const UNDETECTED_ENGINE_WARNING: &str = "Cannot detect engine. This mod contains no reference files (_polymod_meta.json, pack.json, or executable) to index its engine. Check the folder to verify if it is a playable mod.";

fn default_true() -> bool {
    true
}

fn deserialize_string_or_number<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let val = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(match val {
        Some(serde_json::Value::String(s)) => s,
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => String::new(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct InstalledModRecord {
    #[serde(deserialize_with = "deserialize_string_or_number")]
    pub id: String,
    #[serde(alias = "name")]
    pub title: String,
    pub description: String,
    pub author: String,
    pub engine_id: String,
    pub engine_name: String,
    pub folder_name: String,
    pub install_path: String,
    #[serde(alias = "localCoverPath")]
    pub cover_path: String,
    #[serde(alias = "fileSize")]
    pub size_bytes: u64,
    pub installed_at: i64,
    #[serde(default = "default_true")]
    pub visible: bool,
    #[serde(alias = "hasEngineWarning")]
    pub has_warning: bool,
    #[serde(alias = "engineWarningMessage")]
    pub warning_message: String,
    #[serde(alias = "sourceUrl")]
    pub page_url: String,
}

impl Default for InstalledModRecord {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            description: String::new(),
            author: String::new(),
            engine_id: "psych".to_string(),
            engine_name: "Psych Engine".to_string(),
            folder_name: String::new(),
            install_path: String::new(),
            cover_path: String::new(),
            size_bytes: 0,
            installed_at: 0,
            visible: true,
            has_warning: false,
            warning_message: String::new(),
            page_url: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct ModProfile {
    pub id: String,
    pub name: String,
    pub mod_ids: Vec<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct ModInstallRequest {
    pub mod_id: String,
    pub title: String,
    pub description: String,
    pub author: String,
    pub engine_id: String,
    pub engine_name: String,
    pub download_url: String,
    pub archive_filename: String,
    pub cover_url: Option<String>,
    pub page_url: String,
}

pub struct ModManager;

impl Default for ModManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ModManager {
    pub fn new() -> Self {
        Self
    }

    pub fn load_installed_mods(&self, paths: &LibraryPaths) -> anyhow::Result<Vec<InstalledModRecord>> {
        let mut records: Vec<InstalledModRecord> = read_json_or_default(&paths.installed_mods_json);

        // Also check legacy installed-mods.json if primary manifest was empty
        if records.is_empty() {
            let legacy_path = paths.data_dir.join("installed-mods.json");
            if legacy_path.exists() {
                records = read_json_or_default(&legacy_path);
            }
        }

        let mut known_folders: HashSet<String> = HashSet::new();
        let mut known_ids: HashSet<String> = HashSet::new();

        for rec in &mut records {
            if rec.install_path.is_empty() && !rec.folder_name.is_empty() {
                rec.install_path = paths
                    .mods_dir
                    .join(&rec.folder_name)
                    .to_string_lossy()
                    .to_string();
            }
            if rec.folder_name.is_empty() && !rec.install_path.is_empty() {
                if let Some(name) = Path::new(&rec.install_path).file_name() {
                    rec.folder_name = name.to_string_lossy().to_string();
                }
            }
            if rec.id.is_empty() {
                rec.id = rec.folder_name.clone();
            }
            if rec.title.is_empty() {
                rec.title = rec.folder_name.clone();
            }

            let resolved_dir = PathBuf::from(&rec.install_path);
            if rec.size_bytes == 0 && resolved_dir.exists() {
                rec.size_bytes = dir_size_bytes(&resolved_dir);
            }

            if !rec.folder_name.is_empty() {
                known_folders.insert(rec.folder_name.to_ascii_lowercase());
            }
            if !rec.id.is_empty() {
                known_ids.insert(rec.id.to_ascii_lowercase());
            }
        }

        // Scan mods/ directory for unlisted mod folders
        if paths.mods_dir.exists() {
            if let Ok(entries) = fs::read_dir(&paths.mods_dir) {
                for entry in entries.flatten() {
                    let Ok(ft) = entry.file_type() else {
                        continue;
                    };
                    if !ft.is_dir() {
                        continue;
                    }
                    let folder_name = entry.file_name().to_string_lossy().to_string();
                    if folder_name.starts_with('.') {
                        continue;
                    }
                    if known_folders.contains(&folder_name.to_ascii_lowercase()) {
                        continue;
                    }

                    let dir_path = entry.path();
                    let discovered = self.build_record_from_folder(&dir_path, &folder_name);
                    if !known_ids.contains(&discovered.id.to_ascii_lowercase()) {
                        known_folders.insert(folder_name.to_ascii_lowercase());
                        known_ids.insert(discovered.id.to_ascii_lowercase());
                        records.push(discovered);
                    }
                }
            }
        }

        Ok(records)
    }

    pub fn save_installed_mods(
        &self,
        paths: &LibraryPaths,
        mods: &[InstalledModRecord],
    ) -> anyhow::Result<()> {
        write_json_atomic(&paths.installed_mods_json, mods)
    }

    pub fn load_id_set(&self, path: &Path) -> anyhow::Result<HashSet<String>> {
        let items: Vec<serde_json::Value> = read_json_or_default(path);
        let mut set = HashSet::with_capacity(items.len());
        for item in items {
            match item {
                serde_json::Value::String(s) if !s.is_empty() => {
                    set.insert(s);
                }
                serde_json::Value::Number(n) => {
                    set.insert(n.to_string());
                }
                _ => {}
            }
        }
        Ok(set)
    }

    pub fn toggle_id(&self, path: &Path, id: &str) -> anyhow::Result<bool> {
        let mut set = self.load_id_set(path)?;
        let now_present = if set.contains(id) {
            set.remove(id);
            false
        } else {
            set.insert(id.to_string());
            true
        };
        let mut list: Vec<String> = set.into_iter().collect();
        list.sort();
        write_json_atomic(path, &list)?;
        Ok(now_present)
    }

    pub fn load_favorites(&self, paths: &LibraryPaths) -> anyhow::Result<HashSet<String>> {
        self.load_id_set(&paths.favorites_json)
    }

    pub fn toggle_favorite(&self, paths: &LibraryPaths, mod_id: &str) -> anyhow::Result<bool> {
        self.toggle_id(&paths.favorites_json, mod_id)
    }

    pub fn load_bookmarks(&self, paths: &LibraryPaths) -> anyhow::Result<HashSet<String>> {
        self.load_id_set(&paths.bookmarks_json)
    }

    pub fn toggle_bookmark(&self, paths: &LibraryPaths, mod_id: &str) -> anyhow::Result<bool> {
        self.toggle_id(&paths.bookmarks_json, mod_id)
    }

    pub fn load_profiles(&self, paths: &LibraryPaths) -> anyhow::Result<Vec<ModProfile>> {
        Ok(read_json_or_default(&paths.profiles_json))
    }

    pub fn save_profiles(&self, paths: &LibraryPaths, profiles: &[ModProfile]) -> anyhow::Result<()> {
        write_json_atomic(&paths.profiles_json, profiles)
    }

    pub fn create_profile(&self, paths: &LibraryPaths, name: &str) -> anyhow::Result<ModProfile> {
        let mut profiles = self.load_profiles(paths)?;
        let trimmed = name.trim();
        let profile_name = if trimmed.is_empty() {
            "New Profile".to_string()
        } else {
            trimmed.to_string()
        };
        let now = Utc::now().timestamp_millis();
        let profile = ModProfile {
            id: format!("profile-{now}-{}", profiles.len() + 1),
            name: profile_name,
            mod_ids: Vec::new(),
            created_at: now,
        };
        profiles.push(profile.clone());
        self.save_profiles(paths, &profiles)?;
        Ok(profile)
    }

    pub fn delete_profile(&self, paths: &LibraryPaths, profile_id: &str) -> anyhow::Result<bool> {
        let mut profiles = self.load_profiles(paths)?;
        let prev_len = profiles.len();
        profiles.retain(|p| p.id != profile_id);
        if profiles.len() == prev_len {
            return Ok(false);
        }
        self.save_profiles(paths, &profiles)?;
        Ok(true)
    }

    pub fn toggle_mod_in_profile(
        &self,
        paths: &LibraryPaths,
        profile_id: &str,
        mod_id: &str,
    ) -> anyhow::Result<bool> {
        let mut profiles = self.load_profiles(paths)?;
        let profile = profiles
            .iter_mut()
            .find(|p| p.id == profile_id)
            .ok_or_else(|| anyhow::anyhow!("Perfil no encontrado: {profile_id}"))?;

        let now_in_profile = if let Some(pos) = profile.mod_ids.iter().position(|id| id == mod_id) {
            profile.mod_ids.remove(pos);
            false
        } else {
            profile.mod_ids.push(mod_id.to_string());
            true
        };

        self.save_profiles(paths, &profiles)?;
        Ok(now_in_profile)
    }

    pub fn toggle_visibility(&self, paths: &LibraryPaths, mod_id: &str) -> anyhow::Result<bool> {
        let mut mods = self.load_installed_mods(paths)?;
        let record = mods
            .iter_mut()
            .find(|m| matches_mod_id(m, mod_id))
            .ok_or_else(|| anyhow::anyhow!("Mod no encontrado: {mod_id}"))?;

        record.visible = !record.visible;
        let next_visible = record.visible;
        self.save_installed_mods(paths, &mods)?;
        Ok(next_visible)
    }

    pub async fn install_mod_from_url<F>(
        &self,
        paths: &LibraryPaths,
        client: &reqwest::Client,
        req: ModInstallRequest,
        cancel: &AtomicBool,
        mut on_progress: F,
    ) -> anyhow::Result<InstalledModRecord>
    where
        F: FnMut(f32, &str) + Send,
    {
        if cancel.load(Ordering::Relaxed) {
            anyhow::bail!("Instalación cancelada");
        }

        tokio::fs::create_dir_all(&paths.mods_dir).await?;
        tokio::fs::create_dir_all(&paths.mod_covers_dir).await?;

        let existing_mods = self.load_installed_mods(paths)?;
        let base_slug = build_mod_slug(&req.title, &req.mod_id);
        let folder_name = existing_mods
            .iter()
            .find(|m| matches_mod_id(m, &req.mod_id))
            .map(|m| m.folder_name.clone())
            .filter(|f| !f.is_empty())
            .unwrap_or_else(|| unique_folder_slug(&paths.mods_dir, &base_slug));

        let mod_dir = paths.mods_dir.join(&folder_name);
        let dir_created = !mod_dir.exists();
        tokio::fs::create_dir_all(&mod_dir).await?;

        let archive_ext = detect_archive_ext(&req.archive_filename, &req.download_url);
        let safe_id = sanitize_folder_name(&req.mod_id);
        let temp_archive = paths
            .mods_dir
            .join(format!(".temp_mod_{safe_id}{archive_ext}"));

        let dl_res = host::download_file_with_progress(
            client,
            &req.download_url,
            &temp_archive,
            cancel,
            |pct| {
                on_progress((pct * 0.80).clamp(0.0, 0.80), "Descargando mod...");
            },
        )
        .await;

        if let Err(err) = dl_res {
            let _ = tokio::fs::remove_file(&temp_archive).await;
            if dir_created {
                let _ = tokio::fs::remove_dir_all(&mod_dir).await;
            }
            return Err(err);
        }

        if cancel.load(Ordering::Relaxed) {
            let _ = tokio::fs::remove_file(&temp_archive).await;
            if dir_created {
                let _ = tokio::fs::remove_dir_all(&mod_dir).await;
            }
            anyhow::bail!("Instalación cancelada");
        }

        on_progress(0.86, "Extrayendo archivos...");
        let archive_clone = temp_archive.clone();
        let mod_dir_clone = mod_dir.clone();
        let ext_owned = archive_ext.to_string();

        let extract_res = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            match ext_owned.as_str() {
                ".zip" | "" => {
                    host::extract_zip_to_dir(&archive_clone, &mod_dir_clone, true)?;
                    let _ = fs::remove_file(&archive_clone);
                    Ok(())
                }
                ".tar.gz" | ".tgz" => {
                    extract_tar_gz(&archive_clone, &mod_dir_clone)?;
                    let _ = fs::remove_file(&archive_clone);
                    host::flatten_single_root_dir(&mod_dir_clone)?;
                    Ok(())
                }
                other => {
                    let _ = fs::remove_file(&archive_clone);
                    anyhow::bail!(
                        "Formato de archivo no soportado ({other}). WeekBox soporta archivos .zip y .tar.gz."
                    );
                }
            }
        })
        .await?;

        if let Err(err) = extract_res {
            let _ = tokio::fs::remove_file(&temp_archive).await;
            if dir_created {
                let _ = tokio::fs::remove_dir_all(&mod_dir).await;
            }
            return Err(err);
        }

        if cancel.load(Ordering::Relaxed) {
            if dir_created {
                let _ = tokio::fs::remove_dir_all(&mod_dir).await;
            }
            anyhow::bail!("Instalación cancelada");
        }

        let mut cover_path = String::new();
        if let Some(cover_url) = req.cover_url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
            on_progress(0.94, "Descargando portada...");
            let cover_dest = paths.mod_covers_dir.join(format!("{safe_id}.img"));
            let dummy_cancel = AtomicBool::new(false);
            if host::download_file_with_progress(client, cover_url, &cover_dest, &dummy_cancel, |_| {})
                .await
                .is_ok()
            {
                cover_path = cover_dest.to_string_lossy().to_string();
            }
        }

        let detected = detect_mod_engine_from_files(&mod_dir, Some(&req.engine_id));
        let final_engine_name = engine_display_name(&detected.engine_id, &req.engine_name);
        let size_bytes = dir_size_bytes(&mod_dir);

        let record = InstalledModRecord {
            id: req.mod_id.clone(),
            title: if req.title.trim().is_empty() {
                folder_name.clone()
            } else {
                req.title
            },
            description: req.description,
            author: req.author,
            engine_id: detected.engine_id,
            engine_name: final_engine_name,
            folder_name,
            install_path: mod_dir.to_string_lossy().to_string(),
            cover_path,
            size_bytes,
            installed_at: Utc::now().timestamp_millis(),
            visible: true,
            has_warning: detected.has_warning,
            warning_message: detected.warning_message,
            page_url: req.page_url,
        };

        let mut mods = self.load_installed_mods(paths)?;
        if let Some(idx) = mods.iter().position(|m| matches_mod_id(m, &record.id)) {
            mods[idx] = record.clone();
        } else {
            mods.insert(0, record.clone());
        }
        self.save_installed_mods(paths, &mods)?;

        on_progress(1.0, "Mod instalado");
        Ok(record)
    }

    pub fn uninstall_mod(&self, paths: &LibraryPaths, mod_id: &str) -> anyhow::Result<()> {
        let mut mods = self.load_installed_mods(paths)?;
        let pos = mods
            .iter()
            .position(|m| matches_mod_id(m, mod_id))
            .ok_or_else(|| anyhow::anyhow!("Mod no encontrado: {mod_id}"))?;

        let removed = mods.remove(pos);

        let mod_dir = if !removed.install_path.is_empty() {
            PathBuf::from(&removed.install_path)
        } else {
            paths.mods_dir.join(&removed.folder_name)
        };
        if mod_dir.exists() {
            fs::remove_dir_all(&mod_dir)?;
        }

        if let Some(cover) = Self::resolve_cover_path(paths, &removed) {
            if cover.exists() {
                let _ = fs::remove_file(&cover);
            }
        }
        let safe_id = sanitize_folder_name(&removed.id);
        let img_cover = paths.mod_covers_dir.join(format!("{safe_id}.img"));
        if img_cover.exists() {
            let _ = fs::remove_file(&img_cover);
        }

        self.save_installed_mods(paths, &mods)?;

        // Clean mod reference from any profiles
        if let Ok(mut profiles) = self.load_profiles(paths) {
            let mut changed = false;
            for profile in &mut profiles {
                let prev = profile.mod_ids.len();
                profile
                    .mod_ids
                    .retain(|id| id != &removed.id && id != mod_id);
                if profile.mod_ids.len() != prev {
                    changed = true;
                }
            }
            if changed {
                let _ = self.save_profiles(paths, &profiles);
            }
        }

        Ok(())
    }

    pub fn sync_engine_mods(
        &self,
        paths: &LibraryPaths,
        engine_id: &str,
        engine_install_dir: &Path,
        mod_ids_override: Option<&[String]>,
    ) -> anyhow::Result<()> {
        let engine_mods_dir = engine_install_dir.join("mods");
        if !engine_mods_dir.exists() {
            fs::create_dir_all(&engine_mods_dir)?;
        }

        let installed = self.load_installed_mods(paths)?;
        let canonical_engine = resolve_mod_engine_id(engine_id);

        let mut known_names = HashSet::new();
        for m in &installed {
            if !m.folder_name.is_empty() {
                known_names.insert(m.folder_name.clone());
            }
            if !m.title.is_empty() {
                known_names.insert(m.title.clone());
            }
        }

        let mut desired_links: Vec<(&InstalledModRecord, String, PathBuf)> = Vec::new();
        let mut desired_folder_names = HashSet::new();

        for record in &installed {
            if !is_mod_compatible_with_engine(&record.engine_id, canonical_engine) {
                continue;
            }

            let included = match mod_ids_override {
                Some(ids) => ids.iter().any(|target| matches_mod_id(record, target)),
                None => record.visible,
            };
            if !included {
                continue;
            }

            let folder = if !record.folder_name.is_empty() {
                record.folder_name.clone()
            } else {
                sanitize_folder_name(&record.title)
            };
            let mod_path = if !record.install_path.is_empty() {
                PathBuf::from(&record.install_path)
            } else {
                paths.mods_dir.join(&folder)
            };

            if mod_path.exists() {
                desired_folder_names.insert(folder.clone());
                desired_links.push((record, folder, mod_path));
            }
        }

        // Remove stale junctions/symlinks or managed mod links not in desired set
        if let Ok(entries) = fs::read_dir(&engine_mods_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if desired_folder_names.contains(&name) {
                    continue;
                }
                let entry_path = entry.path();
                let is_symlink = fs::symlink_metadata(&entry_path)
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false);

                if is_symlink || known_names.contains(&name) {
                    let _ = host::remove_directory_junction(&entry_path);
                }
            }
        }

        // Create junctions/symlinks for active compatible mods
        for (_record, folder, mod_path) in desired_links {
            let link_path = engine_mods_dir.join(&folder);
            host::create_directory_junction(&link_path, &mod_path)?;
        }

        Ok(())
    }

    pub fn launch_mod(
        &self,
        paths: &LibraryPaths,
        engines: &EngineManager,
        mod_id: &str,
        preferred_version: Option<&str>,
    ) -> anyhow::Result<u32> {
        let mods = self.load_installed_mods(paths)?;
        let record = mods
            .iter()
            .find(|m| matches_mod_id(m, mod_id))
            .ok_or_else(|| anyhow::anyhow!("Mod no encontrado: {mod_id}"))?;

        let mod_dir = if !record.install_path.is_empty() {
            PathBuf::from(&record.install_path)
        } else {
            paths.mods_dir.join(&record.folder_name)
        };
        if !mod_dir.exists() {
            anyhow::bail!(
                "La carpeta del mod \"{}\" no existe en disco",
                record.title
            );
        }

        let lower_engine = record.engine_id.trim().to_ascii_lowercase();
        if lower_engine == "executable" || lower_engine == "exe" {
            let exe = EngineManager::find_engine_executable(&mod_dir, "").ok_or_else(|| {
                anyhow::anyhow!(
                    "No se encontró ningún ejecutable dentro de \"{}\"",
                    record.title
                )
            })?;
            return host::launch_executable(&exe, exe.parent());
        }

        let target_engine = resolve_mod_engine_id(&record.engine_id);
        let single_mod_ids = [record.id.clone()];
        self.launch_engine_with_mods(
            paths,
            engines,
            target_engine,
            preferred_version,
            Some(&single_mod_ids),
        )
    }

    pub fn launch_engine_with_mods(
        &self,
        paths: &LibraryPaths,
        engines: &EngineManager,
        engine_id: &str,
        version: Option<&str>,
        mod_ids_override: Option<&[String]>,
    ) -> anyhow::Result<u32> {
        let canonical_engine = resolve_mod_engine_id(engine_id);
        let mut state = EngineUpdateState::load(paths)?;
        if state
            .engines
            .get(canonical_engine)
            .is_none_or(|r| r.installed_versions.is_empty())
        {
            state = engines.scan_local_engines(paths)?;
        }

        let record = state.engines.get(canonical_engine).ok_or_else(|| {
            let name = engines
                .definition(canonical_engine)
                .map(|d| d.name)
                .unwrap_or(canonical_engine);
            anyhow::anyhow!(
                "El engine requerido ({name}) no está instalado. Instálalo en la pestaña Instancias."
            )
        })?;

        let chosen = if let Some(ver) = version.filter(|v| !v.is_empty()) {
            record
                .installed_versions
                .iter()
                .find(|v| v.version == ver || v.tag == ver)
        } else {
            let active = record.active_version.as_deref();
            record
                .installed_versions
                .iter()
                .find(|v| Some(v.version.as_str()) == active)
                .or_else(|| record.installed_versions.iter().find(|v| v.is_default))
                .or_else(|| record.installed_versions.first())
        }
        .ok_or_else(|| {
            anyhow::anyhow!("No hay ninguna versión instalada de '{canonical_engine}'")
        })?;

        let install_dir = PathBuf::from(&chosen.install_path);
        if !install_dir.exists() {
            anyhow::bail!(
                "La carpeta del engine {} ({}) no existe",
                canonical_engine,
                chosen.version
            );
        }

        self.sync_engine_mods(paths, canonical_engine, &install_dir, mod_ids_override)?;
        engines.launch_version(paths, canonical_engine, Some(&chosen.version))
    }

    pub fn resolve_cover_path(paths: &LibraryPaths, record: &InstalledModRecord) -> Option<PathBuf> {
        if !record.cover_path.is_empty() {
            let p = PathBuf::from(&record.cover_path);
            if p.is_absolute() && p.exists() {
                return Some(p);
            }
            let rel_data = paths.data_dir.join(&record.cover_path);
            if rel_data.exists() {
                return Some(rel_data);
            }
            let stripped = record
                .cover_path
                .trim_start_matches("mod-covers/")
                .trim_start_matches("mod-covers\\");
            let in_covers = paths.mod_covers_dir.join(stripped);
            if in_covers.exists() {
                return Some(in_covers);
            }
        }

        let safe_id = sanitize_folder_name(&record.id);
        for ext in ["img", "webp", "png", "jpg", "jpeg"] {
            let candidate = paths.mod_covers_dir.join(format!("{safe_id}.{ext}"));
            if candidate.exists() {
                return Some(candidate);
            }
        }
        None
    }

    fn build_record_from_folder(&self, dir_path: &Path, folder_name: &str) -> InstalledModRecord {
        let mod_json = dir_path.join("mod.json");
        if mod_json.exists() {
            if let Ok(raw) = fs::read_to_string(&mod_json) {
                if let Ok(mut parsed) = serde_json::from_str::<InstalledModRecord>(&raw) {
                    if parsed.id.is_empty() {
                        parsed.id = folder_name.to_string();
                    }
                    if parsed.title.is_empty() {
                        parsed.title = folder_name.to_string();
                    }
                    parsed.folder_name = folder_name.to_string();
                    parsed.install_path = dir_path.to_string_lossy().to_string();
                    if parsed.size_bytes == 0 {
                        parsed.size_bytes = dir_size_bytes(dir_path);
                    }
                    let detected = detect_mod_engine_from_files(dir_path, Some(&parsed.engine_id));
                    parsed.engine_id = detected.engine_id.clone();
                    parsed.engine_name =
                        engine_display_name(&detected.engine_id, &parsed.engine_name);
                    parsed.has_warning = detected.has_warning;
                    parsed.warning_message = detected.warning_message;
                    return parsed;
                }
            }
        }

        let detected = detect_mod_engine_from_files(dir_path, None);
        let engine_name = engine_display_name(&detected.engine_id, "");
        let (meta_title, meta_desc, meta_author) = read_embedded_mod_metadata(dir_path);

        let installed_at = fs::metadata(dir_path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or_else(|| Utc::now().timestamp_millis());

        InstalledModRecord {
            id: folder_name.to_string(),
            title: meta_title.unwrap_or_else(|| folder_name.to_string()),
            description: meta_desc.unwrap_or_default(),
            author: meta_author.unwrap_or_default(),
            engine_id: detected.engine_id,
            engine_name,
            folder_name: folder_name.to_string(),
            install_path: dir_path.to_string_lossy().to_string(),
            cover_path: String::new(),
            size_bytes: dir_size_bytes(dir_path),
            installed_at,
            visible: true,
            has_warning: detected.has_warning,
            warning_message: detected.warning_message,
            page_url: String::new(),
        }
    }
}

pub fn resolve_mod_engine_id(raw: &str) -> &'static str {
    let lower = raw.trim().to_ascii_lowercase();
    match lower.as_str() {
        "executable" | "exe" => "executable",
        "base" | "basegame" | "base game" | "juego base" | "v-slice" | "vslice" => "vslice",
        "psych" | "psych engine" => "psych",
        "codename" | "codename engine" => "codename",
        "pslice" | "p-slice" | "p-slice engine" => "pslice",
        "psychonline" | "psych online" => "psychonline",
        "fpsplus" | "fps plus" => "fpsplus",
        _ => {
            if lower.contains("codename") {
                "codename"
            } else if lower.contains("pslice") || lower.contains("p-slice") {
                "pslice"
            } else if lower.contains("psychonline") || lower.contains("online") {
                "psychonline"
            } else if lower.contains("fpsplus") || lower.contains("fps plus") {
                "fpsplus"
            } else if lower.contains("vslice") || lower.contains("v-slice") {
                "vslice"
            } else {
                "psych"
            }
        }
    }
}

pub fn is_mod_compatible_with_engine(mod_engine_id: &str, target_engine_id: &str) -> bool {
    let lower = mod_engine_id.trim().to_ascii_lowercase();
    if lower == "executable" || lower == "exe" {
        return false;
    }
    resolve_mod_engine_id(mod_engine_id) == resolve_mod_engine_id(target_engine_id)
}

struct DetectedModEngine {
    engine_id: String,
    has_warning: bool,
    warning_message: String,
}

fn detect_mod_engine_from_files(mod_dir: &Path, fallback: Option<&str>) -> DetectedModEngine {
    let mut has_exe = false;
    let mut has_polymod_meta = false;
    let mut has_pack_json = false;
    let mut has_codename_meta = false;

    let mut queue = VecDeque::new();
    queue.push_back((mod_dir.to_path_buf(), 0usize));

    while let Some((current, depth)) = queue.pop_front() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().to_string();
            let lower = name.to_ascii_lowercase();

            if ft.is_dir() {
                if depth < 2 && !lower.starts_with('.') {
                    queue.push_back((entry.path(), depth + 1));
                }
                continue;
            }
            if !ft.is_file() {
                continue;
            }

            if (lower.ends_with(".exe")
                || lower.ends_with(".appimage")
                || lower.ends_with(".x86_64"))
                && !lower.contains("unins")
                && !lower.contains("crash")
                && !lower.contains("unitycrash")
            {
                has_exe = true;
            } else if lower == "_polymod_meta.json" {
                has_polymod_meta = true;
            } else if lower == "pack.json" {
                has_pack_json = true;
            } else if lower == "meta.xml" || lower == "config.xml" {
                has_codename_meta = true;
            }
        }
    }

    let clean_fallback = fallback
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "unknown");

    if let Some(fb) = clean_fallback {
        let canonical = resolve_mod_engine_id(fb);
        if canonical == "vslice" {
            if has_polymod_meta {
                return DetectedModEngine {
                    engine_id: "vslice".into(),
                    has_warning: false,
                    warning_message: String::new(),
                };
            }
            if has_pack_json {
                return DetectedModEngine {
                    engine_id: "psych".into(),
                    has_warning: false,
                    warning_message: String::new(),
                };
            }
            if has_exe {
                return DetectedModEngine {
                    engine_id: "executable".into(),
                    has_warning: false,
                    warning_message: String::new(),
                };
            }
            return DetectedModEngine {
                engine_id: "vslice".into(),
                has_warning: true,
                warning_message: UNDETECTED_ENGINE_WARNING.into(),
            };
        }

        if has_exe {
            return DetectedModEngine {
                engine_id: "executable".into(),
                has_warning: false,
                warning_message: String::new(),
            };
        }
        if has_polymod_meta {
            return DetectedModEngine {
                engine_id: "vslice".into(),
                has_warning: false,
                warning_message: String::new(),
            };
        }
        if has_codename_meta && canonical == "codename" {
            return DetectedModEngine {
                engine_id: "codename".into(),
                has_warning: false,
                warning_message: String::new(),
            };
        }
        if has_pack_json && !matches!(canonical, "pslice" | "psychonline" | "fpsplus") {
            return DetectedModEngine {
                engine_id: "psych".into(),
                has_warning: false,
                warning_message: String::new(),
            };
        }

        return DetectedModEngine {
            engine_id: canonical.into(),
            has_warning: false,
            warning_message: String::new(),
        };
    }

    // No fallback (unlisted local folder scan)
    if has_exe {
        return DetectedModEngine {
            engine_id: "executable".into(),
            has_warning: false,
            warning_message: String::new(),
        };
    }
    if has_polymod_meta {
        return DetectedModEngine {
            engine_id: "vslice".into(),
            has_warning: false,
            warning_message: String::new(),
        };
    }
    if has_pack_json {
        return DetectedModEngine {
            engine_id: "psych".into(),
            has_warning: false,
            warning_message: String::new(),
        };
    }
    if has_codename_meta {
        return DetectedModEngine {
            engine_id: "codename".into(),
            has_warning: false,
            warning_message: String::new(),
        };
    }

    DetectedModEngine {
        engine_id: "psych".into(),
        has_warning: true,
        warning_message: UNDETECTED_ENGINE_WARNING.into(),
    }
}

fn read_embedded_mod_metadata(mod_dir: &Path) -> (Option<String>, Option<String>, Option<String>) {
    let poly_path = mod_dir.join("_polymod_meta.json");
    if poly_path.exists() {
        if let Ok(raw) = fs::read_to_string(&poly_path) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                let title = v.get("title").and_then(|x| x.as_str()).map(String::from);
                let desc = v
                    .get("description")
                    .and_then(|x| x.as_str())
                    .map(String::from);
                let author = v.get("author").and_then(|x| x.as_str()).map(String::from);
                return (title, desc, author);
            }
        }
    }

    let pack_path = mod_dir.join("pack.json");
    if pack_path.exists() {
        if let Ok(raw) = fs::read_to_string(&pack_path) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                let title = v.get("name").and_then(|x| x.as_str()).map(String::from);
                let desc = v
                    .get("description")
                    .and_then(|x| x.as_str())
                    .map(String::from);
                return (title, desc, None);
            }
        }
    }

    (None, None, None)
}

fn engine_display_name(engine_id: &str, fallback_name: &str) -> String {
    if engine_id == "executable" {
        return "Executable".to_string();
    }
    if let Some(def) = ENGINE_DEFINITIONS.iter().find(|d| d.id == engine_id) {
        return def.name.to_string();
    }
    if !fallback_name.trim().is_empty() {
        return fallback_name.to_string();
    }
    "Psych Engine".to_string()
}

fn matches_mod_id(record: &InstalledModRecord, target_id: &str) -> bool {
    if record.id == target_id || record.folder_name == target_id {
        return true;
    }
    let rec_base = record.id.split("--").next().unwrap_or(&record.id);
    let target_base = target_id.split("--").next().unwrap_or(target_id);
    !rec_base.is_empty() && rec_base == target_base
}

fn build_mod_slug(title: &str, mod_id: &str) -> String {
    let clean_title: String = title
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(32)
        .collect();
    let base_id = sanitize_folder_name(mod_id.split("--").next().unwrap_or(mod_id));
    if clean_title.is_empty() {
        format!("mod-{base_id}")
    } else if base_id.is_empty() || base_id == "item" {
        clean_title
    } else {
        format!("{clean_title}-{base_id}")
    }
}

fn unique_folder_slug(mods_dir: &Path, base_slug: &str) -> String {
    let candidate = mods_dir.join(base_slug);
    if !candidate.exists() {
        return base_slug.to_string();
    }
    for idx in 2..1000 {
        let next = format!("{base_slug}-{idx}");
        if !mods_dir.join(&next).exists() {
            return next;
        }
    }
    format!("{base_slug}-{}", Utc::now().timestamp_millis())
}

fn detect_archive_ext(filename: &str, url: &str) -> &'static str {
    let src = if !filename.trim().is_empty() {
        filename.to_ascii_lowercase()
    } else {
        url.split('?').next().unwrap_or(url).to_ascii_lowercase()
    };
    if src.ends_with(".tar.gz") {
        ".tar.gz"
    } else if src.ends_with(".tgz") {
        ".tgz"
    } else if src.ends_with(".7z") {
        ".7z"
    } else if src.ends_with(".rar") {
        ".rar"
    } else if src.ends_with(".tar") {
        ".tar"
    } else {
        ".zip"
    }
}
