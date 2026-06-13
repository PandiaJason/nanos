mod cli;
mod server;

use anyhow::Result;
use clap::Parser;
use tracing::{Level, error, info};
use tracing_subscriber::FmtSubscriber;

use cli::{Cli, Commands};
use nanos::manifest::AgentManifest;

fn main() -> Result<()> {
    // Initialize production-grade logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("Setting default subscriber failed");

    let cli = Cli::parse();

    match &cli.command {
        Commands::Run { manifest } => {
            info!("nanos spawning process...");

            let resolved_path =
                match resolve_manifest(manifest, &["agent.nano", "test_e2e.nano", "mcp_test.nano"])
                {
                    Ok(path) => path,
                    Err(e) => {
                        error!("{}", e);
                        std::process::exit(1);
                    }
                };

            match AgentManifest::load_from_file(&resolved_path) {
                Ok(agent_manifest) => {
                    let name = agent_manifest.name.as_deref().unwrap_or("nanos-agent");
                    let goal = agent_manifest
                        .goal
                        .as_deref()
                        .unwrap_or("No goal specified");
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
        Commands::Serve { port, host } => {
            info!("nanos starting daemon mode...");
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            rt.block_on(async {
                if let Err(e) = server::start_server(host, *port).await {
                    error!("Server failed to start: {:?}", e);
                    std::process::exit(1);
                }
            });
        }
        Commands::Orchestrate {
            manifest,
            network,
            port,
            token,
        } => {
            let resolved_path = match resolve_manifest(manifest, &["fleet.nano"]) {
                Ok(path) => path,
                Err(e) => {
                    error!("{}", e);
                    std::process::exit(1);
                }
            };
            if *network {
                let manifest_token = match AgentManifest::load_from_file(&resolved_path) {
                    Ok(m) => m.token,
                    Err(_) => None,
                };
                let token_str = token
                    .clone()
                    .or(manifest_token)
                    .or_else(|| std::env::var("NANOS_FLEET_TOKEN").ok());

                info!("nanos orchestrating fleet over network on port {}...", port);
                if let Err(e) =
                    nanos::network::start_orchestrator_server(*port, &resolved_path, token_str)
                {
                    error!("Fleet network orchestration failed: {:?}", e);
                    std::process::exit(1);
                }
            } else {
                info!("nanos orchestrating fleet...");
                if let Err(e) = nanos::orchestrator::orchestrate(&resolved_path) {
                    error!("Fleet orchestration failed: {:?}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Node {
            connect,
            name,
            token,
        } => {
            let token_str = token
                .clone()
                .or_else(|| std::env::var("NANOS_FLEET_TOKEN").ok());

            info!("nanos node client starting for agent: {}...", name);
            if let Err(e) = nanos::network::start_agent_node(connect, name, token_str) {
                error!("Agent node connection failed: {:?}", e);
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
                    info!(
                        "Loaded Agent Model for Benchmark: {:?}",
                        agent_manifest.model.path
                    );

                    // Use proper ChatML format (Qwen) with a general-purpose prompt
                    // that generates long-form text for accurate tok/s measurement
                    let full_prompt = "<|im_start|>system\nYou are a helpful assistant.<|im_end|>\n<|im_start|>user\nExplain why sandboxed AI agents are important for security in 50 words.<|im_end|>\n<|im_start|>assistant\n";

                    let engine = nanos::llm::LlmEngine::new(&agent_manifest.model).unwrap();

                    info!("Running warmup inference...");
                    let _ = engine.infer(full_prompt).unwrap();

                    info!("Running timed benchmark inference...");
                    let response = engine.infer(full_prompt).unwrap();

                    let prompt_tps = if response.prompt_eval_ms > 0.0 {
                        response.prompt_tokens as f64 / (response.prompt_eval_ms / 1000.0)
                    } else {
                        0.0
                    };
                    let gen_tps = if response.gen_ms > 0.0 {
                        response.gen_tokens as f64 / (response.gen_ms / 1000.0)
                    } else {
                        0.0
                    };

                    println!();
                    println!("=== nanos Benchmark Results ===");
                    println!(
                        "Prompt:     {} tokens, {:.1} ms ({:.1} tok/s)",
                        response.prompt_tokens, response.prompt_eval_ms, prompt_tps
                    );
                    println!(
                        "Generation: {} tokens, {:.1} ms ({:.1} tok/s)",
                        response.gen_tokens, response.gen_ms, gen_tps
                    );
                    drop(engine);
                    std::thread::sleep(std::time::Duration::from_millis(150));
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
