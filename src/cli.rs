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
    },
    /// View real-time agent execution status and Time-Travel debug console
    Dashboard {
        manifest: Option<PathBuf>,
    },
}
