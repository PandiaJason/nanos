use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use tracing::{error, info};

use crate::manifest::AgentManifest;

// Message bus payload schemas
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
enum BusMessage {
    #[serde(rename = "register")]
    Register { name: String, token: Option<String> },
    #[serde(rename = "init")]
    Init {
        goal: String,
        tools: Vec<String>,
        manifest: AgentManifest,
    },
    #[serde(rename = "send")]
    Send { target: String, msg: String },
    #[serde(rename = "recv")]
    Recv,
    #[serde(rename = "msg")]
    Msg { content: Option<String> },
    #[serde(rename = "ok")]
    Ok,
    #[serde(rename = "exit")]
    Exit { status: String, summary: String },
}

struct ServerState {
    queues: HashMap<String, Vec<String>>,
    active_connections: HashMap<String, TcpStream>,
    finished_agents: HashMap<String, String>,
}

pub fn start_orchestrator_server(
    port: u16,
    manifest_path: &Path,
    expected_token: Option<String>,
) -> Result<()> {
    let manifest = AgentManifest::load_from_file(manifest_path)?;
    let agents_list = manifest
        .agents
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No agents list defined in multi-agent fleet manifest"))?;

    let expected_agents: Vec<String> = agents_list.iter().map(|a| a.name.clone()).collect();
    info!(
        "🛰️ Starting Distributed Orchestration Server on port {} for expected agents: {:?}",
        port, expected_agents
    );

    let state = Arc::new(Mutex::new(ServerState {
        queues: HashMap::new(),
        active_connections: HashMap::new(),
        finished_agents: HashMap::new(),
    }));

    let listener = TcpListener::bind(format!("0.0.0.0:{}", port))?;

    // Spawn server loop thread
    let state_clone = state.clone();
    let manifest_clone = manifest.clone();
    let expected_agents_clone = expected_agents.clone();
    let expected_token_clone = expected_token.clone();

    let _server_handle = thread::spawn(move || -> Result<()> {
        for stream in listener.incoming() {
            let stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    error!("TCP connection accept failed: {:?}", e);
                    continue;
                }
            };

            let state = state_clone.clone();
            let manifest = manifest_clone.clone();
            let expected_agents = expected_agents_clone.clone();
            let expected_token = expected_token_clone.clone();

            thread::spawn(move || {
                if let Err(e) = handle_node_connection(
                    stream,
                    state,
                    &manifest,
                    &expected_agents,
                    expected_token,
                ) {
                    error!("Node communication error: {:?}", e);
                }
            });
        }
        Ok(())
    });

    // Poll state until all expected agents register and exit cleanly
    loop {
        thread::sleep(std::time::Duration::from_millis(500));
        let lock = state.lock().unwrap();
        let all_exited = expected_agents
            .iter()
            .all(|name| lock.finished_agents.contains_key(name));
        if all_exited {
            info!(
                "🎉 All expected agents have completed their goals. Shutting down distributed fleet server."
            );
            for (name, summary) in &lock.finished_agents {
                info!("  - Agent [{}]: {}", name, summary);
            }
            break;
        }
    }

    Ok(())
}

fn handle_node_connection(
    mut stream: TcpStream,
    state: Arc<Mutex<ServerState>>,
    manifest: &AgentManifest,
    expected_agents: &[String],
    expected_token: Option<String>,
) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();

    // 1. Read registration message
    reader.read_line(&mut line)?;
    let reg_msg: BusMessage = serde_json::from_str(&line)?;
    let (agent_name, token) = match reg_msg {
        BusMessage::Register { name, token } => (name, token),
        _ => return Err(anyhow::anyhow!("First message must be Register")),
    };

    // Authenticate token if expected
    if let Some(expected) = expected_token {
        let matches = match token {
            Some(t) => t == expected,
            None => false,
        };
        if !matches {
            let error_msg = BusMessage::Exit {
                status: "failed".to_string(),
                summary: "[Security] Unauthorized connection: invalid or missing fleet token"
                    .to_string(),
            };
            let _ = writeln!(stream, "{}", serde_json::to_string(&error_msg)?);
            let _ = stream.flush();
            return Err(anyhow::anyhow!(
                "Connection rejected: invalid or missing token for agent '{}'",
                agent_name
            ));
        }
    }

    if !expected_agents.contains(&agent_name) {
        return Err(anyhow::anyhow!(
            "Unregistered agent node '{}' tried to connect",
            agent_name
        ));
    }

    info!("🔌 Remote agent node registered: {}", agent_name);

    // Register active stream
    {
        let mut lock = state.lock().unwrap();
        lock.active_connections
            .insert(agent_name.clone(), stream.try_clone()?);
    }

    // 2. Fetch specific manifest config for agent
    let spec = manifest
        .agents
        .as_ref()
        .unwrap()
        .iter()
        .find(|a| a.name == agent_name)
        .unwrap();
    let mut agent_manifest = manifest.clone();
    agent_manifest.name = Some(spec.name.clone());
    agent_manifest.goal = Some(spec.goal.clone());
    agent_manifest.tools = Some(spec.tools.clone());
    agent_manifest.agents = None; // Strip out fleet config for the node

    // 3. Send Init message
    let init_msg = BusMessage::Init {
        goal: spec.goal.clone(),
        tools: spec.tools.clone(),
        manifest: agent_manifest,
    };
    writeln!(stream, "{}", serde_json::to_string(&init_msg)?)?;
    stream.flush()?;

    // 4. Message loop for FFI syscall requests
    line.clear();
    while reader.read_line(&mut line)? > 0 {
        let msg: BusMessage = serde_json::from_str(&line)?;
        match msg {
            BusMessage::Send { target, msg } => {
                info!(
                    "✉️ Routing message from [{}] to [{}]: {}",
                    agent_name, target, msg
                );
                let mut lock = state.lock().unwrap();
                lock.queues.entry(target).or_default().push(msg);
                writeln!(stream, "{}", serde_json::to_string(&BusMessage::Ok)?)?;
                stream.flush()?;
            }
            BusMessage::Recv => {
                let mut lock = state.lock().unwrap();
                let queue = lock.queues.entry(agent_name.clone()).or_default();
                let content = if queue.is_empty() {
                    None
                } else {
                    Some(queue.remove(0))
                };
                writeln!(
                    stream,
                    "{}",
                    serde_json::to_string(&BusMessage::Msg { content })?
                )?;
                stream.flush()?;
            }
            BusMessage::Exit { status, summary } => {
                info!(
                    "🏁 Agent node [{}] finished with status '{}': {}",
                    agent_name, status, summary
                );
                let mut lock = state.lock().unwrap();
                lock.finished_agents.insert(agent_name.clone(), summary);
                break;
            }
            _ => {
                error!("Invalid message type received from node '{}'", agent_name);
            }
        }
        line.clear();
    }

    Ok(())
}

pub fn start_agent_node(connect_addr: &str, agent_name: &str, token: Option<String>) -> Result<()> {
    info!(
        "🚀 Connecting Agent Node [{}] to distributed message bus at {}...",
        agent_name, connect_addr
    );
    let stream = TcpStream::connect(connect_addr).with_context(|| {
        format!(
            "Failed to connect to orchestrator server at {}",
            connect_addr
        )
    })?;

    let mut reader = BufReader::new(stream.try_clone()?);

    // 1. Send registration
    let reg_msg = BusMessage::Register {
        name: agent_name.to_string(),
        token,
    };
    let mut stream_write = stream.try_clone()?;
    writeln!(stream_write, "{}", serde_json::to_string(&reg_msg)?)?;
    stream_write.flush()?;

    // 2. Read initialization
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let init_msg: BusMessage = serde_json::from_str(&line)?;
    let (goal, _tools, manifest) = match init_msg {
        BusMessage::Init {
            goal,
            tools,
            manifest,
        } => (goal, tools, manifest),
        BusMessage::Exit { status, summary } => {
            return Err(anyhow::anyhow!(
                "Server rejected registration: {} (Status: {})",
                summary,
                status
            ));
        }
        _ => return Err(anyhow::anyhow!("Server did not respond with Init")),
    };

    info!("Loaded initialization manifest from server. Goal: {}", goal);

    // 3. Create a network-backed MessageBus proxy
    set_node_socket(stream);

    // Execute the agent sandbox run locally!
    info!("Starting sandboxed execution locally...");
    let run_res = crate::sandbox::execute_sandbox(manifest, None, None);

    // Clean up global connection
    if let Some(stream) = take_node_socket() {
        let mut stream_ref = stream.lock().unwrap();
        let exit_msg = BusMessage::Exit {
            status: if run_res.is_ok() {
                "success".to_string()
            } else {
                "failed".to_string()
            },
            summary: format!(
                "{:?}",
                run_res
                    .as_ref()
                    .map(|_| "Execution completed cleanly")
                    .unwrap_or("Execution failed")
            ),
        };
        let _ = writeln!(*stream_ref, "{}", serde_json::to_string(&exit_msg)?);
        let _ = stream_ref.flush();
    }

    run_res.map(|_| ())
}

// Global TCP stream for remote node FFI syscall forwarding
static NODE_SOCKET: std::sync::OnceLock<Mutex<Option<Arc<Mutex<TcpStream>>>>> =
    std::sync::OnceLock::new();

fn get_node_socket() -> &'static Mutex<Option<Arc<Mutex<TcpStream>>>> {
    NODE_SOCKET.get_or_init(|| Mutex::new(None))
}

pub fn set_node_socket(stream: TcpStream) {
    *get_node_socket().lock().unwrap() = Some(Arc::new(Mutex::new(stream)));
}

pub fn take_node_socket() -> Option<Arc<Mutex<TcpStream>>> {
    get_node_socket().lock().unwrap().take()
}

pub fn is_node_mode() -> bool {
    get_node_socket().lock().unwrap().is_some()
}

// Helper function to execute a network FFI syscall
pub fn call_network_bus(method: &str, params: Value) -> Result<Value> {
    let socket_opt = get_node_socket().lock().unwrap().clone();
    if let Some(socket_arc) = socket_opt {
        let mut socket = socket_arc.lock().unwrap();
        let request = json!({
            "type": method,
            "target": params.get("target").and_then(|v| v.as_str()),
            "msg": params.get("msg").and_then(|v| v.as_str())
        });

        writeln!(*socket, "{}", serde_json::to_string(&request)?)?;
        socket.flush()?;

        let mut reader = BufReader::new(socket.try_clone()?);
        let mut line = String::new();
        reader.read_line(&mut line)?;

        let response_val: Value = serde_json::from_str(&line)?;
        if method == "recv" {
            // Return message content
            Ok(response_val.get("content").cloned().unwrap_or(Value::Null))
        } else {
            Ok(Value::String("OK".to_string()))
        }
    } else {
        Err(anyhow::anyhow!("Node network socket not initialized"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn test_network_message_bus() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();

        let server_thread = thread::spawn(move || -> Result<()> {
            let (mut stream, _) = listener.accept()?;
            let mut reader = BufReader::new(stream.try_clone()?);
            let mut line = String::new();

            // Read Register
            reader.read_line(&mut line)?;
            let reg: BusMessage = serde_json::from_str(&line)?;
            assert!(matches!(reg, BusMessage::Register { ref name, .. } if name == "test-agent"));

            // Send Init (mock manifest)
            let manifest: AgentManifest = serde_yaml::from_str(
                r#"
                model:
                  context_window: 2048
                resources:
                  memory: "512mb"
                  max_steps: 10
                permissions:
                  network: false
                "#,
            )
            .unwrap();
            let init_msg = BusMessage::Init {
                goal: "test-goal".to_string(),
                tools: vec!["fs_read".to_string()],
                manifest,
            };
            writeln!(stream, "{}", serde_json::to_string(&init_msg)?)?;
            stream.flush()?;

            // Message loop
            line.clear();
            while reader.read_line(&mut line)? > 0 {
                let msg: BusMessage = serde_json::from_str(&line)?;
                match msg {
                    BusMessage::Send { target, msg } => {
                        assert_eq!(target, "writer");
                        assert_eq!(msg, "hello");
                        writeln!(stream, "{}", serde_json::to_string(&BusMessage::Ok)?)?;
                        stream.flush()?;
                    }
                    BusMessage::Recv => {
                        let resp = BusMessage::Msg {
                            content: Some("secret".to_string()),
                        };
                        writeln!(stream, "{}", serde_json::to_string(&resp)?)?;
                        stream.flush()?;
                    }
                    BusMessage::Exit { .. } => {
                        break;
                    }
                    _ => {}
                }
                line.clear();
            }
            Ok(())
        });

        // Client side simulation
        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))?;
        let reg_msg = BusMessage::Register {
            name: "test-agent".to_string(),
            token: None,
        };
        writeln!(stream, "{}", serde_json::to_string(&reg_msg)?)?;
        stream.flush()?;

        let mut client_reader = BufReader::new(stream.try_clone()?);
        let mut init_line = String::new();
        client_reader.read_line(&mut init_line)?;

        set_node_socket(stream);
        assert!(is_node_mode());

        // Test call_network_bus for "send"
        let send_res = call_network_bus(
            "send",
            serde_json::json!({
                "target": "writer",
                "msg": "hello"
            }),
        )?;
        assert_eq!(send_res, Value::String("OK".to_string()));

        // Test call_network_bus for "recv"
        let recv_res = call_network_bus("recv", serde_json::json!({}))?;
        assert_eq!(recv_res, Value::String("secret".to_string()));

        // Clean up
        if let Some(stream_arc) = take_node_socket() {
            let mut stream = stream_arc.lock().unwrap();
            let exit_msg = BusMessage::Exit {
                status: "success".to_string(),
                summary: "completed".to_string(),
            };
            writeln!(*stream, "{}", serde_json::to_string(&exit_msg)?)?;
            stream.flush()?;
        }

        server_thread.join().unwrap()?;
        assert!(!is_node_mode());
        Ok(())
    }

    #[test]
    fn test_network_authentication_success() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();

        let server_thread = thread::spawn(move || -> Result<()> {
            let (stream, _) = listener.accept()?;
            let manifest: AgentManifest = serde_yaml::from_str(
                r#"
                model:
                  context_window: 2048
                resources:
                  memory: "512mb"
                  max_steps: 10
                permissions:
                  network: false
                agents:
                  - name: "test-agent"
                    goal: "do something"
                    tools: ["fs_read"]
                "#,
            )
            .unwrap();
            let expected_agents = vec!["test-agent".to_string()];
            let res = handle_node_connection(
                stream,
                Arc::new(Mutex::new(ServerState {
                    queues: HashMap::new(),
                    active_connections: HashMap::new(),
                    finished_agents: HashMap::new(),
                })),
                &manifest,
                &expected_agents,
                Some("secret123".to_string()),
            );
            assert!(res.is_ok());
            Ok(())
        });

        // Client side - sends correct token
        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))?;
        let reg_msg = BusMessage::Register {
            name: "test-agent".to_string(),
            token: Some("secret123".to_string()),
        };
        writeln!(stream, "{}", serde_json::to_string(&reg_msg)?)?;
        stream.flush()?;

        let mut client_reader = BufReader::new(stream.try_clone()?);
        let mut init_line = String::new();
        client_reader.read_line(&mut init_line)?;
        let init_msg: BusMessage = serde_json::from_str(&init_line)?;
        assert!(matches!(init_msg, BusMessage::Init { .. }));

        drop(client_reader);
        drop(stream);

        server_thread.join().unwrap()?;
        Ok(())
    }

    #[test]
    fn test_network_authentication_failure() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();

        let server_thread = thread::spawn(move || -> Result<()> {
            let (stream, _) = listener.accept()?;
            let manifest: AgentManifest = serde_yaml::from_str(
                r#"
                model:
                  context_window: 2048
                resources:
                  memory: "512mb"
                  max_steps: 10
                permissions:
                  network: false
                agents:
                  - name: "test-agent"
                    goal: "do something"
                    tools: ["fs_read"]
                "#,
            )
            .unwrap();
            let expected_agents = vec!["test-agent".to_string()];
            let res = handle_node_connection(
                stream,
                Arc::new(Mutex::new(ServerState {
                    queues: HashMap::new(),
                    active_connections: HashMap::new(),
                    finished_agents: HashMap::new(),
                })),
                &manifest,
                &expected_agents,
                Some("secret123".to_string()),
            );
            // Should fail due to invalid token
            assert!(res.is_err());
            Ok(())
        });

        // Client side - sends incorrect token
        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))?;
        let reg_msg = BusMessage::Register {
            name: "test-agent".to_string(),
            token: Some("wrong_token".to_string()),
        };
        writeln!(stream, "{}", serde_json::to_string(&reg_msg)?)?;
        stream.flush()?;

        let mut client_reader = BufReader::new(stream.try_clone()?);
        let mut error_line = String::new();
        client_reader.read_line(&mut error_line)?;
        let exit_msg: BusMessage = serde_json::from_str(&error_line)?;
        assert!(matches!(exit_msg, BusMessage::Exit { ref status, .. } if status == "failed"));

        server_thread.join().unwrap()?;
        Ok(())
    }
}
