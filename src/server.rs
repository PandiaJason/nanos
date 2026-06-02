use anyhow::Result;
use axum::{
    Json, Router,
    http::StatusCode,
    routing::{get, post},
};
use nanos::manifest::AgentManifest;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use tracing::info;

#[derive(Serialize)]
struct StatusResponse {
    status: String,
    version: String,
    auto_discovered_manifests: Vec<String>,
}

#[derive(Serialize)]
struct RunResponse {
    status: String,
    traces: Vec<nanos::trace::AgentTrace>,
}

#[derive(Serialize)]
struct OrchestrateResponse {
    status: String,
}

// Handler for status
async fn get_status() -> Json<StatusResponse> {
    let mut auto_discovered_manifests = Vec::new();
    let common_manifests = &["agent.nano", "fleet.nano", "test_e2e.nano", "mcp_test.nano"];
    for name in common_manifests {
        if std::path::Path::new(name).exists() {
            auto_discovered_manifests.push(name.to_string());
        }
        let examples_path = std::path::Path::new("examples").join(name);
        if examples_path.exists() {
            auto_discovered_manifests.push(format!("examples/{}", name));
        }
    }

    Json(StatusResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        auto_discovered_manifests,
    })
}

// Handler for running agent from payload
async fn post_run(
    Json(payload): Json<AgentManifest>,
) -> Result<Json<RunResponse>, (StatusCode, String)> {
    info!("HTTP: Spawn sandboxed agent: {:?}", payload.name);
    match nanos::sandbox::execute_sandbox(payload, None, None) {
        Ok(traces) => Ok(Json(RunResponse {
            status: "success".to_string(),
            traces,
        })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Agent execution failed: {:?}", e),
        )),
    }
}

// Handler for fleet orchestration
#[derive(Deserialize)]
struct OrchestrateRequest {
    manifest_path: String,
}

async fn post_orchestrate(
    Json(payload): Json<OrchestrateRequest>,
) -> Result<Json<OrchestrateResponse>, (StatusCode, String)> {
    info!(
        "HTTP: Spawning fleet orchestration from: {}",
        payload.manifest_path
    );
    if !std::path::Path::new(&payload.manifest_path).exists() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Manifest file not found: {}", payload.manifest_path),
        ));
    }
    match nanos::orchestrator::orchestrate(&payload.manifest_path) {
        Ok(_) => Ok(Json(OrchestrateResponse {
            status: "success".to_string(),
        })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Fleet orchestration failed: {:?}", e),
        )),
    }
}

// Handler for visual debugger dashboard
async fn get_dashboard() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("web/index.html"))
}

// Handler for divergent trace replay
#[derive(Deserialize)]
struct ReplayRequest {
    manifest: AgentManifest,
    history: Vec<nanos::trace::AgentTrace>,
    override_observation: String,
}

async fn post_replay(
    Json(payload): Json<ReplayRequest>,
) -> Result<Json<RunResponse>, (StatusCode, String)> {
    info!(
        "HTTP: Spawn time-travel replay agent. Target Step: {}",
        payload.history.len() + 1
    );
    let target_step = (payload.history.len() + 1) as u32;
    let replay_config = nanos::sandbox::ReplayConfig {
        target_step,
        override_observation: payload.override_observation,
    };
    match nanos::sandbox::execute_sandbox_with_replay(
        payload.manifest,
        None,
        None,
        Some(replay_config),
    ) {
        Ok(traces) => Ok(Json(RunResponse {
            status: "success".to_string(),
            traces,
        })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Time-travel replay failed: {:?}", e),
        )),
    }
}

pub async fn start_server(host: &str, port: u16) -> Result<()> {
    let app = Router::new()
        .route("/", get(get_dashboard))
        .route("/dashboard", get(get_dashboard))
        .route("/v1/status", get(get_status))
        .route("/v1/run", post(post_run))
        .route("/v1/replay", post(post_replay))
        .route("/v1/orchestrate", post(post_orchestrate));

    let addr_str = format!("{}:{}", host, port);
    let addr: SocketAddr = addr_str.parse()?;

    info!("📊 nanos server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
