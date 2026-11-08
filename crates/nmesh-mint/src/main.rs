use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use tracing_subscriber::EnvFilter;

/// Local Cashu test mint (CDK + fake Lightning): real tokens, test sats.
#[derive(Parser)]
#[command(version, about)]
struct Args {
    #[arg(long, default_value = "127.0.0.1:3338")]
    listen: SocketAddr,
    /// SQLite file for mint state (keys, spent proofs). Omit for in-memory.
    #[arg(long, default_value = ".nmesh/mint/mint.sqlite")]
    db: PathBuf,
    /// Keep everything in memory (forget all tokens on exit).
    #[arg(long)]
    memory: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let a = Args::parse();
    let db = (!a.memory).then_some(a.db);
    let mnemonic = match &db {
        Some(p) => Some(nmesh_mint::load_or_create_seed(p)?),
        None => None,
    };
    let mint = nmesh_mint::start(nmesh_mint::MintConfig {
        listen: a.listen,
        url: None,
        db,
        mnemonic,
    })
    .await?;
    println!(
        "\nnmesh test mint (CDK, fake Lightning) on {}\nTest sats only. Ctrl-C to stop.\n",
        mint.url
    );
    tokio::signal::ctrl_c().await?;
    mint.stop();
    Ok(())
}
