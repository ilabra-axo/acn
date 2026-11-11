//! Conformance suites for the Binding API (docs/harness-binding-API.md §2). Passing a level's
//! suite is the only claim a binding may make about its level (`K_SVC` `binding_level`).
//!
//! [`l0`] is the L0 suite (Transactional: T0 and T1). A scripted requester, with its own key and
//! a wallet at the mint, discovers the binding, sends it requests over the relays, pays it and
//! closes the commitments, then checks:
//!
//! - **Discovery:** a `K_SVC` that declares `binding_level` L0, the profiles it serves, its mints
//!   and their capabilities, its Blossom servers (also as a BUD-03 server list), and the tool;
//!   an operator binding in force.
//! - **T0** (when the price is 0): a `REPORT` whose artifacts and derivation record can be
//!   fetched from those servers and match their hashes; `SATISFY` closes it. A priced binding
//!   must decline a free request instead. The same with the terms sent by reference.
//! - **T1** (when offered): a hash-locked token in the `REQUEST`; the `REPORT` with the same
//!   evidence checks; `SATISFY` with the preimage; the binding redeems the token. The same with
//!   the terms sent by reference. A token whose lock doesn't match the terms is declined
//!   `payment_invalid`.
//! - **Declines:** an `open` condition, profile N, held-out instances, an unknown tool and an
//!   expired deadline are each answered with `DECLINE`, never with work (no `REPORT`).
//! - **Executions** (when the harness exposes a counter, [`Target::executions`]): every served
//!   request ran the executor exactly once, every declined one not at all.
//! - **Clean acts:** no act the binding signs carries any anomaly from the reference validator.
//! - **Paid terms with too early a locktime** (Core §8.2: the refund path must open after the
//!   deadline, the review window and a redeem margin) are declined `locktime_too_early`.
//!
//! [`l1`] is the L1 suite (Negotiated: profile N on top of L0). Every L0 case runs (a binding
//! that serves N no longer has to decline it), plus:
//!
//! - **Promise:** an N request is answered with `PROMISE` binding the request's terms hash, then
//!   a `REPORT` answering the promise, with the L0 evidence checks; `SATISFY` closes it (and the
//!   token is redeemed when priced).
//! - **Counter closing open conditions:** a request with an `open` condition gets a `COUNTER`
//!   whose terms are closed, and no work; after the requester's `ACCEPT`, a `REPORT`.
//! - **Requester counters:** a closed counter the binding can meet (a later deadline) is
//!   `ACCEPT`ed and worked under; one it can't (an expired deadline) is declined, with no work.
//! - **Rework:** a `REJECT` naming a condition, within the rework limit, gets a second `REPORT`
//!   answering it (two executions).
//! - **Held-out instances:** the binding act carries `performer_seed_hash` and the `REPORT`
//!   reveals a `performer_seed` that hashes to it.
//! - **Disqualifiers:** terms with one are served; after a counted hit and a `REJECT` naming it,
//!   the commitment ends Rejected and the binding does no rework.
//! - **Disputes:** a rejection contradicted by a passing attestation from a listed verifier,
//!   under terms naming an arbiter, is disputed; the arbiter's ruling for the performer settles
//!   it (and its preimage is redeemed when priced).
//! - **Governance** by a key the binding's gate doesn't hold is declined
//!   `governance_unsupported`.
//!
//! [`l2`] is the L2 suite (Governed: L1 plus a gate). `K_SVC` names the gate's governance key
//! and policy (`governance`), and then:
//!
//! - **Endorsed acts:** terms requiring endorsed `PROMISE` and `REPORT` by that key get both, each
//!   with a `K_ENDORSE` from the key, naming the act, with the advertised policy hash, inside the
//!   window; the validator applies them (no `pending_endorsement`) and `SATISFY` closes it. The
//!   same for a T-profile `REPORT`.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use acn_core::draft::{Draft, Template};
use acn_core::kinds::*;
use acn_core::{fold, Event, Phase};
use nmesh_core::{Body, IdentityKind, Keys, PublicKey, Session, SessionConfig, Timestamp};
use serde::Serialize;
use serde_json::{json, Value};

/// Where the suite runs: the relays the binding listens on, and the mint (for T1).
#[derive(Debug, Clone)]
pub struct Env {
    pub relays: Vec<String>,
    pub mint: Option<String>,
    /// The requester's Blossom server. Set it to test terms sent by reference (Core §7).
    pub blossom: Option<String>,
}

/// The binding under test and a call it can serve.
#[derive(Debug, Clone)]
pub struct Target {
    /// Agent key, hex.
    pub agent: String,
    pub tool: String,
    pub input: Value,
    /// How many times the binding's executor has run so far, if the harness can tell (e.g. a
    /// test executor's call log). With it the suite also checks that every declined request
    /// caused no work and every served one exactly one run.
    pub executions: Option<Executions>,
}

/// A counter of executor runs, read by the suite (see [`Target::executions`]).
#[derive(Clone)]
pub struct Executions(pub Arc<dyn Fn() -> u64 + Send + Sync>);

impl std::fmt::Debug for Executions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Executions({})", (self.0)())
    }
}

impl Target {
    fn executions(&self) -> Option<u64> {
        self.executions.as_ref().map(|e| (e.0)())
    }

    /// `want` runs since `before` (a count taken before the request), if the count is known.
    fn ran(&self, before: Option<u64>, want: u64) -> Result<String, String> {
        match (before, self.executions()) {
            (Some(b), Some(n)) if n.saturating_sub(b) == want => Ok(format!("{want} execution(s)")),
            (Some(b), Some(n)) => Err(format!(
                "the executor ran {} time(s), expected {want}",
                n.saturating_sub(b)
            )),
            _ => Ok("execution count not observable".into()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Case {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub level: &'static str,
    pub agent: String,
    pub cases: Vec<Case>,
}

impl Report {
    pub fn passed(&self) -> bool {
        !self.cases.is_empty() && self.cases.iter().all(|c| c.passed)
    }
    pub fn summary(&self) -> String {
        let mut s = format!(
            "{} suite for {}: {}\n",
            self.level,
            &self.agent[..12.min(self.agent.len())],
            if self.passed() { "PASS" } else { "FAIL" }
        );
        for c in &self.cases {
            s.push_str(&format!(
                "  {} {}: {}\n",
                if c.passed { "ok  " } else { "FAIL" },
                c.name,
                c.detail
            ));
        }
        s
    }
}

const WAIT: Duration = Duration::from_secs(30);

struct Requester {
    session: Session,
    agent: PublicKey,
    acts: Arc<Mutex<Vec<Event>>>,
    sent: Mutex<Vec<Event>>,
    /// The binding's public events (operator binding, templates): folded with every commitment.
    public: Mutex<Vec<Event>>,
    /// Terms documents the suite sent by reference, folded with every commitment.
    docs: Mutex<BTreeMap<String, Value>>,
}

impl Requester {
    async fn connect(env: &Env, agent: &str) -> anyhow::Result<Self> {
        let session = Session::connect(
            Keys::generate(),
            SessionConfig {
                relays: env.relays.clone(),
                kind: IdentityKind::Human,
                display_name: Some("ACN conformance".into()),
                about: None,
                announce: true,
            },
        )
        .await?;
        let acts = Arc::new(Mutex::new(Vec::new()));
        let mut inbox = session.inbox().await?;
        let sink = acts.clone();
        tokio::spawn(async move {
            while let Some(m) = inbox.recv().await {
                if let Body::Act(e) = m.body {
                    sink.lock().unwrap().push(*e);
                }
            }
        });
        tokio::time::sleep(Duration::from_millis(300)).await;
        Ok(Self {
            session,
            agent: PublicKey::parse(agent)?,
            acts,
            sent: Mutex::new(Vec::new()),
            public: Mutex::new(Vec::new()),
            docs: Mutex::new(BTreeMap::new()),
        })
    }

    fn me(&self) -> String {
        self.session.public_key().to_hex()
    }

    fn events(&self, c: &str) -> Vec<Event> {
        let of = |e: &Event| {
            e.id == c
                || e.tags.iter().any(|t| {
                    t.first().map(String::as_str) == Some("c")
                        && t.get(1).map(String::as_str) == Some(c)
                })
        };
        let mut v: Vec<Event> = self
            .sent
            .lock()
            .unwrap()
            .iter()
            .filter(|e| of(e))
            .cloned()
            .collect();
        v.extend(self.acts.lock().unwrap().iter().filter(|e| of(e)).cloned());
        v.extend(self.public.lock().unwrap().iter().cloned());
        v
    }

    async fn send(&self, d: &Draft, not_before: Option<u64>) -> anyhow::Result<Event> {
        let act = self.session.sign_act(d, not_before)?;
        self.session.send_act(self.agent, &act).await?;
        self.sent.lock().unwrap().push(act.clone());
        Ok(act)
    }

    async fn request(
        &self,
        profile: &str,
        terms: &Value,
        token_ct: Option<&str>,
    ) -> anyhow::Result<String> {
        let d = Draft::request(&self.agent.to_hex(), profile, terms, token_ct)?;
        Ok(self.send(&d, None).await?.id)
    }

    /// The binding's answer to commitment `c`: its first act of one of `kinds`.
    async fn answer(&self, c: &str, kinds: &[u16]) -> Option<Event> {
        let agent = self.agent.to_hex();
        let deadline = tokio::time::Instant::now() + WAIT;
        while tokio::time::Instant::now() < deadline {
            if let Some(e) = self
                .events(c)
                .into_iter()
                .find(|e| e.pubkey == agent && kinds.contains(&e.kind))
            {
                return Some(e);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        None
    }

    fn docs(&self) -> BTreeMap<String, Value> {
        self.docs.lock().unwrap().clone()
    }

    /// The binding's first act of one of `kinds` answering `prev`.
    async fn answer_to(&self, c: &str, prev: &str, kinds: &[u16]) -> Option<Event> {
        let agent = self.agent.to_hex();
        let deadline = tokio::time::Instant::now() + WAIT;
        while tokio::time::Instant::now() < deadline {
            if let Some(e) = self.events(c).into_iter().find(|e| {
                e.pubkey == agent && kinds.contains(&e.kind) && e.e_marked("prev").contains(&prev)
            }) {
                return Some(e);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        None
    }

    /// Our next act on `c` (or one signed with `key`, e.g. the arbiter's), after the head.
    async fn next(
        &self,
        c: &str,
        key: Option<&Keys>,
        d: impl FnOnce(&str, &[&str]) -> Draft,
    ) -> Result<Event, String> {
        let (cm, st) = self.state(c).ok_or("no state")?;
        let parties = [cm.requester.as_str(), cm.performer.as_str()];
        let draft = d(&st.head, &parties);
        match key {
            None => self
                .send(&draft, Some(st.head_at))
                .await
                .map_err(|e| e.to_string()),
            Some(k) => {
                let at = Timestamp::now().as_secs().max(st.head_at);
                let act = draft
                    .sign(&k.secret_key().secret_bytes(), at)
                    .map_err(|e| e.to_string())?;
                self.session
                    .send_act(self.agent, &act)
                    .await
                    .map_err(|e| e.to_string())?;
                self.sent.lock().unwrap().push(act.clone());
                Ok(act)
            }
        }
    }

    /// An annotating act signed by us (an attestation), delivered to the binding.
    async fn annotate(&self, d: &Draft) -> Result<Event, String> {
        self.send(d, None).await.map_err(|e| e.to_string())
    }

    /// The binding's acts of `kind` in `c`.
    fn agent_acts(&self, c: &str, kind: u16) -> usize {
        let agent = self.agent.to_hex();
        self.events(c)
            .iter()
            .filter(|e| e.pubkey == agent && e.kind == kind)
            .count()
    }

    fn state(&self, c: &str) -> Option<(acn_core::Commitment, acn_core::State)> {
        let out = fold(&self.events(c), &self.docs());
        let cm = out.commitments.get(c)?.clone();
        let st = cm.state.clone()?;
        Some((cm, st))
    }

    /// Anomalies on any act the binding signed in `c`.
    fn agent_anomalies(&self, c: &str) -> Vec<String> {
        let events = self.events(c);
        let agent = self.agent.to_hex();
        let public: Vec<String> = self
            .public
            .lock()
            .unwrap()
            .iter()
            .map(|e| e.id.clone())
            .collect();
        let mine: Vec<&str> = events
            .iter()
            .filter(|e| e.pubkey == agent && !public.contains(&e.id))
            .map(|e| e.id.as_str())
            .collect();
        fold(&events, &self.docs())
            .anomalies
            .into_iter()
            .filter(|a| mine.contains(&a.event.as_str()))
            .map(|a| format!("{} ({})", a.code.as_str(), a.detail))
            .collect()
    }

    async fn satisfy(&self, c: &str, preimage: Option<&str>) -> anyhow::Result<Phase> {
        let (cm, st) = self.state(c).ok_or_else(|| anyhow::anyhow!("no state"))?;
        let parties = [cm.requester.as_str(), cm.performer.as_str()];
        self.send(
            &Draft::satisfy(c, &st.head, &parties, preimage),
            Some(st.head_at),
        )
        .await?;
        Ok(self
            .state(c)
            .map(|(_, s)| s.phase)
            .unwrap_or(Phase::Requested))
    }
}

fn reason(e: &Event) -> String {
    serde_json::from_str::<Value>(&e.content)
        .ok()
        .and_then(|v| v["reason"].as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The `REPORT`'s evidence: the derivation record and every `x` artifact fetch from the
/// binding's Blossom servers and match their hashes; the record lists the artifacts.
async fn check_evidence(report: &Event, servers: &[String]) -> Result<String, String> {
    // The binding's own advertised servers (local ones in a test setup).
    let policy = nmesh_blossom::FetchPolicy::trusting(servers);
    let content: Value = serde_json::from_str(&report.content).map_err(|e| e.to_string())?;
    let d = content["derivation"]
        .as_str()
        .ok_or("the REPORT carries no derivation record")?;
    let bytes = nmesh_blossom::fetch_from(servers, d, &policy)
        .await
        .map_err(|e| format!("derivation record: {e}"))?;
    let record: Value =
        serde_json::from_slice(&bytes).map_err(|_| "the derivation record is not JSON")?;
    for f in ["inputs", "outputs", "tools"] {
        if !record[f].is_array() {
            return Err(format!("the derivation record has no `{f}`"));
        }
    }
    let listed: Vec<&str> = record["outputs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|o| o["hash"].as_str())
        .collect();
    let x: Vec<&str> = report
        .tags
        .iter()
        .filter(|t| t.first().map(String::as_str) == Some("x"))
        .filter_map(|t| t.get(1).map(String::as_str))
        .collect();
    if x.is_empty() {
        return Err("the REPORT lists no artifacts (`x` tags)".into());
    }
    for h in &x {
        if !listed.contains(h) {
            return Err(format!("artifact {h} is not in the derivation record"));
        }
        nmesh_blossom::fetch_from(servers, h, &policy)
            .await
            .map_err(|e| format!("artifact: {e}"))?;
    }
    Ok(format!(
        "{} artifact(s) and the derivation record fetched and verified",
        x.len()
    ))
}

fn base_terms(tool: &str, input: &Value, price: u64) -> Value {
    let now = Timestamp::now().as_secs();
    acn_core::draft::tool_call_terms(tool, input.clone(), now + 600, 3600, price)
}

/// Run the L0 suite against `target`.
pub async fn l0(env: &Env, target: &Target) -> anyhow::Result<Report> {
    suite(env, target, acn_binding::Level::L0).await
}

/// Run the L1 suite against `target`: every L0 case, then profile N.
pub async fn l1(env: &Env, target: &Target) -> anyhow::Result<Report> {
    suite(env, target, acn_binding::Level::L1).await
}

/// Run the L2 suite against `target`: every L1 case, then governed acts.
pub async fn l2(env: &Env, target: &Target) -> anyhow::Result<Report> {
    suite(env, target, acn_binding::Level::L2).await
}

fn parse_level(v: &Value) -> Option<acn_binding::Level> {
    use acn_binding::Level::*;
    [L0, L1, L2, L3]
        .into_iter()
        .find(|l| v.as_str() == Some(l.as_str()))
}

async fn suite(env: &Env, target: &Target, level: acn_binding::Level) -> anyhow::Result<Report> {
    let r = Requester::connect(env, &target.agent).await?;
    let mut cases = Vec::new();
    let mut case = |name: &str, res: Result<String, String>| {
        let (passed, detail) = match res {
            Ok(d) => (true, d),
            Err(d) => (false, d),
        };
        cases.push(Case {
            name: name.into(),
            passed,
            detail,
        });
    };

    // ---- discovery ----
    let (svc, public) = r
        .session
        .acn_public(r.agent, Duration::from_secs(5))
        .await?;
    let svc = svc.unwrap_or(Value::Null);
    *r.public.lock().unwrap() = public.clone();
    let now = Timestamp::now().as_secs();
    let min_price = svc["price"]["min"].as_u64().unwrap_or(0);
    let profiles: Vec<&str> = svc["profiles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let blossom: Vec<String> = svc["blossom"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_owned))
        .collect();
    let tool = svc["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t["name"] == target.tool.as_str())
        .cloned();
    case(
        "discovery",
        (|| {
            if svc.is_null() {
                return Err("no K_SVC service advertisement".to_string());
            }
            match parse_level(&svc["binding_level"]) {
                Some(l) if l >= level => {}
                _ => {
                    return Err(format!(
                        "K_SVC binding_level is {}, not {} or above",
                        svc["binding_level"],
                        level.as_str()
                    ))
                }
            }
            if level >= acn_binding::Level::L1 && !profiles.contains(&"N") {
                return Err(format!("profiles {profiles:?}: an L1 binding serves N"));
            }
            if level >= acn_binding::Level::L2
                && svc["governance"]["keys"]
                    .as_array()
                    .is_none_or(|k| k.is_empty())
            {
                return Err("an L2 binding names its gate's governance key in K_SVC".into());
            }
            if !profiles.contains(&"T0") && !profiles.contains(&"T1") {
                return Err(format!(
                    "profiles {profiles:?}: an L0 binding serves T0 or T1"
                ));
            }
            if min_price > 0 && !profiles.contains(&"T1") {
                return Err("priced, but T1 isn't offered".into());
            }
            if profiles.contains(&"T1") {
                let ok = svc["mints"].as_array().into_iter().flatten().any(|m| {
                    m["capabilities"]
                        .as_array()
                        .is_some_and(|c| c.iter().any(|x| x == "hashlock"))
                });
                if !ok {
                    return Err("T1 offered, but no mint declares the hashlock capability".into());
                }
            }
            if blossom.is_empty() {
                return Err("K_SVC names no Blossom servers for evidence".into());
            }
            if tool.is_none() {
                return Err(format!("K_SVC doesn't list the tool `{}`", target.tool));
            }
            match acn_core::binding_status(&public, &target.agent, K_REPORT, min_price, None, now) {
                acn_core::BindingStatus::Bound { .. } => Ok(format!(
                    "K_SVC {}, profiles {profiles:?}, {} Blossom server(s), operator-bound",
                    svc["binding_level"].as_str().unwrap_or_default(),
                    blossom.len()
                )),
                other => Err(format!("no operator binding in force: {other:?}")),
            }
        })(),
    );
    let servers_listed = r
        .session
        .client()
        .fetch_events(
            nostr_sdk::Filter::new()
                .author(r.agent)
                .kind(nostr_sdk::Kind::Custom(nmesh_blossom::SERVER_LIST_KIND)),
            Duration::from_secs(5),
        )
        .await
        .map(|evs| {
            evs.iter().any(|e| {
                blossom
                    .iter()
                    .all(|s| e.tags.iter().any(|t| t.content() == Some(s.as_str())))
            })
        })
        .unwrap_or(false);
    case(
        "blossom-server-list",
        if servers_listed {
            Ok("BUD-03 list names the K_SVC servers".into())
        } else {
            Err("no kind 10063 server list naming the K_SVC servers".into())
        },
    );

    // ---- T0 ----
    let free = base_terms(&target.tool, &target.input, 0);
    let before = target.executions();
    let c = r.request("T0", &free, None).await?;
    if min_price == 0 {
        let res = async {
            let rep = r
                .answer(&c, &[K_REPORT, K_DECLINE])
                .await
                .ok_or("no answer to a T0 request")?;
            if rep.kind != K_REPORT {
                return Err(format!(
                    "declined a T0 call it advertises: {}",
                    reason(&rep)
                ));
            }
            let ev = check_evidence(&rep, &blossom).await?;
            let runs = target.ran(before, 1)?;
            let phase = r.satisfy(&c, None).await.map_err(|e| e.to_string())?;
            if phase != Phase::Satisfied {
                return Err(format!("SATISFY left it {phase:?}"));
            }
            Ok(format!("REPORT; {ev}; {runs}; Satisfied"))
        }
        .await;
        case("t0-call", res);
    } else {
        let res = match r.answer(&c, &[K_REPORT, K_DECLINE]).await {
            Some(e) if e.kind == K_DECLINE => target
                .ran(before, 0)
                .map(|runs| format!("declined ({}); {runs}", reason(&e))),
            Some(_) => Err(format!("did work for 0 sat below its price of {min_price}")),
            None => Err("no answer".into()),
        };
        case("free-call-below-price-declined", res);
    }
    case("t0-acts-clean", clean(&r, &c));

    // ---- terms by reference (Core §7): on the requester's Blossom server, not inline ----
    if let (Some(server), true) = (&env.blossom, min_price == 0) {
        let res = async {
            let terms = base_terms(&target.tool, &target.input, 0);
            let client =
                nmesh_blossom::BlossomClient::new(vec![server.clone()], r.session.keys().clone());
            let bytes = acn_core::jcs::canonical(&terms).map_err(|e| e.to_string())?;
            let (h, _) = client
                .upload(bytes.as_bytes(), "application/json")
                .await
                .map_err(|e| e.to_string())?;
            r.docs.lock().unwrap().insert(h.clone(), terms);
            let before = target.executions();
            let d = Draft::request_ref(&target.agent, "T0", &h, server, None);
            let ev = r.send(&d, None).await.map_err(|e| e.to_string())?;
            let rep = r
                .answer(&ev.id, &[K_REPORT, K_DECLINE])
                .await
                .ok_or("no answer to a by-reference request")?;
            if rep.kind != K_REPORT {
                return Err(format!(
                    "declined terms sent by reference: {}",
                    reason(&rep)
                ));
            }
            let evidence = check_evidence(&rep, &blossom).await?;
            let runs = target.ran(before, 1)?;
            clean(&r, &ev.id)?;
            Ok(format!(
                "fetched terms {}… from the hint and REPORTed against them; {evidence}; {runs}",
                &h[..12]
            ))
        }
        .await;
        case("t0-terms-by-reference", res);
    }

    // ---- T1 ----
    if profiles.contains(&"T1") {
        let mint = env
            .mint
            .clone()
            .ok_or_else(|| anyhow::anyhow!("T1 needs Env.mint"))?;
        let wallet = nmesh_wallet::CashuWallet::open(
            &mint,
            &r.session.keys().secret_key().secret_bytes(),
            None,
        )
        .await?;
        let offer: Option<Template> = tool
            .as_ref()
            .and_then(|t| serde_json::from_value(t["terms_template"].clone()).ok());
        let price = min_price.max(1);
        wallet.top_up(price * 4).await?;
        let paid = |hashlock: &str| -> anyhow::Result<Value> {
            match offer.as_ref().filter(|o| o.profile == "T1") {
                Some(o) => acn_core::draft::instantiate(
                    o,
                    &target.tool,
                    target.input.clone(),
                    Timestamp::now().as_secs(),
                    Some(hashlock),
                    None,
                )
                .map_err(anyhow::Error::msg),
                None => anyhow::bail!("the tool has no T1 terms offer"),
            }
        };
        let call = Paid {
            r: &r,
            wallet: &wallet,
            target,
            blossom: &blossom,
        };
        // A paid call.
        let s = Keys::generate().secret_key().to_secret_hex();
        let h = acn_core::jcs::sha256_hex(&hex_bytes(&s));
        let res = async {
            let terms = paid(&h).map_err(|e| e.to_string())?;
            call.run(terms, &s, &h, None).await
        }
        .await;
        case("t1-paid-call", res);
        // The same, with the terms on the requester's Blossom server (Core §7): the same
        // evidence, settlement and redemption checks.
        if let Some(server) = &env.blossom {
            let s2 = Keys::generate().secret_key().to_secret_hex();
            let h2 = acn_core::jcs::sha256_hex(&hex_bytes(&s2));
            let res = async {
                let terms = paid(&h2).map_err(|e| e.to_string())?;
                call.run(terms, &s2, &h2, Some(server)).await
            }
            .await;
            case("t1-terms-by-reference", res);
        }
        // A token whose lock doesn't match the terms.
        let res = async {
            let terms = paid(&h).map_err(|e| e.to_string())?;
            let amount = terms["price"]["amount"].as_u64().unwrap_or(0);
            let locktime = terms["payment"]["locktime"].as_u64().unwrap_or(0);
            let other = acn_core::jcs::sha256_hex(b"another secret");
            let token = wallet
                .lock_htlc(amount, &other, &target.agent, locktime, None)
                .await
                .map_err(|e| e.to_string())?;
            let ct = r
                .session
                .nip44_encrypt(&r.agent, &token)
                .map_err(|e| e.to_string())?;
            let before = target.executions();
            let c = r
                .request("T1", &terms, Some(&ct))
                .await
                .map_err(|e| e.to_string())?;
            let ans = r
                .answer(&c, &[K_REPORT, K_DECLINE])
                .await
                .ok_or("no answer")?;
            clean(&r, &c)?;
            if ans.kind != K_DECLINE {
                return Err("did the work against a token locked to another hash".into());
            }
            if reason(&ans) != "payment_invalid" {
                return Err(format!(
                    "declined for `{}`, expected `payment_invalid`",
                    reason(&ans)
                ));
            }
            let runs = target.ran(before, 0)?;
            Ok(format!("declined (payment_invalid); {runs}"))
        }
        .await;
        case("t1-bad-token-declined", res);
    }
    // ---- what L0 must decline, each for its own reason ----
    // Otherwise valid terms: the tool's paid offer when priced (the refusal comes before the
    // token is looked at), else a free call.
    let paid_offer: Option<Template> = tool
        .as_ref()
        .and_then(|t| serde_json::from_value(t["terms_template"].clone()).ok())
        .filter(|o: &Template| o.profile == "T1");
    let base = |tool_name: &str| -> Value {
        match (&paid_offer, min_price > 0) {
            (Some(o), true) => acn_core::draft::instantiate(
                o,
                tool_name,
                target.input.clone(),
                Timestamp::now().as_secs(),
                Some(&"ab".repeat(32)),
                None,
            )
            .unwrap_or_else(|_| base_terms(tool_name, &target.input, min_price)),
            _ => base_terms(tool_name, &target.input, min_price),
        }
    };
    let p1 = if min_price > 0 { "T1" } else { "T0" };
    let serves_n = profiles.contains(&"N");
    let mut declines: Vec<(&str, &str, &str, Value)> = vec![
        ("decline-open-condition", p1, "terms_unclosed", {
            let mut t = base(&target.tool);
            t["conditions"] = json!([{"id": "c1", "class": "open", "description": "Do it well", "suggested_class": "human"}]);
            t
        }),
        (
            "decline-profile-n",
            "N",
            "profile_unsupported",
            base(&target.tool),
        ),
        ("decline-held-out", p1, "instances_unsupported", {
            let mut t = base(&target.tool);
            t["verifiers"] = json!([{"pubkey": r.me(), "classes": ["mechanical"]}]);
            t["conditions"] = json!([{"id": "c1", "class": "mechanical", "description": "held-out tests pass",
                "instances": {"pool_description": "cases", "pool_rule": "curated", "pool_root": "cd".repeat(32),
                    "pool_size": 4, "sample_size": 2, "requester_seed_hash": "ef".repeat(32)}}]);
            t
        }),
        (
            "decline-unknown-tool",
            p1,
            "unknown_tool",
            base("no-such-tool-acn-conformance"),
        ),
        ("decline-expired", p1, "expired", {
            let mut t = base(&target.tool);
            t["deadline"] = json!(Timestamp::now().as_secs().saturating_sub(60));
            t
        }),
    ];
    // A binding that serves N has nothing to decline there.
    declines.retain(|(name, ..)| !(serves_n && *name == "decline-profile-n"));
    if min_price > 0 && paid_offer.is_some() {
        declines.push(("decline-locktime-too-early", "T1", "locktime_too_early", {
            let mut t = base(&target.tool);
            t["payment"]["locktime"] = t["deadline"].clone();
            t
        }));
    }
    if level >= acn_binding::Level::L1 {
        declines.push(("n-decline-governed", "N", "governance_unsupported", {
            let mut t = base(&target.tool);
            t["governance"] =
                json!({"keys": [r.me()], "kinds": [K_PROMISE, K_REPORT], "window": 600});
            t
        }));
    }
    for (name, profile, want, terms) in declines {
        let before = target.executions();
        let c = r.request(profile, &terms, None).await?;
        let res = match r
            .answer(&c, &[K_REPORT, K_DECLINE, K_PROMISE, K_COUNTER])
            .await
        {
            Some(e) if e.kind == K_DECLINE && reason(&e) == want => clean(&r, &c)
                .and_then(|_| no_work(&r, target, &c, before))
                .map(|runs| format!("declined ({want}); {runs}")),
            Some(e) if e.kind == K_DECLINE => {
                Err(format!("declined for `{}`, expected `{want}`", reason(&e)))
            }
            Some(e) => Err(format!(
                "answered with {} instead of DECLINE",
                acn_core::kinds::name(e.kind)
            )),
            None => Err("no answer".into()),
        };
        case(name, res);
    }
    if level >= acn_binding::Level::L1 {
        let wallet = match (&env.mint, min_price > 0) {
            (Some(mint), true) => {
                let w = nmesh_wallet::CashuWallet::open(
                    mint,
                    &r.session.keys().secret_key().secret_bytes(),
                    None,
                )
                .await?;
                w.top_up(min_price.max(1) * 12).await?;
                Some(w)
            }
            _ => None,
        };
        let n = Negotiated {
            r: &r,
            target,
            blossom: &blossom,
            wallet: wallet.as_ref(),
            offer: paid_offer.clone(),
            min_price,
        };
        for (name, res) in n.cases().await {
            case(name, res);
        }
        if level >= acn_binding::Level::L2 {
            for (name, res) in n.governed(&svc["governance"]).await {
                case(name, res);
            }
        }
    }
    r.session.shutdown().await;
    Ok(Report {
        level: level.as_str(),
        agent: target.agent.clone(),
        cases,
    })
}

/// The L1 cases (profile N), against a free or a priced binding.
struct Negotiated<'a> {
    r: &'a Requester,
    target: &'a Target,
    blossom: &'a [String],
    /// The requester's wallet, when the binding is priced.
    wallet: Option<&'a nmesh_wallet::CashuWallet>,
    /// The tool's paid terms offer (priced bindings).
    offer: Option<Template>,
    min_price: u64,
}

/// A request the suite sent: its id, and the payment secret and token when priced.
struct Sent {
    c: String,
    preimage: Option<String>,
    token: Option<String>,
}

impl Negotiated<'_> {
    /// Terms the binding can meet: the tool's paid offer when priced (with a fresh hashlock),
    /// else a free call; then `f`.
    fn terms(&self, hashlock: Option<&str>, f: impl FnOnce(&mut Value)) -> Value {
        let mut t = match (&self.offer, hashlock) {
            (Some(o), Some(h)) => acn_core::draft::instantiate(
                o,
                &self.target.tool,
                self.target.input.clone(),
                Timestamp::now().as_secs(),
                Some(h),
                None,
            )
            .unwrap_or_else(|_| base_terms(&self.target.tool, &self.target.input, self.min_price)),
            _ => base_terms(&self.target.tool, &self.target.input, 0),
        };
        f(&mut t);
        t
    }

    /// Send an N request with terms from `f` (and a token locked to them when priced). With
    /// `arbiter`, the payment secret is escrowed to it (Core §8.4).
    async fn request(
        &self,
        arbiter: Option<&Keys>,
        f: impl FnOnce(&mut Value),
    ) -> Result<Sent, String> {
        let r = self.r;
        let secret = self
            .wallet
            .map(|_| Keys::generate().secret_key().to_secret_hex());
        let h = secret
            .as_deref()
            .map(|s| acn_core::jcs::sha256_hex(&hex_bytes(s)));
        let mut terms = self.terms(h.as_deref(), f);
        // Each case's request is its own commitment: identical terms signed in the same second
        // would be the same event.
        let nonce = &Keys::generate().public_key().to_hex()[..8];
        let desc = terms["deliverable"]["description"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        terms["deliverable"]["description"] = json!(format!("{desc} (conformance {nonce})"));
        let token = match (self.wallet, &h) {
            (Some(w), Some(h)) => Some(
                w.lock_htlc(
                    terms["price"]["amount"].as_u64().unwrap_or(0),
                    h,
                    &self.target.agent,
                    terms["payment"]["locktime"].as_u64().unwrap_or(0),
                    None,
                )
                .await
                .map_err(|e| e.to_string())?,
            ),
            _ => None,
        };
        let ct = match &token {
            Some(t) => Some(
                r.session
                    .nip44_encrypt(&r.agent, t)
                    .map_err(|e| e.to_string())?,
            ),
            None => None,
        };
        let mut d = Draft::request(&self.target.agent, "N", &terms, ct.as_deref())
            .map_err(|e| e.to_string())?;
        if let (Some(a), Some(s)) = (arbiter, &secret) {
            let esc = r
                .session
                .nip44_encrypt(&a.public_key(), s)
                .map_err(|e| e.to_string())?;
            d = d.tag(vec!["escrow".into(), esc, a.public_key().to_hex()]);
        }
        let ev = r.send(&d, None).await.map_err(|e| e.to_string())?;
        Ok(Sent {
            c: ev.id,
            preimage: secret,
            token,
        })
    }

    /// The token was redeemed by the binding (priced), within the wait.
    async fn redeemed(&self, sent: &Sent) -> Result<&'static str, String> {
        let (Some(w), Some(t)) = (self.wallet, &sent.token) else {
            return Ok("free");
        };
        let deadline = tokio::time::Instant::now() + WAIT;
        while tokio::time::Instant::now() < deadline {
            if !w.inspect(t).await.map(|v| v.unspent).unwrap_or(true) {
                return Ok("token redeemed");
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        Err("the preimage is public, but the binding didn't redeem the token".into())
    }

    /// SATISFY (with the preimage when priced), check Satisfied, the redemption and clean acts.
    async fn close(&self, sent: &Sent) -> Result<String, String> {
        let phase = self
            .r
            .satisfy(&sent.c, sent.preimage.as_deref())
            .await
            .map_err(|e| e.to_string())?;
        if phase != Phase::Satisfied {
            return Err(format!("SATISFY left it {phase:?}"));
        }
        let paid = self.redeemed(sent).await?;
        clean(self.r, &sent.c)?;
        Ok(format!("Satisfied; {paid}"))
    }

    async fn report_to(&self, c: &str, prev: &str) -> Result<Event, String> {
        let e = self
            .r
            .answer_to(c, prev, &[K_REPORT, K_DECLINE, K_REVOKE])
            .await
            .ok_or("no REPORT")?;
        if e.kind != K_REPORT {
            return Err(format!(
                "{} ({}) instead of REPORT",
                acn_core::kinds::name(e.kind),
                reason(&e)
            ));
        }
        Ok(e)
    }

    async fn promised(&self, c: &str) -> Result<Event, String> {
        let e = self
            .r
            .answer(c, &[K_PROMISE, K_COUNTER, K_DECLINE, K_REPORT])
            .await
            .ok_or("no answer to an N request")?;
        if e.kind != K_PROMISE {
            return Err(format!(
                "{} ({}) instead of PROMISE",
                acn_core::kinds::name(e.kind),
                reason(&e)
            ));
        }
        Ok(e)
    }

    async fn countered(&self, c: &str) -> Result<Event, String> {
        let e = self
            .r
            .answer(c, &[K_PROMISE, K_COUNTER, K_DECLINE, K_REPORT])
            .await
            .ok_or("no answer")?;
        if e.kind != K_COUNTER {
            return Err(format!(
                "{} ({}) instead of COUNTER",
                acn_core::kinds::name(e.kind),
                reason(&e)
            ));
        }
        Ok(e)
    }

    fn terms_of(e: &Event) -> Result<Value, String> {
        let v: Value = serde_json::from_str(&e.content).map_err(|e| e.to_string())?;
        Ok(v["terms"].clone())
    }

    async fn cases(&self) -> Vec<(&'static str, Result<String, String>)> {
        let open = |t: &mut Value| {
            t["conditions"] = json!([{"id": "c1", "class": "open", "description": "Do it well"}]);
        };
        vec![
            ("n-promise-report-satisfy", self.promise_report().await),
            (
                "n-counter-closes-open-condition",
                self.counter_closes(open).await,
            ),
            (
                "n-requester-counter-accepted",
                self.requester_counter(open, true).await,
            ),
            (
                "n-unmeetable-counter-declined",
                self.requester_counter(open, false).await,
            ),
            ("n-rework", self.rework().await),
            ("n-held-out-seed", self.held_out().await),
            ("n-disqualifier-breach", self.disqualifier().await),
            ("n-contradicted-rejection-disputed", self.dispute().await),
        ]
    }

    /// The endorsement of `act` in `c`, by a key in `keys`, with `policy`, within `window`.
    async fn endorsement(
        &self,
        c: &str,
        act: &Event,
        keys: &[String],
        policy: &str,
        window: u64,
    ) -> Result<String, String> {
        let deadline = tokio::time::Instant::now() + WAIT;
        while tokio::time::Instant::now() < deadline {
            if let Some(e) =
                self.r.events(c).into_iter().find(|e| {
                    e.kind == K_ENDORSE && e.e_marked("endorses").contains(&act.id.as_str())
                })
            {
                if !keys.contains(&e.pubkey) {
                    return Err(format!(
                        "{} endorsed by a key K_SVC doesn't name",
                        acn_core::kinds::name(act.kind)
                    ));
                }
                if e.values("policy").first() != Some(&policy) {
                    return Err("the endorsement's policy hash isn't the advertised one".into());
                }
                if e.created_at > act.created_at + window {
                    return Err("the endorsement came after the window".into());
                }
                return Ok(format!(
                    "{} endorsed ({})",
                    acn_core::kinds::name(act.kind),
                    e.values("endorser").first().copied().unwrap_or("?")
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err(format!(
            "no endorsement of the {}",
            acn_core::kinds::name(act.kind)
        ))
    }

    async fn governed(&self, gov: &Value) -> Vec<(&'static str, Result<String, String>)> {
        let keys: Vec<String> = gov["keys"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|k| k.as_str().map(str::to_owned))
            .collect();
        let policy = gov["policy"].as_str().unwrap_or_default().to_string();
        let n = async {
            let g = json!({"keys": keys, "kinds": [K_PROMISE, K_REPORT], "window": 600});
            let sent = self.request(None, |t| t["governance"] = g).await?;
            let pr = self.promised(&sent.c).await?;
            let e1 = self.endorsement(&sent.c, &pr, &keys, &policy, 600).await?;
            let rep = self.report_to(&sent.c, &pr.id).await?;
            let e2 = self.endorsement(&sent.c, &rep, &keys, &policy, 600).await?;
            let out = fold(&self.r.events(&sent.c), &self.r.docs());
            if out
                .anomalies
                .iter()
                .any(|a| a.code == acn_core::anomaly::Code::PendingEndorsement)
            {
                return Err("an endorsed act is still pending".into());
            }
            let closed = self.close(&sent).await?;
            Ok(format!("{e1}; {e2}; {closed}"))
        }
        .await;
        let t = async {
            let g = json!({"keys": keys, "kinds": [K_REPORT], "window": 600});
            let r = self.r;
            let (profile, terms, ct, secret, token) = match (self.wallet, &self.offer) {
                (Some(w), Some(o)) => {
                    let s = Keys::generate().secret_key().to_secret_hex();
                    let h = acn_core::jcs::sha256_hex(&hex_bytes(&s));
                    let mut terms = acn_core::draft::instantiate(
                        o,
                        &self.target.tool,
                        self.target.input.clone(),
                        Timestamp::now().as_secs(),
                        Some(&h),
                        None,
                    )
                    .map_err(|e| e.to_string())?;
                    terms["governance"] = g;
                    let tok = w
                        .lock_htlc(
                            terms["price"]["amount"].as_u64().unwrap_or(0),
                            &h,
                            &self.target.agent,
                            terms["payment"]["locktime"].as_u64().unwrap_or(0),
                            None,
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                    let ct = r
                        .session
                        .nip44_encrypt(&r.agent, &tok)
                        .map_err(|e| e.to_string())?;
                    ("T1", terms, Some(ct), Some(s), Some(tok))
                }
                _ => {
                    let mut terms = base_terms(&self.target.tool, &self.target.input, 0);
                    terms["governance"] = g;
                    ("T0", terms, None, None, None)
                }
            };
            let c = r
                .request(profile, &terms, ct.as_deref())
                .await
                .map_err(|e| e.to_string())?;
            let rep = r
                .answer(&c, &[K_REPORT, K_DECLINE])
                .await
                .ok_or("no answer")?;
            if rep.kind != K_REPORT {
                return Err(format!("declined ({})", reason(&rep)));
            }
            let e = self.endorsement(&c, &rep, &keys, &policy, 600).await?;
            let closed = self
                .close(&Sent {
                    c,
                    preimage: secret,
                    token,
                })
                .await?;
            Ok(format!("{profile} REPORT; {e}; {closed}"))
        }
        .await;
        vec![
            ("g-endorsed-promise-and-report", n),
            ("g-endorsed-t-report", t),
        ]
    }

    async fn promise_report(&self) -> Result<String, String> {
        let before = self.target.executions();
        let sent = self.request(None, |_| {}).await?;
        let pr = self.promised(&sent.c).await?;
        let (_, st) = self.r.state(&sent.c).ok_or("no state")?;
        let want = self.r.events(&sent.c)[0]
            .values("terms")
            .first()
            .map(|h| h.to_string());
        if pr.values("terms").first().map(|h| h.to_string()) != want {
            return Err("the PROMISE doesn't bind the request's terms hash".into());
        }
        let p_on_time = serde_json::from_str::<Value>(&pr.content)
            .ok()
            .and_then(|v| v["p_on_time"].as_f64());
        let rep = self.report_to(&sent.c, &pr.id).await?;
        let ev = check_evidence(&rep, self.blossom).await?;
        let runs = self.target.ran(before, 1)?;
        let closed = self.close(&sent).await?;
        let _ = st;
        Ok(format!(
            "PROMISE (p_on_time {}), REPORT; {ev}; {runs}; {closed}",
            p_on_time.map_or("absent".into(), |p| p.to_string())
        ))
    }

    async fn counter_closes(&self, open: impl Fn(&mut Value)) -> Result<String, String> {
        let before = self.target.executions();
        let sent = self.request(None, open).await?;
        let co = self.countered(&sent.c).await?;
        let t = acn_core::Terms::parse(&Self::terms_of(&co)?)?;
        if let Some(why) = t.unclosed() {
            return Err(format!("the COUNTER's terms aren't closed: {why}"));
        }
        self.target.ran(before, 0)?;
        let h = co
            .values("terms")
            .first()
            .map(|h| h.to_string())
            .ok_or("no terms hash")?;
        let acc = self
            .r
            .next(&sent.c, None, |head, pa| {
                Draft::accept(&sent.c, head, pa, &h)
            })
            .await?;
        self.report_to(&sent.c, &acc.id).await?;
        let runs = self.target.ran(before, 1)?;
        let closed = self.close(&sent).await?;
        Ok(format!(
            "COUNTER closing c1 ({}), no work before ACCEPT; REPORT; {runs}; {closed}",
            reason(&co)
        ))
    }

    async fn requester_counter(
        &self,
        open: impl Fn(&mut Value),
        meetable: bool,
    ) -> Result<String, String> {
        let sent = self.request(None, open).await?;
        let co = self.countered(&sent.c).await?;
        let mut t = Self::terms_of(&co)?;
        if meetable {
            t["deadline"] = json!(t["deadline"].as_u64().unwrap_or(0) + 600);
        } else {
            t["deadline"] = json!(Timestamp::now().as_secs().saturating_sub(60));
        }
        let h = acn_core::jcs::hash(&t).map_err(|e| e.to_string())?;
        let before = self.target.executions();
        let ours = self
            .r
            .next(&sent.c, None, |head, pa| {
                Draft::counter(&sent.c, head, pa, &t, "a different deadline").unwrap()
            })
            .await?;
        let ans = self
            .r
            .answer_to(&sent.c, &ours.id, &[K_ACCEPT, K_DECLINE, K_COUNTER])
            .await
            .ok_or_else(|| {
                let agent = self.r.agent.to_hex();
                let seen: Vec<String> = self
                    .r
                    .events(&sent.c)
                    .iter()
                    .map(|e| {
                        format!(
                            "{}{} {}… prev {:?}",
                            if e.pubkey == agent { "binding " } else { "" },
                            acn_core::kinds::name(e.kind),
                            &e.id[..8],
                            e.e_marked("prev")
                                .iter()
                                .map(|p| &p[..8])
                                .collect::<Vec<_>>()
                        )
                    })
                    .collect();
                format!(
                    "no answer to the requester's counter {}…; seen: {seen:?}",
                    &ours.id[..8]
                )
            })?;
        if !meetable {
            if ans.kind != K_DECLINE {
                return Err(format!(
                    "{} instead of DECLINE",
                    acn_core::kinds::name(ans.kind)
                ));
            }
            let runs = self.target.ran(before, 0)?;
            clean(self.r, &sent.c)?;
            return Ok(format!("declined ({}); {runs}", reason(&ans)));
        }
        if ans.kind != K_ACCEPT || ans.values("terms").first() != Some(&h.as_str()) {
            return Err(format!(
                "{} ({}) instead of ACCEPT of the counter's terms",
                acn_core::kinds::name(ans.kind),
                reason(&ans)
            ));
        }
        self.report_to(&sent.c, &ans.id).await?;
        let (_, st) = self.r.state(&sent.c).ok_or("no state")?;
        if st.terms != h {
            return Err("the work wasn't done under the accepted terms".into());
        }
        let runs = self.target.ran(before, 1)?;
        let closed = self.close(&sent).await?;
        Ok(format!(
            "ACCEPT of the later deadline; REPORT under it; {runs}; {closed}"
        ))
    }

    async fn rework(&self) -> Result<String, String> {
        let before = self.target.executions();
        let sent = self.request(None, |t| t["rework_limit"] = json!(1)).await?;
        let pr = self.promised(&sent.c).await?;
        self.report_to(&sent.c, &pr.id).await?;
        let (cm, _) = self.r.state(&sent.c).ok_or("no state")?;
        let cond = cm
            .terms
            .as_ref()
            .and_then(|t| t.conditions.first())
            .map(|c| c.id.clone())
            .ok_or("no condition")?;
        let rej = self
            .r
            .next(&sent.c, None, |head, pa| {
                Draft::reject(&sent.c, head, pa, vec![(&cond, "again, please")])
            })
            .await?;
        let rep2 = self.report_to(&sent.c, &rej.id).await?;
        check_evidence(&rep2, self.blossom).await?;
        let runs = self.target.ran(before, 2)?;
        let closed = self.close(&sent).await?;
        Ok(format!(
            "REJECT → Reworking → a second REPORT answering it; {runs}; {closed}"
        ))
    }

    async fn held_out(&self) -> Result<String, String> {
        let me = self.r.me();
        let sent = self
            .request(None, |t| {
                t["rework_limit"] = json!(0);
                t["verifiers"] = json!([{"pubkey": me, "classes": ["mechanical"]}]);
                t["conditions"] = json!([{"id": "c1", "class": "mechanical", "description": "held-out tests pass",
                    "instances": {"pool_description": "cases", "pool_rule": "curated", "pool_root": "cd".repeat(32),
                        "pool_size": 4, "sample_size": 2, "requester_seed_hash": "ef".repeat(32)}}]);
            })
            .await?;
        let pr = self.promised(&sent.c).await?;
        let committed = serde_json::from_str::<Value>(&pr.content)
            .ok()
            .and_then(|v| v["performer_seed_hash"].as_str().map(str::to_owned))
            .ok_or("the PROMISE carries no performer_seed_hash")?;
        let rep = self.report_to(&sent.c, &pr.id).await?;
        let seed = serde_json::from_str::<Value>(&rep.content)
            .ok()
            .and_then(|v| v["performer_seed"].as_str().map(str::to_owned))
            .ok_or("the REPORT reveals no performer_seed")?;
        if acn_core::jcs::sha256_hex(&hex_bytes(&seed)) != committed {
            return Err("the revealed seed doesn't hash to the committed one".into());
        }
        let closed = self.close(&sent).await?;
        Ok(format!(
            "seed committed in PROMISE, revealed in REPORT, hashes match; {closed}"
        ))
    }

    async fn disqualifier(&self) -> Result<String, String> {
        let me = self.r.me();
        let before = self.target.executions();
        let sent = self
            .request(None, |t| {
                t["rework_limit"] = json!(1);
                t["disqualifiers"] = json!([{"id": "d1", "class": "mechanical", "description": "modifies existing tests", "runners": [me]}]);
            })
            .await?;
        let pr = self.promised(&sent.c).await?;
        let rep = self.report_to(&sent.c, &pr.id).await?;
        self.r
            .annotate(&Draft::attest(
                K_ATTEST_MECH,
                &sent.c,
                &rep.id,
                "d1",
                json!({"result": "hit", "method": "conformance"}),
            ))
            .await?;
        self.r
            .next(&sent.c, None, |head, pa| {
                Draft::reject(&sent.c, head, pa, vec![("d1", "tests were modified")])
            })
            .await?;
        let (_, st) = self.r.state(&sent.c).ok_or("no state")?;
        if st.phase != Phase::Rejected {
            return Err(format!("the breach left it {:?}, not Rejected", st.phase));
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        if self.r.agent_acts(&sent.c, K_REPORT) != 1 {
            return Err("reworked after a disqualifier hit".into());
        }
        let runs = self.target.ran(before, 1)?;
        // A counted hit is recorded on the REPORT (V-25: `disqualified`, a flag): expected here,
        // and the only anomaly allowed on the binding's acts.
        let a = self.r.agent_anomalies(&sent.c);
        if !a.iter().any(|x| x.starts_with("disqualified")) {
            return Err("the hit wasn't recorded on the REPORT".into());
        }
        let other: Vec<&String> = a
            .iter()
            .filter(|x| !x.starts_with("disqualified"))
            .collect();
        if !other.is_empty() {
            return Err(format!("anomalies on the binding's acts: {other:?}"));
        }
        // The requester's lock stays refundable after its locktime: nothing to redeem here.
        Ok(format!("served with a disqualifier; hit → `disqualified` → REJECT → Rejected, no rework; {runs}"))
    }

    async fn dispute(&self) -> Result<String, String> {
        let me = self.r.me();
        let arbiter = Keys::generate();
        let arb = arbiter.public_key().to_hex();
        let before = self.target.executions();
        let sent = self
            .request(Some(&arbiter), |t| {
                t["rework_limit"] = json!(0);
                t["arbiter"] = json!(arb);
                t["verifiers"] = json!([{"pubkey": me, "classes": ["mechanical"]}]);
                t["conditions"] = json!([{"id": "tests", "class": "mechanical", "description": "the tests pass"}]);
            })
            .await?;
        let pr = self.promised(&sent.c).await?;
        let rep = self.report_to(&sent.c, &pr.id).await?;
        self.r
            .annotate(&Draft::attest(
                K_ATTEST_MECH,
                &sent.c,
                &rep.id,
                "tests",
                json!({"result": "pass", "method": "conformance"}),
            ))
            .await?;
        let rej = self
            .r
            .next(&sent.c, None, |head, pa| {
                Draft::reject(&sent.c, head, pa, vec![("tests", "not convinced")])
            })
            .await?;
        let dis = self
            .r
            .answer_to(&sent.c, &rej.id, &[K_DISPUTE])
            .await
            .ok_or("the contradicted rejection wasn't disputed")?;
        let preimage = sent.preimage.clone();
        self.r
            .next(&sent.c, Some(&arbiter), |head, pa| {
                Draft::ruling(&sent.c, head, pa, true, preimage.as_deref())
            })
            .await?;
        let (_, st) = self.r.state(&sent.c).ok_or("no state")?;
        if st.phase != Phase::Satisfied {
            return Err(format!("the ruling left it {:?}", st.phase));
        }
        let paid = self.redeemed(&sent).await?;
        let runs = self.target.ran(before, 1)?;
        clean(self.r, &sent.c)?;
        let _ = dis;
        Ok(format!("passing attestation; REJECT → Rejected (contradicted) → DISPUTE; RULING for the performer → Satisfied; {paid}; {runs}"))
    }
}

/// A declined request caused no work: no REPORT, and (when observable) no executor run.
fn no_work(r: &Requester, target: &Target, c: &str, before: Option<u64>) -> Result<String, String> {
    if r.agent_acts(c, K_REPORT) > 0 {
        return Err("declined, but also REPORTed".into());
    }
    target.ran(before, 0)
}

/// A paid T1 call, end to end: lock a token to the hashlock, request (terms inline, or on
/// `by_ref`'s Blossom server), check the REPORT's evidence and that the work ran once, SATISFY
/// with the preimage, wait for the binding to redeem, and check its acts are clean.
struct Paid<'a> {
    r: &'a Requester,
    wallet: &'a nmesh_wallet::CashuWallet,
    target: &'a Target,
    blossom: &'a [String],
}

impl Paid<'_> {
    async fn run(
        &self,
        terms: Value,
        preimage: &str,
        hashlock: &str,
        by_ref: Option<&String>,
    ) -> Result<String, String> {
        let (r, target) = (self.r, self.target);
        let amount = terms["price"]["amount"].as_u64().unwrap_or(0);
        let locktime = terms["payment"]["locktime"].as_u64().unwrap_or(0);
        let token = self
            .wallet
            .lock_htlc(amount, hashlock, &target.agent, locktime, None)
            .await
            .map_err(|e| e.to_string())?;
        let ct = r
            .session
            .nip44_encrypt(&r.agent, &token)
            .map_err(|e| e.to_string())?;
        let before = target.executions();
        let (c, how) = match by_ref {
            None => (
                r.request("T1", &terms, Some(&ct))
                    .await
                    .map_err(|e| e.to_string())?,
                "terms inline".to_string(),
            ),
            Some(server) => {
                let client = nmesh_blossom::BlossomClient::new(
                    vec![server.clone()],
                    r.session.keys().clone(),
                );
                let bytes = acn_core::jcs::canonical(&terms).map_err(|e| e.to_string())?;
                let (th, _) = client
                    .upload(bytes.as_bytes(), "application/json")
                    .await
                    .map_err(|e| e.to_string())?;
                r.docs.lock().unwrap().insert(th.clone(), terms.clone());
                let ev = r
                    .send(
                        &Draft::request_ref(&target.agent, "T1", &th, server, Some(&ct)),
                        None,
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                (ev.id, format!("terms {}… by reference", &th[..12]))
            }
        };
        let rep = r
            .answer(&c, &[K_REPORT, K_DECLINE])
            .await
            .ok_or("no answer to a paid T1 request")?;
        if rep.kind != K_REPORT {
            return Err(format!("declined a valid paid call: {}", reason(&rep)));
        }
        let ev = check_evidence(&rep, self.blossom).await?;
        let runs = target.ran(before, 1)?;
        let phase = r
            .satisfy(&c, Some(preimage))
            .await
            .map_err(|e| e.to_string())?;
        if phase != Phase::Satisfied {
            return Err(format!("SATISFY left it {phase:?}"));
        }
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            if !self
                .wallet
                .inspect(&token)
                .await
                .map(|v| v.unspent)
                .unwrap_or(true)
            {
                break;
            }
            if tokio::time::Instant::now() > deadline {
                return Err(
                    "SATISFY revealed the preimage, but the binding didn't redeem the token".into(),
                );
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        clean(r, &c)?;
        Ok(format!(
            "{amount} sat locked, {how}, REPORT; {ev}; {runs}; Satisfied; token redeemed"
        ))
    }
}

fn clean(r: &Requester, c: &str) -> Result<String, String> {
    let a = r.agent_anomalies(c);
    if a.is_empty() {
        Ok("no anomalies on the binding's acts".into())
    } else {
        Err(format!("anomalies on the binding's acts: {}", a.join("; ")))
    }
}

fn hex_bytes(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .filter_map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok())
        .collect()
}
