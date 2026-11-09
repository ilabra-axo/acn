//! Terms documents (spec §7).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The only terms version this prototype accepts: one coherent set of rules, no older meanings.
pub const TERMS_VERSION: &str = "acn-terms/0.5";

/// Unknown fields are an error, so a field the parties believe binds (a `holdback`, an `audit`
/// window) can't be silently ignored: commitments are atomic (spec §1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Terms {
    pub v: String,
    #[serde(default)]
    pub deliverable: Deliverable,
    #[serde(default)]
    pub conditions: Vec<Condition>,
    /// Latest REPORT time (unix seconds).
    pub deadline: u64,
    /// Seconds after REPORT for SATISFY/REJECT.
    pub review_window: u64,
    #[serde(default)]
    pub rework_limit: u32,
    pub price: Price,
    #[serde(default)]
    pub payment: Option<Payment>,
    #[serde(default)]
    pub verifiers: Vec<Verifier>,
    #[serde(default)]
    pub arbiter: Option<String>,
    #[serde(default)]
    pub provenance: Option<Value>,
    #[serde(default)]
    pub subcontracting: bool,
    #[serde(default)]
    pub disclosure: Disclosure,
    #[serde(default)]
    pub governance: Option<Governance>,
    /// Who can release payment (CR-002). Default: the requester.
    #[serde(default, skip_serializing_if = "SettlementTrigger::is_requester")]
    pub settlement_trigger: SettlementTrigger,
    /// Holds the escrowed secret and releases it on pass (`verifier` trigger).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settlement_verifier: Option<String>,
    /// Seconds the requester has to fund after a passing attestation (`contingent`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fund_window: Option<u64>,
    /// Public predicates whose attested hit fails the commitment (CR-003 §2.2).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disqualifiers: Vec<Disqualifier>,
}

/// A disqualifier (spec §7, CR-003 §2.2): an act the performer agreed not to commit. An attested
/// `hit` is breach, distinct from a condition that scored low. Commitments are atomic, so a
/// disqualifier is checked before settlement and there is no audit window after it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Disqualifier {
    pub id: String,
    /// `mechanical` or `judged` (human and grounding disqualifiers are open decisions).
    pub class: ConditionClass,
    #[serde(default)]
    pub description: String,
    /// Class fields: `runners` (keys allowed to attest it); for `mechanical` also `check_hash` /
    /// `check_ref`; for `judged`, the judged fields.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

/// Fields a `mechanical` disqualifier may carry besides `id`, `class` and `description`.
const MECHANICAL_DISQUALIFIER_FIELDS: &[&str] =
    &["runners", "check_hash", "check_ref", "instances"];

/// Held-out instances (spec §7, CR-003 §2.3): the criteria are public, but which cases from a
/// fixed pool get tested is drawn from two seeds after delivery. The pool's description and rule
/// are criteria (INV-14); its contents are not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instances {
    /// Public: what kind of cases the pool holds.
    pub pool_description: String,
    /// Public: how the pool was generated or curated.
    pub pool_rule: String,
    /// RFC 6962 Merkle root over the pool items (`instances` module).
    pub pool_root: String,
    pub pool_size: u32,
    pub sample_size: u32,
    /// sha256 of the requester's 32-byte seed, committed in this terms version.
    pub requester_seed_hash: String,
}

impl Instances {
    fn check(&self, id: &str) -> Result<(), String> {
        let bad = |m: &str| Err(format!("`{id}` instances: {m}"));
        if !is_sha256(&self.pool_root) || !is_sha256(&self.requester_seed_hash) {
            return bad("pool_root and requester_seed_hash must be lowercase sha256 hex");
        }
        if self.pool_size == 0 || self.sample_size == 0 || self.sample_size > self.pool_size {
            return bad("need 1 ≤ sample_size ≤ pool_size");
        }
        Ok(())
    }
}

/// A condition's or disqualifier's `instances`, if it declares any (for `judged`, inside the
/// judged fields; otherwise a class field).
pub fn instances_of(c: &Condition) -> Option<Instances> {
    if c.class == ConditionClass::Judged {
        return c.judged().ok()?.instances;
    }
    serde_json::from_value(c.extra.get("instances")?.clone()).ok()
}

/// What an attestation says about a disqualifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisqualifierStatus {
    /// A counted attestation found the act.
    Hit,
    /// Counted attestations found nothing.
    Clear,
    /// No counted attestation yet.
    Unchecked,
}

impl Disqualifier {
    /// The same fields read as a condition of its class (for the judged fields and checks).
    pub fn as_condition(&self) -> Condition {
        Condition {
            id: self.id.clone(),
            class: self.class,
            description: self.description.clone(),
            extra: self.extra.clone(),
        }
    }

    /// Keys whose attestations count (CR-003 §2.2: only keys named in the disqualifier).
    pub fn runners(&self) -> Vec<String> {
        if self.class == ConditionClass::Judged {
            return self
                .as_condition()
                .judged()
                .map(|j| j.runners)
                .unwrap_or_default();
        }
        self.extra
            .get("runners")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn check(&self) -> Result<(), String> {
        let bad = |m: &str| Err(format!("disqualifier `{}`: {m}", self.id));
        match self.class {
            ConditionClass::Mechanical => {}
            ConditionClass::Judged => {
                self.as_condition().judged()?;
            }
            ConditionClass::Human => {
                return bad("human disqualifiers need a declaration kind, which doesn't exist yet (open decision D-30)")
            }
            _ => return bad("must be mechanical or judged (open decision D-29)"),
        }
        for f in ["window", "holdback", "audit"] {
            if self.extra.contains_key(f) {
                return bad(&format!("`{f}`: commitments are atomic, so a disqualifier is checked before settlement and nothing is held back after it"));
            }
        }
        if self.class == ConditionClass::Mechanical {
            if let Some(f) = self
                .extra
                .keys()
                .find(|k| !MECHANICAL_DISQUALIFIER_FIELDS.contains(&k.as_str()))
            {
                return bad(&format!("unknown field `{f}`"));
            }
        }
        let runners = self.runners();
        if runners.is_empty()
            || self
                .extra
                .get("runners")
                .and_then(Value::as_array)
                .is_none()
        {
            return bad("needs `runners`, the keys allowed to attest it");
        }
        Ok(())
    }
}

/// Who can cause payment to be released (spec §7, §8; CR-002).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettlementTrigger {
    /// The requester's `SATISFY` reveals the secret (default).
    #[default]
    Requester,
    /// The `settlement_verifier` holds the secret and reveals it in `RELEASE` on pass.
    Verifier,
    /// The performer holds the key to an encrypted deliverable; paying reveals it (Phase C).
    Contingent,
}

impl SettlementTrigger {
    pub fn is_requester(&self) -> bool {
        *self == SettlementTrigger::Requester
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deliverable {
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_contract: Option<String>,
    /// For MCP-backed services: tool name and arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionClass {
    Mechanical,
    Grounding,
    /// A pinned judge scored the work at or above a threshold (CR-001). Certifies the score,
    /// never that the work is good.
    Judged,
    Interpretive,
    Human,
    /// Not yet drafted (CR-003 §2.4): the requester's intent in prose, to be closed in a
    /// COUNTER before anyone can promise.
    Open,
}

impl ConditionClass {
    /// Derived assurance label (spec §7). Never asserted by a party. `None` for `open`.
    pub fn assurance(self) -> Option<Assurance> {
        Some(match self {
            ConditionClass::Mechanical => Assurance::Check,
            ConditionClass::Grounding => Assurance::Grounded,
            ConditionClass::Judged | ConditionClass::Interpretive => Assurance::Opinion,
            ConditionClass::Human => Assurance::Human,
            ConditionClass::Open => return None,
        })
    }
}

/// Assurance label, ordered weakest first: `opinion` < `human` < `check` < `grounded`
/// (open decision D-14 on the ordering).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Assurance {
    Opinion,
    Human,
    Check,
    Grounded,
}

/// One pinned judge (spec §7, `judged`). Compared exactly, field by field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgeFingerprint {
    pub model_id: String,
    /// Provider-pinned version.
    pub model_version: String,
    pub prompt_template_hash: String,
    pub harness_hash: String,
    /// Temperature, max tokens, seed if supported.
    pub sampling: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Aggregation {
    /// Every judge's score ≥ θ.
    All,
    Mean,
    Median,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scale {
    pub min: f64,
    pub max: f64,
}

/// Class-specific fields of a `judged` condition (spec §7). Unknown fields are an error, so a
/// field the parties believe binds (e.g. a dropped `holdback`) can't be silently ignored. Audits
/// are separate conditions of satisfaction, not fields here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Judged {
    pub judges: Vec<JudgeFingerprint>,
    pub rubric_hash: String,
    /// Required: a retrievable reference (Blossom) to the rubric text (INV-14). Missing is
    /// `hidden_criteria`, so it is checked by [`Terms::check`] rather than by serde.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rubric_ref: Option<String>,
    /// Optional: the rubric text inline (≤ 16 KiB with the terms); must hash to `rubric_hash`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rubric_text: Option<String>,
    pub scale: Scale,
    pub threshold: f64,
    pub aggregation: Aggregation,
    pub repeats: u32,
    /// Keys allowed to run the judges and sign `K_ATTEST_JUDGED`. MUST NOT include the performer.
    pub runners: Vec<String>,
    /// Held-out instances the judges score (CR-003 §2.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instances: Option<Instances>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Condition {
    pub id: String,
    pub class: ConditionClass,
    #[serde(default)]
    pub description: String,
    /// Class-specific fields (test ids, referent, rubric hash...).
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

impl Condition {
    /// The `judged` fields, checked for internal consistency.
    pub fn judged(&self) -> Result<Judged, String> {
        if self.class != ConditionClass::Judged {
            return Err(format!("condition `{}` is not judged", self.id));
        }
        let j: Judged = serde_json::from_value(Value::Object(self.extra.clone()))
            .map_err(|e| format!("judged condition `{}`: {e}", self.id))?;
        let bad = |m: &str| Err(format!("judged condition `{}`: {m}", self.id));
        if j.judges.is_empty() {
            return bad("needs at least one judge");
        }
        if j.repeats == 0 {
            return bad("repeats must be ≥ 1");
        }
        if j.runners.is_empty() {
            return bad("needs at least one runner");
        }
        let scale_ok = j.scale.min < j.scale.max;
        if !scale_ok || !(j.scale.min..=j.scale.max).contains(&j.threshold) {
            return bad("threshold must lie within the scale");
        }
        if !is_sha256(&j.rubric_hash) {
            return bad("rubric_hash must be a lowercase sha256 hex");
        }

        Ok(j)
    }
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Price {
    pub amount: u64,
    #[serde(default = "sat")]
    pub unit: String,
}

fn sat() -> String {
    "sat".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Payment {
    pub mint: String,
    #[serde(default)]
    pub keysets: Vec<String>,
    /// H = sha256(s), hex. Present for conditional profiles (T1, N).
    #[serde(default)]
    pub hashlock: Option<String>,
    #[serde(default)]
    pub locktime: Option<u64>,
    #[serde(default)]
    pub refund_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verifier {
    pub pubkey: String,
    pub classes: Vec<ConditionClass>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disclosure {
    #[default]
    Public,
    Private,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Governance {
    pub keys: Vec<String>,
    /// Act kinds by the governed party that need an endorsement.
    pub kinds: Vec<u16>,
    /// Seconds after the act within which the endorsement must be created.
    pub window: u64,
    /// Which party is governed. Default: performer.
    #[serde(default)]
    pub party: GovernedParty,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GovernedParty {
    #[default]
    Performer,
    Requester,
}

impl Terms {
    pub fn parse(v: &Value) -> Result<Self, String> {
        Self::check(v).map_err(|(_, e)| e)
    }

    /// [`Terms::parse`] with the anomaly code a validator reports on the act carrying the
    /// terms: `hidden_criteria` (INV-14) or `malformed`.
    pub fn check(v: &Value) -> Result<Self, (crate::anomaly::Code, String)> {
        use crate::anomaly::Code;
        let malformed = |e: String| (Code::Malformed, e);
        if let Some(id) = Self::commit_reveal(v) {
            return Err((
                Code::HiddenCriteria,
                format!("`{id}`: a commit_reveal rubric is a hidden criterion"),
            ));
        }
        let t: Terms = serde_json::from_value(v.clone()).map_err(|e| malformed(e.to_string()))?;
        t.check_inner().map_err(malformed)?;
        t.hidden_criteria()?;
        Ok(t)
    }

    /// The judged / check fields named in `v`, before parsing: a `commit_reveal` rubric is a
    /// hidden criterion, not merely an unknown field.
    fn commit_reveal(v: &Value) -> Option<String> {
        let lists = [&v["conditions"], &v["disqualifiers"]];
        lists
            .iter()
            .filter_map(|l| l.as_array())
            .flatten()
            .find(|c| c["rubric_disclosure"] == "commit_reveal")
            .map(|c| c["id"].as_str().unwrap_or("?").to_string())
    }

    /// INV-14 (CR-003 §2.1): every criterion is readable by the performer before it promises.
    fn hidden_criteria(&self) -> Result<(), (crate::anomaly::Code, String)> {
        use crate::anomaly::Code;
        let disq: Vec<Condition> = self
            .disqualifiers
            .iter()
            .map(Disqualifier::as_condition)
            .collect();
        for c in self.conditions.iter().chain(&disq) {
            if c.class == ConditionClass::Judged {
                let j = c.judged().map_err(|e| (Code::Malformed, e))?;
                if j.rubric_ref.as_deref().is_none_or(|r| r.trim().is_empty()) {
                    return Err((
                        Code::HiddenCriteria,
                        format!(
                            "`{}`: a judged condition needs rubric_ref, a retrievable rubric",
                            c.id
                        ),
                    ));
                }
                if j.rubric_text
                    .as_deref()
                    .is_some_and(|text| crate::jcs::sha256_hex(text.as_bytes()) != j.rubric_hash)
                {
                    return Err((
                        Code::Malformed,
                        format!("`{}`: rubric_text does not hash to rubric_hash", c.id),
                    ));
                }
            }
            if let Some(i) = instances_of(c) {
                if i.pool_description.trim().is_empty() || i.pool_rule.trim().is_empty() {
                    return Err((
                        Code::HiddenCriteria,
                        format!(
                            "`{}`: an instance pool needs a public description and rule",
                            c.id
                        ),
                    ));
                }
            }
            if c.class != ConditionClass::Judged && Self::check_hidden(c) {
                return Err((
                    Code::HiddenCriteria,
                    format!(
                        "`{}`: a check referenced by check_hash needs a retrievable check_ref",
                        c.id
                    ),
                ));
            }
        }
        Ok(())
    }

    /// A check definition named only by hash (`check_hash` without `check_ref`).
    fn check_hidden(c: &Condition) -> bool {
        let has = |f: &str| {
            c.extra
                .get(f)
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty())
        };
        c.extra.contains_key("check_hash") && !has("check_ref")
    }

    fn check_inner(&self) -> Result<(), String> {
        let t = self;
        if t.v != TERMS_VERSION {
            return Err(format!(
                "unsupported terms version `{}` (this prototype accepts {TERMS_VERSION} only)",
                t.v
            ));
        }
        for c in t
            .conditions
            .iter()
            .filter(|c| c.class == ConditionClass::Open)
        {
            if let Some(s) = c.extra.get("suggested_class") {
                match serde_json::from_value::<ConditionClass>(s.clone()) {
                    Ok(ConditionClass::Open) | Err(_) => {
                        return Err(format!(
                            "open condition `{}`: suggested_class must be a closed class",
                            c.id
                        ))
                    }
                    Ok(_) => {}
                }
            }
        }
        // Attestations name a condition or a disqualifier by id, so ids share one namespace.
        let mut ids: Vec<&str> = t
            .conditions
            .iter()
            .map(|c| c.id.as_str())
            .chain(t.disqualifiers.iter().map(|d| d.id.as_str()))
            .collect();
        ids.sort_unstable();
        if ids.windows(2).any(|w| w[0] == w[1]) {
            return Err("duplicate condition or disqualifier id".into());
        }
        for d in &t.disqualifiers {
            d.check()?;
        }
        // Instances: well-formed, only where something can test them, and never reused across a
        // rework round (the first attestation reveals the requester's seed and the sample, so a
        // rework would be graded on known cases; a retry is a new commitment with new seeds).
        let disq: Vec<Condition> = t
            .disqualifiers
            .iter()
            .map(Disqualifier::as_condition)
            .collect();
        for c in t.conditions.iter().chain(&disq) {
            // Class-specific fields are open (test ids, referents), but nothing that would hold
            // payment back after settlement: commitments are atomic (spec §1).
            if let Some(f) = ["holdback", "audit", "window"].iter().find(|f| c.extra.contains_key(**f)) {
                return Err(format!("`{}`: `{f}`: commitments are atomic, so nothing is held back or audited after settlement", c.id));
            }
            let declared = c.extra.contains_key("instances");
            if declared && !matches!(c.class, ConditionClass::Mechanical | ConditionClass::Judged) {
                return Err(format!(
                    "`{}`: only mechanical and judged items take instances",
                    c.id
                ));
            }
            if c.class == ConditionClass::Mechanical && declared {
                let i: Instances = serde_json::from_value(c.extra["instances"].clone())
                    .map_err(|e| format!("`{}` instances: {e}", c.id))?;
                i.check(&c.id)?;
            }
            if c.class == ConditionClass::Judged {
                if let Some(i) = c.judged()?.instances {
                    i.check(&c.id)?;
                }
            }
        }
        if t.has_instances() && t.rework_limit > 0 {
            return Err("held-out instances need rework_limit 0: a rework would be graded on cases the first attestation revealed".into());
        }
        for c in t
            .conditions
            .iter()
            .filter(|c| c.class == ConditionClass::Judged)
        {
            c.judged()?;
        }
        Ok(())
    }

    pub fn condition(&self, id: &str) -> Option<&Condition> {
        self.conditions.iter().find(|c| c.id == id)
    }

    pub fn disqualifier(&self, id: &str) -> Option<&Disqualifier> {
        self.disqualifiers.iter().find(|d| d.id == id)
    }

    /// Any condition or disqualifier declares held-out instances (the performer then commits a
    /// seed when it binds, and reveals it in its REPORT).
    pub fn has_instances(&self) -> bool {
        self.conditions.iter().any(|c| instances_of(c).is_some())
            || self
                .disqualifiers
                .iter()
                .any(|d| instances_of(&d.as_condition()).is_some())
    }

    /// Overall assurance: the weakest condition's label. With no conditions, closure rests on
    /// the requester's own judgement (`human`). `None` while a condition is still `open`.
    pub fn assurance(&self) -> Option<Assurance> {
        self.conditions
            .iter()
            .map(|c| c.class.assurance())
            .collect::<Option<Vec<_>>>()
            .map(|v| v.into_iter().min().unwrap_or(Assurance::Human))
    }

    /// CR-003 §2.4 closure rule: why these terms can't be promised or accepted yet, if so. A
    /// condition is closed when it has a class other than `open`, every field its class needs,
    /// public criteria (checked at parse, INV-14) and a named evidence producer: a
    /// listed verifier of its class (`mechanical`, `grounding`), its runners (`judged`), or the
    /// requester itself (`human`, `interpretive`). Disqualifiers name their runners at parse.
    pub fn unclosed(&self) -> Option<String> {
        if let Some(c) = self
            .conditions
            .iter()
            .find(|c| c.class == ConditionClass::Open)
        {
            return Some(format!("condition `{}` is still open", c.id));
        }
        self.conditions
            .iter()
            .find(|c| {
                matches!(
                    c.class,
                    ConditionClass::Mechanical | ConditionClass::Grounding
                ) && !self.verifiers.iter().any(|v| v.classes.contains(&c.class))
            })
            .map(|c| {
                format!(
                    "condition `{}` names no evidence producer (no listed {:?} verifier)",
                    c.id, c.class
                )
            })
    }

    /// CR-002 valid combinations (spec §7). The fold reports a failure with the returned code
    /// (`invalid_settlement_combo`, or `contingent_nondeterministic`) on the act carrying the
    /// terms.
    pub fn settlement_combo(&self) -> Result<(), (crate::anomaly::Code, String)> {
        use crate::anomaly::Code;
        self.settlement_combo_inner()
            .map_err(|e| (Code::InvalidSettlementCombo, e))?;
        if self.settlement_trigger == SettlementTrigger::Contingent {
            // Open conditions are checked once a COUNTER closes them (CR-003 §2.4).
            if let Some(c) = self
                .conditions
                .iter()
                .find(|c| !matches!(c.class, ConditionClass::Mechanical | ConditionClass::Open))
            {
                return Err((
                    Code::ContingentNondeterministic,
                    format!("condition `{}` is {:?}; contingent settlement needs every condition mechanical", c.id, c.class),
                ));
            }
        }
        Ok(())
    }

    fn settlement_combo_inner(&self) -> Result<(), String> {
        match self.settlement_trigger {
            SettlementTrigger::Requester => {
                if self.settlement_verifier.is_some() || self.fund_window.is_some() {
                    return Err(
                        "settlement_verifier / fund_window need a verifier or contingent trigger"
                            .into(),
                    );
                }
            }
            SettlementTrigger::Verifier => {
                if self.settlement_verifier.is_none() {
                    return Err("the verifier trigger needs a settlement_verifier".into());
                }
                if self.hashlock().is_none() {
                    return Err("the verifier trigger needs a hash-locked payment".into());
                }
                if self.arbiter.is_some() {
                    return Err("an arbiter is only available with the requester trigger".into());
                }
                if self.fund_window.is_some() {
                    return Err("fund_window belongs to the contingent trigger".into());
                }
                if self.conditions.is_empty() {
                    return Err("the verifier trigger needs conditions to verify".into());
                }
                if let Some(c) = self.conditions.iter().find(|c| {
                    matches!(
                        c.class,
                        ConditionClass::Human | ConditionClass::Interpretive
                    )
                }) {
                    return Err(format!(
                        "condition `{}` is {:?}: no attestation can pass it, so a verifier can't release on it",
                        c.id, c.class
                    ));
                }
            }
            SettlementTrigger::Contingent => {
                let Some(v) = self.settlement_verifier.as_deref() else {
                    return Err(
                        "the contingent trigger needs a settlement_verifier to attest".into(),
                    );
                };
                if !self.verifier_counts(v, ConditionClass::Mechanical) {
                    return Err(
                        "the settlement_verifier must be a listed mechanical verifier".into(),
                    );
                }
                if self.fund_window.is_none() {
                    return Err("the contingent trigger needs a fund_window".into());
                }
                if self.price.amount == 0 || self.payment.as_ref().is_none_or(|p| p.mint.is_empty())
                {
                    return Err("the contingent trigger needs a price and a payment mint".into());
                }
                if self.hashlock().is_some() {
                    return Err(
                        "under contingent the hashlock comes from the REPORT, not the terms".into(),
                    );
                }
                if self.arbiter.is_some() {
                    return Err("an arbiter is only available with the requester trigger".into());
                }
                if self.conditions.is_empty() {
                    return Err("the contingent trigger needs conditions to attest".into());
                }
            }
        }
        Ok(())
    }

    pub fn hashlock(&self) -> Option<&str> {
        self.payment.as_ref().and_then(|p| p.hashlock.as_deref())
    }

    pub fn verifier_counts(&self, pubkey: &str, class: ConditionClass) -> bool {
        self.verifiers
            .iter()
            .any(|v| v.pubkey == pubkey && v.classes.contains(&class))
    }
}
