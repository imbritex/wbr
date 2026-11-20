use reqwest::Client;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct GitHubRelease {
    pub tag_name: String,
    pub name: Option<String>,
    pub html_url: String,
}

pub struct GitHubClient {
    http: Client,
}

impl GitHubClient {
    pub fn new() -> Self {
        Self {
            http: Client::builder()
                .user_agent("wb-r/0.1.0")
                .build()
                .unwrap_or_else(|_| Client::new()),
        }
    }

    pub async fn latest_release(&self, owner: &str, repo: &str) -> anyhow::Result<GitHubRelease> {
        let url = format!("https://api.github.com/repos/{owner}/{repo}/releases/latest");
        let release = self.http.get(url).send().await?.error_for_status()?.json().await?;
        Ok(release)
    }
}
