use serde::{Serialize, Deserialize};
use tokio::process::Child;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EffectType {
    EQ,
    LPF,
    HPF,
}

impl EffectType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EffectType::EQ => "EQ",
            EffectType::LPF => "LPF",
            EffectType::HPF => "HPF",
        }
    }
    pub fn filter_type_id(&self) -> &'static str {
        match self {
            EffectType::LPF => "0",
            EffectType::HPF => "1",
            EffectType::EQ => "0",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "EQ" => Some(EffectType::EQ),
            "LPF" => Some(EffectType::LPF),
            "HPF" => Some(EffectType::HPF),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum FilterConfigValue {
    Single(f32),
    Bands(Vec<f32>),
}

#[derive(Debug)]
pub struct FilterInstance {

    pub child: Child,
    pub node_name: String,
}

impl FilterInstance {    
    pub async fn kill(mut self) -> Result<(), std::io::Error> {
        self.child.kill().await?;
        let _ = self.child.wait().await;
        Ok(())
    }
}

#[derive(Debug)]
pub enum BackendError {
    CommandFailed(String),
    IoError(std::io::Error),
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackendError::CommandFailed(msg) => write!(f, "Command failed: {}", msg),
            BackendError::IoError(err) => write!(f, "IO error: {}", err),
        }
    }
}

