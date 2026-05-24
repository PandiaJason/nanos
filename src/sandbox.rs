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
        llm: Some(LlmEngine::new(&manifest.model.path)?),
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
        
        let response = if path == "/docs" {
            "The system requires memory isolation using WASM and an LLM running natively via llama.cpp for true zero-latency AI agents."
        } else {
            "File not found."
        };
        
        let resp_bytes = response.as_bytes();
        let len_to_copy = std::cmp::min(resp_bytes.len(), out_max as usize);
        
        memory.write(&mut caller, out_ptr as usize, &resp_bytes[..len_to_copy]).unwrap();
        
        len_to_copy as i32
    })?;
    
    linker.func_wrap("env", "llm_infer", |mut caller: Caller<'_, AgentState>, prompt_ptr: i32, prompt_len: i32, out_ptr: i32, out_max: i32| -> i32 {
        let memory = caller.get_export("memory").unwrap().into_memory().unwrap();
        
        let mut prompt_bytes = vec![0u8; prompt_len as usize];
        memory.read(&caller, prompt_ptr as usize, &mut prompt_bytes).unwrap();
        let prompt = String::from_utf8_lossy(&prompt_bytes).to_string();
        
        let state = caller.data_mut();
        let response = if let Some(llm) = &mut state.llm {
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
