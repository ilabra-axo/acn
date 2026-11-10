//! `acn-cli-shim --config shim.toml`: serve a command-line harness as an ACN performer.

use clap::Parser;

#[derive(Parser)]
struct Args {
    /// Path to the shim's TOML config (see `crates/acn-cli-shim/shim.example.toml`).
    #[arg(long, default_value = "shim.toml")]
    config: std::path::PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,nostr_relay_pool=warn".into()),
        )
        .init();
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cfg = acn_cli_shim::Config::load(&Args::parse().config)?;
    let shim = acn_cli_shim::CliShim::start(cfg).await?;
    println!(
        "acn-cli-shim: {} is serving {} tool(s)",
        shim.npub(),
        shim.tools()
    );
    shim.run().await
}
