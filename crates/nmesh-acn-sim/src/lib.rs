//! `nmesh-acn-sim`: the ACN demo cast as real Nostr peers (library; see `main.rs`).
//!
//! ```text
//! cargo run -p nmesh-acn-sim -- --relay ws://127.0.0.1:7777
//! ```
//!
//! Prints the npubs of **Atlas** (operator-bound performer) and **Rogue** (unbound). Add them in
//! the desktop app and request a scenario (`/quick`, `/negotiate`, `/verified`, ...). Every act is
//! a real signed ACN event sent privately (Core.md §5.5); the requester's validator judges it.
//!
//! Payments are real Cashu ecash (test sats): the cast hosts `nmesh-mint` (CDK, fake Lightning)
//! unless pointed at another mint, Atlas has a CDK wallet, verifies each hash-locked token with
//! the mint before working (DLEQ + NUT-07), declines `payment_invalid` otherwise, and redeems
//! once the preimage is revealed by SATISFY or a RULING.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use acn_core::draft::Draft;
use acn_core::kinds::*;
use acn_sim::{Actor, Cast, PaymentCheck, SCENARIOS};
use acn_wallet::settlement::{advise, check_payment, Advice};
use nmesh_core::{
    Body, IdentityKind, InboxOptions, Keys, PublicKey, Session, SessionConfig, Timestamp, ToBech32,
};
use nmesh_proto::command::{ArgSpec, ArgType, SlashCommandDefinition};
use nmesh_proto::manifest::{AgentManifest, DataHandling, Hosting, ToolSummary};
use nmesh_wallet::CashuWallet;
use serde_json::{json, Value};
use tokio::sync::mpsc;

pub struct Config {
    pub relays: Vec<String>,
    /// Directory for the cast's keys (kept stable across runs so contacts survive).
    pub data: PathBuf,
    /// Multiply all pacing delays (e.g. 0.5 for a faster demo).
    pub pace: f64,
    pub mint: MintSetup,
    /// Real judges for the judged scenarios (Verity and Muse run them). `None`: scripted judges.
    pub judges: Option<acn_judge::Panel>,
}

/// Where the cast's Cashu mint comes from.
pub enum MintSetup {
    /// Run `nmesh-mint` in-process. `db = None` keeps it in memory.
    Host {
        listen: std::net::SocketAddr,
        db: Option<PathBuf>,
    },
    /// Use an already running mint (e.g. `nmesh-mint`).
    External(String),
}

/// A running cast.
pub struct Running {
    sim: Arc<Sim>,
    task: tokio::task::JoinHandle<()>,
    mint: Option<nmesh_mint::RunningMint>,
}

impl Running {
    pub fn public_key(&self, a: Actor) -> PublicKey {
        self.sim.sessions[&a].public_key()
    }

    pub fn mint_url(&self) -> &str {
        &self.sim.cast.mint
    }

    /// Atlas's wallet (the only cast member that gets paid).
    pub async fn performer_wallet(&self) -> anyhow::Result<acn_wallet::WalletView> {
        Ok(self.sim.wallet.view().await?)
    }

    pub async fn shutdown(self) {
        self.task.abort();
        if let Some(m) = self.mint {
            m.stop();
        }
        for s in self.sim.sessions.values() {
            s.shutdown().await;
        }
    }
}

#[derive(Default)]
struct Store {
    by_commitment: Mutex<HashMap<String, BTreeMap<String, acn_core::Event>>>,
    escrow: Mutex<BTreeMap<String, String>>,
    scheduled: Mutex<HashSet<String>>,
    /// commitment -> requester
    requester: Mutex<HashMap<String, PublicKey>>,
    /// Atlas's verdict on each priced REQUEST's token.
    payments: Mutex<BTreeMap<String, PaymentCheck>>,
    /// commitment -> decrypted Cashu token addressed to Atlas
    tokens: Mutex<HashMap<String, String>>,
    redeemed: Mutex<HashSet<String>>,
    /// Atlas's secret per commitment: the delivery key `k` when contingent (Core.md §8.2), and
    /// what its performer seed for held-out instances is derived from.
    keys: Mutex<BTreeMap<String, String>>,
    /// Requester seeds the runner decrypted from a REQUEST's `seed` tag (CR-003 §2.3).
    seeds: Mutex<BTreeMap<String, String>>,
}

impl Store {
    fn add(&self, e: acn_core::Event) -> (String, bool) {
        let c = if e.kind == K_REQUEST {
            e.id.clone()
        } else {
            e.tags
                .iter()
                .find(|t| t[0] == "c")
                .and_then(|t| t.get(1).cloned())
                .unwrap_or_default()
        };
        let fresh = self
            .by_commitment
            .lock()
            .unwrap()
            .entry(c.clone())
            .or_default()
            .insert(e.id.clone(), e)
            .is_none();
        (c, fresh)
    }
    fn events(&self, c: &str) -> Vec<acn_core::Event> {
        self.by_commitment
            .lock()
            .unwrap()
            .get(c)
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default()
    }
}

struct Sim {
    sessions: BTreeMap<Actor, Session>,
    cast: Cast,
    store: Store,
    dirty: mpsc::UnboundedSender<String>,
    pace: f64,
    wallet: CashuWallet,
    /// Commitments requested before the cast started are history, not work (relays replay them).
    started: u64,
    judges: Option<acn_judge::Panel>,
}

/// Connect the cast, publish profiles / `K_SVC` / bindings, and start reacting.
pub async fn start(args: Config) -> anyhow::Result<Running> {
    let (mint, mint_url) = match args.mint {
        MintSetup::Host { listen, db } => {
            let mnemonic = match &db {
                Some(p) => Some(nmesh_mint::load_or_create_seed(p)?),
                None => None,
            };
            let m = nmesh_mint::start(nmesh_mint::MintConfig {
                listen,
                url: None,
                db,
                mnemonic,
            })
            .await?;
            tracing::info!("test mint (CDK, fake Lightning) on {}", m.url);
            let url = m.url.clone();
            (Some(m), url)
        }
        MintSetup::External(u) => (None, u.trim_end_matches('/').to_string()),
    };
    let mut sessions = BTreeMap::new();
    for actor in Actor::ALL {
        let keys = nmesh_core::load_or_create_keys(
            &args
                .data
                .join(format!("{}.nsec", actor.display_name().to_lowercase())),
        )?;
        let s = Session::connect(
            keys,
            SessionConfig {
                relays: args.relays.clone(),
                kind: IdentityKind::Agent,
                display_name: Some(actor.display_name().into()),
                about: Some(actor.about().into()),
                announce: true,
            },
        )
        .await?;
        sessions.insert(actor, s);
    }
    let pk = |a: Actor| sessions[&a].public_key().to_hex();
    let mut cast = Cast {
        performer: pk(Actor::Performer),
        rogue: pk(Actor::Rogue),
        verifier: pk(Actor::Verifier),
        judge: pk(Actor::Judge),
        arbiter: pk(Actor::Arbiter),
        gatekeeper: pk(Actor::Gatekeeper),
        mint: mint_url.clone(),
        judge_panel: args
            .judges
            .as_ref()
            .map(|p| serde_json::to_value(p.fingerprints()).unwrap_or_default()),
        templates: Default::default(),
    };
    if let Some(p) = &args.judges {
        let names: Vec<String> = p
            .fingerprints()
            .iter()
            .map(|f| format!("{} {}", f.model_id, f.model_version))
            .collect();
        tracing::info!("real judges for the judged scenarios: {}", names.join(", "));
    }
    let wallet = CashuWallet::open(
        &mint_url,
        &sessions[&Actor::Performer]
            .keys()
            .secret_key()
            .secret_bytes(),
        Some(&args.data.join("wallets")),
    )
    .await?;

    let operator = nmesh_core::load_or_create_keys(&args.data.join("operator.nsec"))?;
    for agent in [Actor::Performer, Actor::Rogue] {
        advertise(&sessions[&agent], agent, &cast).await?;
    }
    bind(&sessions[&Actor::Performer], &operator).await?;
    // Public terms templates (CR-003 §2.5); the cast drafts counters against their ids.
    for (name, actor, d) in acn_sim::templates() {
        let ev = sessions[&actor].sign_act(&d, None)?;
        sessions[&actor].publish_act_pinned(&ev).await?;
        cast.templates.insert(name, ev.id);
    }

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let sim = Arc::new(Sim {
        sessions,
        cast,
        store: Store::default(),
        dirty: tx,
        pace: args.pace,
        wallet,
        started: Timestamp::now().as_secs(),
        judges: args.judges,
    });

    for agent in [Actor::Performer, Actor::Rogue] {
        let s = sim.clone();
        let mut inbox = sim.sessions[&agent]
            .inbox_with(InboxOptions::default())
            .await?;
        tokio::spawn(async move {
            while let Some(m) = inbox.recv().await {
                match m.body {
                    Body::Act(e) => s.ingest(*e, m.sender).await,
                    Body::Text(t) if agent == Actor::Performer && t.trim() == "wallet" => {
                        let msg = match s.wallet.view().await {
                            Ok(v) => format!(
                                "Atlas's wallet at {}: {} sat available, {} sat incoming (locked to me, waiting for a preimage).",
                                v.mint, v.available, v.incoming
                            ),
                            Err(e) => format!("Wallet unavailable: {e}"),
                        };
                        let _ = s.sessions[&agent].send_text(m.sender, &msg).await;
                    }
                    Body::Text(_) => {
                        let _ = s.sessions[&agent].send_text(m.sender, &help(agent)).await;
                    }
                    _ => {}
                }
            }
        });
    }

    let looped = sim.clone();
    let task = tokio::spawn(async move {
        while let Some(c) = rx.recv().await {
            looped.clone().react(&c);
        }
    });
    Ok(Running { sim, task, mint })
}

fn help(agent: Actor) -> String {
    let mut lines = vec![format!(
        "{} is an ACN demo agent. Turn on \"Request commands as commitments\" and try:",
        agent.display_name()
    )];
    for s in SCENARIOS.iter().filter(|s| s.actor == agent) {
        lines.push(format!("/{} — {} ({})", s.id, s.title, s.profile));
    }
    if agent == Actor::Performer {
        lines.push("Send \"wallet\" to see my balance.".into());
    }
    lines.join("\n")
}

/// Profile extras, nmesh manifest (consent dialog), NIP-CD commands and ACN `K_SVC` with templates.
async fn advertise(s: &Session, agent: Actor, cast: &Cast) -> anyhow::Result<()> {
    let scenarios: Vec<_> = SCENARIOS.iter().filter(|x| x.actor == agent).collect();
    let data_handling = DataHandling {
        sees_plaintext: true,
        retention: "none (demo)".into(),
        forwards_to: vec![],
    };
    let manifest = AgentManifest {
        id: format!("acn-sim-{}", agent.display_name().to_lowercase()),
        name: agent.display_name().into(),
        description: agent.about().into(),
        hosting: Hosting::SelfHosted,
        operator: "ACN demo".into(),
        data_handling: data_handling.clone(),
        tools: scenarios
            .iter()
            .map(|x| ToolSummary {
                name: x.id.into(),
                description: x.summary.into(),
                output_contract: Some("nmesh.card/v1".into()),
            })
            .collect(),
    };
    s.publish_pinned_builder(manifest.to_builder()?).await?;
    for x in &scenarios {
        let def = SlashCommandDefinition {
            id: format!("{}:{}", manifest.id, x.id),
            command: x.id.into(),
            title: x.title.into(),
            description: format!("[{}] {}", x.profile, x.summary),
            args: vec![ArgSpec {
                name: "brief".into(),
                ty: ArgType::Text,
                required: false,
                label: "What to work on".into(),
                choices: vec![],
            }],
            scopes: vec![format!("kind:{}", nmesh_proto::kinds::ACN_PRIVATE_ACT)],
        };
        s.publish_pinned_builder(def.to_builder()).await?;
    }
    let tools: Vec<Value> = scenarios
        .iter()
        .map(|x| {
            json!({
                "name": x.id,
                "description": x.summary,
                "title": x.title,
                "profile": x.profile,
                "shows": x.shows,
                "contract": "nmesh.card/v1",
                "terms_template": acn_sim::template(x.id, cast),
            })
        })
        .collect();
    let svc = json!({
        "v": VERSION,
        "tools": tools,
        "profiles": scenarios.iter().map(|x| x.profile).collect::<HashSet<_>>(),
        "price": {"min": 0, "unit": "sat"},
        // Rail capabilities per mint (Core.md §8.6, CR-002). Bindings check them before
        // committing. The CDK test mint declares NUT-07 + NUT-14, so it reveals preimages to
        // payers (Phase B spike, docs/acn/spikes/cashu-preimage-visibility.md).
        "mints": [{"url": cast.mint, "capabilities": ["hashlock", "timelock_refund", "p2pk", "reveals_preimage_to_payer"]}],
        "transport": "nostr",
        "terms_defaults": {"deadline_secs": 600, "review_window_secs": 3600},
        "silence_threshold_sat": 0,
        "data_handling": data_handling,
        "simulated": true,
    });
    let d = Draft::new(K_SVC)
        .tag(vec!["d".into(), manifest.id.clone()])
        .tag(vec![
            "alt".into(),
            "ACN service advertisement (demo)".into(),
        ])
        .content(svc);
    let ev = s.sign_act(&d, None)?;
    s.publish_act_pinned(&ev).await?;
    Ok(())
}

async fn bind(s: &Session, operator: &Keys) -> anyhow::Result<()> {
    let now = Timestamp::now().as_secs();
    let me = s.public_key();
    let (_, existing) = s.acn_public(me, Duration::from_secs(3)).await?;
    if matches!(
        acn_core::binding_status(&existing, &me.to_hex(), K_REPORT, 0, None, now),
        acn_core::BindingStatus::Bound { .. }
    ) {
        // Keep the existing binding on the relays across restarts too.
        let binding: Vec<_> = existing
            .into_iter()
            .filter(|e| matches!(e.kind, K_BIND | K_BIND_ACK | K_BIND_REVOKE))
            .collect();
        s.pin_acts(&binding);
        return Ok(());
    }
    let aux = Keys::generate().secret_key().secret_bytes();
    let bind = Draft::bind(&me.to_hex(), now + 30 * 24 * 3600, &[]).sign_with_aux(
        &operator.secret_key().secret_bytes(),
        now,
        &aux,
    )?;
    s.publish_act_pinned(&bind).await?;
    let ack = s.sign_act(&Draft::bind_ack(&bind.id), Some(now))?;
    s.publish_act_pinned(&ack).await?;
    tracing::info!(
        "Atlas bound to operator {}",
        operator.public_key().to_bech32()?
    );
    Ok(())
}

impl Sim {
    async fn ingest(self: &Arc<Self>, e: acn_core::Event, sender: PublicKey) {
        if e.kind == K_REQUEST && e.created_at + 60 < self.started {
            return;
        }
        if e.kind == K_REQUEST {
            self.store
                .requester
                .lock()
                .unwrap()
                .insert(e.id.clone(), sender);
            // Escrowed preimage for its holder (Core.md §8.2, §8.4): the arbiter or the
            // settlement verifier, named by the tag's third element (the arbiter if absent).
            if let Some(t) = e.tags.iter().find(|t| t[0] == "escrow" && t.len() > 1) {
                let holder = t
                    .get(2)
                    .and_then(|pk| self.actor_of(pk))
                    .unwrap_or(Actor::Arbiter);
                let keys = self.sessions[&holder].keys();
                match nostr_sdk::nips::nip44::decrypt(keys.secret_key(), &sender, &t[1]) {
                    Ok(s) => {
                        self.store.escrow.lock().unwrap().insert(e.id.clone(), s);
                        tracing::info!(commitment = %e.id, "{} received the escrowed preimage", holder.display_name());
                    }
                    Err(err) => tracing::warn!(error = %err, "escrow could not be decrypted"),
                }
            }
            // The requester's seed for held-out instances, for the runner it names.
            if let Some(t) = e.tags.iter().find(|t| t[0] == "seed" && t.len() > 2) {
                if let Some(runner) = self.actor_of(&t[2]) {
                    let keys = self.sessions[&runner].keys();
                    match nostr_sdk::nips::nip44::decrypt(keys.secret_key(), &sender, &t[1]) {
                        Ok(s) => {
                            self.store.seeds.lock().unwrap().insert(e.id.clone(), s);
                        }
                        Err(err) => {
                            tracing::warn!(error = %err, "requester seed could not be decrypted")
                        }
                    }
                }
            }
            if self.sessions[&Actor::Performer].public_key().to_hex() == p_tag(&e) {
                // Atlas's per-commitment secret: `k` when contingent (H = sha256(k) goes in the
                // offer); its seed for held-out instances is derived from it.
                let k = Keys::generate().secret_key().to_secret_hex();
                self.store.keys.lock().unwrap().insert(e.id.clone(), k);
                self.check_payment(&e, sender).await;
            }
        }
        let atlas = self.sessions[&Actor::Performer].public_key().to_hex();
        if e.kind == K_FUND
            && e.tags
                .iter()
                .any(|t| t[0] == "p" && t.get(1) == Some(&atlas))
        {
            self.check_fund(&e, sender).await;
        }
        let (c, fresh) = self.store.add(e);
        if fresh {
            let _ = self.dirty.send(c);
        }
    }

    fn actor_of(&self, pk: &str) -> Option<Actor> {
        self.sessions
            .iter()
            .find(|(_, s)| s.public_key().to_hex() == pk)
            .map(|(a, _)| *a)
    }

    /// Contingent (Core.md §8.2): Atlas checks the FUND token is locked to the offer's hashlock,
    /// to him, for the price, before claiming (and so revealing `k`).
    async fn check_fund(&self, e: &acn_core::Event, sender: PublicKey) {
        let Some(c) = e
            .tags
            .iter()
            .find(|t| t[0] == "c")
            .and_then(|t| t.get(1))
            .cloned()
        else {
            return;
        };
        let keys = self.sessions[&Actor::Performer].keys();
        let me = keys.public_key().to_hex();
        let content: Value = serde_json::from_str(&e.content).unwrap_or_default();
        let verdict = async {
            let ct = content["token"].as_str().ok_or("no token in FUND")?;
            let token = nostr_sdk::nips::nip44::decrypt(keys.secret_key(), &sender, ct)
                .map_err(|_| "token is not encrypted to me")?;
            let v = self
                .wallet
                .inspect(&token)
                .await
                .map_err(|e| e.to_string())?;
            let k = self
                .store
                .keys
                .lock()
                .unwrap()
                .get(&c)
                .cloned()
                .ok_or("no key")?;
            let h = acn_core::jcs::sha256_hex(&from_hex(&k).ok_or("bad key")?);
            if !v.unspent || v.hashlock != h || v.receiver != me {
                return Err("token does not match the offer".to_string());
            }
            if Some(v.amount) != content["amount"].as_u64() {
                return Err(format!("token is {} sat", v.amount));
            }
            self.wallet.note_incoming(&token, &v, &sender.to_hex(), &c);
            self.store.tokens.lock().unwrap().insert(c.clone(), token);
            Ok::<_, String>(v.amount)
        }
        .await;
        match verdict {
            Ok(n) => tracing::info!(commitment = %c, "Atlas verified the {n} sat FUND token"),
            Err(reason) => tracing::warn!(commitment = %c, %reason, "FUND token rejected"),
        }
    }

    /// Core.md §8.2 step 3: decrypt the token, verify it with the mint, check it against the terms.
    async fn check_payment(&self, e: &acn_core::Event, sender: PublicKey) {
        let content: Value = serde_json::from_str(&e.content).unwrap_or_default();
        let Some(terms) = content
            .get("terms")
            .and_then(|t| serde_json::from_value::<acn_core::Terms>(t.clone()).ok())
        else {
            return;
        };
        // Contingent requests carry no token: the requester pays later, by FUND.
        if terms.payment.is_none()
            || terms.price.amount == 0
            || content["terms"]["settlement_trigger"] == "contingent"
        {
            return;
        }
        let keys = self.sessions[&Actor::Performer].keys();
        let me = keys.public_key().to_hex();
        let verdict = async {
            let ct = content
                .get("token")
                .and_then(Value::as_str)
                .ok_or("no token in the request")?;
            let token = nostr_sdk::nips::nip44::decrypt(keys.secret_key(), &sender, ct)
                .map_err(|_| "token is not encrypted to me")?;
            let v = self
                .wallet
                .inspect(&token)
                .await
                .map_err(|e| e.to_string())?;
            check_payment(&terms, &v, &me)?;
            self.wallet
                .note_incoming(&token, &v, &sender.to_hex(), &e.id);
            self.store
                .tokens
                .lock()
                .unwrap()
                .insert(e.id.clone(), token);
            Ok::<_, String>(v.amount)
        }
        .await;
        let check = match verdict {
            Ok(amount) => {
                tracing::info!(commitment = %e.id, "Atlas verified a {amount} sat token with the mint");
                PaymentCheck::Ok
            }
            Err(reason) => {
                tracing::warn!(commitment = %e.id, %reason, "payment rejected");
                PaymentCheck::Invalid { reason }
            }
        };
        self.store
            .payments
            .lock()
            .unwrap()
            .insert(e.id.clone(), check);
    }

    /// Redeem once the preimage is public (SATISFY or a performer-side RULING).
    fn maybe_redeem(self: &Arc<Self>, c: &str, events: &[acn_core::Event]) {
        let me = self.sessions[&Actor::Performer].public_key().to_hex();
        let Some(Advice::Redeem { preimage, .. }) =
            advise(events, c, &me, Timestamp::now().as_secs())
        else {
            return;
        };
        let Some(token) = self.store.tokens.lock().unwrap().get(c).cloned() else {
            return;
        };
        if !self.store.redeemed.lock().unwrap().insert(c.to_string()) {
            return;
        }
        let this = self.clone();
        let c = c.to_string();
        tokio::spawn(async move {
            let msg = match this.wallet.redeem(&token, &preimage, Some(&c)).await {
                Ok(n) => {
                    let bal = this.wallet.balance().await.unwrap_or_default();
                    tracing::info!(commitment = %c, "Atlas redeemed {n} sat");
                    format!("Redeemed {n} sat with the revealed preimage. My balance: {bal} sat.")
                }
                Err(e) => {
                    tracing::error!(commitment = %c, error = %e, "redeem failed");
                    format!("I could not redeem the payment: {e}")
                }
            };
            let to = this.store.requester.lock().unwrap().get(&c).copied();
            if let Some(to) = to {
                let _ = this.sessions[&Actor::Performer].send_text(to, &msg).await;
            }
        });
    }

    fn react(self: Arc<Self>, c: &str) {
        let events = self.store.events(c);
        let escrow = self.store.escrow.lock().unwrap().clone();
        let payments = self.store.payments.lock().unwrap().clone();
        self.maybe_redeem(c, &events);
        let keys = self.store.keys.lock().unwrap().clone();
        let seeds = self.store.seeds.lock().unwrap().clone();
        for m in acn_sim::react_with(&self.cast, &events, &escrow, Some(&payments), &keys, &seeds) {
            // CLAIM only once the FUND token is verified (and redeem it with k first).
            if m.draft.kind == K_CLAIM && !self.store.tokens.lock().unwrap().contains_key(c) {
                continue;
            }
            if !self.store.scheduled.lock().unwrap().insert(m.key.clone()) {
                continue;
            }
            let this = self.clone();
            let c = c.to_string();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(
                    (m.delay_ms as f64 * this.pace) as u64,
                ))
                .await;
                let Some(to) = this.store.requester.lock().unwrap().get(&c).copied() else {
                    return;
                };
                let session = &this.sessions[&m.actor];
                let mut draft = m.draft.clone();
                if let Some(job) = &m.judge {
                    match this.judge(&c, job).await {
                        Some(content) => draft.content = content.to_string(),
                        None => {
                            let msg = format!("(Demo cast) {} could not run the judges, so there is no judged attestation and the review window applies. The cast's log has the error.", m.actor.display_name());
                            let _ = this.sessions[&Actor::Performer].send_text(to, &msg).await;
                            return;
                        }
                    }
                }
                if let Some(esc) = &m.escrow {
                    // The key goes to its holder, NIP-44 (and in-process, since we host both).
                    let to_pk = this.sessions[&esc.to].public_key();
                    let ct = match session.nip44_encrypt(&to_pk, &esc.secret) {
                        Ok(ct) => ct,
                        Err(e) => return tracing::error!(error = %e, "escrow encrypt failed"),
                    };
                    draft = draft.tag(vec!["escrow".into(), ct, to_pk.to_hex()]);
                    this.store
                        .escrow
                        .lock()
                        .unwrap()
                        .insert(c.clone(), esc.secret.clone());
                }
                if draft.kind == K_CLAIM {
                    let token = this.store.tokens.lock().unwrap().get(&c).cloned();
                    let k = this.store.keys.lock().unwrap().get(&c).cloned();
                    let (Some(token), Some(k)) = (token, k) else {
                        return;
                    };
                    this.store.redeemed.lock().unwrap().insert(c.clone());
                    match this.wallet.redeem(&token, &k, Some(&c)).await {
                        Ok(n) => tracing::info!(commitment = %c, "Atlas claimed {n} sat with k"),
                        Err(e) => {
                            return tracing::error!(commitment = %c, error = %e, "claim redeem failed")
                        }
                    }
                }
                let act = match session.sign_act(&draft, Some(m.not_before)) {
                    Ok(a) => a,
                    Err(e) => return tracing::error!(error = %e, "sign failed"),
                };
                tracing::info!(
                    "{} → {}: {}",
                    m.actor.display_name(),
                    name(act.kind),
                    m.narration
                );
                this.store.add(act.clone());
                if let Err(e) = session.send_act(to, &act).await {
                    tracing::error!(error = %e, "send failed");
                }
                let _ = this.dirty.send(c);
            });
        }
    }
}

impl Sim {
    /// Run the real judges on a job (a few tries: endpoints time out), or give up.
    async fn judge(&self, c: &str, job: &acn_sim::JudgeJob) -> Option<Value> {
        let panel = self.judges.as_ref()?;
        for attempt in 1..=3u64 {
            match panel
                .evaluate(&job.condition, &job.rubric, &job.task, &job.artifact)
                .await
            {
                Ok(ev) => {
                    tracing::info!(commitment = %c, "judges: {}", ev.summary);
                    for o in &ev.judges {
                        for (i, r) in o.rationales.iter().enumerate() {
                            tracing::debug!(commitment = %c, model = %o.model_id, run = i + 1, score = o.runs[i], "rationale: {r}");
                        }
                    }
                    return Some(ev.content);
                }
                // Not retryable: the terms pin judges or a rubric this panel doesn't have.
                Err(
                    e @ (acn_judge::Error::NotPinned(_)
                    | acn_judge::Error::Rubric
                    | acn_judge::Error::Terms(_)),
                ) => {
                    tracing::error!(commitment = %c, error = %e, "cannot judge");
                    return None;
                }
                Err(e) => {
                    tracing::warn!(commitment = %c, attempt, error = %e, "judging failed");
                    tokio::time::sleep(Duration::from_secs(2 * attempt)).await;
                }
            }
        }
        tracing::error!(commitment = %c, "judging failed 3 times; no attestation");
        None
    }
}

fn p_tag(e: &acn_core::Event) -> String {
    e.tags
        .iter()
        .find(|t| t[0] == "p")
        .and_then(|t| t.get(1))
        .cloned()
        .unwrap_or_default()
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}
