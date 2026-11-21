use reqwest::Client;
use serde::Deserialize;

pub const FEATURED_URL: &str =
    "https://raw.githubusercontent.com/Crew-Awesome/weekbox.featured/main/public/featured.json";

const GB_MOD_INDEX_URL: &str = "https://gamebanana.com/apiv11/Mod/Index";
const FNF_GAME_ID: u64 = 8694;

#[derive(Debug, Clone, Deserialize)]
pub struct GameBananaModSummary {
    #[serde(rename = "_idRow")]
    pub id: u64,
    #[serde(rename = "_sName")]
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct FeaturedEngineRaw {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct FeaturedModRaw {
    id: u64,
    title: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    image: String,
    #[serde(default)]
    engine: Option<FeaturedEngineRaw>,
}

#[derive(Debug, Clone, Deserialize)]
struct FeaturedRankingRaw {
    label: String,
    #[serde(default)]
    mods: Vec<FeaturedModRaw>,
}

#[derive(Debug, Clone, Deserialize)]
struct FeaturedSchemaRaw {
    #[serde(default)]
    rankings: Vec<FeaturedRankingRaw>,
}

#[derive(Debug, Clone, Deserialize)]
struct GbSubmitterRaw {
    #[serde(rename = "_sName", default)]
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct GbRootCategoryRaw {
    #[serde(rename = "_idRow", default)]
    id: u64,
    #[serde(rename = "_sName", default)]
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct GbPreviewImageRaw {
    #[serde(rename = "_sBaseUrl", default)]
    base_url: String,
    #[serde(rename = "_sFile530", default)]
    file_530: String,
    #[serde(rename = "_sFile", default)]
    file: String,
}

#[derive(Debug, Clone, Deserialize)]
struct GbPreviewMediaRaw {
    #[serde(rename = "_aImages", default)]
    images: Vec<GbPreviewImageRaw>,
}

#[derive(Debug, Clone, Deserialize)]
struct GbModRecordRaw {
    #[serde(rename = "_idRow")]
    id: u64,
    #[serde(rename = "_sName", default)]
    name: String,
    #[serde(rename = "_nLikeCount", default)]
    like_count: i32,
    #[serde(rename = "_aSubmitter", default)]
    submitter: Option<GbSubmitterRaw>,
    #[serde(rename = "_aRootCategory", default)]
    root_category: Option<GbRootCategoryRaw>,
    #[serde(rename = "_aPreviewMedia", default)]
    preview_media: Option<GbPreviewMediaRaw>,
}

#[derive(Debug, Clone, Deserialize)]
struct GbModIndexResponseRaw {
    #[serde(rename = "_aRecords", default)]
    records: Vec<GbModRecordRaw>,
}

#[derive(Debug, Clone)]
pub struct FeaturedMod {
    pub id: u64,
    pub title: String,
    pub author: String,
    pub engine_id: String,
    pub engine: String,
    pub badge: String,
    pub image_url: String,
}

#[derive(Debug, Clone)]
pub struct HomeMod {
    pub id: u64,
    pub title: String,
    pub author: String,
    pub engine_id: String,
    pub engine_name: String,
    pub likes: i32,
    pub image_url: String,
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

pub struct GameBananaClient {
    http: Client,
}

impl GameBananaClient {
    pub fn new() -> Self {
        Self {
            http: Client::builder()
                .user_agent("wb-r/0.1.0")
                .build()
                .unwrap_or_else(|_| Client::new()),
        }
    }

    pub async fn fetch_featured_mods(&self) -> anyhow::Result<Vec<FeaturedMod>> {
        let resp = self.http.get(FEATURED_URL).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("Featured JSON HTTP {}", resp.status());
        }
        let schema: FeaturedSchemaRaw = resp.json().await?;
        let mut out = Vec::new();
        for ranking in schema.rankings {
            let badge = ranking.label;
            for m in ranking.mods {
                let (engine_id, engine) = match m.engine {
                    Some(e) => (e.id, e.name),
                    None => (String::new(), String::new()),
                };
                out.push(FeaturedMod {
                    id: m.id,
                    title: m.title,
                    author: m.author,
                    engine_id,
                    engine,
                    badge: badge.clone(),
                    image_url: m.image,
                });
            }
        }
        Ok(out)
    }

    pub async fn fetch_popular_mods_page(&self, page: u32, per_page: u32) -> anyhow::Result<Vec<HomeMod>> {
        let url = format!(
            "{GB_MOD_INDEX_URL}?_aFilters[Generic_Game]={FNF_GAME_ID}&_sSort=Generic_MostLiked&_nPerpage={per_page}&_nPage={page}"
        );
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("GameBanana Mod/Index HTTP {}", resp.status());
        }
        let parsed: GbModIndexResponseRaw = resp.json().await?;
        let mut out = Vec::with_capacity(parsed.records.len());
        for rec in parsed.records {
            let author = rec.submitter.map(|s| s.name).unwrap_or_default();
            let (cat_id, engine_name) = rec
                .root_category
                .map(|c| (c.id, c.name))
                .unwrap_or((0, String::new()));
            let image_url = rec
                .preview_media
                .and_then(|pm| pm.images.into_iter().next())
                .map(|img| {
                    let file = if !img.file_530.is_empty() {
                        img.file_530
                    } else {
                        img.file
                    };
                    if img.base_url.is_empty() || file.is_empty() {
                        String::new()
                    } else {
                        format!("{}/{}", img.base_url.trim_end_matches('/'), file)
                    }
                })
                .unwrap_or_default();

            out.push(HomeMod {
                id: rec.id,
                title: rec.name,
                author,
                engine_id: map_category_id_to_engine_id(cat_id).to_string(),
                engine_name,
                likes: rec.like_count,
                image_url,
            });
        }
        Ok(out)
    }

    pub async fn fetch_cover_rgba(&self, url: &str) -> Option<(u32, u32, Vec<u8>)> {
        if url.is_empty() {
            return None;
        }
        let resp = self.http.get(url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let bytes = resp.bytes().await.ok()?;
        let mut img = image::load_from_memory(&bytes).ok()?;
        if img.width() > 720 {
            img = img.resize(720, 405, image::imageops::FilterType::Triangle);
        }
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        Some((w, h, rgba.into_raw()))
    }

    pub async fn search_mods(&self, query: &str) -> anyhow::Result<Vec<GameBananaModSummary>> {
        let _ = (&self.http, query);
        // TODO: Conectar con API v11 de GameBanana (https://gamebanana.com/apiv11/...)
        Ok(Vec::new())
    }
}
