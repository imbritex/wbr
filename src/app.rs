use std::sync::Arc;

use slint::{ComponentHandle, ModelRc, VecModel};
use tokio::runtime::Handle;

use crate::core::config::AppConfig;
use crate::core::engines::EngineRegistry;
use crate::core::mods::ModManager;
use crate::services::gamebanana::GameBananaClient;
use crate::services::github::GitHubClient;
use crate::services::news::NewsService;
use crate::{EngineCardData, MainWindow, MainWindowAdapter, ModCardData, NewsItemData};

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
