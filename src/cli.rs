use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// nanos: The AI-Native WASM Runtime
#[derive(Parser)]
#[command(name = "nanos")]
#[command(about = "A lightning-fast WASM runtime for AI Agents", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Run an AI agent from a nano manifest
    Run {
        /// Path to the agent.nano manifest file
        manifest: Option<PathBuf>,
    },
    /// Serve an AI agent daemon over HTTP
    Serve {
        /// Port to listen on (default: 8080)
        #[arg(short, long, default_value = "8080")]
        port: u16,
        /// Host to bind to (default: 127.0.0.1)
        #[arg(short, long, default_value = "127.0.0.1")]
        host: String,
    },
    /// Run a latency benchmark against the model using native FFI
    Bench {
        manifest: Option<PathBuf>,
    },
    /// Orchestrate a multi-agent fleet from a manifest
    Orchestrate {
        manifest: Option<PathBuf>,
        /// Enable network orchestration mode (as TCP server)
        #[arg(short, long)]
        network: bool,
        /// Port to bind orchestrator server to (default: 9090)
        #[arg(short, long, default_value = "9090")]
        port: u16,
    },
    /// Connect a remote agent node to a distributed orchestrator
    Node {
        /// Address to connect to, e.g. 127.0.0.1:9090
        #[arg(short, long)]
        connect: String,
        /// Name of this agent node matching fleet manifest configuration
        #[arg(short, long)]
        name: String,
    },
}
