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
        #[arg(default_value = "agent.nano")]
        manifest: PathBuf,
    },
    /// Serve an AI agent via local socket (TODO)
    Serve {
        #[arg(default_value = "agent.nano")]
        manifest: PathBuf,
    },
    /// Run a latency benchmark against the model using native FFI
    Bench {
        #[arg(default_value = "agent.nano")]
        manifest: PathBuf,
    },
}
