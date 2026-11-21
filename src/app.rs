use std::sync::Arc;

use slint::{ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};
use tokio::runtime::Handle;

use crate::core::config::AppConfig;
use crate::core::engines::EngineRegistry;
use crate::core::mods::ModManager;
use crate::services::gamebanana::GameBananaClient;
use crate::services::github::GitHubClient;
use crate::services::news::NewsService;
use crate::{
    EngineCardData, FeaturedModCardData, MainWindow, MainWindowAdapter, ModCardData, NewsItemData,
};

#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub runtime: Handle,
    pub engines: Arc<EngineRegistry>,
    pub mods: Arc<ModManager>,
    pub gamebanana: Arc<GameBananaClient>,
    pub github: Arc<GitHubClient>,
    pub news: Arc<NewsService>,
}

pub fn bind(window: &MainWindow, runtime: Handle, config: AppConfig) {
    let state = AppState {
        config,
        runtime,
        engines: Arc::new(EngineRegistry::with_defaults()),
        mods: Arc::new(ModManager::new()),
        gamebanana: Arc::new(GameBananaClient::new()),
        github: Arc::new(GitHubClient::new()),
        news: Arc::new(NewsService::new()),
    };

    populate_initial_ui(window, &state);
    load_featured_mods(window, &state);

    let weak = window.as_weak();
    let state_clone = state.clone();
    window
        .global::<MainWindowAdapter>()
        .on_refresh_requested(move || {
            let weak = weak.clone();
            let state = state_clone.clone();
            state.runtime.spawn(async move {
                let running = state.engines.detect_running_engines();
                tracing::info!(running_count = running.len(), "Escaneo de engines completado");
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = weak.upgrade() {
                        win.global::<MainWindowAdapter>()
                            .set_status_message(format!("Engines activos: {}", running.len()).into());
                    }
                });
            });
        });

    let weak = window.as_weak();
    let state_clone = state.clone();
    window
        .global::<MainWindowAdapter>()
        .on_search_mods(move |query| {
            let query = query.to_string();
            let weak = weak.clone();
            let state = state_clone.clone();
            state.runtime.spawn(async move {
                tracing::info!(%query, "Buscando mods en GameBanana");
                let _results = state.gamebanana.search_mods(&query).await;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = weak.upgrade() {
                        win.global::<MainWindowAdapter>().set_status_message(
                            format!("Búsqueda completada: \"{query}\"").into(),
                        );
                    }
                });
            });
        });

    let weak = window.as_weak();
    window
        .global::<MainWindowAdapter>()
        .on_featured_mod_clicked(move |id| {
            if let Some(win) = weak.upgrade() {
                win.global::<MainWindowAdapter>()
                    .set_status_message(format!("Mod destacado seleccionado: #{id}").into());
            }
        });

    let weak = window.as_weak();
    window
        .global::<MainWindowAdapter>()
        .on_open_info(move || {
            if let Some(win) = weak.upgrade() {
                let adapter = win.global::<MainWindowAdapter>();
                let next = !adapter.get_info_open();
                adapter.set_info_open(next);
                if next {
                    adapter.set_settings_open(false);
                    adapter.set_status_message("Información de WeekBox".into());
                }
            }
        });

    let weak = window.as_weak();
    window
        .global::<MainWindowAdapter>()
        .on_open_settings(move || {
            if let Some(win) = weak.upgrade() {
                let adapter = win.global::<MainWindowAdapter>();
                let next = !adapter.get_settings_open();
                adapter.set_settings_open(next);
                if next {
                    adapter.set_info_open(false);
                    adapter.set_status_message("Configuración".into());
                }
            }
        });
}

fn load_featured_mods(window: &MainWindow, state: &AppState) {
    let weak = window.as_weak();
    let state = state.clone();

    state.runtime.spawn(async move {
        match state.gamebanana.fetch_featured_mods().await {
            Ok(mods) => {
                let image_urls: Vec<(usize, String)> = mods
                    .iter()
                    .enumerate()
                    .map(|(i, m)| (i, m.image_url.clone()))
                    .collect();

                let cards: Vec<FeaturedModCardData> = mods
                    .into_iter()
                    .map(|m| FeaturedModCardData {
                        id: m.id.to_string().into(),
                        title: m.title.into(),
                        author: m.author.into(),
                        engine: m.engine.into(),
                        badge: m.badge.into(),
                        cover: Image::default(),
                        has_cover: false,
                    })
                    .collect();

                let weak_ui = weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = weak_ui.upgrade() {
                        let adapter = win.global::<MainWindowAdapter>();
                        adapter.set_featured_mods(ModelRc::new(VecModel::from(cards)));
                        adapter.set_featured_loading(false);
                    }
                });

                for (idx, url) in image_urls {
                    let gb = state.gamebanana.clone();
                    let weak_img = weak.clone();
                    state.runtime.spawn(async move {
                        if let Some((w, h, rgba)) = gb.fetch_cover_rgba(&url).await {
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(win) = weak_img.upgrade() {
                                    let model = win.global::<MainWindowAdapter>().get_featured_mods();
                                    if let Some(mut item) = model.row_data(idx) {
                                        let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
                                            &rgba, w, h,
                                        );
                                        item.cover = Image::from_rgba8(buf);
                                        item.has_cover = true;
                                        model.set_row_data(idx, item);
                                    }
                                }
                            });
                        }
                    });
                }
            }
            Err(err) => {
                tracing::warn!(%err, "No se pudieron cargar los mods destacados de GameBanana");
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = weak.upgrade() {
                        win.global::<MainWindowAdapter>().set_featured_loading(false);
                    }
                });
            }
        }
    });
}

fn populate_initial_ui(window: &MainWindow, state: &AppState) {
    let adapter = window.global::<MainWindowAdapter>();
    adapter.set_library_path(state.config.library_path.to_string_lossy().as_ref().into());
    adapter.set_status_message("Listo".into());

    let engine_cards: Vec<EngineCardData> = state
        .engines
        .list()
        .iter()
        .map(|e| EngineCardData {
            id: e.id.clone().into(),
            name: e.name.clone().into(),
            version: e.version.clone().into(),
            installed: e.installed,
            playtime_label: format!("{} min", e.playtime_minutes).into(),
        })
        .collect();
    adapter.set_engines(ModelRc::new(VecModel::from(engine_cards)));

    let mod_cards: Vec<ModCardData> = state
        .mods
        .sample_entries()
        .into_iter()
        .map(|m| ModCardData {
            id: m.id.into(),
            title: m.title.into(),
            author: m.author.into(),
            engine: m.engine.into(),
        })
        .collect();
    adapter.set_mods(ModelRc::new(VecModel::from(mod_cards)));

    let news_items: Vec<NewsItemData> = state
        .news
        .placeholder_items()
        .into_iter()
        .map(|n| NewsItemData {
            title: n.title.into(),
            source: n.source.into(),
            summary: n.summary.into(),
        })
        .collect();
    adapter.set_news(ModelRc::new(VecModel::from(news_items)));
}
