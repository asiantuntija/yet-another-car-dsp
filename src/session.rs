use serde::{Serialize, Deserialize};
use std::fs;
use std::collections::HashMap;
use log::{error, info};

#[derive(Serialize, Deserialize, Default, Clone, PartialEq)]
pub struct EffectSettings {
    pub active: bool,
    #[serde(default)]
    pub value: f32,
    #[serde(default)]
    pub bands: Vec<f32>,
}

#[derive(Serialize, Deserialize, Default, Clone,PartialEq)]
pub struct DeviceSettings {
    pub active: bool,
    pub name: String,
    #[serde(default = "default_volume")]
    pub volume: f32,
    #[serde(default)]
    pub eq: EffectSettings,
    #[serde(default)]
    pub lpf: EffectSettings,
    #[serde(default)]
    pub hpf: EffectSettings,
    #[serde(default = "default_effect_order")]
    pub effect_order: Vec<crate::effects::EffectType>,
}

fn default_volume() -> f32 {
    0.5
}

fn default_effect_order() -> Vec<crate::effects::EffectType> {
    vec![crate::effects::EffectType::EQ, crate::effects::EffectType::LPF, crate::effects::EffectType::HPF]
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct SessionData {
    pub devices: HashMap<String, DeviceSettings>,
}

pub struct SessionManager {
    path: String,
}

impl SessionManager {
    pub fn new(path: &str) -> Self {
        Self { path: path.to_string() }
    }

    pub fn load(&self) -> SessionData {
        info!("Loading session from {}", self.path);
        match fs::read_to_string(&self.path) {
            Ok(content) => {
                serde_json::from_str(&content).unwrap_or_else(|e| {
                    error!("Failed to parse session file at {}: {}", self.path, e);
                    SessionData::default()
                })
            }
            Err(e) => {
                info!("No session file found at {}: {}. Using defaults.", self.path, e);
                SessionData::default()
            }
        }
    }

    pub fn save(&self, data: &SessionData) {
        info!("Saving session to {}", self.path);
        match serde_json::to_string_pretty(data) {
            Ok(json) => {
                if let Err(e) = fs::write(&self.path, json) {
                    error!("Failed to write session file to {}: {}", self.path, e);
                }
            }
            Err(e) => error!("Failed to serialize session data to JSON: {}", e),
        }
    }
}

