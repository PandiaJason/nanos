use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AgentManifest {
    pub name: String,
    pub model: ModelConfig,
    pub resources: ResourceLimits,
    pub permissions: Permissions,
    pub tools: Vec<String>,
    pub goal: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ModelConfig {
    pub path: String,
    pub context_window: u32,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ResourceLimits {
    pub memory: String,
    pub max_steps: u32,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Permissions {
    pub fs_read: Option<Vec<String>>,
    pub fs_write: Option<Vec<String>>,
    pub network: bool,
}

impl AgentManifest {
    /// Load and parse the manifest from a given file path.
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_ref = path.as_ref();
        let yaml_content = fs::read_to_string(path_ref)
            .with_context(|| format!("Failed to read manifest file at {:?}", path_ref))?;
        
        serde_yaml::from_str(&yaml_content)
            .context("Failed to parse agent manifest YAML syntax")
    }
}
