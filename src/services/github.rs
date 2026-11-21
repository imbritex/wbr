use std::collections::HashMap;
use std::time::{Duration, Instant};

use reqwest::Client;
use serde::Deserialize;
use tokio::sync::RwLock;

const CACHE_TTL: Duration = Duration::from_secs(20 * 60);

#[derive(Debug, Clone)]
pub struct GitHubReleaseAsset {
    pub id: u64,
    pub name: String,
    pub size: u64,
    pub download_url: String,
    pub download_count: u64,
}

#[derive(Debug, Clone)]
pub struct GitHubRelease {
    pub id: u64,
    pub tag_name: String,
    pub name: String,
    pub body: String,
    pub published_at: String,
    pub prerelease: bool,
    pub draft: bool,
    pub html_url: String,
    pub assets: Vec<GitHubReleaseAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawGitHubAsset {
    #[serde(default)]
    id: u64,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    browser_download_url: Option<String>,
    #[serde(default)]
    download_count: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawGitHubRelease {
    #[serde(default)]
    id: u64,
    #[serde(default)]
    tag_name: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    prerelease: Option<bool>,
    #[serde(default)]
    draft: Option<bool>,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    assets: Option<Vec<RawGitHubAsset>>,
}

impl From<RawGitHubRelease> for GitHubRelease {
    fn from(raw: RawGitHubRelease) -> Self {
        let tag_name = raw.tag_name.unwrap_or_default();
        let name = raw
            .name
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| {
                if tag_name.is_empty() {
                    "Release".to_string()
                } else {
                    tag_name.clone()
                }
            });

        let assets = raw
            .assets
            .unwrap_or_default()
            .into_iter()
            .map(|a| GitHubReleaseAsset {
                id: a.id,
                name: a.name.unwrap_or_default(),
                size: a.size.unwrap_or(0),
                download_url: a.browser_download_url.unwrap_or_default(),
                download_count: a.download_count.unwrap_or(0),
            })
            .collect();

        Self {
            id: raw.id,
            tag_name,
            name,
            body: raw.body.unwrap_or_default(),
            published_at: raw.published_at.unwrap_or_default(),
            prerelease: raw.prerelease.unwrap_or(false),
            draft: raw.draft.unwrap_or(false),
            html_url: raw.html_url.unwrap_or_default(),
            assets,
        }
    }
}

pub struct GitHubClient {
    http: Client,
    cache: RwLock<HashMap<String, (Instant, Vec<GitHubRelease>)>>,
}

impl Default for GitHubClient {
    fn default() -> Self {
        Self::new()
    }
}

impl GitHubClient {
    pub fn new() -> Self {
        let http = Client::builder()
            .user_agent("wb-r/3.0.0")
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            http,
            cache: RwLock::new(HashMap::new()),
        }
    }

    pub async fn fetch_engine_releases(
        &self,
        repo: &str,
        force_refresh: bool,
    ) -> anyhow::Result<Vec<GitHubRelease>> {
        let repo = repo.trim();
        if repo.is_empty() || !repo.contains('/') {
            anyhow::bail!("Invalid GitHub repository '{repo}'");
        }

        if !force_refresh {
            let cache = self.cache.read().await;
            if let Some((fetched_at, releases)) = cache.get(repo) {
                if fetched_at.elapsed() < CACHE_TTL {
                    return Ok(releases.clone());
                }
            }
        }

        let url = format!("https://api.github.com/repos/{repo}/releases?per_page=30");
        let fetch_result = self
            .http
            .get(&url)
            .header("Accept", "application/vnd.github.v3+json")
            .send()
            .await;

        let resp = match fetch_result {
            Ok(resp) => resp,
            Err(err) => {
                if let Some(stale) = self.cached_releases(repo).await {
                    tracing::warn!(%repo, %err, "Failed to fetch live GitHub releases; returning cached copy");
                    return Ok(stale);
                }
                return Err(err.into());
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            if status.as_u16() == 403 {
                if let Some(stale) = self.cached_releases(repo).await {
                    tracing::warn!(%repo, "GitHub API rate limit reached; returning cached releases");
                    return Ok(stale);
                }
                anyhow::bail!("GitHub API rate limit reached. Please wait before refreshing.");
            }
            if status.as_u16() == 404 {
                anyhow::bail!("Repository {repo} not found on GitHub.");
            }
            if let Some(stale) = self.cached_releases(repo).await {
                tracing::warn!(%repo, %status, "GitHub API error; returning cached releases");
                return Ok(stale);
            }
            anyhow::bail!("GitHub API error ({status}) for {repo}");
        }

        let raw_releases: Vec<RawGitHubRelease> = match resp.json().await {
            Ok(list) => list,
            Err(err) => {
                if let Some(stale) = self.cached_releases(repo).await {
                    tracing::warn!(%repo, %err, "Failed to parse GitHub releases; returning cached copy");
                    return Ok(stale);
                }
                return Err(err.into());
            }
        };

        let releases: Vec<GitHubRelease> = raw_releases
            .into_iter()
            .filter(|r| !r.draft.unwrap_or(false))
            .map(GitHubRelease::from)
            .collect();

        {
            let mut cache = self.cache.write().await;
            cache.insert(repo.to_string(), (Instant::now(), releases.clone()));
        }

        Ok(releases)
    }

    async fn cached_releases(&self, repo: &str) -> Option<Vec<GitHubRelease>> {
        let cache = self.cache.read().await;
        cache
            .get(repo)
            .filter(|(_, releases)| !releases.is_empty())
            .map(|(_, releases)| releases.clone())
    }
}
