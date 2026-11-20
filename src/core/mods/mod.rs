use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModEntry {
    pub id: String,
    pub title: String,
    pub author: String,
    pub engine: String,
}

pub struct ModManager;

impl ModManager {
    pub fn new() -> Self {
        Self
    }

    pub fn sample_entries(&self) -> Vec<ModEntry> {
        vec![
            ModEntry {
                id: "sample-1".into(),
                title: "Biblioteca vacía".into(),
                author: "WeekBox Rust".into(),
                engine: "Psych Engine".into(),
            },
        ]
    }
}
