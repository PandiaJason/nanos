mod cli;

use anyhow::Result;
use clap::Parser;
use tracing::{info, error, Level};
use tracing_subscriber::FmtSubscriber;

use cli::{Cli, Commands};
use nanos::manifest::AgentManifest;

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
            
            let resolved_path = match resolve_manifest(manifest, &["agent.nano", "test_e2e.nano", "mcp_test.nano"]) {
                Ok(path) => path,
                Err(e) => {
                    error!("{}", e);
                    std::process::exit(1);
                }
            };

            match AgentManifest::load_from_file(&resolved_path) {
                Ok(agent_manifest) => {
                    let name = agent_manifest.name.as_deref().unwrap_or("nanos-agent");
                    let goal = agent_manifest.goal.as_deref().unwrap_or("No goal specified");
                    info!("Loaded Agent: {}", name);
                    info!("Goal: {}", goal);
                    
                    if let Err(e) = nanos::sandbox::execute_sandbox(agent_manifest, None, None) {
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
        Commands::Orchestrate { manifest } => {
            info!("nanos orchestrating fleet...");
            let resolved_path = match resolve_manifest(manifest, &["fleet.nano"]) {
                Ok(path) => path,
                Err(e) => {
                    error!("{}", e);
                    std::process::exit(1);
                }
            };
            if let Err(e) = nanos::orchestrator::orchestrate(&resolved_path) {
                error!("Fleet orchestration failed: {:?}", e);
                std::process::exit(1);
            }
        }
        Commands::Dashboard { manifest } => {
            let resolved_path = match resolve_manifest(manifest, &["fleet.nano", "agent.nano", "test_e2e.nano"]) {
                Ok(path) => path,
                Err(e) => {
                    error!("{}", e);
                    std::process::exit(1);
                }
            };
            if let Err(e) = nanos::dashboard::run_dashboard(&resolved_path) {
                error!("Dashboard failed: {:?}", e);
                std::process::exit(1);
            }
        }
        Commands::Bench { manifest } => {
            info!("nanos benchmark mode...");
            let resolved_path = match resolve_manifest(manifest, &["agent.nano", "test_e2e.nano"]) {
                Ok(path) => path,
                Err(e) => {
                    error!("{}", e);
                    std::process::exit(1);
                }
            };
            match AgentManifest::load_from_file(&resolved_path) {
                Ok(agent_manifest) => {
                    info!("Loaded Agent Model for Benchmark: {:?}", agent_manifest.model.path);
                    
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

                    let engine = nanos::llm::LlmEngine::new(&agent_manifest.model).unwrap();
                    
                    info!("Running warmup inference...");
                    let _ = engine.infer(&full_prompt).unwrap();

                    info!("Running timed benchmark inference...");
                    let start = std::time::Instant::now();
                    let response = engine.infer(&full_prompt).unwrap();
                    let elapsed = start.elapsed();
                    
                    println!("✅ FFI Inference completed in: {:.2?}", elapsed);
                    println!("Output length: {} bytes", response.response.len());
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

fn resolve_manifest(
    manifest: &Option<std::path::PathBuf>,
    default_names: &[&str],
) -> Result<std::path::PathBuf> {
    if let Some(path) = manifest {
        if path.exists() {
            return Ok(path.clone());
        }
        return Err(anyhow::anyhow!("Manifest file not found at: {:?}", path));
    }
    
    // Auto-discover in current directory and common locations
    for name in default_names {
        let paths = [
            std::path::PathBuf::from(name),
            std::path::PathBuf::from("examples").join(name),
        ];
        for p in &paths {
            if p.exists() {
                info!("Auto-discovered manifest: {:?}", p);
                return Ok(p.clone());
            }
        }
    }
    
    Err(anyhow::anyhow!(
        "No manifest file specified, and could not auto-discover any default manifests (e.g. '{}') in the current directory.\n\n\
        To resolve this, please do one of the following:\n\
        1. Run the command from the nanos directory (which contains the 'examples/' folder).\n\
        2. Specify the path to your manifest explicitly, e.g.:\n\
           nanos dashboard examples/fleet.nano\n\
           nanos run /path/to/agent.nano",
        default_names.first().unwrap_or(&"agent.nano")
    ))
}
