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
//!   must decline a free request instead.
//! - **T1** (when offered): a hash-locked token in the `REQUEST`; the `REPORT`; `SATISFY` with
//!   the preimage; the binding redeems the token. A token whose lock doesn't match the terms is
//!   declined.
//! - **Declines:** an `open` condition, profile N, held-out instances, an unknown tool and an
//!   expired deadline are each answered with `DECLINE`, never with work.
//! - **Clean acts:** no act the binding signs carries any anomaly from the reference validator.

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

    fn state(&self, c: &str) -> Option<(acn_core::Commitment, acn_core::State)> {
        let out = fold(&self.events(c), &BTreeMap::new());
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
        fold(&events, &BTreeMap::new())
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
    let http = reqwest::Client::new();
    let content: Value = serde_json::from_str(&report.content).map_err(|e| e.to_string())?;
    let d = content["derivation"]
        .as_str()
        .ok_or("the REPORT carries no derivation record")?;
    let bytes = nmesh_blossom::fetch_from(&http, servers, d)
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
        nmesh_blossom::fetch_from(&http, servers, h)
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
            if svc["binding_level"] != "L0" {
                return Err(format!(
                    "K_SVC binding_level is {}, not \"L0\"",
                    svc["binding_level"]
                ));
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
                    "K_SVC L0, profiles {profiles:?}, {} Blossom server(s), operator-bound",
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
            let phase = r.satisfy(&c, None).await.map_err(|e| e.to_string())?;
            if phase != Phase::Satisfied {
                return Err(format!("SATISFY left it {phase:?}"));
            }
            Ok(format!("REPORT; {ev}; Satisfied"))
        }
        .await;
        case("t0-call", res);
    } else {
        let res = match r.answer(&c, &[K_REPORT, K_DECLINE]).await {
            Some(e) if e.kind == K_DECLINE => Ok(format!("declined ({})", reason(&e))),
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
            let events = r.events(&ev.id);
            let docs = BTreeMap::from([(h.clone(), terms)]);
            let out = fold(&events, &docs);
            let mine: Vec<&str> = events
                .iter()
                .filter(|e| e.pubkey == target.agent && e.kind != K_BIND_ACK)
                .map(|e| e.id.as_str())
                .collect();
            let bad: Vec<String> = out
                .anomalies
                .iter()
                .filter(|a| mine.contains(&a.event.as_str()))
                .map(|a| a.code.as_str().to_string())
                .collect();
            if !bad.is_empty() {
                return Err(format!("anomalies on the binding's acts: {bad:?}"));
            }
            Ok(format!(
                "fetched terms {}… from the hint and REPORTed against them",
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
        // A paid call.
        let s = Keys::generate().secret_key().to_secret_hex();
        let h = acn_core::jcs::sha256_hex(&hex_bytes(&s));
        let res = async {
            let terms = paid(&h).map_err(|e| e.to_string())?;
            let amount = terms["price"]["amount"].as_u64().unwrap_or(0);
            let locktime = terms["payment"]["locktime"].as_u64().unwrap_or(0);
            let token = wallet
                .lock_htlc(amount, &h, &target.agent, locktime, None)
                .await
                .map_err(|e| e.to_string())?;
            let ct = r
                .session
                .nip44_encrypt(&r.agent, &token)
                .map_err(|e| e.to_string())?;
            let c = r
                .request("T1", &terms, Some(&ct))
                .await
                .map_err(|e| e.to_string())?;
            let rep = r
                .answer(&c, &[K_REPORT, K_DECLINE])
                .await
                .ok_or("no answer to a paid T1 request")?;
            if rep.kind != K_REPORT {
                return Err(format!("declined a valid paid call: {}", reason(&rep)));
            }
            let ev = check_evidence(&rep, &blossom).await?;
            let phase = r.satisfy(&c, Some(&s)).await.map_err(|e| e.to_string())?;
            if phase != Phase::Satisfied {
                return Err(format!("SATISFY left it {phase:?}"));
            }
            let deadline = tokio::time::Instant::now() + WAIT;
            loop {
                if !wallet
                    .inspect(&token)
                    .await
                    .map(|v| v.unspent)
                    .unwrap_or(true)
                {
                    break;
                }
                if tokio::time::Instant::now() > deadline {
                    return Err(
                        "SATISFY revealed the preimage, but the binding didn't redeem the token"
                            .into(),
                    );
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            clean(&r, &c)?;
            Ok(format!(
                "{amount} sat locked, REPORT; {ev}; Satisfied; token redeemed"
            ))
        }
        .await;
        case("t1-paid-call", res);
        // The same, with the terms on the requester's Blossom server (Core §7).
        if let Some(server) = &env.blossom {
            let s2 = Keys::generate().secret_key().to_secret_hex();
            let h2 = acn_core::jcs::sha256_hex(&hex_bytes(&s2));
            let res = async {
                let terms = paid(&h2).map_err(|e| e.to_string())?;
                let amount = terms["price"]["amount"].as_u64().unwrap_or(0);
                let locktime = terms["payment"]["locktime"].as_u64().unwrap_or(0);
                let token = wallet
                    .lock_htlc(amount, &h2, &target.agent, locktime, None)
                    .await
                    .map_err(|e| e.to_string())?;
                let ct = r
                    .session
                    .nip44_encrypt(&r.agent, &token)
                    .map_err(|e| e.to_string())?;
                let client = nmesh_blossom::BlossomClient::new(
                    vec![server.clone()],
                    r.session.keys().clone(),
                );
                let bytes = acn_core::jcs::canonical(&terms).map_err(|e| e.to_string())?;
                let (th, _) = client
                    .upload(bytes.as_bytes(), "application/json")
                    .await
                    .map_err(|e| e.to_string())?;
                let ev = r
                    .send(
                        &Draft::request_ref(&target.agent, "T1", &th, server, Some(&ct)),
                        None,
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                let rep = r
                    .answer(&ev.id, &[K_REPORT, K_DECLINE])
                    .await
                    .ok_or("no answer to a by-reference paid request")?;
                if rep.kind != K_REPORT {
                    return Err(format!(
                        "declined a paid call with terms by reference: {}",
                        reason(&rep)
                    ));
                }
                Ok(format!(
                    "fetched terms {}… from the hint, verified the token and REPORTed",
                    &th[..12]
                ))
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
            let c = r
                .request("T1", &terms, Some(&ct))
                .await
                .map_err(|e| e.to_string())?;
            let ans = r
                .answer(&c, &[K_REPORT, K_DECLINE])
                .await
                .ok_or("no answer")?;
            clean(&r, &c)?;
            if ans.kind == K_DECLINE {
                Ok(format!("declined ({})", reason(&ans)))
            } else {
                Err("did the work against a token locked to another hash".into())
            }
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
    let declines: Vec<(&str, &str, &str, Value)> = vec![
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
    for (name, profile, want, terms) in declines {
        let c = r.request(profile, &terms, None).await?;
        let res = match r
            .answer(&c, &[K_REPORT, K_DECLINE, K_PROMISE, K_COUNTER])
            .await
        {
            Some(e) if e.kind == K_DECLINE && reason(&e) == want => {
                clean(&r, &c).map(|_| format!("declined ({want})"))
            }
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
    r.session.shutdown().await;
    Ok(Report {
        level: "L0",
        agent: target.agent.clone(),
        cases,
    })
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
