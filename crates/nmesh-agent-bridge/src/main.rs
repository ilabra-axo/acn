use std::path::PathBuf;

use clap::Parser;
use nmesh_agent_bridge::{config::AgentConfig, Bridge};
use tracing_subscriber::EnvFilter;

/// Run an MCP-backed nmesh agent.
#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// Path to agent TOML config (see agent.example.toml).
    #[arg(short, long, env = "NMESH_AGENT_CONFIG")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    // rmcp's HTTP client is built without a bundled TLS provider; install ring process-wide.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let args = Args::parse();
    let config = AgentConfig::load(&args.config)?;
    let bridge = Bridge::start(config).await?;
    bridge.run().await
}
