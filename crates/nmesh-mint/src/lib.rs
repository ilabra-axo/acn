//! A local Cashu **test mint**: the CDK mint with its fake Lightning backend.
//!
//! Tokens, blind signatures, DLEQ proofs (NUT-12), state checks (NUT-07) and HTLC/P2PK spending
//! conditions (NUT-10/11/14) are all real; only the Lightning payment is faked, so top-ups are
//! free test sats. Use it for demos and tests, never with real funds.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use cdk::mint::{MintBuilder, MintMeltLimits};
use cdk::nuts::nut00::KnownMethod;
use cdk::nuts::{CurrencyUnit, PaymentMethod};
use cdk::types::{FeeReserve, QuoteTTL};

pub struct MintConfig {
    pub listen: SocketAddr,
    /// Public URL wallets use (defaults to `http://<listen>`).
    pub url: Option<String>,
    /// SQLite file; `None` = in memory (lost on restart).
    pub db: Option<PathBuf>,
    /// Seed phrase for the mint keys; random if `None`.
    pub mnemonic: Option<String>,
}

pub struct RunningMint {
    pub url: String,
    task: tokio::task::JoinHandle<()>,
}

impl RunningMint {
    pub fn stop(self) {
        self.task.abort();
    }
}

/// A persistent mint DB needs stable keys: keep the seed phrase next to it (`<db>.seed`).
pub fn load_or_create_seed(db: &std::path::Path) -> anyhow::Result<String> {
    if let Some(dir) = db.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let f = db.with_extension("seed");
    match std::fs::read_to_string(&f) {
        Ok(s) => Ok(s.trim().to_string()),
        Err(_) => {
            let m = bip39::Mnemonic::generate(12)?.to_string();
            std::fs::write(&f, &m)?;
            Ok(m)
        }
    }
}

pub async fn start(cfg: MintConfig) -> anyhow::Result<RunningMint> {
    let db = match &cfg.db {
        Some(p) => Arc::new(cdk_sqlite::MintSqliteDatabase::new(p).await?),
        None => Arc::new(cdk_sqlite::mint::memory::empty().await?),
    };
    let mut builder = MintBuilder::new(db.clone());
    let fee_reserve = FeeReserve {
        min_fee_reserve: 1.into(),
        percent_fee_reserve: 1.0,
    };
    // Invoices are "paid" automatically after 1 s.
    let backend = cdk_fake_wallet::FakeWallet::new(
        fee_reserve,
        HashMap::default(),
        HashSet::default(),
        1,
        CurrencyUnit::Sat,
    );
    builder
        .add_payment_processor(
            CurrencyUnit::Sat,
            PaymentMethod::Known(KnownMethod::Bolt11),
            MintMeltLimits::new(1, 1_000_000),
            Arc::new(backend),
        )
        .await?;

    let listener = tokio::net::TcpListener::bind(cfg.listen)
        .await
        .with_context(|| format!("binding {}", cfg.listen))?;
    let addr = listener.local_addr()?;
    let url = cfg.url.clone().unwrap_or_else(|| format!("http://{addr}"));
    let mnemonic = match &cfg.mnemonic {
        Some(m) => bip39::Mnemonic::parse(m)?,
        None => bip39::Mnemonic::generate(12)?,
    };
    builder = builder
        .with_name("nmesh test mint".into())
        .with_description("CDK mint with a fake Lightning backend. Test sats only.".into())
        .with_urls(vec![url.clone()]);
    let mint = Arc::new(
        builder
            .build_with_seed(db, &mnemonic.to_seed_normalized(""))
            .await?,
    );
    mint.set_quote_ttl(QuoteTTL::new(10_000, 10_000)).await?;
    mint.start().await?;

    // bolt11 routes are only mounted when listed.
    let router = cdk_axum::create_mint_router(mint, vec!["bolt11".to_string()]).await?;
    let task = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, router).await {
            tracing::error!(error = %e, "mint server stopped");
        }
    });
    Ok(RunningMint { url, task })
}
