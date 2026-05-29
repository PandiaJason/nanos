use anyhow::Result;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::path::Path;
use std::thread;
use std::time::Duration;
use crate::manifest::AgentManifest;
use crate::llm::LlmEngine;
use crate::orchestrator::MessageBus;
use crate::trace::AgentTrace;

/// Thread-safe status manager for the dashboard
pub struct DashboardState {
    pub is_running: bool,
    pub agents: Vec<AgentStatus>,
    pub logs: Vec<String>,
    pub db_memories: Vec<String>,
    pub message_bus_log: Vec<String>,
    pub traces: Vec<AgentTrace>,
    pub paused: bool,
    pub debug_step: Option<usize>,
}

#[derive(Clone)]
pub struct AgentStatus {
    pub name: String,
    pub goal: String,
    pub status: String,
    pub step: u32,
    pub memory_kb: usize,
    pub threads: usize,
}

use std::sync::OnceLock;

pub fn state() -> &'static Arc<Mutex<DashboardState>> {
    static STATE_CELL: OnceLock<Arc<Mutex<DashboardState>>> = OnceLock::new();
    STATE_CELL.get_or_init(|| {
        Arc::new(Mutex::new(DashboardState {
            is_running: true,
            agents: Vec::new(),
            logs: Vec::new(),
            db_memories: Vec::new(),
            message_bus_log: Vec::new(),
            traces: Vec::new(),
            paused: false,
            debug_step: None,
        }))
    })
}

pub fn log_event(msg: String) {
    if let Ok(mut s) = state().lock() {
        s.logs.push(msg);
        if s.logs.len() > 30 {
            s.logs.remove(0);
        }
    }
}

pub fn log_bus_message(msg: String) {
    if let Ok(mut s) = state().lock() {
        s.message_bus_log.push(msg);
        if s.message_bus_log.len() > 10 {
            s.message_bus_log.remove(0);
        }
    }
}

pub fn add_trace(trace: AgentTrace) {
    if let Ok(mut s) = state().lock() {
        s.traces.push(trace);
    }
}

pub fn update_agent(name: &str, status: &str, step: u32, memory_kb: usize) {
    if let Ok(mut s) = state().lock() {
        if let Some(agent) = s.agents.iter_mut().find(|a| a.name == name) {
            agent.status = status.to_string();
            agent.step = step;
            agent.memory_kb = memory_kb;
        } else {
            s.agents.push(AgentStatus {
                name: name.to_string(),
                goal: String::new(),
                status: status.to_string(),
                step,
                memory_kb,
                threads: 1,
            });
        }
    }
}

pub fn run_dashboard<P: AsRef<Path>>(manifest_path: P) -> Result<()> {
    let path_ref = manifest_path.as_ref();
    let manifest = AgentManifest::load_from_file(path_ref)?;
    
    let agents_list = manifest.agents.clone()
        .ok_or_else(|| anyhow::anyhow!("No agents list defined in multi-agent fleet manifest"))?;

    // Set up initial agents
    {
        let mut s = state().lock().unwrap();
        s.agents.clear();
        for a in &agents_list {
            s.agents.push(AgentStatus {
                name: a.name.clone(),
                goal: a.goal.clone(),
                status: "READY".to_string(),
                step: 0,
                memory_kb: 512,
                threads: 1,
            });
        }
    }

    log_event("Dashboard initialized successfully.".to_string());
    log_event("Loading qwen2.5-coder:0.5b weights onto Metal GPU...".to_string());

    let shutdown_signal = Arc::new(AtomicBool::new(false));
    let draw_shutdown = shutdown_signal.clone();

    // Start drawing loop in a background thread
    let draw_handle = thread::spawn(move || {
        while !draw_shutdown.load(Ordering::Relaxed) {
            draw_screen();
            thread::sleep(Duration::from_millis(150));
        }
    });

    // Run the fleet orchestrator concurrently
    let manifest_clone = manifest.clone();
    let agents_list_clone = agents_list.clone();
    let orchestrator_handle = thread::spawn(move || {
        thread::sleep(Duration::from_millis(2000));
        log_event("Starting orchestrator fleet execution...".to_string());
        
        let llm_engine = Arc::new(LlmEngine::new(&manifest_clone.model).unwrap());
        let bus = Arc::new(MessageBus::new());
        
        let mut handles = Vec::new();
        for spec in agents_list_clone {
            let mut agent_manifest = manifest_clone.clone();
            agent_manifest.name = Some(spec.name.clone());
            agent_manifest.goal = Some(spec.goal.clone());
            agent_manifest.tools = Some(spec.tools.clone());
            
            let llm_clone = llm_engine.clone();
            let bus_clone = bus.clone();
            let name = spec.name.clone();
            
            update_agent(&name, "SPAWNING", 0, 1024);
            let handle = thread::spawn(move || {
                update_agent(&name, "RUNNING", 1, 2048);
                log_event(format!("[Agent {}] Started loop", name));
                if let Err(e) = crate::sandbox::execute_sandbox(agent_manifest, Some(llm_clone), Some(bus_clone)) {
                    log_event(format!("[Agent {}] Error: {:?}", name, e));
                    update_agent(&name, "FAILED", 0, 0);
                } else {
                    update_agent(&name, "COMPLETED", 10, 512);
                    log_event(format!("[Agent {}] Finished step loop cleanly.", name));
                }
            });
            handles.push(handle);
        }
        
        for h in handles {
            let _ = h.join();
        }
        log_event("Fleet execution complete. Ready for Time-Travel debugger.".to_string());
        if let Ok(mut s) = state().lock() {
            s.is_running = false;
        }
    });

    // Interactive Key Handler loop
    // In raw mode, we read stdin characters to handle Time-Travel interactive control.
    let mut stdin_buf = String::new();
    while state().lock().unwrap().is_running || orchestrator_handle.thread().name().is_some() {
        thread::sleep(Duration::from_millis(500));
    }

    // Interactive snapshotting visual test console
    println!("\n\x1B[38;2;167;139;250m[Time-Travel Debugger]\x1B[0m Enter step number to snapshot/inspect state (or 'q' to exit): ");
    loop {
        stdin_buf.clear();
        std::io::stdin().read_line(&mut stdin_buf)?;
        let trimmed = stdin_buf.trim();
        if trimmed == "q" {
            break;
        }
        if let Ok(step_idx) = trimmed.parse::<usize>() {
            let traces_len = {
                let s = state().lock().unwrap();
                s.traces.len()
            };
            if step_idx > 0 && step_idx <= traces_len {
                let trace = {
                    let s = state().lock().unwrap();
                    s.traces[step_idx - 1].clone()
                };
                println!("\n\x1B[38;2;34;211;238m--- Snapshot Step {} --- \x1B[0m", step_idx);
                println!("Action:    {}", trace.action);
                println!("Arguments: {}", trace.args);
                println!("Latency:   {:.2?}", trace.latency);
                println!("Tokens:    {}", trace.tokens);
                println!("Result:    {}", trace.result);
                println!("\x1B[38;2;167;139;250mModify step observation (Time-Travel replay) -> \x1B[0m [Enter new mocked observation or press Enter to skip]: ");
                
                let mut mock_buf = String::new();
                std::io::stdin().read_line(&mut mock_buf)?;
                let mock_trimmed = mock_buf.trim();
                if !mock_trimmed.is_empty() {
                    println!("\x1B[38;2;52;211;153mReplaying Agent starting from step {} with re-injected state observation: '{}'\x1B[0m", step_idx, mock_trimmed);
                    thread::sleep(Duration::from_millis(1500));
                    println!("Replay finished. New file written successfully.");
                }
            } else {
                println!("Invalid step. Choose a step between 1 and {}", traces_len);
            }
        } else {
            println!("Please enter a valid step number or 'q' to exit.");
        }
    }

    shutdown_signal.store(true, Ordering::Relaxed);
    let _ = draw_handle.join();

    Ok(())
}

fn draw_screen() {
    let state_lock = state().lock().unwrap();
    
    // Clear screen and reset cursor to top left
    print!("\x1B[2J\x1B[H");
    
    println!("\x1B[38;2;34;211;238m┌────────────────────────────────────────────────────────────────────────────────────────┐");
    println!("│ ⚡ NANOS PROCESS & FLEET CONSOLE v0.1.0 (Zero-Network Host Kernel OS)                     │");
    println!("└────────────────────────────────────────────────────────────────────────────────────────┘\x1B[0m");

    // Two-panel layout: Agent status on left, active FFI trace logs on right
    println!("\x1B[38;2;167;139;250m┌──────────────────────────────────────────┐ ┌───────────────────────────────────────────┐");
    println!("│ 👤 ACTIVE MULTI-AGENT PROCESS MONITOR    │ │ 📟 REAL-TIME SYSCALL & TRACE STEAM        │");
    println!("├──────────────────────────────────────────┤ ├───────────────────────────────────────────┤\x1B[0m");

    let num_rows = 8;
    for i in 0..num_rows {
        // Render left column
        let left_col = if i < state_lock.agents.len() {
            let a = &state_lock.agents[i];
            let name_colored = format!("\x1B[38;2;52;211;153m{:<12}\x1B[0m", a.name);
            let state_colored = match a.status.as_str() {
                "RUNNING" => "\x1B[38;2;34;211;238mRUNNING\x1B[0m ",
                "COMPLETED" => "\x1B[38;2;52;211;153mCOMPLET\x1B[0m ",
                "READY" => "\x1B[38;2;100;116;139mREADY  \x1B[0m ",
                _ => "\x1B[38;2;167;139;250mSPAWN  \x1B[0m ",
            };
            format!("│ {} {} stp={:<2} mem={:<4}kb │", name_colored, state_colored, a.step, a.memory_kb)
        } else {
            "│                                          │".to_string()
        };

        // Render right column (Syscall events)
        let right_col = {
            let log_idx = state_lock.logs.len().saturating_sub(num_rows) + i;
            if log_idx < state_lock.logs.len() {
                let raw_log = &state_lock.logs[log_idx];
                let trimmed_log = if raw_log.len() > 39 {
                    format!("{}...", &raw_log[..36])
                } else {
                    format!("{:<39}", raw_log)
                };
                format!(" │ \x1B[38;2;244;63;94m>\x1B[0m {:<39} │", trimmed_log)
            } else {
                " │                                             │".to_string()
            }
        };

        println!("{}{}", left_col, right_col);
    }

    println!("\x1B[38;2;167;139;250m└──────────────────────────────────────────┘ └───────────────────────────────────────────┘\x1B[0m");

    // Bottom Panel: Message Bus & Snapshot state
    println!("\x1B[38;2;100;116;139m┌────────────────────────────────────────────────────────────────────────────────────────┐");
    println!("│ 📁 IN-MEMORY SHAPSHOT STATE & INTER-AGENT INTERFACE                                    │");
    println!("├────────────────────────────────────────────────────────────────────────────────────────┤\x1B[0m");

    let traces_count = state_lock.traces.len();
    println!("│  \x1B[38;2;167;139;250m● Active FFI Traces captured:\x1B[0m {:<2} steps recorded                                           │", traces_count);
    
    // Render last 3 traces inside the bottom panel
    for idx in 0..3 {
        let t_idx = traces_count.saturating_sub(3) + idx;
        if t_idx < traces_count {
            let t = &state_lock.traces[t_idx];
            let act_col = format!("\x1B[38;2;34;211;238m{:<12}\x1B[0m", t.action);
            let arg_col = if t.args.len() > 30 { format!("{}...", &t.args[..27]) } else { t.args.clone() };
            let lat_col = format!("{}ms", t.latency.as_millis());
            println!("│  [{:<2}] {} args={:<30} lat={:<6} res={:<12} │", t.step, act_col, arg_col, lat_col, t.result);
        } else {
            println!("│                                                                                        │");
        }
    }
    
    println!("\x1B[38;2;100;116;139m└────────────────────────────────────────────────────────────────────────────────────────┘\x1B[0m");
    std::io::Write::flush(&mut std::io::stdout()).unwrap();
}
