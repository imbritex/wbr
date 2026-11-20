use std::path::Path;
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("No se pudo iniciar el proceso: {0}")]
    ProcessSpawn(#[from] std::io::Error),
}

pub fn launch_executable(path: &Path, working_dir: Option<&Path>) -> Result<(), HostError> {
    let mut cmd = Command::new(path);
    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }
    cmd.spawn()?;
    Ok(())
}
