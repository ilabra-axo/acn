//! The CLI shim passes the L0 suite with `wc -w` as its harness, priced (T1) and free (T0), on a
//! local relay, CDK test mint and Blossom server. No MCP and no agent-bridge code involved.

use std::sync::Arc;
use std::time::Duration;

use acn_cli_shim::{CliShim, Config, Disclosure, Stdin, ToolConfig};
use acn_conformance::{l0, Env, Executions, Target};
use nostr_relay_builder::prelude::*;
use serde_json::json;

#[tokio::test]
async fn cli_shim_passes_l0_priced_and_free() -> anyhow::Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let relay = LocalRelay::new(RelayBuilder::default().rate_limit(RateLimit {
        max_reqs: 500,
        notes_per_minute: 6_000,
    }));
    relay.run().await?;
    let url = relay.url().await.to_string();
    let mint = nmesh_mint::start(nmesh_mint::MintConfig {
        listen: "127.0.0.1:0".parse()?,
        url: None,
        db: None,
        mnemonic: None,
    })
    .await?;
    let blossom = nmesh_blossom::serve("127.0.0.1:0".parse()?, None).await?;
    let env = Env {
        relays: vec![url.clone()],
        mint: Some(mint.url.clone()),
        blossom: Some(blossom.url.clone()),
    };
    for (name, price) in [("priced", 3), ("free", 0)] {
        let dir = std::env::temp_dir().join(format!("acn-cli-l0-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        // Each run of the harness appends a line, so the suite can count executions.
        let log = dir.join("runs.log");
        let script = format!("echo run >> '{}'; exec wc -w", log.display());
        let shim = CliShim::start(Config {
            id: "cli-shim".into(),
            name: "Word counter".into(),
            about: "test".into(),
            operator: "tests".into(),
            relays: vec![url.clone()],
            key_file: dir.join("agent.nsec"),
            operator_key_file: Some(dir.join("operator.nsec")),
            blossom: vec![blossom.url.clone()],
            evidence: Disclosure::Public,
            journal: None,
            price_sat: price,
            mint: (price > 0).then(|| mint.url.clone()),
            mint_capabilities: vec![
                "hashlock".into(),
                "timelock_refund".into(),
                "p2pk".into(),
                "reveals_preimage_to_payer".into(),
            ],
            wallet_dir: None,
            deadline_secs: 600,
            review_window_secs: 3600,
            timeout_secs: 20,
            tools: vec![ToolConfig {
                name: "wordcount".into(),
                description: "Count words".into(),
                command: "sh".into(),
                args: vec!["-c".into(), script],
                stdin: Stdin::Text,
                version: None,
            }],
        })
        .await?;
        let agent = shim.session().public_key().to_hex();
        tokio::spawn(async move { shim.run().await });
        tokio::time::sleep(Duration::from_millis(2000)).await;
        let report = l0(
            &env,
            &Target {
                agent,
                tool: "wordcount".into(),
                input: json!({"text": "one two three"}),
                executions: Some(Executions(Arc::new(move || {
                    std::fs::read_to_string(&log)
                        .map(|s| s.lines().count() as u64)
                        .unwrap_or(0)
                }))),
            },
        )
        .await?;
        println!("{}", report.summary());
        assert!(report.passed(), "{name}:\n{}", report.summary());
        let _ = std::fs::remove_dir_all(&dir);
    }
    blossom.stop();
    mint.stop();
    relay.shutdown();
    Ok(())
}
