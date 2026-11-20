use reqwest::Client;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct GameBananaModSummary {
    #[serde(rename = "_idRow")]
    pub id: u64,
    #[serde(rename = "_sName")]
    pub name: String,
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

    pub async fn search_mods(&self, query: &str) -> anyhow::Result<Vec<GameBananaModSummary>> {
        let _ = (&self.http, query);
        // TODO: Conectar con API v11 de GameBanana (https://gamebanana.com/apiv11/...)
        Ok(Vec::new())
    }
}
