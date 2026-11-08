//! Terms documents (spec §7).

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const TERMS_VERSION: &str = "acn-terms/0.3";
/// Still parsed (no `judged` conditions); CR-001 introduced 0.3.
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
    pub rubric_disclosure: RubricDisclosure,
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
        let t: Terms = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
        if t.v != TERMS_VERSION && t.v != TERMS_VERSION_0_2 {
            return Err(format!("unsupported terms version `{}`", t.v));
        }
        if t.v == TERMS_VERSION_0_2
            && t.conditions
                .iter()
                .any(|c| c.class == ConditionClass::Judged)
        {
            return Err("`judged` conditions need acn-terms/0.3".into());
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
        Ok(t)
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

    pub fn hashlock(&self) -> Option<&str> {
        self.payment.as_ref().and_then(|p| p.hashlock.as_deref())
    }

    pub fn verifier_counts(&self, pubkey: &str, class: ConditionClass) -> bool {
        self.verifiers
            .iter()
            .any(|v| v.pubkey == pubkey && v.classes.contains(&class))
    }
}
