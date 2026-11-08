//! `nmesh` — a tiny terminal client for development.
//!
//! ```text
//! nmesh relay                       # run a local dev relay on ws://127.0.0.1:7777
//! nmesh whoami
//! nmesh chat <npub>                 # interactive DM; lines starting with / are agent commands
//! nmesh send <npub> "hello"
//! nmesh card <npub> "Title" "Body"  # send a structured nmesh.card/v1
//! ```

mod render;

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use nmesh_contracts::{builtin, ContractRegistry};
use nmesh_core::{Keys, PublicKey, Session, SessionConfig, ToBech32};
use nmesh_proto::StructuredMessage;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(version, about = "nmesh dev client")]
struct Cli {
    /// Secret key file (created if missing).
    #[arg(
        long,
        env = "NMESH_KEY",
        default_value = ".nmesh/me.nsec",
        global = true
    )]
    key: PathBuf,
    /// Relay URL(s). Repeat for more than one.
    #[arg(
        long = "relay",
        env = "NMESH_RELAY",
        default_value = "ws://127.0.0.1:7777",
        global = true
    )]
    relays: Vec<String>,
    /// Display name to publish.
    #[arg(long, global = true)]
    name: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run an in-memory development relay.
    Relay {
        #[arg(long, default_value_t = 7777)]
        port: u16,
    },
    /// Print this identity's npub.
    Whoami,
    /// Send one text DM.
    Send { to: String, text: String },
    /// Send a structured card.
    Card {
        to: String,
        title: String,
        body: Option<String>,
    },
    /// Interactive chat with one peer (human or agent).
    Chat { with: String },
    /// Print the inbox as it arrives.
    Listen,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()))
        .init();
    let cli = Cli::parse();

    if let Cmd::Relay { port } = cli.cmd {
        return run_relay(port).await;
    }

    let keys: Keys = nmesh_core::load_or_create_keys(&cli.key)?;
    if let Cmd::Whoami = cli.cmd {
        println!("{}", keys.public_key().to_bech32()?);
        return Ok(());
    }

    let session = Session::connect(
        keys,
        SessionConfig {
            relays: cli.relays.clone(),
            display_name: cli.name.clone(),
            ..Default::default()
        },
    )
    .await?;
    let registry = ContractRegistry::with_builtins();

    match cli.cmd {
        Cmd::Send { to, text } => {
            session.send_text(PublicKey::parse(&to)?, &text).await?;
        }
        Cmd::Card { to, title, body } => {
            let mut payload = json!({ "title": title });
            if let Some(b) = &body {
                payload["body"] = json!(b);
            }
            let alt = match &body {
                Some(b) => format!("{title}: {b}"),
                None => title.clone(),
            };
            let msg =
                StructuredMessage::new(registry.pinned_ref(builtin::CARD).unwrap(), payload, alt);
            registry.validate_message(&msg)?;
            session
                .send_structured(PublicKey::parse(&to)?, &msg)
                .await?;
        }
        Cmd::Listen => {
            let mut inbox = session.inbox().await?;
            eprintln!("listening as {}", session.public_key().to_bech32()?);
            while let Some(m) = inbox.recv().await {
                println!("{}", render::render(&registry, &m));
            }
        }
        Cmd::Chat { with } => chat(&session, &registry, PublicKey::parse(&with)?).await?,
        Cmd::Relay { .. } | Cmd::Whoami => unreachable!(),
    }
    session.shutdown().await;
    Ok(())
}

async fn chat(
    session: &Session,
    registry: &ContractRegistry,
    peer: PublicKey,
) -> anyhow::Result<()> {
    let mut inbox = session.inbox().await?;
    let mut stdin = BufReader::new(tokio::io::stdin()).lines();
    eprintln!(
        "chatting with {} as {} — ctrl-d to quit",
        peer.to_bech32()?,
        session.public_key().to_bech32()?
    );
    loop {
        tokio::select! {
            line = stdin.next_line() => match line? {
                Some(l) if !l.trim().is_empty() => { session.send_text(peer, &l).await?; }
                Some(_) => {}
                None => break,
            },
            msg = inbox.recv() => match msg {
                Some(m) if m.sender == peer => println!("{}", render::render(registry, &m)),
                Some(_) => {}
                None => break,
            },
        }
    }
    Ok(())
}

async fn run_relay(port: u16) -> anyhow::Result<()> {
    use nostr_relay_builder::prelude::*;
    // The default 60 notes/minute per connection is too low for a demo cast: every act is two
    // gift wraps, and a presenter can run several scenarios a minute.
    let builder = RelayBuilder::default()
        .addr(IpAddr::V4(Ipv4Addr::LOCALHOST))
        .port(port)
        .rate_limit(RateLimit {
            max_reqs: 500,
            notes_per_minute: 6_000,
        });
    let relay = LocalRelay::new(builder);
    relay.run().await?;
    eprintln!("dev relay (in-memory) listening on {}", relay.url().await);
    tokio::signal::ctrl_c().await?;
    relay.shutdown();
    Ok(())
}
