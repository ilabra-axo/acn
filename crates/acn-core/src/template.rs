//! Terms templates (spec §7 "Terms templates", CR-003 §2.5): a signed, parameterized fragment of
//! terms (conditions and disqualifiers with typed slots) that many commitments can reuse, so a
//! track record attaches to a definition of done and not only to a party.
//!
//! A template is the content of a `K_TERMS_TEMPLATE` event (regular, so its event id pins it).
//! Terms that were drafted from one MAY carry a `template_ref`; the validator re-instantiates the
//! template with the reference's slot values and checks that it reproduces the terms' conditions
//! and disqualifiers exactly (V-28). The reference is provenance only: the validator always
//! evaluates the terms themselves.
//!
//! **Placeholders.** `{{slot}}` inside any string of the fragment. A string that is exactly one
//! placeholder becomes the slot's value with its JSON type (a number stays a number); a
//! placeholder inside a longer string is replaced by the value's text.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::terms::{Condition, Disqualifier};

pub const TEMPLATE_VERSION: &str = "acn-template/1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotType {
    String,
    Number,
    /// 32-byte x-only key, lowercase hex.
    Pubkey,
    /// Lowercase sha256 hex.
    Sha256,
    /// Seconds (non-negative integer).
    Duration,
    /// Satoshis (non-negative integer).
    Sats,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slot {
    #[serde(rename = "type")]
    pub ty: SlotType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    /// Prose for whoever fills the slot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guidance: Option<String>,
}

/// `K_TERMS_TEMPLATE` content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TermsTemplate {
    pub v: String,
    /// e.g. `code-change/v1`.
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub slots: BTreeMap<String, Slot>,
    pub conditions: Vec<Value>,
    #[serde(default)]
    pub disqualifiers: Vec<Value>,
    /// Optional one-line rationale per item id (why the item is there), with placeholders like
    /// the items. Not terms content: drafters show it beside the diff (CR-003 §3).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub notes: BTreeMap<String, String>,
}

/// `template_ref` in terms: which template, which version of it, and the slot values used.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateRef {
    /// The `K_TERMS_TEMPLATE` event id.
    pub event: String,
    /// sha256 of the JCS template content.
    pub hash: String,
    /// The values filled in (validator re-instantiates from these; their hash is the JCS hash of
    /// this object).
    #[serde(default)]
    pub slots: BTreeMap<String, Value>,
}

fn hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn type_ok(ty: SlotType, v: &Value) -> bool {
    match ty {
        SlotType::String => v.is_string(),
        SlotType::Number => v.is_number(),
        SlotType::Pubkey | SlotType::Sha256 => v.as_str().is_some_and(hex64),
        SlotType::Duration | SlotType::Sats => v.is_u64(),
    }
}

/// Every `{{name}}` in a string.
fn placeholders(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(i) = rest.find("{{") {
        let Some(j) = rest[i + 2..].find("}}") else {
            break;
        };
        out.push(rest[i + 2..i + 2 + j].trim().to_string());
        rest = &rest[i + 2 + j + 2..];
    }
    out
}

fn walk_strings<'a>(v: &'a Value, f: &mut impl FnMut(&'a str)) {
    match v {
        Value::String(s) => f(s),
        Value::Array(a) => a.iter().for_each(|x| walk_strings(x, f)),
        Value::Object(o) => o.values().for_each(|x| walk_strings(x, f)),
        _ => {}
    }
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Replace placeholders: a string that is exactly one placeholder becomes the typed value.
fn subst(v: &Value, filled: &BTreeMap<&str, Value>) -> Value {
    match v {
        Value::String(s) => {
            let ph = placeholders(s);
            let t = s.trim();
            if ph.len() == 1
                && t.starts_with("{{")
                && t.ends_with("}}")
                && t[2..t.len() - 2].trim() == ph[0]
            {
                return filled[ph[0].as_str()].clone();
            }
            let mut out = s.clone();
            for p in ph {
                out = out.replace(&format!("{{{{{p}}}}}"), &text(&filled[p.as_str()]));
                out = out.replace(&format!("{{{{ {p} }}}}"), &text(&filled[p.as_str()]));
            }
            Value::String(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| subst(x, filled)).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, x)| (k.clone(), subst(x, filled)))
                .collect(),
        ),
        other => other.clone(),
    }
}

impl TermsTemplate {
    /// Parse and check a template's content.
    pub fn parse(v: &Value) -> Result<Self, String> {
        let t: TermsTemplate = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
        if t.v != TEMPLATE_VERSION {
            return Err(format!("unsupported template version `{}`", t.v));
        }
        if t.name.trim().is_empty() {
            return Err("a template needs a name".into());
        }
        for (name, slot) in &t.slots {
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            {
                return Err(format!(
                    "slot name `{name}`: use lowercase letters, digits and `_`"
                ));
            }
            if let Some(d) = &slot.default {
                if !type_ok(slot.ty, d) {
                    return Err(format!("slot `{name}`: default is not a {:?}", slot.ty));
                }
            }
        }
        let mut unknown = None;
        let notes: Vec<Value> = t.notes.values().map(|n| Value::String(n.clone())).collect();
        for item in t.conditions.iter().chain(&t.disqualifiers).chain(&notes) {
            walk_strings(item, &mut |s| {
                for p in placeholders(s) {
                    if !t.slots.contains_key(&p) && unknown.is_none() {
                        unknown = Some(p);
                    }
                }
            });
        }
        let ids: Vec<&str> = t
            .conditions
            .iter()
            .chain(&t.disqualifiers)
            .filter_map(|i| i["id"].as_str())
            .collect();
        if let Some(k) = t.notes.keys().find(|k| !ids.contains(&k.as_str())) {
            return Err(format!("note `{k}` names no condition or disqualifier"));
        }
        if let Some(p) = unknown {
            return Err(format!("placeholder `{{{{{p}}}}}` names no slot"));
        }
        Ok(t)
    }

    /// Each slot's value: the given one, else the default; typed-checked.
    fn filled(&self, values: &BTreeMap<String, Value>) -> Result<BTreeMap<&str, Value>, String> {
        if let Some(k) = values.keys().find(|k| !self.slots.contains_key(*k)) {
            return Err(format!("slot `{k}` is not in the template"));
        }
        let mut filled: BTreeMap<&str, Value> = BTreeMap::new();
        for (name, slot) in &self.slots {
            let v = values
                .get(name)
                .or(slot.default.as_ref())
                .ok_or_else(|| format!("slot `{name}` has no value and no default"))?;
            if !type_ok(slot.ty, v) {
                return Err(format!("slot `{name}`: value is not a {:?}", slot.ty));
            }
            filled.insert(name, v.clone());
        }
        Ok(filled)
    }

    /// The notes with the slots filled in (item id → one line).
    pub fn notes_for(
        &self,
        values: &BTreeMap<String, Value>,
    ) -> Result<BTreeMap<String, String>, String> {
        let filled = self.filled(values)?;
        Ok(self
            .notes
            .iter()
            .map(|(k, n)| (k.clone(), text(&subst(&Value::String(n.clone()), &filled))))
            .collect())
    }

    /// Fill the slots (given values, else defaults) and return the conditions and disqualifiers
    /// as the terms would carry them (normalized through the terms types).
    pub fn instantiate(
        &self,
        values: &BTreeMap<String, Value>,
    ) -> Result<(Vec<Condition>, Vec<Disqualifier>), String> {
        let filled = self.filled(values)?;
        let conditions = self
            .conditions
            .iter()
            .map(|c| {
                serde_json::from_value(subst(c, &filled)).map_err(|e| format!("condition: {e}"))
            })
            .collect::<Result<Vec<Condition>, String>>()?;
        let disqualifiers = self
            .disqualifiers
            .iter()
            .map(|d| {
                serde_json::from_value(subst(d, &filled)).map_err(|e| format!("disqualifier: {e}"))
            })
            .collect::<Result<Vec<Disqualifier>, String>>()?;
        Ok((conditions, disqualifiers))
    }
}

/// How a terms version's `template_ref` checks out (V-28).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TemplateStatus {
    /// The template re-instantiates to exactly these conditions and disqualifiers.
    Counted,
    /// It doesn't (or the reference doesn't match the template); `template_mismatch`.
    Mismatch { reason: String },
    /// The template event isn't in the event set (fetching it is I/O, like V-12).
    Unavailable,
}

/// V-28 for one terms document against the template event content, if supplied.
pub fn check(terms: &crate::Terms, r: &TemplateRef, template: Option<&Value>) -> TemplateStatus {
    let Some(content) = template else {
        return TemplateStatus::Unavailable;
    };
    let mismatch = |reason: String| TemplateStatus::Mismatch { reason };
    if crate::jcs::hash(content).ok().as_deref() != Some(r.hash.as_str()) {
        return mismatch("template_ref.hash is not the hash of the template's content".into());
    }
    let t = match TermsTemplate::parse(content) {
        Ok(t) => t,
        Err(e) => return mismatch(format!("template: {e}")),
    };
    let (conditions, disqualifiers) = match t.instantiate(&r.slots) {
        Ok(x) => x,
        Err(e) => return mismatch(e),
    };
    if conditions != terms.conditions {
        return mismatch("re-instantiated conditions differ from the terms".into());
    }
    if disqualifiers != terms.disqualifiers {
        return mismatch("re-instantiated disqualifiers differ from the terms".into());
    }
    TemplateStatus::Counted
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tpl() -> Value {
        json!({
            "v": TEMPLATE_VERSION, "name": "demo/v1",
            "slots": {
                "scope": {"type": "string", "guidance": "paths"},
                "checker": {"type": "pubkey"},
                "max": {"type": "number", "default": 5}
            },
            "conditions": [{"id": "c1", "class": "mechanical", "description": "within {{max}}% for {{scope}}", "limit": "{{max}}"}],
            "disqualifiers": [{"id": "d1", "class": "mechanical", "description": "touches files outside {{ scope }}", "runners": ["{{checker}}"]}]
        })
    }

    #[test]
    fn instantiates_typed_and_textual() {
        let t = TermsTemplate::parse(&tpl()).unwrap();
        let slots = BTreeMap::from([
            ("scope".to_string(), json!("src/")),
            ("checker".to_string(), json!("ab".repeat(32))),
        ]);
        let (c, d) = t.instantiate(&slots).unwrap();
        assert_eq!(c[0].description, "within 5% for src/");
        assert_eq!(c[0].extra["limit"], json!(5));
        assert_eq!(d[0].description, "touches files outside src/");
        assert_eq!(d[0].runners(), vec!["ab".repeat(32)]);
    }

    #[test]
    fn notes_fill_placeholders_and_name_items() {
        let mut v = tpl();
        v["notes"] = json!({"c1": "within {{max}}% for {{scope}}"});
        let t = TermsTemplate::parse(&v).unwrap();
        let slots = BTreeMap::from([
            ("scope".to_string(), json!("src/")),
            ("checker".to_string(), json!("ab".repeat(32))),
        ]);
        assert_eq!(t.notes_for(&slots).unwrap()["c1"], "within 5% for src/");
        v["notes"] = json!({"c9": "names nothing"});
        assert!(TermsTemplate::parse(&v).is_err());
        v["notes"] = json!({"c1": "{{nope}}"});
        assert!(TermsTemplate::parse(&v).is_err());
    }

    #[test]
    fn rejects_bad_templates_and_values() {
        let mut bad = tpl();
        bad["conditions"][0]["description"] = json!("{{nope}}");
        assert!(TermsTemplate::parse(&bad).is_err());
        let t = TermsTemplate::parse(&tpl()).unwrap();
        let none = BTreeMap::new();
        assert!(t.instantiate(&none).is_err(), "scope has no default");
        let wrong = BTreeMap::from([
            ("scope".to_string(), json!("x")),
            ("checker".to_string(), json!("not a key")),
        ]);
        assert!(t.instantiate(&wrong).is_err());
        let extra = BTreeMap::from([
            ("scope".to_string(), json!("x")),
            ("checker".to_string(), json!("ab".repeat(32))),
            ("price".to_string(), json!(1)),
        ]);
        assert!(t.instantiate(&extra).is_err());
    }
}
