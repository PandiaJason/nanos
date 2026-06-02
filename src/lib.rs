//! # nanos — The AI-Native WASM Micro-Runtime
//!
//! Embed sandboxed AI agents directly in your Rust application.
//!
//! ## Quick Start
//! ```rust,no_run
//! use nanos::{nanos_spawn, NanosConfig};
//!
//! let mut handle = nanos_spawn("agent.nano").expect("failed to spawn agent");
//! handle.wait().expect("agent failed");
//! println!("Traces: {:?}", handle.traces());
//! ```

pub mod dashboard;
pub mod llm;
pub mod manifest;
pub mod mcp_client;
pub mod network;
pub mod orchestrator;
pub mod sandbox;
pub mod trace;

use anyhow::Result;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

/// Configuration overrides for spawning an agent.
pub struct NanosConfig {
    /// Override the LLM engine (share across agents).
    pub shared_llm: Option<Arc<llm::LlmEngine>>,
    /// Override the message bus (for inter-agent communication).
    pub shared_bus: Option<Arc<orchestrator::MessageBus>>,
}

impl Default for NanosConfig {
    fn default() -> Self {
        Self {
            shared_llm: None,
            shared_bus: None,
        }
    }
}

/// Handle to a running nanos agent. Allows waiting, killing, inspecting traces,
/// and sending inter-agent messages.
pub struct NanosHandle {
    name: String,
    handle: Option<JoinHandle<Result<()>>>,
    bus: Option<Arc<orchestrator::MessageBus>>,
    traces: Arc<Mutex<Vec<trace::AgentTrace>>>,
    killed: Arc<std::sync::atomic::AtomicBool>,
}

impl NanosHandle {
    /// Block until the agent finishes execution.
    pub fn wait(&mut self) -> Result<()> {
        if let Some(h) = self.handle.take() {
            h.join()
                .map_err(|_| anyhow::anyhow!("Agent thread panicked"))??;
        }
        Ok(())
    }

    /// Check if the agent has finished.
    pub fn is_finished(&self) -> bool {
        self.handle.as_ref().map_or(true, |h| h.is_finished())
    }

    /// Kill the running agent.
    pub fn kill(&self) {
        self.killed
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Get a snapshot of all recorded execution traces.
    pub fn traces(&self) -> Vec<trace::AgentTrace> {
        self.traces.lock().unwrap().clone()
    }

    /// Send a message to this agent via the message bus.
    pub fn send_message(&self, msg: &str) -> Result<()> {
        if let Some(bus) = &self.bus {
            bus.send(&self.name, msg.to_string());
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "No message bus configured for agent '{}'",
                self.name
            ))
        }
    }

    /// Get the agent's name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Spawn a single sandboxed agent from a manifest file.
///
/// # Example
/// ```rust,no_run
/// let mut handle = nanos::nanos_spawn("agent.nano").unwrap();
/// handle.wait().unwrap();
/// ```
pub fn nanos_spawn(manifest_path: &str) -> Result<NanosHandle> {
    nanos_spawn_with_config(manifest_path, NanosConfig::default())
}

/// Spawn a single sandboxed agent with custom configuration.
pub fn nanos_spawn_with_config(manifest_path: &str, config: NanosConfig) -> Result<NanosHandle> {
    let manifest = manifest::AgentManifest::load_from_file(manifest_path)?;
    let name = manifest
        .name
        .clone()
        .unwrap_or_else(|| "nanos-agent".to_string());
    let killed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let traces = Arc::new(Mutex::new(Vec::new()));
    let bus = config.shared_bus.clone();

    let llm_engine = config.shared_llm;
    let bus_clone = config.shared_bus;
    let manifest_clone = manifest;

    let handle = thread::spawn(move || -> Result<()> {
        sandbox::execute_sandbox(manifest_clone, llm_engine, bus_clone).map(|_| ())
    });

    Ok(NanosHandle {
        name,
        handle: Some(handle),
        bus,
        traces,
        killed,
    })
}

/// Spawn an entire multi-agent fleet from a fleet manifest file.
///
/// # Example
/// ```rust,no_run
/// let mut handles = nanos::nanos_spawn_fleet("fleet.nano").unwrap();
/// for h in &mut handles {
///     h.wait().unwrap();
/// }
/// ```
pub fn nanos_spawn_fleet(manifest_path: &str) -> Result<Vec<NanosHandle>> {
    let manifest = manifest::AgentManifest::load_from_file(manifest_path)?;
    let agents_list = manifest
        .agents
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No agents list in fleet manifest"))?;

    let llm_engine = Arc::new(llm::LlmEngine::new(&manifest.model)?);
    let bus = Arc::new(orchestrator::MessageBus::new());

    let mut handles = Vec::new();

    for spec in agents_list {
        let mut agent_manifest = manifest.clone();
        agent_manifest.name = Some(spec.name.clone());
        agent_manifest.goal = Some(spec.goal.clone());
        agent_manifest.tools = Some(spec.tools.clone());

        let name = spec.name.clone();
        let llm_clone = llm_engine.clone();
        let bus_clone = bus.clone();
        let killed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let traces = Arc::new(Mutex::new(Vec::new()));

        let thread_handle = thread::spawn(move || -> Result<()> {
            sandbox::execute_sandbox(agent_manifest, Some(llm_clone), Some(bus_clone)).map(|_| ())
        });

        handles.push(NanosHandle {
            name,
            handle: Some(thread_handle),
            bus: Some(bus.clone()),
            traces,
            killed,
        });
    }

    Ok(handles)
}
