use anyhow::{Context, Result};
use tracing::{info, debug};
use wasmtime::*;

use crate::manifest::AgentManifest;
use crate::llm::LlmEngine;

/// State injected into the WASM Sandbox.
pub struct AgentState {
    pub manifest: AgentManifest,
    pub llm: Option<LlmEngine>,
}

/// Initializes the WebAssembly sandbox, binds the MCP host functions, and executes the agent.
pub fn execute_sandbox(manifest: AgentManifest) -> Result<()> {
    info!("Configuring Wasmtime Engine...");
    
    // In production, we configure limits (memory, fuel) on the Config object here.
    let mut config = Config::new();
    config.wasm_component_model(false); // We are using core wasm for now
    
    let engine = Engine::new(&config)?;
    
    let mut linker = Linker::new(&engine);
    
    // Bind native MCP Tools as WASM Host Functions
    bind_mcp_syscalls(&mut linker)?;
    
    let mut store = Store::new(&engine, AgentState {
        manifest: manifest.clone(),
        llm: Some(LlmEngine::new(&manifest.model.path, manifest.model.context_window)?),
    });
    
    info!("Sandbox configured and ready.");
    
    // Load and execute the pre-compiled WASM module
    let wasm_path = "nanos-core-agent/target/wasm32-unknown-unknown/debug/nanos_core_agent.wasm";
    debug!("Loading WASM module from: {}", wasm_path);
    let module = Module::from_file(&engine, wasm_path)?;
        
    let instance = linker.instantiate(&mut store, &module)?;
    
    let run_agent = instance.get_typed_func::<(), ()>(&mut store, "run_agent")?;
    
    info!("Booting Agent Loop inside Sandbox...");
    run_agent.call(&mut store, ())?;
    
    info!("Agent nano-process died cleanly.");
    
    Ok(())
}

/// Exposes Native tools to the WebAssembly module via FFI.
fn bind_mcp_syscalls(linker: &mut Linker<AgentState>) -> Result<()> {
    linker.func_wrap("env", "fs_read", |mut caller: Caller<'_, AgentState>, ptr: i32, len: i32, out_ptr: i32, out_max: i32| -> i32 {
        let memory = caller.get_export("memory").unwrap().into_memory().unwrap();
        
        let mut path_bytes = vec![0u8; len as usize];
        memory.read(&caller, ptr as usize, &mut path_bytes).unwrap();
        let path = String::from_utf8_lossy(&path_bytes).to_string();
        
        info!("[MCP Syscall] 'fs_read' invoked for path: {}", path);
        
        let manifest = &caller.data().manifest;
        let mut allowed = false;
        if let Some(fs_read_rules) = &manifest.permissions.fs_read {
            for rule in fs_read_rules {
                if rule.ends_with("**") {
                    let prefix = &rule[..rule.len()-2];
                    if path.starts_with(prefix) { allowed = true; break; }
                } else if rule.ends_with("*") {
                    let prefix = &rule[..rule.len()-1];
                    if path.starts_with(prefix) { allowed = true; break; }
                } else if &path == rule {
                    allowed = true;
                    break;
                }
            }
        }

        let response = if !allowed {
            info!("[Security] 'fs_read' blocked path: {}", path);
            String::from("[Security] PERMISSION_DENIED")
        } else {
            if path == "/docs" {
                String::from("The system requires memory isolation using WASM and an LLM running natively via llama.cpp for true zero-latency AI agents.")
            } else {
                String::from("File not found.")
            }
        };
        
        let resp_bytes = response.as_bytes();
        let len_to_copy = std::cmp::min(resp_bytes.len(), out_max as usize);
        
        memory.write(&mut caller, out_ptr as usize, &resp_bytes[..len_to_copy]).unwrap();
        
        len_to_copy as i32
    })?;
    
    linker.func_wrap("env", "web_get", |mut caller: Caller<'_, AgentState>, ptr: i32, len: i32, out_ptr: i32, out_max: i32| -> i32 {
        let memory = caller.get_export("memory").unwrap().into_memory().unwrap();
        
        let mut url_bytes = vec![0u8; len as usize];
        memory.read(&caller, ptr as usize, &mut url_bytes).unwrap();
        let url = String::from_utf8_lossy(&url_bytes).to_string();
        
        info!("[MCP Syscall] 'web_get' invoked for url: {}", url);
        
        let manifest = &caller.data().manifest;
        
        let response_text = if !manifest.permissions.network {
            info!("[Security] 'web_get' blocked network access for url: {}", url);
            String::from("[Security] NETWORK_DISABLED_IN_MANIFEST")
        } else {
            match ureq::get(&url).call() {
                Ok(res) => res.into_string().unwrap_or_else(|_| "Failed to decode response".to_string()),
                Err(e) => format!("HTTP Request failed: {}", e),
            }
        };
        
        let resp_bytes = response_text.as_bytes();
        let len_to_copy = std::cmp::min(resp_bytes.len(), out_max as usize);
        
        memory.write(&mut caller, out_ptr as usize, &resp_bytes[..len_to_copy]).unwrap();
        
        len_to_copy as i32
    })?;
    
    linker.func_wrap("env", "memory_store", |mut caller: Caller<'_, AgentState>, ptr: i32, len: i32| -> i32 {
        let memory = caller.get_export("memory").unwrap().into_memory().unwrap();
        
        let mut content_bytes = vec![0u8; len as usize];
        memory.read(&caller, ptr as usize, &mut content_bytes).unwrap();
        let content = String::from_utf8_lossy(&content_bytes).to_string();
        
        info!("[MCP Syscall] 'memory_store' saving: {}", content);
        
        if let Ok(conn) = rusqlite::Connection::open("nanos_memory.db") {
            conn.execute("CREATE TABLE IF NOT EXISTS memories (id INTEGER PRIMARY KEY, content TEXT)", []).unwrap();
            conn.execute("INSERT INTO memories (content) VALUES (?1)", rusqlite::params![content]).unwrap();
            1 // Success
        } else {
            0 // Failure
        }
    })?;
    
    linker.func_wrap("env", "memory_recall", |mut caller: Caller<'_, AgentState>, ptr: i32, len: i32, out_ptr: i32, out_max: i32| -> i32 {
        let memory = caller.get_export("memory").unwrap().into_memory().unwrap();
        
        let mut query_bytes = vec![0u8; len as usize];
        memory.read(&caller, ptr as usize, &mut query_bytes).unwrap();
        let query = String::from_utf8_lossy(&query_bytes).to_string();
        
        info!("[MCP Syscall] 'memory_recall' searching for: {}", query);
        
        let mut result = String::from("No memory found.");
        
        if let Ok(conn) = rusqlite::Connection::open("nanos_memory.db") {
            let sql = "SELECT content FROM memories WHERE content LIKE ?1 LIMIT 1";
            let like_query = format!("%{}%", query);
            let mut stmt = conn.prepare(sql).unwrap();
            if let Ok(mut rows) = stmt.query(rusqlite::params![like_query]) {
                if let Ok(Some(row)) = rows.next() {
                    let content: String = row.get(0).unwrap();
                    result = content;
                }
            }
        }
        
        let resp_bytes = result.as_bytes();
        let len_to_copy = std::cmp::min(resp_bytes.len(), out_max as usize);
        
        memory.write(&mut caller, out_ptr as usize, &resp_bytes[..len_to_copy]).unwrap();
        
        len_to_copy as i32
    })?;
    
    linker.func_wrap("env", "llm_infer", |mut caller: Caller<'_, AgentState>, prompt_ptr: i32, prompt_len: i32, out_ptr: i32, out_max: i32| -> i32 {
        let memory = caller.get_export("memory").unwrap().into_memory().unwrap();
        
        let mut prompt_bytes = vec![0u8; prompt_len as usize];
        memory.read(&caller, prompt_ptr as usize, &mut prompt_bytes).unwrap();
        let prompt = String::from_utf8_lossy(&prompt_bytes).to_string();
        
        let state = caller.data();
        let response = if let Some(llm) = &state.llm {
            llm.infer(&prompt).unwrap_or_else(|_| "LLM Inference Error".to_string())
        } else {
            "LLM Engine not initialized".to_string()
        };
        
        let resp_bytes = response.as_bytes();
        let len_to_copy = std::cmp::min(resp_bytes.len(), out_max as usize);
        
        memory.write(&mut caller, out_ptr as usize, &resp_bytes[..len_to_copy]).unwrap();
        
        len_to_copy as i32
    })?;
    
    Ok(())
}
