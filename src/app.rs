use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use slint::{Color, ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel, Weak};
use tokio::runtime::Handle;
use tokio::sync::RwLock;

use crate::core::config::AppConfig;
use crate::core::engines::{
    EngineAssetCandidate, EngineManager, EngineUpdateState, detect_platform,
    format_playtime_minutes, get_compatible_asset,
};
use crate::core::host;
use crate::core::mods::{InstalledModRecord, ModInstallRequest, ModManager};
use crate::core::storage::{
    LibraryPaths, ensure_weekbox_library, format_compact_number, format_file_size,
};
use crate::services::gamebanana::{GameBananaClient, ModDetails};
use crate::services::github::{GitHubClient, GitHubRelease};
use crate::services::news::{DEFAULT_NEWS_SOURCES, NewsPost, NewsService};
use crate::{
    CalloutData, ConfirmDialogData, EngineOptionData, EngineReleaseData, FeaturedModCardData,
    HomeModCardData, InstalledModCardData, MainWindow, MainWindowAdapter, ModDetailsData,
    ModFileData, ModProfileData, NewsPostData, NewsSourceData,
};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<RwLock<AppConfig>>,
    pub paths: Arc<LibraryPaths>,
    pub runtime: Handle,
    pub http_client: reqwest::Client,
    pub engines: Arc<EngineManager>,
    pub mods: Arc<ModManager>,
    pub gamebanana: Arc<GameBananaClient>,
    pub github: Arc<GitHubClient>,
    pub news: Arc<NewsService>,
    pub home_page: Arc<AtomicU32>,
    pub home_request_gen: Arc<AtomicU64>,
    pub callout_seq: Arc<AtomicU64>,
    pub engine_cancel: Arc<Mutex<Arc<AtomicBool>>>,
    pub mod_cancel: Arc<Mutex<Arc<AtomicBool>>>,
    pub cached_releases: Arc<RwLock<Vec<(GitHubRelease, Option<EngineAssetCandidate>)>>>,
    pub news_cursors: Arc<RwLock<HashMap<String, String>>>,
    pub news_posts_cache: Arc<RwLock<Vec<NewsPost>>>,
    pub active_mod_details: Arc<RwLock<Option<ModDetails>>>,
    pub active_running_session: Arc<Mutex<Option<(String, Instant)>>>,
}

pub fn bind(window: &MainWindow, runtime: Handle, config: AppConfig) {
    let paths = ensure_weekbox_library(&config.library_path)
        .unwrap_or_else(|_| LibraryPaths::from_root(&config.library_path));

    let http_client = reqwest::Client::builder()
        .user_agent("wb-r/3.0.0")
        .connect_timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let state = AppState {
        config: Arc::new(RwLock::new(config.clone())),
        paths: Arc::new(paths),
        runtime,
        http_client,
        engines: Arc::new(EngineManager::new()),
        mods: Arc::new(ModManager::new()),
        gamebanana: Arc::new(GameBananaClient::new()),
        github: Arc::new(GitHubClient::new()),
        news: Arc::new(NewsService::new()),
        home_page: Arc::new(AtomicU32::new(1)),
        home_request_gen: Arc::new(AtomicU64::new(0)),
        callout_seq: Arc::new(AtomicU64::new(0)),
        engine_cancel: Arc::new(Mutex::new(Arc::new(AtomicBool::new(false)))),
        mod_cancel: Arc::new(Mutex::new(Arc::new(AtomicBool::new(false)))),
        cached_releases: Arc::new(RwLock::new(Vec::new())),
        news_cursors: Arc::new(RwLock::new(HashMap::new())),
        news_posts_cache: Arc::new(RwLock::new(Vec::new())),
        active_mod_details: Arc::new(RwLock::new(None)),
        active_running_session: Arc::new(Mutex::new(None)),
    };

    populate_initial_ui(window, &state, &config);
    load_featured_mods(window.as_weak(), state.clone());
    load_home_mods_page(window.as_weak(), state.clone(), 1, false);
    refresh_library_state(window.as_weak(), state.clone());
    refresh_instances_overview(window.as_weak(), state.clone());
    load_engine_releases(window.as_weak(), state.clone(), false);
    load_news_feed(window.as_weak(), state.clone(), false);
    start_process_monitor(window.as_weak(), state.clone());

    bind_theme_and_modals(window, &state);
    bind_home_callbacks(window, &state);
    bind_mod_details_callbacks(window, &state);
    bind_library_callbacks(window, &state);
    bind_instances_callbacks(window, &state);
    bind_news_callbacks(window, &state);
}

fn populate_initial_ui(window: &MainWindow, state: &AppState, config: &AppConfig) {
    let adapter = window.global::<MainWindowAdapter>();
    adapter.set_dark_mode(config.dark_mode);
    adapter.set_library_path(state.paths.root.to_string_lossy().as_ref().into());
    adapter.set_platform_label(detect_platform().display_label().into());
    adapter.set_status_message("Ready".into());

    let sources: Vec<NewsSourceData> = DEFAULT_NEWS_SOURCES
        .iter()
        .map(|s| NewsSourceData {
            username: s.username.into(),
            name: s.name.into(),
            available: true,
        })
        .collect();
    adapter.set_news_sources(ModelRc::new(VecModel::from(sources)));
}

fn show_callout(weak: Weak<MainWindow>, state: &AppState, title: &str, message: &str, kind: &str) {
    let seq = state.callout_seq.fetch_add(1, Ordering::SeqCst) + 1;
    let title_s = title.to_string();
    let msg_s = message.to_string();
    let kind_s = kind.to_string();
    let weak_show = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(win) = weak_show.upgrade() {
            let adapter = win.global::<MainWindowAdapter>();
            adapter.set_callout(CalloutData {
                visible: true,
                title: title_s.into(),
                message: msg_s.into(),
                kind: kind_s.into(),
            });
        }
    });

    let seq_ref = state.callout_seq.clone();
    state.runtime.clone().spawn(async move {
        tokio::time::sleep(Duration::from_secs(4)).await;
        if seq_ref.load(Ordering::SeqCst) == seq {
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(win) = weak.upgrade() {
                    let adapter = win.global::<MainWindowAdapter>();
                    let mut c = adapter.get_callout();
                    c.visible = false;
                    adapter.set_callout(c);
                }
            });
        }
    });
}

fn rgba_to_slint_image(w: u32, h: u32, rgba: &[u8]) -> Image {
    let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(rgba, w, h);
    Image::from_rgba8(buf)
}

fn decode_local_image_rgba(path: PathBuf, max_w: u32, max_h: u32) -> Option<(u32, u32, Vec<u8>)> {
    let bytes = std::fs::read(path).ok()?;
    let mut img = image::load_from_memory(&bytes).ok()?;
    if max_w > 0 && max_h > 0 && (img.width() > max_w || img.height() > max_h) {
        img = img.resize(max_w, max_h, image::imageops::FilterType::Triangle);
    }
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    Some((w, h, rgba.into_raw()))
}

fn format_iso_date(raw: &str) -> String {
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return dt.format("%Y-%m-%d").to_string();
    }
    raw.chars().take(10).collect()
}

fn format_unix_date(ts: i64) -> String {
    DateTime::<Utc>::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

fn format_duration_hms(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

fn bind_theme_and_modals(window: &MainWindow, state: &AppState) {
    let adapter = window.global::<MainWindowAdapter>();

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_set_dark_mode(move |dark| {
        if let Some(win) = weak.upgrade() {
            win.global::<MainWindowAdapter>().set_dark_mode(dark);
        }
        let st = state_c.clone();
        state_c.runtime.clone().spawn(async move {
            let mut cfg = st.config.write().await;
            cfg.dark_mode = dark;
            let _ = cfg.save();
        });
    });

    let weak = window.as_weak();
    adapter.on_open_info(move || {
        if let Some(win) = weak.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            let next = !ad.get_info_open();
            ad.set_info_open(next);
            if next {
                ad.set_settings_open(false);
            }
        }
    });

    let weak = window.as_weak();
    adapter.on_close_info(move || {
        if let Some(win) = weak.upgrade() {
            win.global::<MainWindowAdapter>().set_info_open(false);
        }
    });

    let weak = window.as_weak();
    adapter.on_open_settings(move || {
        if let Some(win) = weak.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            let next = !ad.get_settings_open();
            ad.set_settings_open(next);
            if next {
                ad.set_info_open(false);
            }
        }
    });

    let weak = window.as_weak();
    adapter.on_close_settings(move || {
        if let Some(win) = weak.upgrade() {
            win.global::<MainWindowAdapter>().set_settings_open(false);
        }
    });

    let weak = window.as_weak();
    adapter.on_open_about_from_settings(move || {
        if let Some(win) = weak.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            ad.set_settings_open(false);
            ad.set_info_open(true);
        }
    });

    let weak = window.as_weak();
    adapter.on_dismiss_callout(move || {
        if let Some(win) = weak.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            let mut c = ad.get_callout();
            c.visible = false;
            ad.set_callout(c);
        }
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_open_library_folder(move || {
        if let Err(err) = host::open_path_in_explorer(&state_c.paths.root) {
            show_callout(weak.clone(), &state_c, "Explorer Error", &err.to_string(), "error");
        }
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_open_external_url(move |url| {
        let url_s = url.to_string();
        if let Err(err) = host::open_url_in_browser(&url_s) {
            show_callout(weak.clone(), &state_c, "Browser Error", &err.to_string(), "error");
        }
    });

    let weak = window.as_weak();
    adapter.on_confirm_dialog_cancel(move || {
        if let Some(win) = weak.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            let mut d = ad.get_confirm_dialog();
            d.open = false;
            ad.set_confirm_dialog(d);
        }
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_confirm_dialog_accept(move || {
        let Some(win) = weak.upgrade() else { return };
        let ad = win.global::<MainWindowAdapter>();
        let mut d = ad.get_confirm_dialog();
        let kind = d.action_kind.to_string();
        let target_id = d.action_target_id.to_string();
        d.open = false;
        ad.set_confirm_dialog(d);

        if kind == "uninstall_mod" && !target_id.is_empty() {
            perform_uninstall_mod(weak.clone(), state_c.clone(), target_id);
        } else if kind == "uninstall_engine" && !target_id.is_empty() {
            if let Some((engine_id, version)) = target_id.split_once("::") {
                perform_uninstall_engine(
                    weak.clone(),
                    state_c.clone(),
                    engine_id.to_string(),
                    version.to_string(),
                );
            }
        }
    });
}

// ============================================================================
// HOME & FEATURED CAROUSEL
// ============================================================================

fn bind_home_callbacks(window: &MainWindow, state: &AppState) {
    let adapter = window.global::<MainWindowAdapter>();

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_set_home_filter(move |filter| {
        if let Some(win) = weak.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            ad.set_home_filter(filter);
            ad.set_home_mods_loading(true);
            ad.set_home_mods_error("".into());
        }
        state_c.home_page.store(1, Ordering::SeqCst);
        load_home_mods_page(weak.clone(), state_c.clone(), 1, false);
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_set_home_engine_filter(move |engine| {
        if let Some(win) = weak.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            ad.set_home_engine_filter(engine);
            ad.set_home_mods_loading(true);
            ad.set_home_mods_error("".into());
        }
        state_c.home_page.store(1, Ordering::SeqCst);
        load_home_mods_page(weak.clone(), state_c.clone(), 1, false);
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_submit_home_search(move |query| {
        let q = query.trim().to_string();
        if let Some(win) = weak.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            ad.set_home_search_query(q.into());
            ad.set_home_mods_loading(true);
            ad.set_home_mods_error("".into());
        }
        state_c.home_page.store(1, Ordering::SeqCst);
        load_home_mods_page(weak.clone(), state_c.clone(), 1, false);
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_clear_home_search(move || {
        if let Some(win) = weak.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            ad.set_home_search_query("".into());
            ad.set_home_mods_loading(true);
            ad.set_home_mods_error("".into());
        }
        state_c.home_page.store(1, Ordering::SeqCst);
        load_home_mods_page(weak.clone(), state_c.clone(), 1, false);
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_retry_home_mods(move || {
        if let Some(win) = weak.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            ad.set_home_mods_loading(true);
            ad.set_home_mods_error("".into());
        }
        state_c.home_page.store(1, Ordering::SeqCst);
        load_home_mods_page(weak.clone(), state_c.clone(), 1, false);
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_load_more_home_mods(move || {
        if let Some(win) = weak.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            if ad.get_home_mods_loading_more() || !ad.get_home_mods_has_more() {
                return;
            }
            ad.set_home_mods_loading_more(true);
        }
        let next_page = state_c.home_page.fetch_add(1, Ordering::SeqCst) + 1;
        load_home_mods_page(weak.clone(), state_c.clone(), next_page, true);
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_featured_mod_clicked(move |id| {
        open_mod_details_flow(weak.clone(), state_c.clone(), id.to_string());
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_home_mod_clicked(move |id| {
        open_mod_details_flow(weak.clone(), state_c.clone(), id.to_string());
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_open_mod_details(move |id| {
        open_mod_details_flow(weak.clone(), state_c.clone(), id.to_string());
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_toggle_mod_favorite(move |id| {
        let mod_id = id.to_string();
        let next_state = state_c
            .mods
            .toggle_favorite(&state_c.paths, &mod_id)
            .unwrap_or(false);
        if let Some(win) = weak.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            let home = ad.get_home_mods();
            for i in 0..home.row_count() {
                if let Some(mut item) = home.row_data(i) {
                    if item.id == mod_id.as_str() {
                        item.is_favorite = next_state;
                        home.set_row_data(i, item);
                    }
                }
            }
            let mut details = ad.get_mod_details();
            if details.id == mod_id.as_str() {
                details.is_favorite = next_state;
                ad.set_mod_details(details);
            }
        }
        refresh_library_state(weak.clone(), state_c.clone());
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_toggle_mod_bookmark(move |id| {
        let mod_id = id.to_string();
        let next_state = state_c
            .mods
            .toggle_bookmark(&state_c.paths, &mod_id)
            .unwrap_or(false);
        if let Some(win) = weak.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            let home = ad.get_home_mods();
            for i in 0..home.row_count() {
                if let Some(mut item) = home.row_data(i) {
                    if item.id == mod_id.as_str() {
                        item.is_bookmarked = next_state;
                        home.set_row_data(i, item);
                    }
                }
            }
            let mut details = ad.get_mod_details();
            if details.id == mod_id.as_str() {
                details.is_bookmarked = next_state;
                ad.set_mod_details(details);
            }
        }
        refresh_library_state(weak.clone(), state_c.clone());
    });
}

fn load_featured_mods(weak: Weak<MainWindow>, state: AppState) {
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
                        let ad = win.global::<MainWindowAdapter>();
                        ad.set_featured_mods(ModelRc::new(VecModel::from(cards)));
                        ad.set_featured_loading(false);
                    }
                });

                for (idx, url) in image_urls {
                    let gb = state.gamebanana.clone();
                    let weak_img = weak.clone();
                    state.runtime.spawn(async move {
                        if let Some((w, h, rgba)) = gb.fetch_cover_rgba(&url, 720, 405).await {
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(win) = weak_img.upgrade() {
                                    let model = win.global::<MainWindowAdapter>().get_featured_mods();
                                    if let Some(mut item) = model.row_data(idx) {
                                        item.cover = rgba_to_slint_image(w, h, &rgba);
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
                tracing::warn!(%err, "Featured mods load warning");
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = weak.upgrade() {
                        win.global::<MainWindowAdapter>().set_featured_loading(false);
                    }
                });
            }
        }
    });
}

fn load_home_mods_page(weak: Weak<MainWindow>, state: AppState, page: u32, append: bool) {
    const PER_PAGE: u32 = 16;
    let request_gen = state.home_request_gen.fetch_add(1, Ordering::SeqCst) + 1;

    let (tx_params, rx_params) = tokio::sync::oneshot::channel::<(String, String, String)>();
    let weak_read = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        let params = if let Some(win) = weak_read.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            (
                ad.get_home_filter().to_string(),
                ad.get_home_engine_filter().to_string(),
                ad.get_home_search_query().to_string(),
            )
        } else {
            ("ripe".to_string(), "all".to_string(), String::new())
        };
        let _ = tx_params.send(params);
    });

    state.runtime.clone().spawn(async move {
        let (filter, engine_filter, search_query) = rx_params
            .await
            .unwrap_or_else(|_| ("ripe".to_string(), "all".to_string(), String::new()));

        match state
            .gamebanana
            .fetch_mods(&filter, &engine_filter, &search_query, page, PER_PAGE)
            .await
        {
            Ok(mods) => {
                if state.home_request_gen.load(Ordering::SeqCst) != request_gen {
                    return;
                }

                let favs = state.mods.load_favorites(&state.paths).unwrap_or_default();
                let books = state.mods.load_bookmarks(&state.paths).unwrap_or_default();
                let has_more = mods.len() >= PER_PAGE as usize;
                let image_urls: Vec<String> = mods.iter().map(|m| m.image_url.clone()).collect();

                let weak_ui = weak.clone();
                let (tx_idx, rx_idx) = tokio::sync::oneshot::channel::<usize>();
                let _ = slint::invoke_from_event_loop(move || {
                    let mut start_idx = 0usize;
                    if let Some(win) = weak_ui.upgrade() {
                        let ad = win.global::<MainWindowAdapter>();
                        let map_card = |m: crate::services::gamebanana::HomeMod| {
                            let id_s = m.id.to_string();
                            let is_fav = favs.contains(&id_s);
                            let is_book = books.contains(&id_s);
                            HomeModCardData {
                                id: id_s.into(),
                                title: m.title.into(),
                                description: m.description.into(),
                                author: m.author.into(),
                                engine_id: m.engine_id.into(),
                                engine_name: m.engine_name.into(),
                                likes: m.likes,
                                views: m.views,
                                downloads: m.downloads,
                                likes_label: format_compact_number(m.likes as i64).into(),
                                views_label: format_compact_number(m.views as i64).into(),
                                downloads_label: format_compact_number(m.downloads as i64).into(),
                                is_favorite: is_fav,
                                is_bookmarked: is_book,
                                is_nsfw: m.is_nsfw,
                                has_warning: m.has_warning,
                                cover: Image::default(),
                                has_cover: false,
                            }
                        };

                        if append {
                            let existing = ad.get_home_mods();
                            start_idx = existing.row_count();
                            let mut combined: Vec<HomeModCardData> =
                                (0..start_idx).filter_map(|i| existing.row_data(i)).collect();
                            combined.extend(mods.into_iter().map(map_card));
                            ad.set_home_mods(ModelRc::new(VecModel::from(combined)));
                        } else {
                            let cards: Vec<HomeModCardData> =
                                mods.into_iter().map(map_card).collect();
                            ad.set_home_mods(ModelRc::new(VecModel::from(cards)));
                        }
                        ad.set_home_mods_has_more(has_more);
                        ad.set_home_mods_loading(false);
                        ad.set_home_mods_loading_more(false);
                        ad.set_home_mods_error("".into());
                    }
                    let _ = tx_idx.send(start_idx);
                });

                if let Ok(base_idx) = rx_idx.await {
                    let gen_ref = state.home_request_gen.clone();
                    for (offset, url) in image_urls.into_iter().enumerate() {
                        let target_idx = base_idx + offset;
                        let gb = state.gamebanana.clone();
                        let weak_img = weak.clone();
                        let gen_chk = gen_ref.clone();
                        state.runtime.spawn(async move {
                            if let Some((w, h, rgba)) = gb.fetch_cover_rgba(&url, 560, 315).await {
                                if gen_chk.load(Ordering::SeqCst) != request_gen {
                                    return;
                                }
                                let _ = slint::invoke_from_event_loop(move || {
                                    if let Some(win) = weak_img.upgrade() {
                                        let model =
                                            win.global::<MainWindowAdapter>().get_home_mods();
                                        if let Some(mut item) = model.row_data(target_idx) {
                                            item.cover = rgba_to_slint_image(w, h, &rgba);
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
                if state.home_request_gen.load(Ordering::SeqCst) != request_gen {
                    return;
                }
                let err_msg = err.to_string();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = weak.upgrade() {
                        let ad = win.global::<MainWindowAdapter>();
                        ad.set_home_mods_loading(false);
                        ad.set_home_mods_loading_more(false);
                        if !append {
                            ad.set_home_mods_error(err_msg.into());
                        }
                    }
                });
            }
        }
    });
}

// ============================================================================
// MOD DETAILS MODAL & INSTALLATION
// ============================================================================

fn bind_mod_details_callbacks(window: &MainWindow, state: &AppState) {
    let adapter = window.global::<MainWindowAdapter>();

    let weak = window.as_weak();
    adapter.on_close_mod_details(move || {
        if let Some(win) = weak.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            let mut d = ad.get_mod_details();
            d.open = false;
            ad.set_mod_details(d);
        }
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_mod_details_prev_media(move || {
        shift_mod_details_media(weak.clone(), state_c.clone(), -1);
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_mod_details_next_media(move || {
        shift_mod_details_media(weak.clone(), state_c.clone(), 1);
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_install_mod_file(move |mod_id, file_url, filename| {
        let mod_id_s = mod_id.to_string();
        let url_s = file_url.to_string();
        let file_s = filename.to_string();
        start_mod_install(weak.clone(), state_c.clone(), mod_id_s, url_s, file_s);
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_play_mod(move |mod_id| {
        let id_s = mod_id.to_string();
        let st = state_c.clone();
        let wk = weak.clone();
        state_c.runtime.clone().spawn(async move {
            let res = tokio::task::spawn_blocking(move || {
                st.mods.launch_mod(&st.paths, &st.engines, &id_s, None)
            })
            .await;

            match res {
                Ok(Ok(_pid)) => {
                    show_callout(wk, &state_c, "Launching Mod", "Game process started.", "success");
                }
                Ok(Err(err)) => {
                    show_callout(wk, &state_c, "Launch Error", &err.to_string(), "error");
                }
                Err(err) => {
                    show_callout(wk, &state_c, "Launch Error", &err.to_string(), "error");
                }
            }
        });
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_uninstall_mod(move |mod_id| {
        let id_s = mod_id.to_string();
        if let Some(win) = weak.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            ad.set_confirm_dialog(ConfirmDialogData {
                open: true,
                title: "Uninstall Mod".into(),
                message: "Are you sure you want to delete this mod from your WeekBoxLibrary?"
                    .into(),
                confirm_text: "Uninstall".into(),
                cancel_text: "Cancel".into(),
                action_kind: "uninstall_mod".into(),
                action_target_id: id_s.into(),
            });
        }
        let _ = &state_c;
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_open_mod_folder(move |mod_id| {
        let id_s = mod_id.to_string();
        if let Ok(mods) = state_c.mods.load_installed_mods(&state_c.paths) {
            if let Some(rec) = mods.into_iter().find(|m| m.id == id_s || m.folder_name == id_s) {
                let p = if !rec.install_path.is_empty() {
                    PathBuf::from(rec.install_path)
                } else {
                    state_c.paths.mods_dir.join(rec.folder_name)
                };
                if let Err(err) = host::open_path_in_explorer(&p) {
                    show_callout(weak.clone(), &state_c, "Folder Error", &err.to_string(), "error");
                }
                return;
            }
        }
        let _ = host::open_path_in_explorer(&state_c.paths.mods_dir);
    });
}

fn open_mod_details_flow(weak: Weak<MainWindow>, state: AppState, mod_id: String) {
    let favs = state.mods.load_favorites(&state.paths).unwrap_or_default();
    let books = state.mods.load_bookmarks(&state.paths).unwrap_or_default();
    let installed = state.mods.load_installed_mods(&state.paths).unwrap_or_default();
    let installed_rec = installed
        .iter()
        .find(|m| m.id == mod_id || m.folder_name == mod_id)
        .cloned();

    let is_fav = favs.contains(&mod_id);
    let is_book = books.contains(&mod_id);
    let is_inst = installed_rec.is_some();

    let weak_init = weak.clone();
    let mod_id_init = mod_id.clone();
    let installed_init = installed_rec.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(win) = weak_init.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            let title = installed_init
                .as_ref()
                .map(|r| r.title.clone())
                .unwrap_or_else(|| format!("Mod #{mod_id_init}"));
            let author = installed_init
                .as_ref()
                .map(|r| r.author.clone())
                .unwrap_or_default();
            let desc = installed_init
                .as_ref()
                .map(|r| r.description.clone())
                .unwrap_or_default();
            let eid = installed_init
                .as_ref()
                .map(|r| r.engine_id.clone())
                .unwrap_or_default();
            let ename = installed_init
                .as_ref()
                .map(|r| r.engine_name.clone())
                .unwrap_or_default();

            ad.set_mod_details(ModDetailsData {
                open: true,
                loading: mod_id_init.parse::<u64>().is_ok(),
                id: mod_id_init.into(),
                title: title.into(),
                author: author.into(),
                description: desc.into(),
                engine_id: eid.into(),
                engine_name: ename.into(),
                version: "".into(),
                likes_label: "0".into(),
                views_label: "0".into(),
                downloads_label: "0".into(),
                date_label: "".into(),
                page_url: installed_init.map(|r| r.page_url).unwrap_or_default().into(),
                is_installed: is_inst,
                is_favorite: is_fav,
                is_bookmarked: is_book,
                is_installing: false,
                install_progress: 0.0,
                install_status: "".into(),
                active_media_index: 0,
                media_count: 0,
                active_media: Image::default(),
                has_active_media: false,
                credits_summary: "".into(),
                files: ModelRc::default(),
            });
        }
    });

    if let Ok(numeric_id) = mod_id.parse::<u64>() {
        let st = state.clone();
        let wk = weak.clone();
        state.runtime.clone().spawn(async move {
            match st.gamebanana.fetch_mod_by_id(numeric_id).await {
                Ok(details) => {
                    {
                        let mut guard = st.active_mod_details.write().await;
                        *guard = Some(details.clone());
                    }

                    let first_media_url = details.preview_urls.first().cloned();
                    let details_for_ui = details.clone();
                    let wk_ui = wk.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(win) = wk_ui.upgrade() {
                            let ad = win.global::<MainWindowAdapter>();
                            let mut cur = ad.get_mod_details();
                            if cur.id != numeric_id.to_string().as_str() {
                                return;
                            }
                            cur.loading = false;
                            cur.title = details_for_ui.title.into();
                            cur.author = details_for_ui.author.into();
                            cur.description = details_for_ui.description.into();
                            cur.engine_id = details_for_ui.engine_id.into();
                            cur.engine_name = details_for_ui.engine_name.into();
                            cur.version = details_for_ui.version.into();
                            cur.likes_label =
                                format_compact_number(details_for_ui.likes as i64).into();
                            cur.views_label =
                                format_compact_number(details_for_ui.views as i64).into();
                            cur.downloads_label =
                                format_compact_number(details_for_ui.downloads as i64).into();
                            cur.date_label = format_unix_date(details_for_ui.submitted_at).into();
                            cur.page_url = details_for_ui.page_url.into();
                            cur.media_count = details_for_ui.preview_urls.len() as i32;
                            cur.active_media_index = 0;
                            cur.credits_summary = details_for_ui.credits_summary.into();

                            let files: Vec<ModFileData> = details_for_ui
                                .files
                                .into_iter()
                                .map(|f| ModFileData {
                                    id: f.id.to_string().into(),
                                    filename: f.filename.into(),
                                    description: f.description.into(),
                                    size_label: format_file_size(f.filesize).into(),
                                    downloads_label: format_compact_number(f.download_count as i64)
                                        .into(),
                                    download_url: f.download_url.into(),
                                    is_installing: false,
                                    progress: 0.0,
                                })
                                .collect();
                            cur.files = ModelRc::new(VecModel::from(files));
                            ad.set_mod_details(cur);
                        }
                    });

                    if let Some(url) = first_media_url {
                        if let Some((w, h, rgba)) = st.gamebanana.fetch_cover_rgba(&url, 860, 484).await {
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(win) = wk.upgrade() {
                                    let ad = win.global::<MainWindowAdapter>();
                                    let mut cur = ad.get_mod_details();
                                    if cur.id == numeric_id.to_string().as_str() {
                                        cur.active_media = rgba_to_slint_image(w, h, &rgba);
                                        cur.has_active_media = true;
                                        ad.set_mod_details(cur);
                                    }
                                }
                            });
                        }
                    }
                }
                Err(err) => {
                    tracing::warn!(%err, numeric_id, "Failed to load mod profile");
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(win) = wk.upgrade() {
                            let ad = win.global::<MainWindowAdapter>();
                            let mut cur = ad.get_mod_details();
                            cur.loading = false;
                            ad.set_mod_details(cur);
                        }
                    });
                }
            }
        });
    } else if let Some(rec) = installed_rec {
        if let Some(cover_path) = ModManager::resolve_cover_path(&state.paths, &rec) {
            state.runtime.clone().spawn(async move {
                let decoded = tokio::task::spawn_blocking(move || {
                    decode_local_image_rgba(cover_path, 860, 484)
                })
                .await
                .ok()
                .flatten();

                if let Some((w, h, rgba)) = decoded {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(win) = weak.upgrade() {
                            let ad = win.global::<MainWindowAdapter>();
                            let mut cur = ad.get_mod_details();
                            if cur.id == mod_id.as_str() {
                                cur.active_media = rgba_to_slint_image(w, h, &rgba);
                                cur.has_active_media = true;
                                cur.media_count = 1;
                                ad.set_mod_details(cur);
                            }
                        }
                    });
                }
            });
        }
    }
}

fn shift_mod_details_media(weak: Weak<MainWindow>, state: AppState, delta: i32) {
    state.runtime.clone().spawn(async move {
        let details = {
            let guard = state.active_mod_details.read().await;
            guard.clone()
        };
        let Some(details) = details else { return };
        if details.preview_urls.is_empty() {
            return;
        }

        let (tx, rx) = tokio::sync::oneshot::channel::<i32>();
        let wk_idx = weak.clone();
        let len = details.preview_urls.len() as i32;
        let _ = slint::invoke_from_event_loop(move || {
            let mut next_idx = 0;
            if let Some(win) = wk_idx.upgrade() {
                let ad = win.global::<MainWindowAdapter>();
                let mut cur = ad.get_mod_details();
                if len > 0 {
                    next_idx = (cur.active_media_index + delta).rem_euclid(len);
                    cur.active_media_index = next_idx;
                    ad.set_mod_details(cur);
                }
            }
            let _ = tx.send(next_idx);
        });

        if let Ok(idx) = rx.await {
            if let Some(url) = details.preview_urls.get(idx as usize) {
                if let Some((w, h, rgba)) = state.gamebanana.fetch_cover_rgba(url, 860, 484).await {
                    let target_id = details.id.to_string();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(win) = weak.upgrade() {
                            let ad = win.global::<MainWindowAdapter>();
                            let mut cur = ad.get_mod_details();
                            if cur.id == target_id.as_str() && cur.active_media_index == idx {
                                cur.active_media = rgba_to_slint_image(w, h, &rgba);
                                cur.has_active_media = true;
                                ad.set_mod_details(cur);
                            }
                        }
                    });
                }
            }
        }
    });
}

fn start_mod_install(
    weak: Weak<MainWindow>,
    state: AppState,
    mod_id: String,
    file_url: String,
    filename: String,
) {
    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut guard) = state.mod_cancel.lock() {
        guard.store(true, Ordering::SeqCst);
        *guard = cancel.clone();
    }

    let _ = slint::invoke_from_event_loop({
        let wk = weak.clone();
        move || {
            if let Some(win) = wk.upgrade() {
                let ad = win.global::<MainWindowAdapter>();
                let mut cur = ad.get_mod_details();
                cur.is_installing = true;
                cur.install_progress = 0.05;
                cur.install_status = "Starting download...".into();
                ad.set_mod_details(cur);
            }
        }
    });

    state.runtime.clone().spawn(async move {
        let cached = {
            let guard = state.active_mod_details.read().await;
            guard.clone()
        };

        let req = match cached.filter(|d| d.id.to_string() == mod_id) {
            Some(d) => ModInstallRequest {
                mod_id: mod_id.clone(),
                title: d.title,
                description: d.description,
                author: d.author,
                engine_id: d.engine_id,
                engine_name: d.engine_name,
                download_url: file_url,
                archive_filename: filename,
                cover_url: d.preview_urls.first().cloned(),
                page_url: d.page_url,
            },
            None => ModInstallRequest {
                mod_id: mod_id.clone(),
                title: format!("Mod #{mod_id}"),
                description: String::new(),
                author: String::new(),
                engine_id: "psych".to_string(),
                engine_name: "Psych Engine".to_string(),
                download_url: file_url,
                archive_filename: filename,
                cover_url: None,
                page_url: format!("https://gamebanana.com/mods/{mod_id}"),
            },
        };

        let wk_prog = weak.clone();
        let mod_id_prog = mod_id.clone();
        let res = state
            .mods
            .install_mod_from_url(
                &state.paths,
                &state.http_client,
                req,
                &cancel,
                move |pct, label| {
                    let wk_inner = wk_prog.clone();
                    let label_s = label.to_string();
                    let id_chk = mod_id_prog.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(win) = wk_inner.upgrade() {
                            let ad = win.global::<MainWindowAdapter>();
                            let mut cur = ad.get_mod_details();
                            if cur.id == id_chk.as_str() {
                                cur.is_installing = true;
                                cur.install_progress = pct;
                                cur.install_status = label_s.into();
                                ad.set_mod_details(cur);
                            }
                        }
                    });
                },
            )
            .await;

        match res {
            Ok(rec) => {
                let wk_done = weak.clone();
                let title_s = rec.title.clone();
                let id_done = mod_id.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = wk_done.upgrade() {
                        let ad = win.global::<MainWindowAdapter>();
                        let mut cur = ad.get_mod_details();
                        if cur.id == id_done.as_str() {
                            cur.is_installing = false;
                            cur.is_installed = true;
                            cur.install_progress = 1.0;
                            cur.install_status = "Installed".into();
                            ad.set_mod_details(cur);
                        }
                    }
                });
                show_callout(
                    weak.clone(),
                    &state,
                    "Mod Installed",
                    &format!("{title_s} added to your library."),
                    "success",
                );
                refresh_library_state(weak, state);
            }
            Err(err) => {
                let err_s = err.to_string();
                let wk_err = weak.clone();
                let id_err = mod_id.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = wk_err.upgrade() {
                        let ad = win.global::<MainWindowAdapter>();
                        let mut cur = ad.get_mod_details();
                        if cur.id == id_err.as_str() {
                            cur.is_installing = false;
                            cur.install_status = "".into();
                            ad.set_mod_details(cur);
                        }
                    }
                });
                show_callout(weak, &state, "Install Failed", &err_s, "error");
            }
        }
    });
}

fn perform_uninstall_mod(weak: Weak<MainWindow>, state: AppState, mod_id: String) {
    let st = state.clone();
    let wk = weak.clone();
    state.runtime.clone().spawn(async move {
        let id_c = mod_id.clone();
        let res = tokio::task::spawn_blocking(move || st.mods.uninstall_mod(&st.paths, &id_c)).await;
        match res {
            Ok(Ok(())) => {
                let wk_ui = wk.clone();
                let id_ui = mod_id.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = wk_ui.upgrade() {
                        let ad = win.global::<MainWindowAdapter>();
                        let mut cur = ad.get_mod_details();
                        if cur.id == id_ui.as_str() {
                            cur.is_installed = false;
                            ad.set_mod_details(cur);
                        }
                    }
                });
                show_callout(wk.clone(), &state, "Mod Uninstalled", "Mod removed from library.", "info");
                refresh_library_state(wk, state);
            }
            Ok(Err(err)) => {
                show_callout(wk, &state, "Uninstall Error", &err.to_string(), "error");
            }
            Err(err) => {
                show_callout(wk, &state, "Uninstall Error", &err.to_string(), "error");
            }
        }
    });
}

// ============================================================================
// LIBRARY & PROFILES
// ============================================================================

fn bind_library_callbacks(window: &MainWindow, state: &AppState) {
    let adapter = window.global::<MainWindowAdapter>();

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_set_library_search(move |q| {
        if let Some(win) = weak.upgrade() {
            win.global::<MainWindowAdapter>().set_library_search_query(q);
        }
        refresh_library_state(weak.clone(), state_c.clone());
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_set_library_engine_filter(move |engine| {
        if let Some(win) = weak.upgrade() {
            win.global::<MainWindowAdapter>().set_library_engine_filter(engine);
        }
        refresh_library_state(weak.clone(), state_c.clone());
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_set_library_sort_by(move |sort_by| {
        if let Some(win) = weak.upgrade() {
            win.global::<MainWindowAdapter>().set_library_sort_by(sort_by);
        }
        refresh_library_state(weak.clone(), state_c.clone());
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_toggle_library_favorites_filter(move || {
        if let Some(win) = weak.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            ad.set_library_only_favorites(!ad.get_library_only_favorites());
        }
        refresh_library_state(weak.clone(), state_c.clone());
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_toggle_library_bookmarks_filter(move || {
        if let Some(win) = weak.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            ad.set_library_only_bookmarks(!ad.get_library_only_bookmarks());
        }
        refresh_library_state(weak.clone(), state_c.clone());
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_clear_library_filters(move || {
        if let Some(win) = weak.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            ad.set_library_search_query("".into());
            ad.set_library_engine_filter("all".into());
            ad.set_library_sort_by("recent".into());
            ad.set_library_only_favorites(false);
            ad.set_library_only_bookmarks(false);
            ad.set_active_profile_id("".into());
        }
        refresh_library_state(weak.clone(), state_c.clone());
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_refresh_library(move || {
        refresh_library_state(weak.clone(), state_c.clone());
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_open_all_mods_folder(move || {
        if let Err(err) = host::open_path_in_explorer(&state_c.paths.mods_dir) {
            show_callout(weak.clone(), &state_c, "Folder Error", &err.to_string(), "error");
        }
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_toggle_mod_visibility(move |id| {
        let id_s = id.to_string();
        if let Err(err) = state_c.mods.toggle_visibility(&state_c.paths, &id_s) {
            show_callout(weak.clone(), &state_c, "Visibility Error", &err.to_string(), "error");
        } else {
            refresh_library_state(weak.clone(), state_c.clone());
        }
    });

    let weak = window.as_weak();
    adapter.on_request_uninstall_mod(move |id| {
        if let Some(win) = weak.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            ad.set_confirm_dialog(ConfirmDialogData {
                open: true,
                title: "Uninstall Mod".into(),
                message: "Are you sure you want to uninstall this mod and remove its files?".into(),
                confirm_text: "Uninstall".into(),
                cancel_text: "Cancel".into(),
                action_kind: "uninstall_mod".into(),
                action_target_id: id,
            });
        }
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_select_profile(move |id| {
        if let Some(win) = weak.upgrade() {
            win.global::<MainWindowAdapter>().set_active_profile_id(id);
        }
        refresh_library_state(weak.clone(), state_c.clone());
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_create_profile(move |name| {
        match state_c.mods.create_profile(&state_c.paths, name.as_str()) {
            Ok(profile) => {
                if let Some(win) = weak.upgrade() {
                    let ad = win.global::<MainWindowAdapter>();
                    ad.set_create_profile_open(false);
                    ad.set_active_profile_id(profile.id.into());
                }
                refresh_library_state(weak.clone(), state_c.clone());
            }
            Err(err) => {
                show_callout(weak.clone(), &state_c, "Profile Error", &err.to_string(), "error");
            }
        }
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_delete_profile(move |id| {
        let id_s = id.to_string();
        let _ = state_c.mods.delete_profile(&state_c.paths, &id_s);
        if let Some(win) = weak.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            if ad.get_active_profile_id() == id_s.as_str() {
                ad.set_active_profile_id("".into());
            }
        }
        refresh_library_state(weak.clone(), state_c.clone());
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_toggle_mod_in_active_profile(move |mod_id| {
        let Some(win) = weak.upgrade() else { return };
        let profile_id = win
            .global::<MainWindowAdapter>()
            .get_active_profile_id()
            .to_string();
        if profile_id.is_empty() {
            return;
        }
        let _ = state_c
            .mods
            .toggle_mod_in_profile(&state_c.paths, &profile_id, mod_id.as_str());
        refresh_library_state(weak.clone(), state_c.clone());
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_play_active_profile(move || {
        let Some(win) = weak.upgrade() else { return };
        let profile_id = win
            .global::<MainWindowAdapter>()
            .get_active_profile_id()
            .to_string();
        if profile_id.is_empty() {
            return;
        }
        let st = state_c.clone();
        let wk = weak.clone();
        state_c.runtime.clone().spawn(async move {
            let res = tokio::task::spawn_blocking(move || -> anyhow::Result<u32> {
                let profiles = st.mods.load_profiles(&st.paths)?;
                let profile = profiles
                    .into_iter()
                    .find(|p| p.id == profile_id)
                    .ok_or_else(|| anyhow::anyhow!("Profile not found"))?;
                if profile.mod_ids.is_empty() {
                    anyhow::bail!("This folder has no mods yet");
                }
                let installed = st.mods.load_installed_mods(&st.paths)?;
                let first_mod = installed
                    .iter()
                    .find(|m| profile.mod_ids.iter().any(|id| id == &m.id || id == &m.folder_name))
                    .ok_or_else(|| anyhow::anyhow!("No installed mods found in this folder"))?;
                if first_mod.engine_id == "executable" || first_mod.engine_id == "exe" {
                    st.mods.launch_mod(&st.paths, &st.engines, &first_mod.id, None)
                } else {
                    st.mods.launch_engine_with_mods(
                        &st.paths,
                        &st.engines,
                        &first_mod.engine_id,
                        None,
                        Some(&profile.mod_ids),
                    )
                }
            })
            .await;

            match res {
                Ok(Ok(_)) => {
                    show_callout(wk, &state_c, "Launching Profile", "Engine launched with profile mods.", "success");
                }
                Ok(Err(err)) => {
                    show_callout(wk, &state_c, "Profile Launch Error", &err.to_string(), "error");
                }
                Err(err) => {
                    show_callout(wk, &state_c, "Profile Launch Error", &err.to_string(), "error");
                }
            }
        });
    });
}

fn refresh_library_state(weak: Weak<MainWindow>, state: AppState) {
    let (tx_filters, rx_filters) =
        tokio::sync::oneshot::channel::<(String, String, String, bool, bool, String)>();
    let wk_read = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        let filters = if let Some(win) = wk_read.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            (
                ad.get_library_search_query().to_string(),
                ad.get_library_engine_filter().to_string(),
                ad.get_library_sort_by().to_string(),
                ad.get_library_only_favorites(),
                ad.get_library_only_bookmarks(),
                ad.get_active_profile_id().to_string(),
            )
        } else {
            (
                String::new(),
                "all".to_string(),
                "recent".to_string(),
                false,
                false,
                String::new(),
            )
        };
        let _ = tx_filters.send(filters);
    });

    state.runtime.clone().spawn(async move {
        let (search_q, engine_f, sort_by, only_fav, only_book, active_profile_id) = rx_filters
            .await
            .unwrap_or_else(|_| (String::new(), "all".into(), "recent".into(), false, false, String::new()));

        let st_scan = state.clone();
        let data_res = tokio::task::spawn_blocking(move || {
            let all_mods = st_scan.mods.load_installed_mods(&st_scan.paths).unwrap_or_default();
            let favs = st_scan.mods.load_favorites(&st_scan.paths).unwrap_or_default();
            let books = st_scan.mods.load_bookmarks(&st_scan.paths).unwrap_or_default();
            let profiles = st_scan.mods.load_profiles(&st_scan.paths).unwrap_or_default();
            (all_mods, favs, books, profiles)
        })
        .await;

        let Ok((all_mods, favs, books, profiles)) = data_res else {
            return;
        };

        let total_count = all_mods.len() as i32;
        let total_bytes: u64 = all_mods.iter().map(|m| m.size_bytes).sum();
        let storage_label = format_file_size(total_bytes);

        let active_profile_ids: Option<Vec<String>> = if active_profile_id.is_empty() {
            None
        } else {
            profiles
                .iter()
                .find(|p| p.id == active_profile_id)
                .map(|p| p.mod_ids.clone())
        };

        let search_lower = search_q.trim().to_ascii_lowercase();
        let mut filtered: Vec<InstalledModRecord> = all_mods
            .into_iter()
            .filter(|m| {
                if let Some(ref pids) = active_profile_ids {
                    if !pids.iter().any(|id| id == &m.id || id == &m.folder_name) {
                        return false;
                    }
                }
                if engine_f != "all" && !m.engine_id.eq_ignore_ascii_case(&engine_f) {
                    return false;
                }
                if only_fav && !favs.contains(&m.id) && !favs.contains(&m.folder_name) {
                    return false;
                }
                if only_book && !books.contains(&m.id) && !books.contains(&m.folder_name) {
                    return false;
                }
                if !search_lower.is_empty() {
                    let hay = format!("{} {} {}", m.title, m.author, m.engine_name).to_ascii_lowercase();
                    if !hay.contains(&search_lower) {
                        return false;
                    }
                }
                true
            })
            .collect();

        match sort_by.as_str() {
            "name" => filtered.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase())),
            "size" => filtered.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes)),
            _ => filtered.sort_by(|a, b| b.installed_at.cmp(&a.installed_at)),
        }

        let cover_paths: Vec<(usize, Option<PathBuf>)> = filtered
            .iter()
            .enumerate()
            .map(|(i, rec)| (i, ModManager::resolve_cover_path(&state.paths, rec)))
            .collect();

        let wk_ui = weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(win) = wk_ui.upgrade() {
                let ad = win.global::<MainWindowAdapter>();
                let cards: Vec<InstalledModCardData> = filtered
                    .into_iter()
                    .map(|m| {
                        let is_fav = favs.contains(&m.id) || favs.contains(&m.folder_name);
                        let is_book = books.contains(&m.id) || books.contains(&m.folder_name);
                        InstalledModCardData {
                            id: m.id.into(),
                            title: m.title.into(),
                            description: m.description.into(),
                            author: m.author.into(),
                            engine_id: m.engine_id.into(),
                            engine_name: m.engine_name.into(),
                            folder_name: m.folder_name.into(),
                            install_path: m.install_path.into(),
                            size_label: format_file_size(m.size_bytes).into(),
                            is_favorite: is_fav,
                            is_bookmarked: is_book,
                            is_visible: m.visible,
                            has_warning: m.has_warning,
                            warning_message: m.warning_message.into(),
                            cover: Image::default(),
                            has_cover: false,
                        }
                    })
                    .collect();

                let prof_data: Vec<ModProfileData> = profiles
                    .into_iter()
                    .map(|p| ModProfileData {
                        id: p.id.into(),
                        name: p.name.into(),
                        mod_count: p.mod_ids.len() as i32,
                        mod_ids_csv: p.mod_ids.join(",").into(),
                    })
                    .collect();

                ad.set_library_mods(ModelRc::new(VecModel::from(cards)));
                ad.set_profiles(ModelRc::new(VecModel::from(prof_data)));
                ad.set_library_total_count(total_count);
                ad.set_library_storage_label(storage_label.into());
                ad.set_library_loading(false);
            }
        });

        for (idx, maybe_path) in cover_paths {
            let Some(path) = maybe_path else { continue };
            let wk_img = weak.clone();
            state.runtime.spawn(async move {
                let decoded = tokio::task::spawn_blocking(move || decode_local_image_rgba(path, 480, 270))
                    .await
                    .ok()
                    .flatten();
                if let Some((w, h, rgba)) = decoded {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(win) = wk_img.upgrade() {
                            let model = win.global::<MainWindowAdapter>().get_library_mods();
                            if let Some(mut item) = model.row_data(idx) {
                                item.cover = rgba_to_slint_image(w, h, &rgba);
                                item.has_cover = true;
                                model.set_row_data(idx, item);
                            }
                        }
                    });
                }
            });
        }
    });
}

// ============================================================================
// INSTANCES & ENGINES
// ============================================================================

fn bind_instances_callbacks(window: &MainWindow, state: &AppState) {
    let adapter = window.global::<MainWindowAdapter>();

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_select_engine(move |engine_id| {
        if let Some(win) = weak.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            ad.set_selected_engine_id(engine_id);
            ad.set_selected_release_index(0);
        }
        refresh_instances_overview(weak.clone(), state_c.clone());
        load_engine_releases(weak.clone(), state_c.clone(), false);
    });

    let weak = window.as_weak();
    adapter.on_select_engine_release(move |idx| {
        if let Some(win) = weak.upgrade() {
            win.global::<MainWindowAdapter>().set_selected_release_index(idx);
        }
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_refresh_engine_releases(move || {
        refresh_instances_overview(weak.clone(), state_c.clone());
        load_engine_releases(weak.clone(), state_c.clone(), true);
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_install_selected_engine(move || {
        install_selected_engine_flow(weak.clone(), state_c.clone());
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_cancel_engine_download(move || {
        if let Ok(guard) = state_c.engine_cancel.lock() {
            guard.store(true, Ordering::SeqCst);
        }
        if let Some(win) = weak.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            ad.set_engine_busy(false);
            ad.set_engine_busy_label("".into());
        }
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_launch_selected_engine(move || {
        let Some(win) = weak.upgrade() else { return };
        let ad = win.global::<MainWindowAdapter>();
        let engine_id = ad.get_selected_engine_id().to_string();
        let rel_idx = ad.get_selected_release_index();
        let version = ad
            .get_engine_releases()
            .row_data(rel_idx.max(0) as usize)
            .filter(|r| r.is_installed)
            .map(|r| r.tag.to_string());

        let st = state_c.clone();
        let wk = weak.clone();
        state_c.runtime.clone().spawn(async move {
            let eid = engine_id.clone();
            let res = tokio::task::spawn_blocking(move || {
                st.mods.launch_engine_with_mods(
                    &st.paths,
                    &st.engines,
                    &eid,
                    version.as_deref(),
                    None,
                )
            })
            .await;

            match res {
                Ok(Ok(_pid)) => {
                    show_callout(wk, &state_c, "Engine Launched", &format!("{engine_id} started."), "success");
                }
                Ok(Err(err)) => {
                    show_callout(wk, &state_c, "Launch Error", &err.to_string(), "error");
                }
                Err(err) => {
                    show_callout(wk, &state_c, "Launch Error", &err.to_string(), "error");
                }
            }
        });
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_stop_selected_engine(move || {
        let Some(win) = weak.upgrade() else { return };
        let engine_id = win.global::<MainWindowAdapter>().get_selected_engine_id().to_string();
        match state_c.engines.stop_engine(&engine_id) {
            Ok(true) => {
                show_callout(weak.clone(), &state_c, "Engine Stopped", "Process terminated.", "info");
            }
            Ok(false) => {
                show_callout(weak.clone(), &state_c, "Not Running", "No active process found.", "info");
            }
            Err(err) => {
                show_callout(weak.clone(), &state_c, "Stop Error", &err.to_string(), "error");
            }
        }
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_set_default_engine_version(move || {
        let Some(win) = weak.upgrade() else { return };
        let ad = win.global::<MainWindowAdapter>();
        let engine_id = ad.get_selected_engine_id().to_string();
        let rel_idx = ad.get_selected_release_index();
        let Some(rel) = ad.get_engine_releases().row_data(rel_idx.max(0) as usize) else {
            return;
        };
        let tag = rel.tag.to_string();
        if let Err(err) = state_c.engines.set_default_version(&state_c.paths, &engine_id, &tag) {
            show_callout(weak.clone(), &state_c, "Error", &err.to_string(), "error");
        } else {
            refresh_instances_overview(weak.clone(), state_c.clone());
            load_engine_releases(weak.clone(), state_c.clone(), false);
        }
    });

    let weak = window.as_weak();
    adapter.on_uninstall_selected_engine(move || {
        let Some(win) = weak.upgrade() else { return };
        let ad = win.global::<MainWindowAdapter>();
        let engine_id = ad.get_selected_engine_id().to_string();
        let rel_idx = ad.get_selected_release_index();
        let Some(rel) = ad.get_engine_releases().row_data(rel_idx.max(0) as usize) else {
            return;
        };
        if !rel.is_installed {
            return;
        }
        let tag = rel.tag.to_string();
        ad.set_confirm_dialog(ConfirmDialogData {
            open: true,
            title: "Uninstall Engine Version".into(),
            message: format!("Uninstall {} ({})?", ad.get_selected_engine_name(), tag).into(),
            confirm_text: "Uninstall".into(),
            cancel_text: "Cancel".into(),
            action_kind: "uninstall_engine".into(),
            action_target_id: format!("{engine_id}::{tag}").into(),
        });
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_open_engine_folder(move || {
        let Some(win) = weak.upgrade() else { return };
        let engine_id = win.global::<MainWindowAdapter>().get_selected_engine_id().to_string();
        let dir = state_c.paths.engines_dir.join(engine_id);
        let _ = std::fs::create_dir_all(&dir);
        if let Err(err) = host::open_path_in_explorer(&dir) {
            show_callout(weak.clone(), &state_c, "Folder Error", &err.to_string(), "error");
        }
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_open_engine_github(move || {
        let Some(win) = weak.upgrade() else { return };
        let engine_id = win.global::<MainWindowAdapter>().get_selected_engine_id().to_string();
        if let Some(def) = state_c.engines.definition(&engine_id) {
            if let Err(err) = host::open_url_in_browser(&def.github_url()) {
                show_callout(weak.clone(), &state_c, "Browser Error", &err.to_string(), "error");
            }
        }
    });
}

fn refresh_instances_overview(weak: Weak<MainWindow>, state: AppState) {
    state.runtime.clone().spawn(async move {
        let st = state.clone();
        let scan_res = tokio::task::spawn_blocking(move || {
            st.engines
                .scan_local_engines(&st.paths)
                .unwrap_or_else(|_| EngineUpdateState::load(&st.paths).unwrap_or_default())
        })
        .await;

        let Ok(update_state) = scan_res else { return };

        let _ = slint::invoke_from_event_loop(move || {
            if let Some(win) = weak.upgrade() {
                let ad = win.global::<MainWindowAdapter>();
                let selected_id = ad.get_selected_engine_id().to_string();

                let opts: Vec<EngineOptionData> = state
                    .engines
                    .definitions()
                    .iter()
                    .map(|def| {
                        let (r, g, b) = def.accent_rgb();
                        let count = update_state.versions_for(def.id).len() as i32;
                        let active = update_state.active_version_for(def.id).unwrap_or("").to_string();
                        EngineOptionData {
                            id: def.id.into(),
                            name: def.name.into(),
                            repo: def.repo.into(),
                            description: def.description.into(),
                            accent: Color::from_rgb_u8(r, g, b),
                            installed_count: count,
                            active_version: active.into(),
                            has_banner: def.has_banner,
                        }
                    })
                    .collect();

                ad.set_engines_list(ModelRc::new(VecModel::from(opts)));

                if let Some(def) = state.engines.definition(&selected_id) {
                    ad.set_selected_engine_name(def.name.into());
                    ad.set_selected_engine_repo(def.repo.into());
                    ad.set_selected_engine_docs(def.docs_markdown.into());
                    let active_ver = update_state
                        .active_version_for(def.id)
                        .unwrap_or("")
                        .to_string();
                    ad.set_selected_engine_active_version(active_ver.into());
                    let mins = state.engines.get_playtime_minutes(&state.paths, def.id);
                    ad.set_engine_playtime_label(format_playtime_minutes(mins).into());
                }
            }
        });
    });
}

fn load_engine_releases(weak: Weak<MainWindow>, state: AppState, force_refresh: bool) {
    let (tx_id, rx_id) = tokio::sync::oneshot::channel::<String>();
    let wk_read = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        let id = if let Some(win) = wk_read.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            ad.set_engine_releases_loading(true);
            ad.set_engine_releases_error("".into());
            ad.get_selected_engine_id().to_string()
        } else {
            "psych".to_string()
        };
        let _ = tx_id.send(id);
    });

    state.runtime.clone().spawn(async move {
        let engine_id = rx_id.await.unwrap_or_else(|_| "psych".to_string());
        let Some(def) = state.engines.definition(&engine_id) else {
            return;
        };

        match state.github.fetch_engine_releases(def.repo, force_refresh).await {
            Ok(releases) => {
                let update_state = EngineUpdateState::load(&state.paths).unwrap_or_default();
                let installed_vers = update_state.versions_for(&engine_id);
                let active_ver = update_state.active_version_for(&engine_id).unwrap_or("");

                let mut paired = Vec::with_capacity(releases.len());
                let mut ui_rows = Vec::with_capacity(releases.len());

                for rel in releases {
                    let candidates: Vec<EngineAssetCandidate> = rel
                        .assets
                        .iter()
                        .map(|a| EngineAssetCandidate {
                            name: a.name.clone(),
                            size: a.size,
                            download_url: a.download_url.clone(),
                        })
                        .collect();
                    let best = get_compatible_asset(&candidates).cloned();
                    let installed_match = installed_vers
                        .iter()
                        .find(|v| v.tag == rel.tag_name || v.version == rel.tag_name);
                    let is_inst = installed_match.is_some();
                    let is_def = is_inst
                        && (installed_match.is_some_and(|v| v.is_default)
                            || active_ver == rel.tag_name);

                    ui_rows.push((
                        rel.tag_name.clone(),
                        rel.name.clone(),
                        format_iso_date(&rel.published_at),
                        rel.body.clone(),
                        best.as_ref().map(|a| a.name.clone()).unwrap_or_default(),
                        best.as_ref().map(|a| format_file_size(a.size)).unwrap_or_default(),
                        best.as_ref().map(|a| a.download_url.clone()).unwrap_or_default(),
                        is_inst,
                        is_def,
                        rel.prerelease,
                        best.is_some(),
                    ));
                    paired.push((rel, best));
                }

                {
                    let mut guard = state.cached_releases.write().await;
                    *guard = paired;
                }

                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = weak.upgrade() {
                        let ad = win.global::<MainWindowAdapter>();
                        if ad.get_selected_engine_id() != engine_id.as_str() {
                            return;
                        }
                        let mapped: Vec<EngineReleaseData> = ui_rows
                            .into_iter()
                            .map(
                                |(
                                    tag,
                                    name,
                                    pub_lbl,
                                    body,
                                    asset_name,
                                    asset_sz,
                                    dl_url,
                                    is_inst,
                                    is_def,
                                    prerelease,
                                    compatible,
                                )| EngineReleaseData {
                                    tag: tag.into(),
                                    name: name.into(),
                                    published_label: pub_lbl.into(),
                                    body: body.into(),
                                    asset_name: asset_name.into(),
                                    asset_size_label: asset_sz.into(),
                                    download_url: dl_url.into(),
                                    is_installed: is_inst,
                                    is_default: is_def,
                                    is_prerelease: prerelease,
                                    compatible,
                                },
                            )
                            .collect();
                        ad.set_engine_releases(ModelRc::new(VecModel::from(mapped)));
                        ad.set_engine_releases_loading(false);
                    }
                });
            }
            Err(err) => {
                let err_s = err.to_string();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = weak.upgrade() {
                        let ad = win.global::<MainWindowAdapter>();
                        ad.set_engine_releases_loading(false);
                        ad.set_engine_releases_error(err_s.into());
                    }
                });
            }
        }
    });
}

fn install_selected_engine_flow(weak: Weak<MainWindow>, state: AppState) {
    let Some(win) = weak.upgrade() else { return };
    let ad = win.global::<MainWindowAdapter>();
    if ad.get_engine_busy() {
        return;
    }
    let engine_id = ad.get_selected_engine_id().to_string();
    let rel_idx = ad.get_selected_release_index().max(0) as usize;
    let Some(rel) = ad.get_engine_releases().row_data(rel_idx) else {
        return;
    };
    if !rel.compatible || rel.download_url.is_empty() {
        show_callout(weak, &state, "Incompatible", "No compatible release archive found.", "error");
        return;
    }

    ad.set_engine_busy(true);
    ad.set_engine_busy_progress(0.02);
    ad.set_engine_busy_label("Starting download...".into());

    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut guard) = state.engine_cancel.lock() {
        guard.store(true, Ordering::SeqCst);
        *guard = cancel.clone();
    }

    let tag = rel.tag.to_string();
    let rel_name = rel.name.to_string();
    let asset = EngineAssetCandidate {
        name: rel.asset_name.to_string(),
        size: 0,
        download_url: rel.download_url.to_string(),
    };

    state.runtime.clone().spawn(async move {
        let wk_prog = weak.clone();
        let res = state
            .engines
            .install_release(
                &state.paths,
                &state.http_client,
                &engine_id,
                &tag,
                &rel_name,
                &asset,
                &cancel,
                move |pct, label| {
                    let wk_inner = wk_prog.clone();
                    let lbl = label.to_string();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(win) = wk_inner.upgrade() {
                            let ad = win.global::<MainWindowAdapter>();
                            ad.set_engine_busy_progress(pct);
                            ad.set_engine_busy_label(lbl.into());
                        }
                    });
                },
            )
            .await;

        let wk_done = weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(win) = wk_done.upgrade() {
                let ad = win.global::<MainWindowAdapter>();
                ad.set_engine_busy(false);
                ad.set_engine_busy_label("".into());
            }
        });

        match res {
            Ok(_) => {
                show_callout(
                    weak.clone(),
                    &state,
                    "Engine Installed",
                    &format!("{engine_id} ({tag}) installed successfully."),
                    "success",
                );
                refresh_instances_overview(weak.clone(), state.clone());
                load_engine_releases(weak, state, false);
            }
            Err(err) => {
                show_callout(weak, &state, "Install Error", &err.to_string(), "error");
            }
        }
    });
}

fn perform_uninstall_engine(
    weak: Weak<MainWindow>,
    state: AppState,
    engine_id: String,
    version: String,
) {
    let st = state.clone();
    let wk = weak.clone();
    state.runtime.clone().spawn(async move {
        let eid = engine_id.clone();
        let ver = version.clone();
        let res = tokio::task::spawn_blocking(move || {
            st.engines.uninstall_version(&st.paths, &eid, &ver)
        })
        .await;

        match res {
            Ok(Ok(())) => {
                show_callout(wk.clone(), &state, "Uninstalled", &format!("Removed {engine_id} {version}."), "info");
                refresh_instances_overview(wk.clone(), state.clone());
                load_engine_releases(wk, state, false);
            }
            Ok(Err(err)) => {
                show_callout(wk, &state, "Uninstall Error", &err.to_string(), "error");
            }
            Err(err) => {
                show_callout(wk, &state, "Uninstall Error", &err.to_string(), "error");
            }
        }
    });
}

fn start_process_monitor(weak: Weak<MainWindow>, state: AppState) {
    state.runtime.clone().spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(2));
        loop {
            interval.tick().await;
            let running = state.engines.detect_running_engines();

            let (tx_sel, rx_sel) = tokio::sync::oneshot::channel::<String>();
            let wk_sel = weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                let sel = wk_sel
                    .upgrade()
                    .map(|w| w.global::<MainWindowAdapter>().get_selected_engine_id().to_string())
                    .unwrap_or_default();
                let _ = tx_sel.send(sel);
            });
            let Ok(selected_id) = rx_sel.await else { break };

            let now_running_first = running.first().map(|r| r.engine_id.clone());
            let mut finished_session: Option<(String, u64)> = None;
            let mut active_secs = 0u64;
            let mut selected_is_running = false;

            if let Ok(mut guard) = state.active_running_session.lock() {
                if let Some((ref cur_id, started)) = *guard {
                    if running.iter().any(|r| &r.engine_id == cur_id) {
                        active_secs = started.elapsed().as_secs();
                        selected_is_running = cur_id == &selected_id;
                    } else {
                        let elapsed = started.elapsed().as_secs();
                        finished_session = Some((cur_id.clone(), elapsed));
                        *guard = None;
                    }
                } else if let Some(eid) = now_running_first {
                    *guard = Some((eid.clone(), Instant::now()));
                    selected_is_running = eid == selected_id;
                }
            }

            if let Some((eid, secs)) = finished_session {
                let _ = state.engines.record_playtime(&state.paths, &eid, secs);
                refresh_instances_overview(weak.clone(), state.clone());
            } else {
                selected_is_running = running.iter().any(|r| r.engine_id == selected_id);
            }

            let wk_ui = weak.clone();
            let sess_label = if selected_is_running {
                format_duration_hms(active_secs)
            } else {
                String::new()
            };
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(win) = wk_ui.upgrade() {
                    let ad = win.global::<MainWindowAdapter>();
                    ad.set_engine_running(selected_is_running);
                    ad.set_engine_session_label(sess_label.into());
                }
            });
        }
    });
}

// ============================================================================
// NEWS FEED
// ============================================================================

fn bind_news_callbacks(window: &MainWindow, state: &AppState) {
    let adapter = window.global::<MainWindowAdapter>();

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_select_news_account(move |account| {
        if let Some(win) = weak.upgrade() {
            win.global::<MainWindowAdapter>()
                .set_news_selected_account(account);
        }
        load_news_feed(weak.clone(), state_c.clone(), false);
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_refresh_news(move || {
        load_news_feed(weak.clone(), state_c.clone(), false);
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_load_more_news(move || {
        load_news_feed(weak.clone(), state_c.clone(), true);
    });

    let weak = window.as_weak();
    let state_c = state.clone();
    adapter.on_open_news_media(move |post_id| {
        let id_s = post_id.to_string();
        let st = state_c.clone();
        let wk = weak.clone();
        state_c.runtime.clone().spawn(async move {
            let post = {
                let guard = st.news_posts_cache.read().await;
                guard.iter().find(|p| p.id == id_s).cloned()
            };
            let Some(p) = post else { return };
            if p.photo_url.is_empty() {
                return;
            }
            if let Some((w, h, rgba)) = st.gamebanana.fetch_cover_rgba(&p.photo_url, 1200, 900).await {
                let caption = format!("@{} — {}", p.author_handle, p.text);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = wk.upgrade() {
                        let ad = win.global::<MainWindowAdapter>();
                        ad.set_news_media_image(rgba_to_slint_image(w, h, &rgba));
                        ad.set_news_media_caption(caption.into());
                        ad.set_news_media_open(true);
                    }
                });
            }
        });
    });

    let weak = window.as_weak();
    adapter.on_close_news_media(move || {
        if let Some(win) = weak.upgrade() {
            win.global::<MainWindowAdapter>().set_news_media_open(false);
        }
    });
}

fn load_news_feed(weak: Weak<MainWindow>, state: AppState, load_more: bool) {
    let (tx_acc, rx_acc) = tokio::sync::oneshot::channel::<String>();
    let wk_read = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        let acc = if let Some(win) = wk_read.upgrade() {
            let ad = win.global::<MainWindowAdapter>();
            if load_more {
                ad.set_news_loading_more(true);
            } else {
                ad.set_news_loading(true);
                ad.set_news_error("".into());
            }
            ad.get_news_selected_account().to_string()
        } else {
            String::new()
        };
        let _ = tx_acc.send(acc);
    });

    state.runtime.clone().spawn(async move {
        let selected_account = rx_acc.await.unwrap_or_default();
        if !load_more {
            let mut cursors = state.news_cursors.write().await;
            cursors.clear();
        }

        let current_cursors = {
            let cursors = state.news_cursors.read().await;
            cursors.clone()
        };

        let acc_opt = if selected_account.trim().is_empty() {
            None
        } else {
            Some(selected_account.as_str())
        };

        match state.news.fetch_feed(acc_opt, &current_cursors).await {
            Ok(batch) => {
                {
                    let mut cursors = state.news_cursors.write().await;
                    for (k, v) in batch.cursors {
                        cursors.insert(k, v);
                    }
                }

                let has_more = {
                    let cursors = state.news_cursors.read().await;
                    !cursors.is_empty() && !batch.posts.is_empty()
                };

                let all_posts: Vec<NewsPost> = {
                    let mut cache = state.news_posts_cache.write().await;
                    if !load_more {
                        *cache = batch.posts;
                    } else {
                        let existing_ids: std::collections::HashSet<String> =
                            cache.iter().map(|p| p.id.clone()).collect();
                        for p in batch.posts {
                            if !existing_ids.contains(&p.id) {
                                cache.push(p);
                            }
                        }
                        cache.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
                    }
                    cache.clone()
                };

                let photo_urls: Vec<(usize, String)> = all_posts
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| !p.photo_url.is_empty())
                    .map(|(i, p)| (i, p.photo_url.clone()))
                    .collect();

                let wk_ui = weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = wk_ui.upgrade() {
                        let ad = win.global::<MainWindowAdapter>();
                        let rows: Vec<NewsPostData> = all_posts
                            .into_iter()
                            .map(|p| NewsPostData {
                                id: p.id.into(),
                                author_name: p.author_name.into(),
                                author_handle: p.author_handle.into(),
                                source_name: p.source_name.into(),
                                text: p.text.into(),
                                time_label: p.time_label.into(),
                                url: p.url.into(),
                                is_repost: p.is_repost,
                                reposted_by: p.reposted_by.into(),
                                quote_author: p.quote_author.into(),
                                quote_text: p.quote_text.into(),
                                likes_label: format_compact_number(p.likes as i64).into(),
                                reposts_label: format_compact_number(p.reposts as i64).into(),
                                replies_label: format_compact_number(p.replies as i64).into(),
                                views_label: format_compact_number(p.views as i64).into(),
                                photo: Image::default(),
                                has_photo: false,
                                photo_url: p.photo_url.into(),
                                has_video: p.has_video,
                            })
                            .collect();
                        ad.set_news_posts(ModelRc::new(VecModel::from(rows)));
                        ad.set_news_loading(false);
                        ad.set_news_loading_more(false);
                        ad.set_news_has_more(has_more);
                    }
                });

                for (idx, url) in photo_urls {
                    let gb = state.gamebanana.clone();
                    let wk_img = weak.clone();
                    state.runtime.spawn(async move {
                        if let Some((w, h, rgba)) = gb.fetch_cover_rgba(&url, 640, 480).await {
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(win) = wk_img.upgrade() {
                                    let model = win.global::<MainWindowAdapter>().get_news_posts();
                                    if let Some(mut item) = model.row_data(idx) {
                                        if item.photo_url == url.as_str() {
                                            item.photo = rgba_to_slint_image(w, h, &rgba);
                                            item.has_photo = true;
                                            model.set_row_data(idx, item);
                                        }
                                    }
                                }
                            });
                        }
                    });
                }
            }
            Err(err) => {
                let err_s = err.to_string();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = weak.upgrade() {
                        let ad = win.global::<MainWindowAdapter>();
                        ad.set_news_loading(false);
                        ad.set_news_loading_more(false);
                        if !load_more {
                            ad.set_news_error(err_s.into());
                        }
                    }
                });
            }
        }
    });
}
