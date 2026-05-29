use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AgentSpec {
    pub name: String,
    pub goal: String,
    pub tools: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AgentManifest {
    pub name: Option<String>,
    pub model: ModelConfig,
    pub resources: ResourceLimits,
    pub permissions: Permissions,
    pub tools: Option<Vec<String>>,
    pub goal: Option<String>,
    pub mcp_servers: Option<Vec<McpServerConfig>>,
    pub agents: Option<Vec<AgentSpec>>,
    pub binary: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ModelConfig {
    pub path: Option<String>,
    pub context_window: u32,
    pub provider: Option<String>,
    pub api_url: Option<String>,
    pub api_key: Option<String>,
    pub model_name: Option<String>,
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
    #[serde(default)]
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

impl ResourceLimits {
    /// Parse memory string like "256MB", "1GB", "512KB" into bytes.
    pub fn memory_bytes(&self) -> usize {
        let s = self.memory.trim().to_uppercase();
        if let Some(val) = s.strip_suffix("GB") {
            val.trim().parse::<usize>().unwrap_or(256) * 1024 * 1024 * 1024
        } else if let Some(val) = s.strip_suffix("MB") {
            val.trim().parse::<usize>().unwrap_or(256) * 1024 * 1024
        } else if let Some(val) = s.strip_suffix("KB") {
            val.trim().parse::<usize>().unwrap_or(256) * 1024
        } else {
            // Default to treating as MB
            s.parse::<usize>().unwrap_or(256) * 1024 * 1024
        }
    }

    /// Convert max_steps into wasmtime fuel units.
    /// Each agent step ~= 100_000 fuel units (covers syscalls + memory ops).
    pub fn fuel_budget(&self) -> u64 {
        (self.max_steps as u64) * 100_000
    }
}
