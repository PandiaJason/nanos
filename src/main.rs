mod cli;
mod manifest;
mod sandbox;
mod llm;

use anyhow::Result;
use clap::Parser;
use tracing::{info, error, Level};
use tracing_subscriber::FmtSubscriber;

use cli::{Cli, Commands};
use manifest::AgentManifest;

fn main() -> Result<()> {
    // Initialize production-grade logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("Setting default subscriber failed");

    let cli = Cli::parse();

    match &cli.command {
        Commands::Run { manifest } => {
            info!("nanos spawning process...");
            
            match AgentManifest::load_from_file(manifest) {
                Ok(agent_manifest) => {
                    info!("Loaded Agent: {}", agent_manifest.name);
                    info!("Goal: {}", agent_manifest.goal);
                    
                    if let Err(e) = sandbox::execute_sandbox(agent_manifest) {
                        error!("Sandbox execution failed: {:?}", e);
                        std::process::exit(1);
                    }
                }
                Err(e) => {
                    error!("Failed to initialize agent: {:?}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Serve { manifest: _ } => {
            info!("Server mode not yet implemented.");
        }
    }

    Ok(())
}
