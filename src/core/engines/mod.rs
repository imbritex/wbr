use serde::{Deserialize, Serialize};
use sysinfo::System;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EngineKind {
    Psych,
    Codename,
    VSlice,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineInstance {
    pub id: String,
    pub name: String,
    pub kind: EngineKind,
    pub version: String,
    pub executable_name: String,
    pub installed: bool,
    pub playtime_minutes: u64,
}

pub struct EngineRegistry {
    instances: Vec<EngineInstance>,
}

impl EngineRegistry {
    pub fn with_defaults() -> Self {
        Self {
            instances: vec![
                EngineInstance {
                    id: "psych".into(),
                    name: "Psych Engine".into(),
                    kind: EngineKind::Psych,
                    version: "0.7.3".into(),
                    executable_name: "PsychEngine".into(),
                    installed: false,
                    playtime_minutes: 0,
                },
                EngineInstance {
                    id: "codename".into(),
                    name: "Codename Engine".into(),
                    kind: EngineKind::Codename,
                    version: "1.0.0".into(),
                    executable_name: "CodenameEngine".into(),
                    installed: false,
                    playtime_minutes: 0,
                },
                EngineInstance {
                    id: "vslice".into(),
                    name: "Funkin' (V-Slice)".into(),
                    kind: EngineKind::VSlice,
                    version: "0.5.x".into(),
                    executable_name: "Funkin".into(),
                    installed: false,
                    playtime_minutes: 0,
                },
            ],
        }
    }

    pub fn list(&self) -> &[EngineInstance] {
        &self.instances
    }

    pub fn detect_running_engines(&self) -> Vec<String> {
        let mut sys = System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

        let mut active = Vec::new();
        for process in sys.processes().values() {
            let proc_name = process.name().to_string_lossy().to_lowercase();
            for engine in &self.instances {
                if proc_name.contains(&engine.executable_name.to_lowercase()) {
                    active.push(engine.id.clone());
                }
            }
        }
        active
    }
}
