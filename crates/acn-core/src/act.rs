//! Structural parsing of ACN events (V-2 version/kind, V-3 structure).

use serde_json::Value;

use crate::anomaly::Code;
use crate::event::Event;
use crate::kinds::*;

pub const INLINE_LIMIT: usize = 16 * 1024;

/// Sibling-resolution class (spec §5.4). Lower wins.
pub fn class_rank(kind: u16) -> u8 {
    match kind {
        K_WITHDRAW | K_DECLINE | K_REVOKE | K_SATISFY | K_RULING | K_RELEASE | K_CLAIM => 0,
        K_PROMISE | K_ACCEPT | K_REPORT | K_REJECT | K_DISPUTE | K_FUND => 1,
        _ => 2, // COUNTER
    }
}

/// Median of a non-empty list (mean of the middle two for an even count).
pub(crate) fn median(xs: &[f64]) -> f64 {
    let mut v = xs.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// `K_ATTEST_JUDGED` content (spec §9): structure and internal consistency only. Agreement
/// with the terms is V-17, in the fold.
fn judged_content(c: &Value) -> Result<(), String> {
    let judges = c
        .get("judges")
        .and_then(Value::as_array)
        .filter(|j| !j.is_empty())
        .ok_or("judged attestation needs a non-empty `judges` array")?;
    for j in judges {
        for f in [
            "model_id",
            "model_version",
            "prompt_template_hash",
            "harness_hash",
            "rationale_hash",
        ] {
            if j.get(f).and_then(Value::as_str).is_none() {
                return Err(format!("judge entry needs `{f}`"));
            }
        }
        if j.get("sampling").is_none() {
            return Err("judge entry needs `sampling`".into());
        }
        let runs: Vec<f64> = j
            .get("runs")
            .and_then(Value::as_array)
            .filter(|r| !r.is_empty())
            .and_then(|r| r.iter().map(Value::as_f64).collect())
            .ok_or("judge entry needs a non-empty numeric `runs` array")?;
        let declared = j
            .get("median")
            .and_then(Value::as_f64)
            .ok_or("judge entry needs a numeric `median`")?;
        if (median(&runs) - declared).abs() > 1e-9 {
            return Err("judge `median` is not the median of its `runs`".into());
        }
    }
    if c.get("aggregate_score").and_then(Value::as_f64).is_none() {
        return Err("`aggregate_score` must be a number".into());
    }
    // `hit` / `clear` for a judged disqualifier (CR-003); which one applies is checked in the fold.
    match c.get("result").and_then(Value::as_str) {
        Some("pass" | "fail" | "hit" | "clear") => {}
        _ => return Err("attestation result must be `pass`, `fail`, `hit` or `clear`".into()),
    }
    if !c
        .get("rubric_text_hash")
        .and_then(Value::as_str)
        .is_some_and(|h| is_hex(h, 64))
    {
        return Err("`rubric_text_hash` must be a lowercase sha256 hex".into());
    }
    match c.get("injection_detected").and_then(Value::as_bool) {
        Some(true)
            if c.get("injection_span_hash")
                .and_then(Value::as_str)
                .is_none() =>
        {
            Err("`injection_span_hash` is required when `injection_detected` is true".into())
        }
        Some(_) => Ok(()),
        None => Err("`injection_detected` must be a boolean".into()),
    }
}

pub fn is_advancing(kind: u16) -> bool {
    ACTS.contains(&kind) && kind != K_REQUEST && kind != K_PROGRESS
}

/// A structurally valid ACN event.
#[derive(Debug, Clone)]
pub struct Parsed<'a> {
    pub ev: &'a Event,
    pub content: Value,
    /// `c` (absent only on REQUEST and bindings).
    pub c: Option<String>,
    /// `e … prev`.
    pub prev: Option<String>,
    pub terms: Option<String>,
}

impl Parsed<'_> {
    pub fn kind(&self) -> u16 {
        self.ev.kind
    }
    pub fn id(&self) -> &str {
        &self.ev.id
    }
    pub fn at(&self) -> u64 {
        self.ev.created_at
    }
    pub fn str_field(&self, k: &str) -> Option<&str> {
        self.content.get(k).and_then(Value::as_str)
    }
}

type R<T> = Result<T, (Code, String)>;

fn malformed<T>(msg: impl Into<String>) -> R<T> {
    Err((Code::Malformed, msg.into()))
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Parse and check structure. Returns `Ok(None)` for events outside ACN entirely (ignored).
pub fn parse(ev: &Event) -> R<Option<Parsed<'_>>> {
    if !BLOCK.contains(&ev.kind) && ev.kind != K_SVC {
        return Ok(None);
    }
    if !is_known(ev.kind) {
        return Err((
            Code::UnknownKind,
            format!("kind {} is in the ACN block but unassigned", ev.kind),
        ));
    }
    if ev.kind == K_SVC || ev.kind == K_SCORE || ev.kind == K_PRIVATE_ACT {
        // Metadata / projections / envelopes: no commitment semantics in the fold.
        return Ok(None);
    }
    match ev.single("v") {
        Ok(v) if v == VERSION => {}
        Ok(v) => return Err((Code::BadVersion, format!("unsupported version `{v}`"))),
        Err(n) => return malformed(format!("expected exactly one `v` tag, found {n}")),
    }
    if ev.content.len() > INLINE_LIMIT {
        return malformed("content exceeds 16 KiB inline limit");
    }
    let content: Value = if ev.content.is_empty() {
        Value::Object(Default::default())
    } else {
        match serde_json::from_str(&ev.content) {
            Ok(v @ Value::Object(_)) => v,
            Ok(_) => return malformed("content must be a JSON object"),
            Err(e) => return malformed(format!("content is not JSON: {e}")),
        }
    };

    let c = ev
        .optional("c")
        .map_err(|n| (Code::Malformed, format!("{n} `c` tags")))?;
    let prevs = ev.e_marked("prev");
    let terms = ev
        .optional("terms")
        .map_err(|n| (Code::Malformed, format!("{n} `terms` tags")))?;
    if let Some(t) = terms {
        if !is_hex(t, 64) {
            return malformed("`terms` must be a lowercase sha256 hex");
        }
    }

    let is_commitment_event = ACTS.contains(&ev.kind)
        || matches!(
            ev.kind,
            K_ATTEST_MECH
                | K_ATTEST_GROUND
                | K_ATTEST_JUDGED
                | K_INTERP_REPORT
                | K_ENDORSE
                | K_ESCROW_ACK
        );

    if ev.kind == K_REQUEST {
        if c.is_some() || !prevs.is_empty() {
            return malformed("REQUEST is the root: no `c` or `prev`");
        }
        match ev.single("profile") {
            Ok("T0" | "T1" | "N") => {}
            Ok(p) => return malformed(format!("unknown profile `{p}`")),
            Err(n) => return malformed(format!("expected one `profile` tag, found {n}")),
        }
        if ev.values("p").len() != 1 {
            return malformed("REQUEST must name exactly one performer `p` (v0.2)");
        }
        if terms.is_none() {
            return malformed("REQUEST requires a `terms` tag");
        }
    } else if is_commitment_event {
        let Some(cv) = c else {
            return malformed("missing `c` tag");
        };
        if !is_hex(cv, 64) {
            return malformed("`c` must be an event id");
        }
        let needs_prev = ACTS.contains(&ev.kind);
        if needs_prev && prevs.len() != 1 {
            return malformed(format!(
                "expected exactly one `prev`, found {}",
                prevs.len()
            ));
        }
    }

    // Per-act content requirements (spec §6.2).
    let need = |k: &str| -> R<()> {
        if content.get(k).is_none() {
            malformed(format!("content.{k} is required"))
        } else {
            Ok(())
        }
    };
    match ev.kind {
        K_COUNTER => {
            if terms.is_none() {
                return malformed("COUNTER requires a `terms` tag");
            }
            need("reason")?;
        }
        K_ACCEPT | K_PROMISE if terms.is_none() => {
            return malformed("requires a `terms` tag");
        }
        K_DECLINE | K_WITHDRAW | K_REVOKE => need("reason")?,
        K_REJECT => {
            let gaps = content.get("gaps").and_then(Value::as_array);
            match gaps {
                Some(g)
                    if !g.is_empty()
                        && g.iter()
                            .all(|x| x.get("condition").and_then(Value::as_str).is_some()) => {}
                _ => return malformed("REJECT requires non-empty `gaps` naming conditions"),
            }
        }
        K_DISPUTE => need("reason")?,
        K_RULING => match content.get("decision").and_then(Value::as_str) {
            Some("performer" | "requester") => {}
            _ => return malformed("RULING.decision must be `performer` or `requester`"),
        },
        K_PROMISE => {
            if let Some(p) = content.get("p_on_time") {
                if !p.as_f64().is_some_and(|x| (0.0..=1.0).contains(&x)) {
                    return malformed("p_on_time must be within 0..1");
                }
            }
        }
        // `hit` / `clear` attest a disqualifier (CR-003); the fold checks which applies.
        K_ATTEST_MECH | K_ATTEST_GROUND => match content.get("result").and_then(Value::as_str) {
            Some("pass" | "fail" | "hit" | "clear") => {}
            _ => return malformed("attestation result must be `pass`, `fail`, `hit` or `clear`"),
        },
        K_INTERP_REPORT => need("verdict")?,
        K_RELEASE => need("preimage")?,
        K_FUND => {
            let hex64 = |k: &str| {
                content
                    .get(k)
                    .and_then(Value::as_str)
                    .is_some_and(|h| is_hex(h, 64))
            };
            if !hex64("hashlock")
                || content.get("amount").and_then(Value::as_u64).is_none()
                || content.get("mint").and_then(Value::as_str).is_none()
                || content.get("locktime").and_then(Value::as_u64).is_none()
            {
                return malformed(
                    "FUND requires `hashlock` (sha256 hex), `amount`, `mint` and `locktime`",
                );
            }
        }
        K_CLAIM => {
            if !content
                .get("key")
                .and_then(Value::as_str)
                .is_some_and(|k| is_hex(k, 64))
            {
                return malformed("CLAIM requires `key`, the 32-byte key as hex");
            }
        }
        K_ESCROW_ACK => {
            if !content
                .get("hashlock")
                .and_then(Value::as_str)
                .is_some_and(|h| is_hex(h, 64))
            {
                return malformed("ESCROW_ACK requires `hashlock`, a lowercase sha256 hex");
            }
        }
        K_ATTEST_JUDGED => judged_content(&content).map_err(|m| (Code::Malformed, m))?,
        _ => {}
    }
    if matches!(
        ev.kind,
        K_ATTEST_MECH | K_ATTEST_GROUND | K_ATTEST_JUDGED | K_INTERP_REPORT
    ) {
        if ev.e_marked("report").len() != 1 {
            return malformed("attestation must reference exactly one REPORT (`e` … `report`)");
        }
        if ev.single("cond").is_err() {
            return malformed("attestation must name exactly one `cond`");
        }
    }
    if ev.kind == K_ENDORSE && ev.e_marked("endorses").len() != 1 {
        return malformed("ENDORSE must reference exactly one act (`e` … `endorses`)");
    }
    if ev.kind == K_BIND {
        if ev.values("p").len() != 1 {
            return malformed("BIND must name exactly one agent `p`");
        }
        if ev
            .single("expires")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .is_none()
        {
            return malformed("BIND requires a numeric `expires`");
        }
    }
    if matches!(ev.kind, K_BIND_ACK | K_BIND_REVOKE) && ev.values("e").len() != 1 {
        return malformed("must reference exactly one BIND (`e`)");
    }

    Ok(Some(Parsed {
        ev,
        content,
        c: c.map(str::to_owned),
        prev: prevs.first().map(|s| s.to_string()),
        terms: terms.map(str::to_owned),
    }))
}
