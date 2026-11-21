use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Context;
use tokio::io::AsyncWriteExt;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn launch_executable(path: &Path, working_dir: Option<&Path>) -> anyhow::Result<u32> {
    if !path.exists() {
        anyhow::bail!("Ejecutable no encontrado: {}", path.display());
    }

    #[cfg(not(target_os = "windows"))]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(path) {
            let mode = meta.permissions().mode();
            if mode & 0o111 == 0 {
                let mut perms = meta.permissions();
                perms.set_mode(mode | 0o755);
                let _ = fs::set_permissions(path, perms);
            }
        }
    }

    let mut cmd = Command::new(path);
    if let Some(dir) = working_dir.or_else(|| path.parent()) {
        if dir.exists() {
            cmd.current_dir(dir);
        }
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let mut child = cmd
        .spawn()
        .with_context(|| format!("No se pudo iniciar {}", path.display()))?;
    let pid = child.id();

    // Reap child process in the background so Linux doesn't keep <defunct> zombies
    std::thread::spawn(move || {
        let _ = child.wait();
    });

    Ok(pid)
}

pub fn open_path_in_explorer(path: &Path) -> anyhow::Result<()> {
    let target = if path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    };

    #[cfg(target_os = "windows")]
    {
        let win_path = target.to_string_lossy().replace('/', "\\");
        Command::new("explorer")
            .arg(win_path)
            .spawn()
            .with_context(|| format!("No se pudo abrir {}", target.display()))?;
    }

    #[cfg(not(target_os = "windows"))]
    {
        #[cfg(target_os = "macos")]
        let opener = "open";
        #[cfg(not(target_os = "macos"))]
        let opener = "xdg-open";

        Command::new(opener)
            .arg(target)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("No se pudo abrir {}", target.display()))?;
    }

    Ok(())
}

pub fn open_url_in_browser(url: &str) -> anyhow::Result<()> {
    let trimmed = url.trim();
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        anyhow::bail!("URL inválida: {trimmed}");
    }
    if trimmed.contains(['"', '\r', '\n']) || trimmed.chars().any(|c| c.is_whitespace()) {
        anyhow::bail!("La URL contiene caracteres no permitidos");
    }

    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .arg("/C")
            .arg("start")
            .arg("")
            .raw_arg(format!("\"{trimmed}\""))
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("No se pudo abrir el enlace {trimmed}"))?;
    }

    #[cfg(not(target_os = "windows"))]
    {
        #[cfg(target_os = "macos")]
        let opener = "open";
        #[cfg(not(target_os = "macos"))]
        let opener = "xdg-open";

        Command::new(opener)
            .arg(trimmed)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("No se pudo abrir el enlace {trimmed}"))?;
    }

    Ok(())
}

pub fn create_directory_junction(link_path: &Path, target_path: &Path) -> anyhow::Result<()> {
    if !target_path.exists() {
        anyhow::bail!(
            "El destino del enlace no existe: {}",
            target_path.display()
        );
    }

    if let Some(parent) = link_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }

    if fs::symlink_metadata(link_path).is_ok() {
        remove_directory_junction(link_path)?;
    }

    #[cfg(target_os = "windows")]
    {
        let win_link = link_path.to_string_lossy().replace('/', "\\");
        let win_target = target_path.to_string_lossy().replace('/', "\\");
        let out = Command::new("cmd")
            .args(["/C", "mklink", "/J", &win_link, &win_target])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .with_context(|| format!("Error ejecutando mklink para {}", link_path.display()))?;

        if !out.status.success() && fs::symlink_metadata(link_path).is_err() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let stdout = String::from_utf8_lossy(&out.stdout);
            anyhow::bail!(
                "No se pudo crear junction {} -> {}: {}{}",
                link_path.display(),
                target_path.display(),
                stderr.trim(),
                stdout.trim()
            );
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        std::os::unix::fs::symlink(target_path, link_path).with_context(|| {
            format!(
                "No se pudo crear enlace simbólico {} -> {}",
                link_path.display(),
                target_path.display()
            )
        })?;
    }

    Ok(())
}

pub fn remove_directory_junction(link_path: &Path) -> anyhow::Result<()> {
    let Ok(meta) = fs::symlink_metadata(link_path) else {
        return Ok(());
    };

    #[cfg(target_os = "windows")]
    {
        if meta.file_type().is_file() {
            fs::remove_file(link_path)?;
        } else if fs::remove_dir(link_path).is_err() {
            let win_link = link_path.to_string_lossy().replace('/', "\\");
            let out = Command::new("cmd")
                .args(["/C", "rmdir", &win_link])
                .creation_flags(CREATE_NO_WINDOW)
                .output()?;
            if !out.status.success() && fs::symlink_metadata(link_path).is_ok() {
                anyhow::bail!(
                    "No se pudo eliminar junction {}: {}",
                    link_path.display(),
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        if meta.file_type().is_symlink() || meta.is_file() {
            fs::remove_file(link_path)?;
        } else if meta.is_dir() {
            fs::remove_dir(link_path)?;
        }
    }

    Ok(())
}

pub fn extract_zip_to_dir(
    zip_path: &Path,
    dest_dir: &Path,
    flatten_single_root: bool,
) -> anyhow::Result<()> {
    let file = fs::File::open(zip_path)
        .with_context(|| format!("No se pudo abrir el archivo ZIP {}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("Archivo ZIP inválido: {}", zip_path.display()))?;

    fs::create_dir_all(dest_dir)?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(rel_path) = entry.enclosed_name() else {
            continue;
        };
        if rel_path.as_os_str().is_empty() {
            continue;
        }
        if rel_path.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            continue;
        }
        if rel_path
            .components()
            .next()
            .is_some_and(|c| c.as_os_str() == "__MACOSX")
        {
            continue;
        }

        let out_path = dest_dir.join(&rel_path);
        if entry.is_dir() {
            fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                if !parent.exists() {
                    fs::create_dir_all(parent)?;
                }
            }
            let mut out_file = fs::File::create(&out_path)?;
            std::io::copy(&mut entry, &mut out_file)?;

            #[cfg(not(target_os = "windows"))]
            if let Some(mode) = entry.unix_mode() {
                use std::os::unix::fs::PermissionsExt;
                let perm = mode & 0o777;
                if perm != 0 {
                    let _ = fs::set_permissions(&out_path, fs::Permissions::from_mode(perm));
                }
            }
        }
    }

    if flatten_single_root {
        flatten_single_root_dir(dest_dir)?;
    }

    Ok(())
}

pub(crate) fn flatten_single_root_dir(dest_dir: &Path) -> anyhow::Result<()> {
    let macosx = dest_dir.join("__MACOSX");
    if macosx.exists() {
        let _ = fs::remove_dir_all(&macosx);
    }

    let entries: Vec<fs::DirEntry> = fs::read_dir(dest_dir)?
        .filter_map(Result::ok)
        .filter(|e| {
            let name = e.file_name();
            let lossy = name.to_string_lossy();
            lossy != "__MACOSX" && lossy != ".DS_Store" && !lossy.starts_with(".temp")
        })
        .collect();

    if entries.len() != 1 {
        return Ok(());
    }

    let Ok(ft) = entries[0].file_type() else {
        return Ok(());
    };
    if !ft.is_dir() {
        return Ok(());
    }

    let single_dir = entries[0].path();
    let temp_dir = unique_temp_dir(dest_dir, ".wb_flatten_tmp");
    fs::rename(&single_dir, &temp_dir)?;

    for child in fs::read_dir(&temp_dir)?.filter_map(Result::ok) {
        let target = dest_dir.join(child.file_name());
        if target.exists() {
            if target.is_dir() {
                let _ = fs::remove_dir_all(&target);
            } else {
                let _ = fs::remove_file(&target);
            }
        }
        fs::rename(child.path(), target)?;
    }

    let _ = fs::remove_dir_all(&temp_dir);
    Ok(())
}

fn unique_temp_dir(parent: &Path, prefix: &str) -> PathBuf {
    let base = parent.join(prefix);
    if !base.exists() {
        return base;
    }
    for idx in 1..1000 {
        let candidate = parent.join(format!("{prefix}_{idx}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{prefix}_{}", std::process::id()))
}

pub async fn download_file_with_progress<F>(
    client: &reqwest::Client,
    url: &str,
    dest_path: &Path,
    cancel: &AtomicBool,
    mut on_progress: F,
) -> anyhow::Result<()>
where
    F: FnMut(f32) + Send,
{
    if cancel.load(Ordering::Relaxed) {
        anyhow::bail!("Descarga cancelada");
    }

    let trimmed = url.trim();
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        anyhow::bail!("URL de descarga inválida: {trimmed}");
    }

    if let Some(parent) = dest_path.parent() {
        if !parent.exists() {
            tokio::fs::create_dir_all(parent).await?;
        }
    }

    let mut part_name = OsString::from(dest_path.as_os_str());
    part_name.push(".part");
    let part_path = PathBuf::from(part_name);

    let mut resp = client
        .get(trimmed)
        .send()
        .await
        .with_context(|| format!("Error conectando a {trimmed}"))?
        .error_for_status()
        .with_context(|| format!("Respuesta HTTP inválida desde {trimmed}"))?;

    let total = resp.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(&part_path).await?;
    let mut downloaded: u64 = 0;
    let mut last_emitted: f32 = -1.0;

    on_progress(0.0);

    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(file);
            let _ = tokio::fs::remove_file(&part_path).await;
            anyhow::bail!("Descarga cancelada");
        }

        match resp.chunk().await {
            Ok(Some(chunk)) => {
                if let Err(err) = file.write_all(&chunk).await {
                    drop(file);
                    let _ = tokio::fs::remove_file(&part_path).await;
                    return Err(err.into());
                }
                downloaded = downloaded.saturating_add(chunk.len() as u64);
                let pct = if total > 0 {
                    ((downloaded as f64 / total as f64) as f32).clamp(0.0, 1.0)
                } else {
                    // Smooth asymptotic curve when Content-Length is missing
                    let mb = downloaded as f64 / (16.0 * 1024.0 * 1024.0);
                    (mb / (1.0 + mb)).min(0.95) as f32
                };
                if (pct - last_emitted).abs() >= 0.01 {
                    last_emitted = pct;
                    on_progress(pct);
                }
            }
            Ok(None) => break,
            Err(err) => {
                drop(file);
                let _ = tokio::fs::remove_file(&part_path).await;
                return Err(err.into());
            }
        }
    }

    file.flush().await?;
    drop(file);

    if cancel.load(Ordering::Relaxed) {
        let _ = tokio::fs::remove_file(&part_path).await;
        anyhow::bail!("Descarga cancelada");
    }

    if dest_path.exists() {
        let _ = tokio::fs::remove_file(dest_path).await;
    }
    tokio::fs::rename(&part_path, dest_path).await?;
    on_progress(1.0);
    Ok(())
}
