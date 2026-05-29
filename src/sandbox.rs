use anyhow::Result;
use tracing::{info, debug, error};
use wasmtime::*;
use serde_json::{json, Value};
use std::process::{Command, Stdio};
use std::io::{BufRead, BufReader, Write};

use crate::manifest::AgentManifest;
use crate::llm::LlmEngine;

/// State injected into the WASM Sandbox.
pub struct AgentState {
    pub name: String,
    pub manifest: AgentManifest,
    pub llm: Option<std::sync::Arc<LlmEngine>>,
    pub traces: Vec<crate::trace::AgentTrace>,
    pub mcp_clients: Vec<crate::mcp_client::McpClient>,
    pub bus: Option<std::sync::Arc<crate::orchestrator::MessageBus>>,
    pub limiter: StoreLimits,
}

impl AgentState {
    pub fn record_trace(&mut self, trace: crate::trace::AgentTrace) {
        crate::dashboard::update_agent(&self.name, "RUNNING", trace.step, 2048);
        crate::dashboard::log_event(format!("[Agent {}] FFI: {} -> {}", self.name, trace.action, trace.result));
        crate::dashboard::add_trace(trace.clone());
        self.traces.push(trace);
    }
}

/// Initializes the WebAssembly sandbox, binds the MCP host functions, and executes the agent.
pub fn execute_sandbox(
    manifest: AgentManifest,
    llm: Option<std::sync::Arc<LlmEngine>>,
    bus: Option<std::sync::Arc<crate::orchestrator::MessageBus>>,
) -> Result<()> {
    info!("Configuring Wasmtime Engine...");
    
    let mut config = Config::new();
    config.wasm_component_model(false);
    config.consume_fuel(true); // Enable fuel-based execution metering
    
    let engine = Engine::new(&config)?;
    
    let mut linker = Linker::new(&engine);
    
    // Spawn MCP Servers specified in manifest
    let mut mcp_clients = Vec::new();
    if let Some(servers) = &manifest.mcp_servers {
        for server in servers {
            match crate::mcp_client::McpClient::spawn(&server.name, &server.command, &server.args) {
                Ok(client) => mcp_clients.push(client),
                Err(e) => error!("Failed to spawn MCP server '{}': {:?}", server.name, e),
            }
        }
    }
    
    // Bind native MCP Tools as WASM Host Functions
    bind_mcp_syscalls(&mut linker)?;
    
    let agent_name = manifest.name.clone().unwrap_or_else(|| "nanos-agent".to_string());
    let llm_engine = match llm {
        Some(e) => e,
        None => std::sync::Arc::new(LlmEngine::new(&manifest.model)?),
    };
    
    // Parse resource limits from manifest
    let memory_limit = manifest.resources.memory_bytes();
    let fuel_budget = manifest.resources.fuel_budget();
    info!(
        "Resource limits: memory={} bytes ({}) | fuel={} (max_steps={})",
        memory_limit, manifest.resources.memory, fuel_budget, manifest.resources.max_steps
    );
    
    // Build store with resource limiter
    let limiter = StoreLimitsBuilder::new()
        .memory_size(memory_limit)
        .table_elements(10_000)
        .instances(1)
        .memories(1)
        .build();
    
    let mut store = Store::new(&engine, AgentState {
        name: agent_name,
        manifest: manifest.clone(),
        llm: Some(llm_engine.clone()),
        traces: Vec::new(),
        mcp_clients,
        bus,
        limiter,
    });
    
    // Activate the resource limiter on this store
    store.limiter(|state| &mut state.limiter);
    
    // Add fuel budget — agent execution will trap when fuel runs out
    store.set_fuel(fuel_budget)?;
    info!("Fuel budget of {} units loaded into sandbox.", fuel_budget);
    
    info!("Sandbox configured and ready.");
    
    // Load and execute the pre-compiled WASM module
    let default_wasm = "nanos-core-agent/target/wasm32-unknown-unknown/debug/nanos_core_agent.wasm".to_string();
    let wasm_path = manifest.binary.as_ref().unwrap_or(&default_wasm);
    debug!("Loading WASM module from: {}", wasm_path);
    
    // Read raw bytes to inspect for the JS bundle custom section
    let wasm_bytes = std::fs::read(wasm_path)?;
    if let Some(pos) = wasm_bytes.windows(15).position(|w| w == b"nanos_js_bundle") {
        let js_bytes = &wasm_bytes[pos + 15..];
        let js_code = String::from_utf8_lossy(js_bytes).to_string();
        info!("Found packaged JavaScript agent bundle inside custom WASM section. Spawning Node.js FFI bridge...");
        
        let mcp_clients = store.data_mut().mcp_clients.drain(..).collect::<Vec<_>>();
        let bus = store.data().bus.clone();
        
        run_js_agent(
            &js_code, 
            &manifest, 
            Some(llm_engine),
            mcp_clients,
            bus
        )?;
        return Ok(());
    }
    
    let module = Module::from_binary(&engine, &wasm_bytes)?;
        
    let instance = linker.instantiate(&mut store, &module)?;
    
    let run_agent = instance.get_typed_func::<(), ()>(&mut store, "run_agent")?;
    
    info!("Booting Agent Loop inside Sandbox...");
    match run_agent.call(&mut store, ()) {
        Ok(()) => info!("Agent nano-process died cleanly."),
        Err(e) => {
            // Check if this was a fuel exhaustion trap
            let remaining_fuel = store.get_fuel().unwrap_or(0);
            if remaining_fuel == 0 {
                info!("Agent terminated: fuel budget exhausted (max_steps={} reached).", manifest.resources.max_steps);
            } else {
                return Err(e.into());
            }
        }
    }
    
    // Report remaining fuel
    if let Ok(remaining) = store.get_fuel() {
        let used = fuel_budget.saturating_sub(remaining);
        info!("Fuel consumed: {} / {} ({:.1}%)", used, fuel_budget, (used as f64 / fuel_budget as f64) * 100.0);
    }
    
    // Print the trace table showing the execution details of the agent
    crate::trace::print_trace_table(&store.data().traces);
    
    Ok(())
}

/// Exposes Native tools to the WebAssembly module via FFI.
fn bind_mcp_syscalls(linker: &mut Linker<AgentState>) -> Result<()> {
    linker.func_wrap("env", "fs_read", |mut caller: Caller<'_, AgentState>, ptr: i32, len: i32, out_ptr: i32, out_max: i32| -> i32 {
        let start_time = std::time::Instant::now();
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
            match std::fs::read_to_string(&path) {
                Ok(contents) => contents,
                Err(e) => format!("Error reading file: {}", e),
            }
        };
        
        let resp_bytes = response.as_bytes();
        let len_to_copy = std::cmp::min(resp_bytes.len(), out_max as usize);
        
        memory.write(&mut caller, out_ptr as usize, &resp_bytes[..len_to_copy]).unwrap();
        
        let latency = start_time.elapsed();
        let state_mut = caller.data_mut();
        let step = (state_mut.traces.len() + 1) as u32;
        state_mut.record_trace(crate::trace::AgentTrace {
            step,
            action: "fs_read".to_string(),
            args: path,
            tokens: "-".to_string(),
            latency,
            result: format!("{} B", len_to_copy),
        });
        
        len_to_copy as i32
    })?;
    
    linker.func_wrap("env", "fs_write", |mut caller: Caller<'_, AgentState>, path_ptr: i32, path_len: i32, content_ptr: i32, content_len: i32, out_ptr: i32, out_max: i32| -> i32 {
        let start_time = std::time::Instant::now();
        let memory = caller.get_export("memory").unwrap().into_memory().unwrap();
        
        let mut path_bytes = vec![0u8; path_len as usize];
        memory.read(&caller, path_ptr as usize, &mut path_bytes).unwrap();
        let path = String::from_utf8_lossy(&path_bytes).to_string();
        
        let mut content_bytes = vec![0u8; content_len as usize];
        memory.read(&caller, content_ptr as usize, &mut content_bytes).unwrap();
        
        info!("[MCP Syscall] 'fs_write' invoked for path: {}", path);
        
        let manifest = &caller.data().manifest;
        let mut allowed = false;
        if let Some(fs_write_rules) = &manifest.permissions.fs_write {
            for rule in fs_write_rules {
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
            info!("[Security] 'fs_write' blocked path: {}", path);
            String::from("[Security] PERMISSION_DENIED")
        } else {
            match std::fs::write(&path, &content_bytes) {
                Ok(_) => String::from("Successfully wrote to file."),
                Err(e) => format!("Failed to write file: {}", e),
            }
        };
        
        let resp_bytes = response.as_bytes();
        let len_to_copy = std::cmp::min(resp_bytes.len(), out_max as usize);
        
        memory.write(&mut caller, out_ptr as usize, &resp_bytes[..len_to_copy]).unwrap();
        
        let latency = start_time.elapsed();
        let state_mut = caller.data_mut();
        let step = (state_mut.traces.len() + 1) as u32;
        state_mut.record_trace(crate::trace::AgentTrace {
            step,
            action: "fs_write".to_string(),
            args: path,
            tokens: "-".to_string(),
            latency,
            result: if response.starts_with("Successfully") { "OK".to_string() } else { "Failed".to_string() },
        });
        
        len_to_copy as i32
    })?;

    linker.func_wrap("env", "get_manifest_goal", |mut caller: Caller<'_, AgentState>, out_ptr: i32, out_max: i32| -> i32 {
        let start_time = std::time::Instant::now();
        let memory = caller.get_export("memory").unwrap().into_memory().unwrap();
        let goal = caller.data().manifest.goal.clone().unwrap_or_default();
        
        let resp_bytes = goal.as_bytes();
        let len_to_copy = std::cmp::min(resp_bytes.len(), out_max as usize);
        
        memory.write(&mut caller, out_ptr as usize, &resp_bytes[..len_to_copy]).unwrap();
        
        let latency = start_time.elapsed();
        let state_mut = caller.data_mut();
        let step = (state_mut.traces.len() + 1) as u32;
        state_mut.record_trace(crate::trace::AgentTrace {
            step,
            action: "get_goal".to_string(),
            args: "-".to_string(),
            tokens: "-".to_string(),
            latency,
            result: format!("{} B", len_to_copy),
        });
        
        len_to_copy as i32
    })?;
    
    linker.func_wrap("env", "web_get", |mut caller: Caller<'_, AgentState>, ptr: i32, len: i32, out_ptr: i32, out_max: i32| -> i32 {
        let start_time = std::time::Instant::now();
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
        
        let latency = start_time.elapsed();
        let state_mut = caller.data_mut();
        let step = (state_mut.traces.len() + 1) as u32;
        state_mut.record_trace(crate::trace::AgentTrace {
            step,
            action: "web_get".to_string(),
            args: url,
            tokens: "-".to_string(),
            latency,
            result: format!("{} B", len_to_copy),
        });
        
        len_to_copy as i32
    })?;
    
    linker.func_wrap("env", "memory_store", |mut caller: Caller<'_, AgentState>, ptr: i32, len: i32| -> i32 {
        let start_time = std::time::Instant::now();
        let memory = caller.get_export("memory").unwrap().into_memory().unwrap();
        
        let mut content_bytes = vec![0u8; len as usize];
        memory.read(&caller, ptr as usize, &mut content_bytes).unwrap();
        let content = String::from_utf8_lossy(&content_bytes).to_string();
        
        info!("[MCP Syscall] 'memory_store' saving: {}", content);
        
        let success = if let Ok(conn) = rusqlite::Connection::open("nanos_memory.db") {
            conn.execute("CREATE TABLE IF NOT EXISTS memories (id INTEGER PRIMARY KEY, content TEXT)", []).unwrap();
            conn.execute("INSERT INTO memories (content) VALUES (?1)", rusqlite::params![content]).unwrap();
            1 // Success
        } else {
            0 // Failure
        };
        
        let latency = start_time.elapsed();
        let state_mut = caller.data_mut();
        let step = (state_mut.traces.len() + 1) as u32;
        state_mut.record_trace(crate::trace::AgentTrace {
            step,
            action: "mem_store".to_string(),
            args: if content.len() > 30 { format!("{}...", &content[..27]) } else { content.clone() },
            tokens: "-".to_string(),
            latency,
            result: if success == 1 { "OK".to_string() } else { "Failed".to_string() },
        });
        
        success
    })?;
    
    linker.func_wrap("env", "memory_recall", |mut caller: Caller<'_, AgentState>, ptr: i32, len: i32, out_ptr: i32, out_max: i32| -> i32 {
        let start_time = std::time::Instant::now();
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
        
        let latency = start_time.elapsed();
        let state_mut = caller.data_mut();
        let step = (state_mut.traces.len() + 1) as u32;
        state_mut.record_trace(crate::trace::AgentTrace {
            step,
            action: "mem_recall".to_string(),
            args: query,
            tokens: "-".to_string(),
            latency,
            result: format!("{} B", len_to_copy),
        });
        
        len_to_copy as i32
    })?;
    
    linker.func_wrap("env", "llm_infer", |mut caller: Caller<'_, AgentState>, prompt_ptr: i32, prompt_len: i32, out_ptr: i32, out_max: i32| -> i32 {
        let start_time = std::time::Instant::now();
        let memory = caller.get_export("memory").unwrap().into_memory().unwrap();
        
        let mut prompt_bytes = vec![0u8; prompt_len as usize];
        memory.read(&caller, prompt_ptr as usize, &mut prompt_bytes).unwrap();
        let prompt = String::from_utf8_lossy(&prompt_bytes).to_string();
        
        let state = caller.data();
        let res = if let Some(llm) = &state.llm {
            llm.infer(&prompt)
        } else {
            Err(anyhow::anyhow!("LLM Engine not initialized"))
        };
        
        let latency = start_time.elapsed();
        
        let (response, prompt_tokens, gen_tokens) = match res {
            Ok(resp) => {
                info!("LLM Raw Response: {}", resp.response);
                (resp.response, resp.prompt_tokens, resp.gen_tokens)
            }
            Err(e) => (format!("LLM Inference Error: {}", e), 0, 0),
        };
        
        let resp_bytes = response.as_bytes();
        let len_to_copy = std::cmp::min(resp_bytes.len(), out_max as usize);
        
        memory.write(&mut caller, out_ptr as usize, &resp_bytes[..len_to_copy]).unwrap();
        
        let state_mut = caller.data_mut();
        let step = (state_mut.traces.len() + 1) as u32;
        state_mut.record_trace(crate::trace::AgentTrace {
            step,
            action: "llm_infer".to_string(),
            args: "(prompt)".to_string(),
            tokens: format!("{}→{}", prompt_tokens, gen_tokens),
            latency,
            result: if response.contains("action") { "JSON OK".to_string() } else { "Text".to_string() },
        });
        
        len_to_copy as i32
    })?;
    
    linker.func_wrap("env", "mcp_call", |mut caller: Caller<'_, AgentState>, server_ptr: i32, server_len: i32, tool_ptr: i32, tool_len: i32, args_ptr: i32, args_len: i32, out_ptr: i32, out_max: i32| -> i32 {
        let start_time = std::time::Instant::now();
        let memory = caller.get_export("memory").unwrap().into_memory().unwrap();
        
        let mut server_bytes = vec![0u8; server_len as usize];
        memory.read(&caller, server_ptr as usize, &mut server_bytes).unwrap();
        let server_name = String::from_utf8_lossy(&server_bytes).to_string();
        
        let mut tool_bytes = vec![0u8; tool_len as usize];
        memory.read(&caller, tool_ptr as usize, &mut tool_bytes).unwrap();
        let tool_name = String::from_utf8_lossy(&tool_bytes).to_string();
        
        let mut args_bytes = vec![0u8; args_len as usize];
        memory.read(&caller, args_ptr as usize, &mut args_bytes).unwrap();
        let args_str = String::from_utf8_lossy(&args_bytes).to_string();
        
        info!("[MCP Syscall] 'mcp_call' invoked on server '{}' for tool '{}' with args: {}", server_name, tool_name, args_str);
        
        let args_val: serde_json::Value = serde_json::from_str(&args_str).unwrap_or(serde_json::Value::Null);
        
        let client_opt = caller.data_mut().mcp_clients.iter_mut().find(|c| c.name == server_name);
        
        let response = match client_opt {
            Some(client) => match client.call_tool(&tool_name, args_val) {
                Ok(resp) => resp,
                Err(e) => format!("MCP Error calling tool: {:?}", e),
            },
            None => format!("MCP Error: Server '{}' not running", server_name),
        };
        
        let resp_bytes = response.as_bytes();
        let len_to_copy = std::cmp::min(resp_bytes.len(), out_max as usize);
        
        memory.write(&mut caller, out_ptr as usize, &resp_bytes[..len_to_copy]).unwrap();
        
        let latency = start_time.elapsed();
        let state_mut = caller.data_mut();
        let step = (state_mut.traces.len() + 1) as u32;
        state_mut.record_trace(crate::trace::AgentTrace {
            step,
            action: format!("mcp:{}", tool_name),
            args: format!("{}: {}", server_name, args_str),
            tokens: "-".to_string(),
            latency,
            result: format!("{} B", len_to_copy),
        });
        
        len_to_copy as i32
    })?;

    linker.func_wrap("env", "agent_send", |mut caller: Caller<'_, AgentState>, target_ptr: i32, target_len: i32, msg_ptr: i32, msg_len: i32| -> i32 {
        let start_time = std::time::Instant::now();
        let memory = caller.get_export("memory").unwrap().into_memory().unwrap();
        
        let mut target_bytes = vec![0u8; target_len as usize];
        memory.read(&caller, target_ptr as usize, &mut target_bytes).unwrap();
        let target = String::from_utf8_lossy(&target_bytes).to_string();
        
        let mut msg_bytes = vec![0u8; msg_len as usize];
        memory.read(&caller, msg_ptr as usize, &mut msg_bytes).unwrap();
        let msg = String::from_utf8_lossy(&msg_bytes).to_string();
        
        info!("[Agent {}] sending msg to {}: {}", caller.data().name, target, msg);
        
        let success = if let Some(bus) = &caller.data().bus {
            bus.send(&target, msg);
            1
        } else {
            0
        };
        
        let latency = start_time.elapsed();
        let state_mut = caller.data_mut();
        let step = (state_mut.traces.len() + 1) as u32;
        state_mut.record_trace(crate::trace::AgentTrace {
            step,
            action: "agent_send".to_string(),
            args: format!("{} -> {}", state_mut.name, target),
            tokens: "-".to_string(),
            latency,
            result: if success == 1 { "OK".to_string() } else { "Failed".to_string() },
        });
        
        success
    })?;

    linker.func_wrap("env", "agent_recv", |mut caller: Caller<'_, AgentState>, out_ptr: i32, out_max: i32| -> i32 {
        let start_time = std::time::Instant::now();
        let memory = caller.get_export("memory").unwrap().into_memory().unwrap();
        
        let name = caller.data().name.clone();
        
        let mut msg_opt = None;
        for _ in 0..100 { // Check every 100ms for up to 10 seconds
            if let Some(bus) = &caller.data().bus {
                if let Some(m) = bus.recv(&name) {
                    msg_opt = Some(m);
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        
        let response = msg_opt.unwrap_or_default();
        let resp_bytes = response.as_bytes();
        let len_to_copy = std::cmp::min(resp_bytes.len(), out_max as usize);
        
        memory.write(&mut caller, out_ptr as usize, &resp_bytes[..len_to_copy]).unwrap();
        
        let latency = start_time.elapsed();
        let state_mut = caller.data_mut();
        let step = (state_mut.traces.len() + 1) as u32;
        state_mut.record_trace(crate::trace::AgentTrace {
            step,
            action: "agent_recv".to_string(),
            args: state_mut.name.clone(),
            tokens: "-".to_string(),
            latency,
            result: format!("{} B", len_to_copy),
        });
        
        len_to_copy as i32
    })?;
    
    linker.func_wrap("env", "eval_js", |mut caller: Caller<'_, AgentState>, code_ptr: i32, code_len: i32, out_ptr: i32, out_max: i32| -> i32 {
        let start_time = std::time::Instant::now();
        let memory = caller.get_export("memory").unwrap().into_memory().unwrap();
        
        let mut code_bytes = vec![0u8; code_len as usize];
        memory.read(&caller, code_ptr as usize, &mut code_bytes).unwrap();
        let js_code = String::from_utf8_lossy(&code_bytes).to_string();
        
        info!("[Sandbox eval_js] Executing sandboxed JS code ({} bytes)...", js_code.len());
        
        // Check if Node.js is available
        let node_check = std::process::Command::new("node").arg("--version").output();
        if node_check.is_err() {
            let err_msg = "eval_js error: Node.js not found on host. Install Node.js >= 20 for sandboxed JS execution.";
            let resp_bytes = err_msg.as_bytes();
            let len_to_copy = std::cmp::min(resp_bytes.len(), out_max as usize);
            memory.write(&mut caller, out_ptr as usize, &resp_bytes[..len_to_copy]).unwrap();
            return len_to_copy as i32;
        }
        
        // Build sandboxed execution command:
        // - timeout 5s: kill process after 5 seconds
        // - permission flag: enable Node.js permission model (--permission or --experimental-permission)
        // - --no-warnings: suppress experimental warnings in output
        // - --allow-worker: allow Worker threads (needed for basic compute)
        // Without explicit --allow-fs-read, --allow-fs-write, --allow-child-process,
        // the permission model denies all filesystem, network, and child_process access.
        let permission_flag = get_node_permission_flag();
        let output = std::process::Command::new("timeout")
            .args(["5", "node", permission_flag, "--no-warnings", "-e"])
            .arg(&js_code)
            .env_clear()
            .env("NODE_NO_WARNINGS", "1")
            .env("HOME", "/tmp/nanos-sandbox") // Isolated HOME
            .output();
            
        let response = match output {
            Ok(out) => {
                let mut result = if out.status.success() {
                    String::from_utf8_lossy(&out.stdout).to_string()
                } else {
                    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                    // Check for timeout (exit code 124 from timeout command)
                    if out.status.code() == Some(124) {
                        "eval_js error: Execution timed out (5s limit exceeded)".to_string()
                    } else if stderr.contains("ERR_ACCESS_DENIED") {
                        format!("eval_js sandbox violation: {}", stderr.lines().next().unwrap_or("Access denied"))
                    } else {
                        format!("JS Error: {}", stderr)
                    }
                };
                // Cap output size to prevent memory abuse
                if result.len() > 65536 {
                    result.truncate(65536);
                    result.push_str("\n[output truncated at 64KB]");
                }
                result
            }
            Err(e) => {
                // Fallback: try without timeout command (not all systems have it)
                let permission_flag = get_node_permission_flag();
                let fallback = std::process::Command::new("node")
                    .args([permission_flag, "--no-warnings", "-e"])
                    .arg(&js_code)
                    .env_clear()
                    .env("NODE_NO_WARNINGS", "1")
                    .output();
                match fallback {
                    Ok(out) => {
                        if out.status.success() {
                            String::from_utf8_lossy(&out.stdout).to_string()
                        } else {
                            format!("JS Error: {}", String::from_utf8_lossy(&out.stderr))
                        }
                    }
                    Err(e2) => format!("eval_js spawn error: {} (fallback: {})", e, e2),
                }
            }
        };
        
        let resp_bytes = response.as_bytes();
        let len_to_copy = std::cmp::min(resp_bytes.len(), out_max as usize);
        
        memory.write(&mut caller, out_ptr as usize, &resp_bytes[..len_to_copy]).unwrap();
        
        let latency = start_time.elapsed();
        let state_mut = caller.data_mut();
        let step = (state_mut.traces.len() + 1) as u32;
        state_mut.record_trace(crate::trace::AgentTrace {
            step,
            action: "eval_js".to_string(),
            args: if js_code.len() > 30 { format!("{}...", &js_code[..27]) } else { js_code },
            tokens: "-".to_string(),
            latency,
            result: format!("{} B", len_to_copy),
        });
        
        len_to_copy as i32
    })?;
    
    Ok(())
}

fn check_fs_read_permission(path: &str, manifest: &AgentManifest) -> bool {
    if let Some(fs_read_rules) = &manifest.permissions.fs_read {
        for rule in fs_read_rules {
            if rule.ends_with("**") {
                let prefix = &rule[..rule.len()-2];
                if path.starts_with(prefix) { return true; }
            } else if rule.ends_with("*") {
                let prefix = &rule[..rule.len()-1];
                if path.starts_with(prefix) { return true; }
            } else if path == rule {
                return true;
            }
        }
    }
    false
}

fn check_fs_write_permission(path: &str, manifest: &AgentManifest) -> bool {
    if let Some(fs_write_rules) = &manifest.permissions.fs_write {
        for rule in fs_write_rules {
            if rule.ends_with("**") {
                let prefix = &rule[..rule.len()-2];
                if path.starts_with(prefix) { return true; }
            } else if rule.ends_with("*") {
                let prefix = &rule[..rule.len()-1];
                if path.starts_with(prefix) { return true; }
            } else if path == rule {
                return true;
            }
        }
    }
    false
}

fn run_js_agent(
    js_code: &str,
    manifest: &AgentManifest,
    llm: Option<std::sync::Arc<LlmEngine>>,
    mcp_clients: Vec<crate::mcp_client::McpClient>,
    bus: Option<std::sync::Arc<crate::orchestrator::MessageBus>>,
) -> Result<()> {
    let temp_js_path = "/tmp/nanos_agent_bundle.js";
    std::fs::write(temp_js_path, js_code)?;
    
    info!("[JS Engine] Spawning Node.js sandbox...");
    let permission_flag = get_node_permission_flag();
    let mut child = Command::new("node")
        .args([permission_flag, "--allow-fs-read=/tmp/*", "--allow-fs-read=/tmp", "--allow-fs-read=/private/tmp/*", "--allow-fs-read=/private/tmp", "--no-warnings", temp_js_path])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
        
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);
    
    let mut traces = Vec::new();
    let mut step = 0u32;
    
    // Convert clients to a mutable reference or wrap
    let mut mcp_clients_mut = mcp_clients;
    
    for line_result in reader.lines() {
        let line = line_result?;
        if line.trim().is_empty() { continue; }
        
        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                info!("[JS Log] {}", line);
                continue;
            }
        };
        
        let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = request.get("params").and_then(|p| p.as_array());
        let request_id = request.get("id").cloned().unwrap_or(Value::Null);
        
        let result = match method {
            "fs_read" => {
                let path = params.and_then(|p| p.get(0)).and_then(|v| v.as_str()).unwrap_or("");
                let allowed = check_fs_read_permission(path, manifest);
                let res = if !allowed {
                    info!("[Security] JS FFI 'fs_read' blocked path: {}", path);
                    "[Security] PERMISSION_DENIED".to_string()
                } else {
                    std::fs::read_to_string(path).unwrap_or_else(|e| format!("Error reading file: {}", e))
                };
                step += 1;
                record_js_trace(&mut traces, step, "fs_read", path, &res, &manifest.name);
                json!(res)
            }
            "fs_write" => {
                let path = params.and_then(|p| p.get(0)).and_then(|v| v.as_str()).unwrap_or("");
                let content = params.and_then(|p| p.get(1)).and_then(|v| v.as_str()).unwrap_or("");
                let allowed = check_fs_write_permission(path, manifest);
                let res = if !allowed {
                    info!("[Security] JS FFI 'fs_write' blocked path: {}", path);
                    "[Security] PERMISSION_DENIED".to_string()
                } else {
                    match std::fs::write(path, content.as_bytes()) {
                        Ok(_) => "Successfully wrote to file.".to_string(),
                        Err(e) => format!("Failed to write file: {}", e),
                    }
                };
                step += 1;
                record_js_trace(&mut traces, step, "fs_write", path, &res, &manifest.name);
                json!(res)
            }
            "llm_infer" => {
                let prompt = params.and_then(|p| p.get(0)).and_then(|v| v.as_str()).unwrap_or("");
                let res = match &llm {
                    Some(engine) => match engine.infer(prompt) {
                        Ok(r) => r.response,
                        Err(e) => format!("LLM inference error: {}", e),
                    },
                    None => "LLM engine not loaded".to_string(),
                };
                step += 1;
                record_js_trace(&mut traces, step, "llm_infer", prompt, &res, &manifest.name);
                json!(res)
            }
            "get_manifest_goal" => {
                let goal = manifest.goal.clone().unwrap_or_default();
                json!(goal)
            }
            "done" => {
                let summary = params.and_then(|p| p.get(0)).and_then(|v| v.as_str()).unwrap_or("Done");
                step += 1;
                record_js_trace(&mut traces, step, "done", summary, "Finished", &manifest.name);
                let response = json!({
                    "jsonrpc": "2.0",
                    "result": "Done",
                    "id": request_id
                });
                let _ = writeln!(stdin, "{}", serde_json::to_string(&response)?);
                let _ = stdin.flush();
                break;
            }
            "web_get" => {
                let url = params.and_then(|p| p.get(0)).and_then(|v| v.as_str()).unwrap_or("");
                let res = if !manifest.permissions.network {
                    info!("[Security] JS FFI 'web_get' blocked network access for url: {}", url);
                    "[Security] NETWORK_DISABLED_IN_MANIFEST".to_string()
                } else {
                    match ureq::get(url).call() {
                        Ok(res) => res.into_string().unwrap_or_else(|_| "Failed to decode response".to_string()),
                        Err(e) => format!("HTTP Request failed: {}", e),
                    }
                };
                step += 1;
                record_js_trace(&mut traces, step, "web_get", url, &res, &manifest.name);
                json!(res)
            }
            "agent_send" => {
                let target = params.and_then(|p| p.get(0)).and_then(|v| v.as_str()).unwrap_or("");
                let msg = params.and_then(|p| p.get(1)).and_then(|v| v.as_str()).unwrap_or("");
                if let Some(ref message_bus) = bus {
                    message_bus.send(target, msg.to_string());
                    step += 1;
                    record_js_trace(&mut traces, step, "agent_send", target, "Message sent", &manifest.name);
                    json!("Message sent successfully.")
                } else {
                    json!("Error: MessageBus not loaded.")
                }
            }
            "agent_recv" => {
                let res = if let Some(ref message_bus) = bus {
                    let agent_name = manifest.name.clone().unwrap_or_default();
                    message_bus.recv(&agent_name).unwrap_or_else(|| "[No messages in queue]".to_string())
                } else {
                    "Error: MessageBus not loaded.".to_string()
                };
                step += 1;
                record_js_trace(&mut traces, step, "agent_recv", &manifest.name.clone().unwrap_or_default(), &res, &manifest.name);
                json!(res)
            }
            "mcp_call" => {
                let server = params.and_then(|p| p.get(0)).and_then(|v| v.as_str()).unwrap_or("");
                let tool = params.and_then(|p| p.get(1)).and_then(|v| v.as_str()).unwrap_or("");
                let arguments = params.and_then(|p| p.get(2)).cloned().unwrap_or(Value::Null);
                
                let mut res = "Error: MCP Server not found.".to_string();
                if let Some(client) = mcp_clients_mut.iter_mut().find(|c| c.name == server) {
                    match client.call_tool(tool, arguments) {
                        Ok(content) => res = content,
                        Err(e) => res = format!("MCP Error: {}", e),
                    }
                }
                step += 1;
                record_js_trace(&mut traces, step, "mcp_call", tool, &res, &manifest.name);
                json!(res)
            }
            _ => json!(format!("Error: Unknown method '{}'", method)),
        };
        
        let response = json!({
            "jsonrpc": "2.0",
            "result": result,
            "id": request_id
        });
        writeln!(stdin, "{}", serde_json::to_string(&response)?)?;
        stdin.flush()?;
    }
    
    let _ = child.kill();
    let _ = child.wait();
    
    crate::trace::print_trace_table(&traces);
    Ok(())
}

fn record_js_trace(
    traces: &mut Vec<crate::trace::AgentTrace>,
    step: u32,
    action: &str,
    args: &str,
    result: &str,
    agent_name: &Option<String>,
) {
    let name = agent_name.clone().unwrap_or_else(|| "nanos-agent".to_string());
    let latency = std::time::Duration::from_millis(0);
    let trace = crate::trace::AgentTrace {
        step,
        action: action.to_string(),
        args: args.to_string(),
        tokens: "-".to_string(),
        latency,
        result: if result.len() > 30 { format!("{} B", result.len()) } else { result.to_string() },
    };
    crate::dashboard::update_agent(&name, "RUNNING", step, 1024);
    crate::dashboard::log_event(format!("[Agent {}] JS-FFI: {} -> {}", name, action, trace.result));
    crate::dashboard::add_trace(trace.clone());
    traces.push(trace);
}

fn get_node_permission_flag() -> &'static str {
    static FLAG: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    *FLAG.get_or_init(|| {
        let output = std::process::Command::new("node")
            .args(["--permission", "--version"])
            .output();
        match output {
            Ok(out) if out.status.success() => "--permission",
            _ => "--experimental-permission",
        }
    })
}
