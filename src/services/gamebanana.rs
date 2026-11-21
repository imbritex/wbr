use reqwest::Client;
use serde::Deserialize;

pub const FEATURED_URL: &str =
    "https://raw.githubusercontent.com/Crew-Awesome/weekbox.featured/main/public/featured.json";

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

#[derive(Debug, Clone)]
pub struct FeaturedMod {
    pub id: u64,
    pub title: String,
    pub author: String,
    pub engine: String,
    pub badge: String,
    pub image_url: String,
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
                out.push(FeaturedMod {
                    id: m.id,
                    title: m.title,
                    author: m.author,
                    engine: m.engine.map(|e| e.name).unwrap_or_default(),
                    badge: badge.clone(),
                    image_url: m.image,
                });
            }
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
        if img.width() > 960 {
            img = img.resize(960, 540, image::imageops::FilterType::Triangle);
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
