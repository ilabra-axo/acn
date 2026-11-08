//! Terms documents (spec §7).

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const TERMS_VERSION: &str = "acn-terms/0.5";
/// Still parsed, without the fields introduced later (`judged`: 0.3, CR-001; settlement
/// triggers: 0.4, CR-002; public rubric references: 0.5, CR-003).
pub const TERMS_VERSION_0_4: &str = "acn-terms/0.4";
pub const TERMS_VERSION_0_3: &str = "acn-terms/0.3";
pub const TERMS_VERSION_0_2: &str = "acn-terms/0.2";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
}

impl ConditionClass {
    /// Derived assurance label (spec §7). Never asserted by a party.
    pub fn assurance(self) -> Assurance {
        match self {
            ConditionClass::Mechanical => Assurance::Check,
            ConditionClass::Grounding => Assurance::Grounded,
            ConditionClass::Judged | ConditionClass::Interpretive => Assurance::Opinion,
            ConditionClass::Human => Assurance::Human,
        }
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
pub struct JudgeFingerprint {
    pub model_id: String,
    /// Provider-pinned version.
    pub model_version: String,
    pub prompt_template_hash: String,
    pub harness_hash: String,
    /// Temperature, max tokens, seed if supported.
    pub sampling: Value,
}

/// Terms before `acn-terms/0.5` only (CR-001). `commit_reveal` no longer parses at any version:
/// it is a hidden criterion (CR-003, INV-14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RubricDisclosure {
    /// The rubric is published with the terms.
    Public,
    /// Only `rubric_hash` is published; the text is revealed in the attestation.
    CommitReveal,
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
    /// Before 0.5 only (required there); removed in 0.5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rubric_disclosure: Option<RubricDisclosure>,
    /// From 0.5, required: a retrievable reference (Blossom) to the rubric text (INV-14).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rubric_ref: Option<String>,
    /// From 0.5, optional: the rubric text inline (≤ 16 KiB with the terms); must hash to
    /// `rubric_hash`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rubric_text: Option<String>,
    pub scale: Scale,
    pub threshold: f64,
    pub aggregation: Aggregation,
    pub repeats: u32,
    /// Keys allowed to run the judges and sign `K_ATTEST_JUDGED`. MUST NOT include the performer.
    pub runners: Vec<String>,
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
pub struct Price {
    pub amount: u64,
    #[serde(default = "sat")]
    pub unit: String,
}

fn sat() -> String {
    "sat".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
        let t: Terms = serde_json::from_value(v.clone()).map_err(|e| malformed(e.to_string()))?;
        t.check_inner().map_err(malformed)?;
        t.hidden_criteria()?;
        Ok(t)
    }

    /// INV-14 (CR-003 §2.1): every criterion is readable by the performer before it promises.
    fn hidden_criteria(&self) -> Result<(), (crate::anomaly::Code, String)> {
        use crate::anomaly::Code;
        let v05 = self.v == TERMS_VERSION;
        for c in &self.conditions {
            if c.class == ConditionClass::Judged {
                let j = c.judged().map_err(|e| (Code::Malformed, e))?;
                if j.rubric_disclosure == Some(RubricDisclosure::CommitReveal) {
                    return Err((
                        Code::HiddenCriteria,
                        format!("`{}`: a commit_reveal rubric is a hidden criterion", c.id),
                    ));
                }
                if !v05 {
                    if j.rubric_disclosure.is_none() {
                        return Err((Code::Malformed, format!("`{}`: judged conditions before acn-terms/0.5 need rubric_disclosure", c.id)));
                    }
                    if j.rubric_ref.is_some() || j.rubric_text.is_some() {
                        return Err((
                            Code::Malformed,
                            format!("`{}`: rubric_ref / rubric_text need acn-terms/0.5", c.id),
                        ));
                    }
                    continue;
                }
                if j.rubric_disclosure.is_some() {
                    return Err((Code::Malformed, format!("`{}`: rubric_disclosure was removed in acn-terms/0.5 (rubrics are public)", c.id)));
                }
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
            } else if v05 && Self::check_hidden(c) {
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
        if ![
            TERMS_VERSION,
            TERMS_VERSION_0_4,
            TERMS_VERSION_0_3,
            TERMS_VERSION_0_2,
        ]
        .contains(&t.v.as_str())
        {
            return Err(format!("unsupported terms version `{}`", t.v));
        }
        if t.v == TERMS_VERSION_0_2
            && t.conditions
                .iter()
                .any(|c| c.class == ConditionClass::Judged)
        {
            return Err("`judged` conditions need acn-terms/0.3".into());
        }
        let settlement_fields = !t.settlement_trigger.is_requester()
            || t.settlement_verifier.is_some()
            || t.fund_window.is_some();
        if settlement_fields && ![TERMS_VERSION, TERMS_VERSION_0_4].contains(&t.v.as_str()) {
            return Err("settlement triggers need acn-terms/0.4".into());
        }
        let mut ids: Vec<&str> = t.conditions.iter().map(|c| c.id.as_str()).collect();
        ids.sort_unstable();
        if ids.windows(2).any(|w| w[0] == w[1]) {
            return Err("duplicate condition id".into());
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

    /// Overall assurance: the weakest condition's label. With no conditions, closure rests on
    /// the requester's own judgement (`human`).
    pub fn assurance(&self) -> Assurance {
        self.conditions
            .iter()
            .map(|c| c.class.assurance())
            .min()
            .unwrap_or(Assurance::Human)
    }

    /// CR-002 valid combinations (spec §7). The fold reports a failure with the returned code
    /// (`invalid_settlement_combo`, or `contingent_nondeterministic`) on the act carrying the
    /// terms.
    pub fn settlement_combo(&self) -> Result<(), (crate::anomaly::Code, String)> {
        use crate::anomaly::Code;
        self.settlement_combo_inner()
            .map_err(|e| (Code::InvalidSettlementCombo, e))?;
        if self.settlement_trigger == SettlementTrigger::Contingent {
            if let Some(c) = self
                .conditions
                .iter()
                .find(|c| c.class != ConditionClass::Mechanical)
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
