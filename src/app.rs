use std::sync::atomic::{AtomicU32, Ordering};
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
    EngineCardData, FeaturedModCardData, HomeModCardData, MainWindow, MainWindowAdapter,
    ModCardData, NewsItemData,
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
    pub home_mods_page: Arc<AtomicU32>,
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
        home_mods_page: Arc::new(AtomicU32::new(1)),
    };

    populate_initial_ui(window, &state);
    load_featured_mods(window, &state);
    load_home_mods_page(window, &state, 1, false);

    let weak = window.as_weak();
    let state_clone = state.clone();
    window
        .global::<MainWindowAdapter>()
        .on_refresh_requested(move || {
            let weak = weak.clone();
            let state = state_clone.clone();
            state.runtime.clone().spawn(async move {
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
            state.runtime.clone().spawn(async move {
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
        .on_home_mod_clicked(move |id| {
            if let Some(win) = weak.upgrade() {
                win.global::<MainWindowAdapter>()
                    .set_status_message(format!("Mod seleccionado: #{id}").into());
            }
        });

    let weak = window.as_weak();
    let state_clone = state.clone();
    window
        .global::<MainWindowAdapter>()
        .on_load_more_home_mods(move || {
            if let Some(win) = weak.upgrade() {
                let adapter = win.global::<MainWindowAdapter>();
                if adapter.get_home_mods_loading_more() || !adapter.get_home_mods_has_more() {
                    return;
                }
                adapter.set_home_mods_loading_more(true);
                let next_page = state_clone.home_mods_page.fetch_add(1, Ordering::SeqCst) + 1;
                load_home_mods_page(&win, &state_clone, next_page, true);
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

    state.runtime.clone().spawn(async move {
        match state.gamebanana.fetch_featured_mods().await {
            Ok(mods) => {
                let image_urls: Vec<(usize, String)> = mods
                    .iter()
                    .enumerate()
                    .map(|(i, m)| (i, m.image_url.clone()))
                    .collect();

                let weak_ui = weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = weak_ui.upgrade() {
                        let cards: Vec<FeaturedModCardData> = mods
                            .into_iter()
                            .map(|m| FeaturedModCardData {
                                id: m.id.to_string().into(),
                                title: m.title.into(),
                                author: m.author.into(),
                                engine_id: m.engine_id.into(),
                                engine: m.engine.into(),
                                badge: m.badge.into(),
                                cover: Image::default(),
                                has_cover: false,
                            })
                            .collect();
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

fn load_home_mods_page(window: &MainWindow, state: &AppState, page: u32, append: bool) {
    const PER_PAGE: u32 = 12;
    let weak = window.as_weak();
    let state = state.clone();

    state.runtime.clone().spawn(async move {
        match state.gamebanana.fetch_popular_mods_page(page, PER_PAGE).await {
            Ok(mods) => {
                let has_more = mods.len() >= PER_PAGE as usize;
                let image_urls: Vec<String> = mods.iter().map(|m| m.image_url.clone()).collect();

                let weak_ui = weak.clone();
                let (tx, rx) = tokio::sync::oneshot::channel::<usize>();
                let _ = slint::invoke_from_event_loop(move || {
                    let mut start_idx = 0usize;
                    if let Some(win) = weak_ui.upgrade() {
                        let adapter = win.global::<MainWindowAdapter>();
                        if append {
                            let existing = adapter.get_home_mods();
                            start_idx = existing.row_count();
                            let mut combined: Vec<HomeModCardData> = (0..start_idx)
                                .filter_map(|i| existing.row_data(i))
                                .collect();
                            combined.extend(mods.into_iter().map(|m| HomeModCardData {
                                id: m.id.to_string().into(),
                                title: m.title.into(),
                                author: m.author.into(),
                                engine_id: m.engine_id.into(),
                                engine_name: m.engine_name.into(),
                                likes: m.likes,
                                cover: Image::default(),
                                has_cover: false,
                            }));
                            adapter.set_home_mods(ModelRc::new(VecModel::from(combined)));
                        } else {
                            let cards: Vec<HomeModCardData> = mods
                                .into_iter()
                                .map(|m| HomeModCardData {
                                    id: m.id.to_string().into(),
                                    title: m.title.into(),
                                    author: m.author.into(),
                                    engine_id: m.engine_id.into(),
                                    engine_name: m.engine_name.into(),
                                    likes: m.likes,
                                    cover: Image::default(),
                                    has_cover: false,
                                })
                                .collect();
                            adapter.set_home_mods(ModelRc::new(VecModel::from(cards)));
                        }
                        adapter.set_home_mods_has_more(has_more);
                        adapter.set_home_mods_loading(false);
                        adapter.set_home_mods_loading_more(false);
                    }
                    let _ = tx.send(start_idx);
                });

                if let Ok(base_idx) = rx.await {
                    for (offset, url) in image_urls.into_iter().enumerate() {
                        let target_idx = base_idx + offset;
                        let gb = state.gamebanana.clone();
                        let weak_img = weak.clone();
                        state.runtime.spawn(async move {
                            if let Some((w, h, rgba)) = gb.fetch_cover_rgba(&url).await {
                                let _ = slint::invoke_from_event_loop(move || {
                                    if let Some(win) = weak_img.upgrade() {
                                        let model = win.global::<MainWindowAdapter>().get_home_mods();
                                        if let Some(mut item) = model.row_data(target_idx) {
                                            let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
                                                &rgba, w, h,
                                            );
                                            item.cover = Image::from_rgba8(buf);
                                            item.has_cover = true;
                                            model.set_row_data(target_idx, item);
                                        }
                                    }
                                });
                            }
                        });
                    }
                }
            }
            Err(err) => {
                tracing::warn!(%err, page, "Error cargando mods populares de GameBanana");
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = weak.upgrade() {
                        let adapter = win.global::<MainWindowAdapter>();
                        adapter.set_home_mods_loading(false);
                        adapter.set_home_mods_loading_more(false);
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
