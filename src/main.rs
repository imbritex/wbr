mod app;
mod core;
mod services;

slint::include_modules!();

use slint::ComponentHandle;
use tracing_subscriber::EnvFilter;

#[cfg(target_os = "windows")]
fn center_window_on_screen(window: &MainWindow) {
    unsafe extern "system" {
        fn GetSystemMetrics(n_index: i32) -> i32;
    }
    const SM_CXSCREEN: i32 = 0;
    const SM_CYSCREEN: i32 = 1;

    let (screen_w, screen_h) = unsafe {
        (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN))
    };

    if screen_w > 0 && screen_h > 0 {
        let slint_win = window.window();
        let scale = slint_win.scale_factor();
        let win_w = (1120.0 * scale) as i32;
        let win_h = (740.0 * scale) as i32;
        let x = ((screen_w - win_w) / 2).max(0);
        let y = ((screen_h - win_h) / 2).max(0);
        slint_win.set_position(slint::PhysicalPosition::new(x, y));
    }
}

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
    #[cfg(target_os = "windows")]
    center_window_on_screen(&window);

    app::bind(&window, runtime.handle().clone(), config);

    window.run()?;
    Ok(())
}
