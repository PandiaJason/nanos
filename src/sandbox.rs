use anyhow::Result;
use tracing::{info, debug, error};
use wasmtime::*;

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
    
    // In production, we configure limits (memory, fuel) on the Config object here.
    let mut config = Config::new();
    config.wasm_component_model(false); // We are using core wasm for now
    
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
    
    let mut store = Store::new(&engine, AgentState {
        name: agent_name,
        manifest: manifest.clone(),
        llm: Some(llm_engine),
        traces: Vec::new(),
        mcp_clients,
        bus,
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
        
        info!("[Sandbox E2B] 'eval_js' executing dynamic JS code...");
        
        // Execute under sandboxed Node process
        let output = std::process::Command::new("node")
            .arg("-e")
            .arg(&js_code)
            .output();
            
        let response = match output {
            Ok(out) => {
                if out.status.success() {
                    String::from_utf8_lossy(&out.stdout).to_string()
                } else {
                    format!("JS Error: {}", String::from_utf8_lossy(&out.stderr))
                }
            }
            Err(e) => format!("Failed to spawn local JS engine (E2B sandbox error): {}", e),
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
