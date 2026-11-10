use std::path::PathBuf;

use acn_sim::Actor;
use clap::Parser;
use nmesh_acn_sim::{start, BlossomSetup, Config, MintSetup};
use nmesh_core::ToBech32;
use tracing_subscriber::EnvFilter;

/// ACN demo cast (live simulator): Atlas, Rogue, Verity, Muse, Solon and Gate as Nostr peers.
#[derive(Parser)]
#[command(version, about)]
struct Args {
    #[arg(
        long = "relay",
        env = "NMESH_RELAY",
        default_value = "ws://127.0.0.1:7777"
    )]
    relays: Vec<String>,
    /// Directory for the cast's keys (stable across runs so contacts survive).
    #[arg(long, default_value = ".nmesh/sim")]
    data: PathBuf,
    /// Multiply all pacing delays (0.5 = twice as fast).
    #[arg(long, default_value_t = 1.0)]
    pace: f64,
    /// Use this Cashu mint instead of hosting the test mint in-process.
    #[arg(long, env = "NMESH_MINT")]
    mint: Option<String>,
    /// Where the in-process test mint listens.
    #[arg(long, default_value = "127.0.0.1:3338")]
    mint_listen: std::net::SocketAddr,
    /// SQLite file of the in-process test mint (kept so wallets stay valid across runs).
    #[arg(long, default_value = ".nmesh/mint/mint.sqlite")]
    mint_db: PathBuf,
    /// Atlas's Blossom server for artifacts and derivation records: hosted here on this address.
    #[arg(long, default_value = "127.0.0.1:3400")]
    blossom_listen: std::net::SocketAddr,
    /// Use running Blossom servers instead (comma-separated), or `none` for inline results only.
    #[arg(long, env = "NMESH_BLOSSOM")]
    blossom: Option<String>,
    /// Real judges for the judged scenarios: a TOML panel (see crates/acn-judge/judges.example.toml).
    /// Without it, the judges are scripted.
    #[arg(long, env = "NMESH_JUDGES")]
    judges: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,nostr_relay_pool=warn".into()),
        )
        .init();
    let _ = rustls::crypto::ring::default_provider().install_default();
    let a = Args::parse();
    let judges = a
        .judges
        .as_deref()
        .map(acn_judge::Panel::load)
        .transpose()?;
    let relays = a.relays.join(", ");
    let running = start(Config {
        relays: a.relays,
        data: a.data.clone(),
        pace: a.pace,
        mint: match a.mint {
            Some(u) => MintSetup::External(u),
            None => MintSetup::Host {
                listen: a.mint_listen,
                db: Some(a.mint_db),
            },
        },
        judges,
        blossom: match a.blossom.as_deref().map(str::trim) {
            Some("none") | Some("") => BlossomSetup::None,
            Some(list) => {
                BlossomSetup::External(list.split(',').map(|x| x.trim().to_string()).collect())
            }
            None => BlossomSetup::Host {
                listen: a.blossom_listen,
                dir: Some(a.data.join("blossom")),
            },
        },
    })
    .await?;

    println!("\nACN demo cast is live on {relays}:\n");
    for actor in [Actor::Performer, Actor::Rogue] {
        println!(
            "  {:<6} {}",
            actor.display_name(),
            running.public_key(actor).to_bech32()?
        );
    }
    println!(
        "\n  Cashu test mint (fake Lightning, test sats only): {}",
        running.mint_url()
    );
    println!(
        "  Judges: {}",
        if a.judges.is_some() {
            "real (acn-judge panel)"
        } else {
            "scripted (pass --judges judges.toml for real ones)"
        }
    );
    println!(
        "\nAdd Atlas (and Rogue) in the desktop app, open the 🎬 Director, and run a scenario.\n"
    );

    tokio::signal::ctrl_c().await?;
    running.shutdown().await;
    Ok(())
}
