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
        Commands::Bench { manifest } => {
            info!("nanos benchmark mode...");
            match AgentManifest::load_from_file(manifest) {
                Ok(agent_manifest) => {
                    info!("Loaded Agent Model for Benchmark: {}", agent_manifest.model.path);
                    
                    let system = "You are an AI agent. When you want to execute a tool, you MUST output a raw JSON object and nothing else.
Allowed tools:
- fs_read: reads a file. Args: absolute path.
- web_get: fetches a URL. Args: the URL.
- done: finishes the task. Args: result summary.

Example output:
{\"action\": \"fs_read\", \"args\": \"/workspace/report.txt\"}
";
                    let prompt = "Read the file /etc/passwd and summarize it. If it fails, output done with 'Failed'.";
                    let full_prompt = format!("<|system|>\n{}\n<|user|>\n{}\n<|assistant|>\n", system, prompt);

                    let engine = llm::LlmEngine::new(&agent_manifest.model.path, agent_manifest.model.context_window).unwrap();
                    
                    info!("Running warmup inference (compiling metal kernels)...");
                    let _ = engine.infer(&full_prompt).unwrap();

                    info!("Running timed benchmark inference...");
                    let start = std::time::Instant::now();
                    let response = engine.infer(&full_prompt).unwrap();
                    let elapsed = start.elapsed();
                    
                    println!("✅ FFI Inference completed in: {:.2?}", elapsed);
                    println!("Output length: {} bytes", response.len());
                }
                Err(e) => {
                    error!("Failed to initialize agent: {:?}", e);
                    std::process::exit(1);
                }
            }
        }
    }

    Ok(())
}
