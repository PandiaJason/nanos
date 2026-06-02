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
    pub token: Option<String>,
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

        serde_yaml::from_str(&yaml_content).context("Failed to parse agent manifest YAML syntax")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_agent_manifest() {
        let yaml = r#"
name: test-agent
model:
  path: models/test.gguf
  context_window: 2048
resources:
  memory: "512MB"
  max_steps: 20
permissions:
  network: false
"#;
        let manifest: AgentManifest = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(manifest.name.as_deref(), Some("test-agent"));
        assert_eq!(manifest.model.context_window, 2048);
        assert_eq!(manifest.resources.memory, "512MB");
        assert_eq!(manifest.resources.max_steps, 20);
        assert!(!manifest.permissions.network);
    }

    #[test]
    fn parse_fleet_manifest_with_agents() {
        let yaml = r#"
model:
  context_window: 4096
resources:
  memory: "1GB"
  max_steps: 50
permissions:
  network: true
agents:
  - name: reader
    goal: "Read files"
    tools: ["fs_read"]
  - name: writer
    goal: "Write files"
    tools: ["fs_write"]
"#;
        let manifest: AgentManifest = serde_yaml::from_str(yaml).unwrap();
        let agents = manifest.agents.unwrap();
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].name, "reader");
        assert_eq!(agents[1].tools, vec!["fs_write"]);
    }

    #[test]
    fn memory_bytes_gb() {
        let rl = ResourceLimits {
            memory: "2GB".to_string(),
            max_steps: 1,
        };
        assert_eq!(rl.memory_bytes(), 2 * 1024 * 1024 * 1024);
    }

    #[test]
    fn memory_bytes_mb() {
        let rl = ResourceLimits {
            memory: "256MB".to_string(),
            max_steps: 1,
        };
        assert_eq!(rl.memory_bytes(), 256 * 1024 * 1024);
    }

    #[test]
    fn memory_bytes_kb() {
        let rl = ResourceLimits {
            memory: "512KB".to_string(),
            max_steps: 1,
        };
        assert_eq!(rl.memory_bytes(), 512 * 1024);
    }

    #[test]
    fn memory_bytes_default_treated_as_mb() {
        let rl = ResourceLimits {
            memory: "128".to_string(),
            max_steps: 1,
        };
        assert_eq!(rl.memory_bytes(), 128 * 1024 * 1024);
    }

    #[test]
    fn fuel_budget_calculation() {
        let rl = ResourceLimits {
            memory: "256MB".to_string(),
            max_steps: 10,
        };
        assert_eq!(rl.fuel_budget(), 1_000_000);

        let rl2 = ResourceLimits {
            memory: "256MB".to_string(),
            max_steps: 0,
        };
        assert_eq!(rl2.fuel_budget(), 0);
    }

    #[test]
    fn missing_required_fields_fails() {
        // Missing model entirely
        let yaml = r#"
resources:
  memory: "256MB"
  max_steps: 10
permissions:
  network: false
"#;
        let result = serde_yaml::from_str::<AgentManifest>(yaml);
        assert!(result.is_err());
    }
}
