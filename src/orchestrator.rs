use anyhow::Result;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::path::Path;
use std::thread;
use tracing::info;
use crate::manifest::AgentManifest;
use crate::llm::LlmEngine;
use crate::sandbox::execute_sandbox;

pub struct MessageBus {
    queues: Mutex<HashMap<String, Vec<String>>>,
}

impl MessageBus {
    pub fn new() -> Self {
        Self {
            queues: Mutex::new(HashMap::new()),
        }
    }

    pub fn send(&self, target: &str, msg: String) {
        let mut lock = self.queues.lock().unwrap();
        lock.entry(target.to_string()).or_default().push(msg);
    }

    pub fn recv(&self, name: &str) -> Option<String> {
        let mut lock = self.queues.lock().unwrap();
        if let Some(queue) = lock.get_mut(name) {
            if !queue.is_empty() {
                return Some(queue.remove(0));
            }
        }
        None
    }
}

pub fn orchestrate<P: AsRef<Path>>(manifest_path: P) -> Result<()> {
    let path_ref = manifest_path.as_ref();
    let manifest = AgentManifest::load_from_file(path_ref)?;
    
    let agents_list = manifest.agents.as_ref()
        .ok_or_else(|| anyhow::anyhow!("No agents list defined in multi-agent fleet manifest"))?;
        
    info!("Starting multi-agent orchestration fleet with {} agents...", agents_list.len());
    
    let llm_engine = Arc::new(LlmEngine::new(&manifest.model)?);
    let bus = Arc::new(MessageBus::new());
    
    let mut handles = Vec::new();
    
    for spec in agents_list {
        let mut agent_manifest = manifest.clone();
        agent_manifest.name = Some(spec.name.clone());
        agent_manifest.goal = Some(spec.goal.clone());
        agent_manifest.tools = Some(spec.tools.clone());
        
        let llm_clone = llm_engine.clone();
        let bus_clone = bus.clone();
        
        let name = spec.name.clone();
        let handle = thread::spawn(move || {
            info!("[Orchestrator] Spawning agent thread: {}", name);
            if let Err(e) = execute_sandbox(agent_manifest, Some(llm_clone), Some(bus_clone)) {
                tracing::error!("Agent {} failed: {:?}", name, e);
            }
            info!("[Orchestrator] Agent thread exited: {}", name);
        });
        handles.push(handle);
    }
    
    for handle in handles {
        let _ = handle.join();
    }
    
    info!("Multi-agent fleet orchestration completed.");
    Ok(())
}
