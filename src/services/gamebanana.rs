use std::collections::{HashMap, HashSet};
use std::time::Duration;

use reqwest::{Client, Url};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::{RwLock, Semaphore};
use tokio::task::JoinSet;

pub const FEATURED_URL: &str =
    "https://raw.githubusercontent.com/Crew-Awesome/weekbox.featured/main/public/featured.json";

const GB_BASE_URL: &str = "https://gamebanana.com/apiv11";
const FNF_GAME_ID: u64 = 8694;
const MAX_IMAGE_CACHE_ENTRIES: usize = 128;
const MULTI_CHUNK_SIZE: usize = 15;

// Excluded non-NSFW categories from wb-rew/src/services/gamebanana/constants.ts
// (43772 is NSFW and is kept with `is_nsfw = true`).
const EXCLUDED_CATEGORY_IDS: &[u64] = &[3833, 44037];
const NSFW_CATEGORY_ID: u64 = 43772;

#[derive(Debug, Clone, Default)]
pub struct FeaturedMod {
    pub id: u64,
    pub title: String,
    pub description: String,
    pub author: String,
    pub engine_id: String,
    pub engine: String,
    pub badge: String,
    pub image_url: String,
}

#[derive(Debug, Clone, Default)]
pub struct HomeMod {
    pub id: u64,
    pub title: String,
    pub description: String,
    pub author: String,
    pub engine_id: String,
    pub engine_name: String,
    pub likes: i32,
    pub views: i32,
    pub downloads: i32,
    pub is_nsfw: bool,
    pub has_warning: bool,
    pub image_url: String,
}

#[derive(Debug, Clone, Default)]
pub struct ModFileEntry {
    pub id: u64,
    pub filename: String,
    pub description: String,
    pub filesize: u64,
    pub download_count: u64,
    pub download_url: String,
}

#[derive(Debug, Clone, Default)]
pub struct ModDetails {
    pub id: u64,
    pub title: String,
    pub description: String,
    pub author: String,
    pub engine_id: String,
    pub engine_name: String,
    pub version: String,
    pub likes: i32,
    pub views: i32,
    pub downloads: i32,
    pub submitted_at: i64,
    pub page_url: String,
    pub preview_urls: Vec<String>,
    pub credits_summary: String,
    pub files: Vec<ModFileEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct FeaturedEngineRaw {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct FeaturedModRaw {
    id: u64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    engine: Option<FeaturedEngineRaw>,
}

#[derive(Debug, Clone, Deserialize)]
struct FeaturedRankingRaw {
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    mods: Vec<FeaturedModRaw>,
}

#[derive(Debug, Clone, Deserialize)]
struct FeaturedSchemaRaw {
    #[serde(default)]
    rankings: Vec<FeaturedRankingRaw>,
}

#[derive(Debug, Clone, Deserialize)]
struct GbModRecordRaw {
    #[serde(rename = "_idRow", default)]
    id: Option<u64>,
    #[serde(rename = "_sName", default)]
    name: Option<String>,
    #[serde(rename = "_sDescription", default)]
    description: Option<String>,
    #[serde(rename = "_sText", default)]
    text: Option<String>,
    #[serde(rename = "_sVersion", default)]
    version: Option<String>,
    #[serde(rename = "_nLikeCount", default)]
    like_count: Option<i64>,
    #[serde(rename = "_nViewCount", default)]
    view_count: Option<i64>,
    #[serde(rename = "_nDownloadCount", default)]
    download_count: Option<i64>,
    #[serde(rename = "_tsDateAdded", default)]
    date_added: Option<i64>,
    #[serde(rename = "_tsDateUpdated", default)]
    date_updated: Option<i64>,
    #[serde(rename = "_tsDateModified", default)]
    date_modified: Option<i64>,
    #[serde(rename = "_bIsNsfw", default)]
    is_nsfw: Option<bool>,
    #[serde(rename = "_bContainsNsfw", default)]
    contains_nsfw: Option<bool>,
    #[serde(rename = "_sProfileUrl", default)]
    profile_url: Option<String>,
    #[serde(rename = "_aGame", default)]
    game: Option<Value>,
    #[serde(rename = "_aSubmitter", default)]
    submitter: Option<Value>,
    #[serde(rename = "_aCategory", default)]
    category: Option<Value>,
    #[serde(rename = "_aSuperCategory", default)]
    super_category: Option<Value>,
    #[serde(rename = "_aRootCategory", default)]
    root_category: Option<Value>,
    #[serde(rename = "_aSubCategory", default)]
    sub_category: Option<Value>,
    #[serde(rename = "_aPreviewMedia", default)]
    preview_media: Option<Value>,
    #[serde(rename = "_aImages", default)]
    images: Option<Value>,
    #[serde(rename = "_aCredits", default)]
    credits: Option<Value>,
    #[serde(rename = "_aFiles", default)]
    files: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct GbRecordsResponseRaw {
    #[serde(rename = "_aRecords", default)]
    records: Option<Vec<GbModRecordRaw>>,
}

#[derive(Debug, Clone, Deserialize)]
struct GbFileRaw {
    #[serde(rename = "_idRow", default)]
    id: Option<u64>,
    #[serde(rename = "_sFile", default)]
    filename: Option<String>,
    #[serde(rename = "_sDescription", default)]
    description: Option<String>,
    #[serde(rename = "_nFilesize", default)]
    filesize: Option<u64>,
    #[serde(rename = "_nDownloadCount", default)]
    download_count: Option<u64>,
    #[serde(rename = "_sDownloadUrl", default)]
    download_url: Option<String>,
}

pub struct GameBananaClient {
    http: Client,
    image_cache: RwLock<HashMap<String, (u32, u32, Vec<u8>)>>,
}

impl Default for GameBananaClient {
    fn default() -> Self {
        Self::new()
    }
}

impl GameBananaClient {
    pub fn new() -> Self {
        let http = Client::builder()
            .user_agent("wb-r/3.0.0")
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            http,
            image_cache: RwLock::new(HashMap::new()),
        }
    }

    pub async fn fetch_bytes(&self, url: &str) -> Option<Vec<u8>> {
        let trimmed = url.trim();
        if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
            return None;
        }
        let resp = self.http.get(trimmed).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let bytes = resp.bytes().await.ok()?;
        Some(bytes.to_vec())
    }

    pub async fn fetch_cover_rgba(
        &self,
        url: &str,
        max_w: u32,
        max_h: u32,
    ) -> Option<(u32, u32, Vec<u8>)> {
        let trimmed = url.trim();
        if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
            return None;
        }

        let cache_key = format!("{trimmed}#{max_w}x{max_h}");
        {
            let cache = self.image_cache.read().await;
            if let Some(hit) = cache.get(&cache_key) {
                return Some(hit.clone());
            }
        }

        let bytes = self.fetch_bytes(trimmed).await?;
        let decoded = tokio::task::spawn_blocking(move || -> Option<(u32, u32, Vec<u8>)> {
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
        })
        .await
        .ok()??;

        {
            let mut cache = self.image_cache.write().await;
            if cache.len() >= MAX_IMAGE_CACHE_ENTRIES && !cache.contains_key(&cache_key) {
                if let Some(evict_key) = cache.keys().next().cloned() {
                    cache.remove(&evict_key);
                }
            }
            cache.insert(cache_key, decoded.clone());
        }

        Some(decoded)
    }

    pub async fn fetch_featured_mods(&self) -> anyhow::Result<Vec<FeaturedMod>> {
        let resp = self.http.get(FEATURED_URL).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("Featured JSON HTTP {}", resp.status());
        }
        let schema: FeaturedSchemaRaw = resp.json().await?;
        let mut out = Vec::new();
        for ranking in schema.rankings {
            let badge = ranking.label.unwrap_or_else(|| "Featured".to_string());
            for m in ranking.mods {
                let (engine_id, engine) = match m.engine {
                    Some(e) => (e.id.unwrap_or_default(), e.name.unwrap_or_default()),
                    None => (String::new(), String::new()),
                };
                let description = m
                    .description
                    .as_deref()
                    .map(html_to_plain_text)
                    .unwrap_or_default();
                let image_url = upgrade_gb_image_url(m.image.as_deref().unwrap_or_default());

                out.push(FeaturedMod {
                    id: m.id,
                    title: m.title.unwrap_or_else(|| "Unknown Mod".to_string()),
                    description,
                    author: m.author.unwrap_or_else(|| "Unknown Creator".to_string()),
                    engine_id,
                    engine,
                    badge: badge.clone(),
                    image_url,
                });
            }
        }
        Ok(out)
    }

    pub async fn fetch_mods(
        &self,
        filter: &str,
        engine_filter: &str,
        search_query: &str,
        page: u32,
        per_page: u32,
    ) -> anyhow::Result<Vec<HomeMod>> {
        let page = page.max(1);
        let per_page = per_page.clamp(1, 50);
        let trimmed_query = search_query.trim();
        let norm_filter = filter.trim().to_ascii_lowercase();
        let norm_engine = engine_filter.trim().to_ascii_lowercase();
        let is_all_engines = norm_engine.is_empty() || norm_engine == "all";

        if !trimmed_query.is_empty() {
            if let Some((mod_id, is_url)) = parse_direct_mod_id(trimmed_query) {
                if page > 1 {
                    return Ok(Vec::new());
                }
                match self.fetch_mod_by_id(mod_id).await {
                    Ok(details) => {
                        if is_all_engines || details.engine_id.eq_ignore_ascii_case(&norm_engine) {
                            return Ok(vec![details_to_home_mod(&details)]);
                        }
                        return Ok(Vec::new());
                    }
                    Err(err) if is_url => {
                        return Err(err);
                    }
                    Err(_) => {
                        // Plain numeric search query that is not a direct FNF mod ID;
                        // fall through to standard search.
                    }
                }
            }

            let raw_records = self
                .fetch_search_records(trimmed_query, &norm_filter, page, per_page)
                .await?;
            let mut mods = self.enrich_and_map_records(raw_records, None).await;
            if !is_all_engines {
                mods.retain(|m| m.engine_id.eq_ignore_ascii_case(&norm_engine));
            }
            return Ok(mods);
        }

        let sort_param = match norm_filter.as_str() {
            "ripe" => "Generic_MostLiked",
            "popular" => "Generic_MostDownloaded",
            "new" => "Generic_Newest",
            "updated" => "Generic_LatestUpdated",
            _ => "Generic_MostLiked",
        };

        let category_ids = resolve_filter_category_ids(&norm_engine);

        match category_ids {
            None => {
                let records = self.fetch_index_page(None, sort_param, page, per_page).await?;
                Ok(self.enrich_and_map_records(records, None).await)
            }
            Some(cat_ids) if cat_ids.len() == 1 => {
                let cat_id = cat_ids[0];
                let records = self
                    .fetch_index_page(Some(cat_id), sort_param, page, per_page)
                    .await?;
                Ok(self.enrich_and_map_records(records, Some(cat_id)).await)
            }
            Some(cat_ids) => {
                // Multi-category engines (e.g. "vslice" -> 29202 & 43777)
                let mut join_set = JoinSet::new();
                for cat_id in cat_ids {
                    let client = self.http.clone();
                    let sort = sort_param.to_string();
                    join_set.spawn(async move {
                        let res = fetch_index_page_with_client(
                            &client,
                            Some(cat_id),
                            &sort,
                            page,
                            per_page,
                        )
                        .await;
                        (cat_id, res)
                    });
                }

                let mut seen_ids = HashSet::new();
                let mut combined: Vec<(GbModRecordRaw, Option<u64>)> = Vec::new();
                let mut had_success = false;
                let mut last_error: Option<anyhow::Error> = None;

                while let Some(joined) = join_set.join_next().await {
                    match joined {
                        Ok((cat_id, Ok(records))) => {
                            had_success = true;
                            for rec in records {
                                let id = rec.id.unwrap_or(0);
                                if id > 0 && !is_excluded(&rec) && seen_ids.insert(id) {
                                    combined.push((rec, Some(cat_id)));
                                }
                            }
                        }
                        Ok((_, Err(err))) => {
                            last_error = Some(err);
                        }
                        Err(err) => {
                            last_error = Some(err.into());
                        }
                    }
                }

                if !had_success {
                    if let Some(err) = last_error {
                        return Err(err);
                    }
                }

                combined.sort_by(|(a, _), (b, _)| compare_records_for_filter(a, b, &norm_filter));
                combined.truncate(per_page as usize);

                let records: Vec<GbModRecordRaw> =
                    combined.iter().map(|(r, _)| r.clone()).collect();
                let fallback_cat = combined.first().and_then(|(_, cat)| *cat);
                Ok(self.enrich_and_map_records(records, fallback_cat).await)
            }
        }
    }

    pub async fn fetch_mod_by_id(&self, mod_id: u64) -> anyhow::Result<ModDetails> {
        let url = format!("{GB_BASE_URL}/Mod/{mod_id}/ProfilePage");
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("GameBanana Mod/{mod_id}/ProfilePage HTTP {}", resp.status());
        }

        let raw: GbModRecordRaw = resp.json().await?;
        let id = raw.id.filter(|v| *v > 0).ok_or_else(|| {
            anyhow::anyhow!("GameBanana returned empty record for mod {mod_id}")
        })?;

        if let Some(game_id) = extract_game_id(raw.game.as_ref()) {
            if game_id != 0 && game_id != FNF_GAME_ID {
                anyhow::bail!(
                    "Mod {mod_id} does not belong to Friday Night Funkin' (Game ID: {game_id})"
                );
            }
        }

        let (engine_id, raw_cat_name) = resolve_engine(&raw, None);
        let engine_name = resolve_engine_display_name(&engine_id, &raw_cat_name);

        let text_clean = raw
            .text
            .as_deref()
            .map(html_to_plain_text)
            .unwrap_or_default();
        let desc_clean = raw
            .description
            .as_deref()
            .map(html_to_plain_text)
            .unwrap_or_default();
        let description = if !text_clean.is_empty() {
            text_clean
        } else {
            desc_clean
        };

        let author = extract_submitter_name(raw.submitter.as_ref())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Unknown Creator".to_string());

        let page_url = raw
            .profile_url
            .filter(|s| s.starts_with("http://") || s.starts_with("https://"))
            .unwrap_or_else(|| format!("https://gamebanana.com/mods/{id}"));

        let preview_urls = extract_preview_urls(&raw, true);
        let credits_summary = extract_credits_summary(raw.credits.as_ref());
        let files = extract_files(raw.files.as_ref());

        Ok(ModDetails {
            id,
            title: raw
                .name
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "Unknown Mod".to_string()),
            description,
            author,
            engine_id,
            engine_name,
            version: raw.version.unwrap_or_default(),
            likes: clamp_i32(raw.like_count),
            views: clamp_i32(raw.view_count),
            downloads: clamp_i32(raw.download_count),
            submitted_at: raw.date_added.unwrap_or(0),
            page_url,
            preview_urls,
            credits_summary,
            files,
        })
    }

    async fn fetch_index_page(
        &self,
        category_id: Option<u64>,
        sort_param: &str,
        page: u32,
        per_page: u32,
    ) -> anyhow::Result<Vec<GbModRecordRaw>> {
        fetch_index_page_with_client(&self.http, category_id, sort_param, page, per_page).await
    }

    async fn fetch_search_records(
        &self,
        query: &str,
        filter: &str,
        page: u32,
        per_page: u32,
    ) -> anyhow::Result<Vec<GbModRecordRaw>> {
        let order = if filter == "new" || filter == "updated" {
            "date"
        } else {
            "popularity"
        };

        let mut url = Url::parse(&format!("{GB_BASE_URL}/Util/Search/Results"))?;
        url.query_pairs_mut()
            .append_pair("_sSearchString", query)
            .append_pair("_idGameRow", &FNF_GAME_ID.to_string())
            .append_pair("_sModelName", "Mod")
            .append_pair("_sOrder", order)
            .append_pair("_nPage", &page.to_string())
            .append_pair("_nPerpage", &per_page.to_string());

        let resp = self.http.get(url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("GameBanana Util/Search/Results HTTP {}", resp.status());
        }

        let parsed: GbRecordsResponseRaw = resp.json().await?;
        Ok(parsed
            .records
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.id.unwrap_or(0) > 0 && !is_excluded(r))
            .collect())
    }

    async fn enrich_and_map_records(
        &self,
        records: Vec<GbModRecordRaw>,
        fallback_cat_id: Option<u64>,
    ) -> Vec<HomeMod> {
        let filtered: Vec<GbModRecordRaw> = records
            .into_iter()
            .filter(|r| r.id.unwrap_or(0) > 0 && !is_excluded(r))
            .collect();

        if filtered.is_empty() {
            return Vec::new();
        }

        let ids: Vec<u64> = filtered.iter().filter_map(|r| r.id).collect();
        let multi_map = self.fetch_multi_metadata(&ids).await;

        filtered
            .into_iter()
            .map(|rec| {
                let id = rec.id.unwrap_or(0);
                let meta = multi_map.get(&id);
                map_record_to_home_mod(&rec, meta, fallback_cat_id)
            })
            .collect()
    }

    async fn fetch_multi_metadata(&self, ids: &[u64]) -> HashMap<u64, GbModRecordRaw> {
        if ids.is_empty() {
            return HashMap::new();
        }

        let sem = std::sync::Arc::new(Semaphore::new(4));
        let mut join_set = JoinSet::new();

        for chunk in ids.chunks(MULTI_CHUNK_SIZE) {
            let csv_ids = chunk
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let client = self.http.clone();
            let sem = sem.clone();

            join_set.spawn(async move {
                let _permit = sem.acquire_owned().await.ok()?;
                let mut url = Url::parse(&format!("{GB_BASE_URL}/Mod/Multi")).ok()?;
                url.query_pairs_mut()
                    .append_pair("_csvRowIds", &csv_ids)
                    .append_pair(
                        "_csvProperties",
                        "_idRow,_sName,_nLikeCount,_nViewCount,_nDownloadCount,_sDescription,_sText,_aCategory,_aSuperCategory,_aRootCategory,_aSubCategory,_aPreviewMedia,_bIsNsfw,_bContainsNsfw",
                    );

                let resp = client.get(url).send().await.ok()?;
                if !resp.status().is_success() {
                    return None;
                }
                let val: Value = resp.json().await.ok()?;
                let raw_items: Vec<GbModRecordRaw> = match val {
                    Value::Array(arr) => arr
                        .into_iter()
                        .filter_map(|v| serde_json::from_value::<GbModRecordRaw>(v).ok())
                        .collect(),
                    Value::Object(map) => map
                        .into_values()
                        .filter_map(|v| serde_json::from_value::<GbModRecordRaw>(v).ok())
                        .collect(),
                    _ => Vec::new(),
                };
                Some(raw_items)
            });
        }

        let mut map = HashMap::new();
        while let Some(joined) = join_set.join_next().await {
            if let Ok(Some(chunk_items)) = joined {
                for item in chunk_items {
                    if let Some(id) = item.id.filter(|v| *v > 0) {
                        map.insert(id, item);
                    }
                }
            }
        }
        map
    }
}

async fn fetch_index_page_with_client(
    client: &Client,
    category_id: Option<u64>,
    sort_param: &str,
    page: u32,
    per_page: u32,
) -> anyhow::Result<Vec<GbModRecordRaw>> {
    let mut url = Url::parse(&format!("{GB_BASE_URL}/Mod/Index"))?;
    {
        let mut qp = url.query_pairs_mut();
        qp.append_pair("_aFilters[Generic_Game]", &FNF_GAME_ID.to_string());
        if let Some(cat_id) = category_id {
            qp.append_pair("_aFilters[Generic_Category]", &cat_id.to_string());
        }
        qp.append_pair("_sSort", sort_param)
            .append_pair("_nPerpage", &per_page.to_string())
            .append_pair("_nPage", &page.to_string());
    }

    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("GameBanana Mod/Index HTTP {}", resp.status());
    }

    let parsed: GbRecordsResponseRaw = resp.json().await?;
    Ok(parsed.records.unwrap_or_default())
}

fn map_record_to_home_mod(
    rec: &GbModRecordRaw,
    meta: Option<&GbModRecordRaw>,
    fallback_cat_id: Option<u64>,
) -> HomeMod {
    let id = rec.id.unwrap_or(0);
    let title = rec
        .name
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| meta.and_then(|m| m.name.as_deref()).filter(|s| !s.trim().is_empty()))
        .unwrap_or("Unknown Mod")
        .to_string();

    let raw_desc = rec
        .description
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| meta.and_then(|m| m.description.as_deref()).filter(|s| !s.trim().is_empty()))
        .or_else(|| rec.text.as_deref().filter(|s| !s.trim().is_empty()))
        .or_else(|| meta.and_then(|m| m.text.as_deref()).filter(|s| !s.trim().is_empty()))
        .unwrap_or_default();
    let description = html_to_plain_text(raw_desc);

    let author = extract_submitter_name(rec.submitter.as_ref())
        .or_else(|| meta.and_then(|m| extract_submitter_name(m.submitter.as_ref())))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Unknown Creator".to_string());

    let (mut engine_id, mut raw_cat_name) = resolve_engine(rec, None);
    if engine_id.is_empty() {
        if let Some(m) = meta {
            let (meta_eid, meta_cname) = resolve_engine(m, fallback_cat_id);
            engine_id = meta_eid;
            if raw_cat_name.is_empty() {
                raw_cat_name = meta_cname;
            }
        } else if let Some(cat_id) = fallback_cat_id {
            engine_id = map_category_id_to_engine_id(cat_id).to_string();
        }
    }
    let engine_name = resolve_engine_display_name(&engine_id, &raw_cat_name);

    let likes = pick_nonzero_i32(rec.like_count, meta.and_then(|m| m.like_count));
    let views = pick_nonzero_i32(rec.view_count, meta.and_then(|m| m.view_count));
    let downloads = pick_nonzero_i32(rec.download_count, meta.and_then(|m| m.download_count));

    let is_nsfw = check_is_nsfw(rec) || meta.is_some_and(check_is_nsfw);
    let has_warning = engine_id.is_empty() || engine_id == "executable";

    let image_url = extract_preview_urls(rec, false)
        .into_iter()
        .next()
        .or_else(|| meta.and_then(|m| extract_preview_urls(m, false).into_iter().next()))
        .unwrap_or_default();

    HomeMod {
        id,
        title,
        description,
        author,
        engine_id,
        engine_name,
        likes,
        views,
        downloads,
        is_nsfw,
        has_warning,
        image_url,
    }
}

fn details_to_home_mod(details: &ModDetails) -> HomeMod {
    let has_warning = details.engine_id.is_empty() || details.engine_id == "executable";
    HomeMod {
        id: details.id,
        title: details.title.clone(),
        description: details.description.clone(),
        author: details.author.clone(),
        engine_id: details.engine_id.clone(),
        engine_name: details.engine_name.clone(),
        likes: details.likes,
        views: details.views,
        downloads: details.downloads,
        is_nsfw: false,
        has_warning,
        image_url: details.preview_urls.first().cloned().unwrap_or_default(),
    }
}

fn compare_records_for_filter(a: &GbModRecordRaw, b: &GbModRecordRaw, filter: &str) -> std::cmp::Ordering {
    match filter {
        "popular" => b
            .download_count
            .unwrap_or(0)
            .cmp(&a.download_count.unwrap_or(0)),
        "new" => b.date_added.unwrap_or(0).cmp(&a.date_added.unwrap_or(0)),
        "updated" => {
            let ta = a.date_updated.or(a.date_modified).or(a.date_added).unwrap_or(0);
            let tb = b.date_updated.or(b.date_modified).or(b.date_added).unwrap_or(0);
            tb.cmp(&ta)
        }
        _ => b.like_count.unwrap_or(0).cmp(&a.like_count.unwrap_or(0)),
    }
}

fn map_category_id_to_engine_id(cat_id: u64) -> &'static str {
    match cat_id {
        29202 | 43777 => "vslice",
        28367 => "psych",
        34764 => "codename",
        3827 => "executable",
        43798 => "pslice",
        43850 => "fpsplus",
        43788 => "psychonline",
        _ => "",
    }
}

fn canonical_engine_name(engine_id: &str) -> &'static str {
    match engine_id {
        "vslice" => "V-Slice / Base Game",
        "psych" => "Psych Engine",
        "codename" => "Codename Engine",
        "executable" => "Executable Mods",
        "pslice" => "P-Slice Engine",
        "fpsplus" => "FPS Plus",
        "psychonline" => "Psych Online",
        _ => "",
    }
}

fn resolve_engine_display_name(engine_id: &str, raw_cat_name: &str) -> String {
    let canonical = canonical_engine_name(engine_id);
    if !canonical.is_empty() {
        canonical.to_string()
    } else if !raw_cat_name.trim().is_empty() {
        raw_cat_name.trim().to_string()
    } else {
        "Friday Night Funkin'".to_string()
    }
}

fn resolve_filter_category_ids(engine_filter: &str) -> Option<Vec<u64>> {
    match engine_filter {
        "" | "all" => None,
        "vslice" => Some(vec![29202, 43777]),
        "psych" => Some(vec![28367]),
        "codename" => Some(vec![34764]),
        "executable" => Some(vec![3827]),
        "pslice" => Some(vec![43798]),
        "fpsplus" => Some(vec![43850]),
        "psychonline" => Some(vec![43788]),
        other => other.parse::<u64>().ok().map(|id| vec![id]),
    }
}

fn extract_category_info(val: Option<&Value>) -> (Option<u64>, Option<String>) {
    let Some(obj) = val.and_then(|v| v.as_object()) else {
        return (None, None);
    };

    let mut id = obj.get("_idRow").and_then(|v| v.as_u64()).filter(|v| *v > 0);
    if id.is_none() {
        if let Some(profile_url) = obj.get("_sProfileUrl").and_then(|v| v.as_str()) {
            id = profile_url
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .and_then(|seg| seg.parse::<u64>().ok())
                .filter(|v| *v > 0);
        }
    }
    let name = obj
        .get("_sName")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    (id, name)
}

fn resolve_engine(rec: &GbModRecordRaw, fallback_cat_id: Option<u64>) -> (String, String) {
    let cat_nodes = [
        rec.category.as_ref(),
        rec.super_category.as_ref(),
        rec.root_category.as_ref(),
        rec.sub_category.as_ref(),
    ];

    let mut ids = Vec::with_capacity(5);
    let mut names = Vec::with_capacity(4);
    for node in cat_nodes {
        let (cid, cname) = extract_category_info(node);
        if let Some(id) = cid {
            ids.push(id);
        }
        if let Some(name) = cname {
            names.push(name);
        }
    }
    if let Some(cid) = fallback_cat_id {
        ids.push(cid);
    }

    let first_name = names.first().cloned().unwrap_or_default();

    for cid in ids {
        let mapped = map_category_id_to_engine_id(cid);
        if !mapped.is_empty() {
            return (mapped.to_string(), first_name);
        }
    }

    for name in &names {
        let lower = name.to_ascii_lowercase();
        if lower.contains("v-slice")
            || lower.contains("vslice")
            || lower.contains("base game")
            || lower.contains("basegame")
            || lower.contains("juego base")
            || lower.contains("(base)")
            || lower.contains("base /")
            || lower.contains("/ base")
            || lower.split_whitespace().any(|w| w == "base")
        {
            return ("vslice".to_string(), first_name);
        }
        if lower.contains("psych online") {
            return ("psychonline".to_string(), first_name);
        }
        if lower.contains("psych") {
            return ("psych".to_string(), first_name);
        }
        if lower.contains("codename") {
            return ("codename".to_string(), first_name);
        }
        if lower.contains("p-slice") || lower.contains("pslice") {
            return ("pslice".to_string(), first_name);
        }
        if lower.contains("fps plus") || lower.contains("fpsplus") {
            return ("fpsplus".to_string(), first_name);
        }
        if lower.contains("executable") {
            return ("executable".to_string(), first_name);
        }
    }

    (String::new(), first_name)
}

fn check_is_nsfw(rec: &GbModRecordRaw) -> bool {
    if rec.is_nsfw.unwrap_or(false) || rec.contains_nsfw.unwrap_or(false) {
        return true;
    }
    for node in [
        rec.category.as_ref(),
        rec.super_category.as_ref(),
        rec.root_category.as_ref(),
        rec.sub_category.as_ref(),
    ] {
        if let (Some(id), _) = extract_category_info(node) {
            if id == NSFW_CATEGORY_ID {
                return true;
            }
        }
    }
    false
}

fn is_excluded(rec: &GbModRecordRaw) -> bool {
    for node in [
        rec.category.as_ref(),
        rec.super_category.as_ref(),
        rec.root_category.as_ref(),
        rec.sub_category.as_ref(),
    ] {
        if let (Some(id), _) = extract_category_info(node) {
            if EXCLUDED_CATEGORY_IDS.contains(&id) {
                return true;
            }
        }
    }
    false
}

fn extract_game_id(game: Option<&Value>) -> Option<u64> {
    game.and_then(|v| v.get("_idRow")).and_then(|v| v.as_u64())
}

fn extract_submitter_name(submitter: Option<&Value>) -> Option<String> {
    submitter
        .and_then(|v| v.get("_sName"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
}

fn extract_preview_urls(rec: &GbModRecordRaw, prefer_high_res: bool) -> Vec<String> {
    let images_val = rec
        .preview_media
        .as_ref()
        .and_then(|pm| pm.get("_aImages"))
        .or(rec.images.as_ref());

    let Some(arr) = images_val.and_then(|v| v.as_array()) else {
        return Vec::new();
    };

    let mut urls = Vec::new();
    for item in arr {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let base_url = obj
            .get("_sBaseUrl")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim_end_matches('/');
        if base_url.is_empty() {
            continue;
        }

        let candidates = if prefer_high_res {
            ["_sFile800", "_sFile", "_sFile530", "_sFile220"]
        } else {
            ["_sFile530", "_sFile", "_sFile800", "_sFile220"]
        };

        let file = candidates
            .iter()
            .filter_map(|key| obj.get(*key).and_then(|v| v.as_str()))
            .find(|s| !s.trim().is_empty());

        if let Some(f) = file {
            urls.push(format!("{base_url}/{}", f.trim_start_matches('/')));
        }
    }
    urls
}

fn extract_files(files_val: Option<&Value>) -> Vec<ModFileEntry> {
    let Some(val) = files_val else {
        return Vec::new();
    };

    let raw_entries: Vec<GbFileRaw> = match val {
        Value::Array(arr) => arr
            .iter()
            .filter_map(|item| serde_json::from_value::<GbFileRaw>(item.clone()).ok())
            .collect(),
        Value::Object(map) => map
            .values()
            .filter_map(|item| serde_json::from_value::<GbFileRaw>(item.clone()).ok())
            .collect(),
        _ => Vec::new(),
    };

    raw_entries
        .into_iter()
        .filter_map(|f| {
            let download_url = f.download_url.unwrap_or_default();
            let filename = f.filename.unwrap_or_default();
            if download_url.is_empty() && filename.is_empty() {
                return None;
            }
            Some(ModFileEntry {
                id: f.id.unwrap_or(0),
                filename,
                description: f
                    .description
                    .as_deref()
                    .map(html_to_plain_text)
                    .unwrap_or_default(),
                filesize: f.filesize.unwrap_or(0),
                download_count: f.download_count.unwrap_or(0),
                download_url,
            })
        })
        .collect()
}

fn parse_credit_author(val: &Value) -> Option<String> {
    match val {
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Value::Array(arr) => {
            let name = arr.first().and_then(|v| v.as_str()).map(str::trim).unwrap_or("");
            if name.is_empty() {
                return None;
            }
            let role = arr.get(1).and_then(|v| v.as_str()).map(str::trim).unwrap_or("");
            if role.is_empty() {
                Some(name.to_string())
            } else {
                Some(format!("{name} ({role})"))
            }
        }
        Value::Object(obj) => {
            let name = obj
                .get("_sName")
                .or_else(|| obj.get("name"))
                .or_else(|| obj.get("_sMemberName"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or("");
            if name.is_empty() {
                return None;
            }
            let role = obj
                .get("_sRole")
                .or_else(|| obj.get("role"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or("");
            if role.is_empty() {
                Some(name.to_string())
            } else {
                Some(format!("{name} ({role})"))
            }
        }
        _ => None,
    }
}

fn extract_credits_summary(credits_val: Option<&Value>) -> String {
    let Some(val) = credits_val else {
        return String::new();
    };

    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    let mut default_authors: Vec<String> = Vec::new();

    match val {
        Value::Array(arr) => {
            for item in arr {
                if let Some(obj) = item.as_object() {
                    if obj.contains_key("_sGroupName")
                        || obj.contains_key("groupName")
                        || obj.contains_key("_aAuthors")
                        || obj.contains_key("authors")
                    {
                        let group_name = obj
                            .get("_sGroupName")
                            .or_else(|| obj.get("groupName"))
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .unwrap_or("Contributors")
                            .to_string();

                        let mut authors = Vec::new();
                        if let Some(raw_authors) = obj
                            .get("_aAuthors")
                            .or_else(|| obj.get("authors"))
                            .and_then(|v| v.as_array())
                        {
                            for a in raw_authors {
                                if let Some(formatted) = parse_credit_author(a) {
                                    authors.push(formatted);
                                }
                            }
                        }
                        if !authors.is_empty() {
                            groups.push((group_name, authors));
                        }
                        continue;
                    }
                }
                if let Some(formatted) = parse_credit_author(item) {
                    default_authors.push(formatted);
                }
            }
        }
        Value::Object(map) => {
            for (group_name, group_val) in map {
                let mut authors = Vec::new();
                if let Some(arr) = group_val.as_array() {
                    for a in arr {
                        if let Some(formatted) = parse_credit_author(a) {
                            authors.push(formatted);
                        }
                    }
                } else if let Some(formatted) = parse_credit_author(group_val) {
                    authors.push(formatted);
                }
                if !authors.is_empty() {
                    groups.push((group_name.clone(), authors));
                }
            }
        }
        _ => {}
    }

    if !default_authors.is_empty() {
        groups.push(("Contributors".to_string(), default_authors));
    }

    groups
        .into_iter()
        .map(|(group, authors)| format!("{group}: {}", authors.join(", ")))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Parses a direct GameBanana mod URL or numeric ID.
/// Returns `(mod_id, is_url)`.
fn parse_direct_mod_id(query: &str) -> Option<(u64, bool)> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(id) = trimmed.parse::<u64>() {
        return (id > 0).then_some((id, false));
    }

    let lower = trimmed.to_ascii_lowercase();
    let (rest, is_url) = if let Some(pos) = lower.find("gamebanana.com/mods/") {
        (&trimmed[pos + "gamebanana.com/mods/".len()..], true)
    } else if lower.starts_with("mods/") {
        (&trimmed["mods/".len()..], true)
    } else {
        return None;
    };

    let digits_end = rest
        .bytes()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(rest.len());
    if digits_end == 0 {
        return None;
    }
    let trailing = &rest[digits_end..];
    if !trailing.is_empty()
        && !trailing.starts_with('/')
        && !trailing.starts_with('?')
        && !trailing.starts_with('#')
    {
        return None;
    }

    let id = rest[..digits_end].parse::<u64>().ok()?;
    (id > 0).then_some((id, is_url))
}

fn upgrade_gb_image_url(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let lower = trimmed.to_ascii_lowercase();
    for ext in ["webp", "png", "jpg", "jpeg"] {
        let suffix = format!("_220.{ext}");
        if lower.ends_with(&suffix) {
            let prefix_len = trimmed.len() - suffix.len();
            let orig_ext = &trimmed[trimmed.len() - ext.len()..];
            return format!("{}_800.{orig_ext}", &trimmed[..prefix_len]);
        }
    }
    trimmed.to_string()
}

fn pick_nonzero_i32(primary: Option<i64>, secondary: Option<i64>) -> i32 {
    let val = primary
        .filter(|v| *v > 0)
        .or_else(|| secondary.filter(|v| *v > 0))
        .or(primary)
        .or(secondary)
        .unwrap_or(0);
    val.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

fn clamp_i32(val: Option<i64>) -> i32 {
    val.unwrap_or(0).clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

fn strip_tag_blocks(input: &str, open_prefix: &str, close_tag: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0usize;

    while let Some(rel_start) = lower[cursor..].find(open_prefix) {
        let start = cursor + rel_start;
        out.push_str(&input[cursor..start]);

        let after_prefix = start + open_prefix.len();
        if let Some(rel_close) = lower[after_prefix..].find(close_tag) {
            cursor = after_prefix + rel_close + close_tag.len();
        } else {
            cursor = input.len();
            break;
        }
    }
    if cursor < input.len() {
        out.push_str(&input[cursor..]);
    }
    out
}

fn decode_html_entities(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0usize;

    while let Some(rel_amp) = input[cursor..].find('&') {
        let amp = cursor + rel_amp;
        out.push_str(&input[cursor..amp]);

        if let Some(rel_semi) = input[amp..].find(';') {
            if rel_semi <= 12 {
                let entity = &input[amp + 1..amp + rel_semi];
                if let Some(decoded) = resolve_html_entity(entity) {
                    out.push(decoded);
                    cursor = amp + rel_semi + 1;
                    continue;
                }
            }
        }
        out.push('&');
        cursor = amp + 1;
    }

    if cursor < input.len() {
        out.push_str(&input[cursor..]);
    }
    out
}

fn resolve_html_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" | "#39" => Some('\''),
        "nbsp" => Some(' '),
        "ndash" => Some('–'),
        "mdash" => Some('—'),
        "hellip" => Some('…'),
        _ => {
            if let Some(hex) = entity.strip_prefix("#x").or_else(|| entity.strip_prefix("#X")) {
                u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
            } else if let Some(dec) = entity.strip_prefix('#') {
                dec.parse::<u32>().ok().and_then(char::from_u32)
            } else {
                None
            }
        }
    }
}

fn html_to_plain_text(html: &str) -> String {
    if html.trim().is_empty() {
        return String::new();
    }

    let without_scripts = strip_tag_blocks(html, "<script", "</script>");
    let without_styles = strip_tag_blocks(&without_scripts, "<style", "</style>");

    let mut stripped = String::with_capacity(without_styles.len());
    let lower = without_styles.to_ascii_lowercase();
    let mut cursor = 0usize;

    while let Some(rel_lt) = without_styles[cursor..].find('<') {
        let lt = cursor + rel_lt;
        stripped.push_str(&without_styles[cursor..lt]);

        if let Some(rel_gt) = without_styles[lt..].find('>') {
            let tag_inner = lower[lt + 1..lt + rel_gt].trim();
            let is_break = tag_inner == "br"
                || tag_inner.starts_with("br ")
                || tag_inner.starts_with("br/")
                || tag_inner == "/p"
                || tag_inner == "/div"
                || tag_inner == "/li"
                || tag_inner == "li"
                || tag_inner.starts_with("li ")
                || tag_inner == "/h1"
                || tag_inner == "/h2"
                || tag_inner == "/h3"
                || tag_inner == "/h4"
                || tag_inner == "/h5"
                || tag_inner == "/h6";
            if is_break && !stripped.ends_with('\n') {
                stripped.push('\n');
            }
            cursor = lt + rel_gt + 1;
        } else {
            stripped.push_str(&without_styles[lt..]);
            cursor = without_styles.len();
            break;
        }
    }

    if cursor < without_styles.len() {
        stripped.push_str(&without_styles[cursor..]);
    }

    let decoded = decode_html_entities(&stripped);
    let mut lines_out = Vec::new();
    let mut blank_streak = 0u32;

    for line in decoded.lines() {
        let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.is_empty() {
            blank_streak += 1;
            if blank_streak <= 1 && !lines_out.is_empty() {
                lines_out.push(String::new());
            }
        } else {
            blank_streak = 0;
            lines_out.push(collapsed);
        }
    }

    while lines_out.last().is_some_and(String::is_empty) {
        lines_out.pop();
    }

    lines_out.join("\n")
}
