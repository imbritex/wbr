mod app;
mod core;
mod services;

slint::include_modules!();

use tracing_subscriber::EnvFilter;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("wb_r=info")),
        )
        .init();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let config = core::config::AppConfig::load_or_default()?;
    tracing::info!(library_path = %config.library_path.display(), "Configuración cargada");

    let window = MainWindow::new()?;
    app::bind(&window, runtime.handle().clone(), config);

    window.run()?;
    Ok(())
}
