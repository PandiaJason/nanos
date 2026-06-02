use crate::trace::AgentTrace;
use std::sync::{Arc, Mutex};
use tracing::info;

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
    info!("[Event] {}", msg);
    if let Ok(mut s) = state().lock() {
        s.logs.push(msg);
    }
}

pub fn log_bus_message(msg: String) {
    info!("[Bus] {}", msg);
    if let Ok(mut s) = state().lock() {
        s.message_bus_log.push(msg);
    }
}

pub fn add_trace(trace: AgentTrace) {
    if let Ok(mut s) = state().lock() {
        s.traces.push(trace);
    }
}

pub fn update_agent(name: &str, status: &str, step: u32, memory_kb: usize) {
    info!(
        "[Agent: {}] Status: {}, Step: {}, Memory: {} KB",
        name, status, step, memory_kb
    );
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
