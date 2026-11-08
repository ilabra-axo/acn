//! Terminal ("tui" target) implementations of the built-in Tier 0 renderers.
//!
//! This is the smallest possible host: validate → resolve → render, falling back to alt text.
//! Real hosts (web, Compose, AR) implement the same decision path with richer output.

use nmesh_contracts::{builtin, resolve_renderer, ContractRegistry, Resolution, TrustPolicy};
use nmesh_core::{Body, Inbound, ToBech32};
use serde_json::Value;

pub fn render(registry: &ContractRegistry, m: &Inbound) -> String {
    let who = m
        .sender
        .to_bech32()
        .map(|s| format!("{}…", &s[..16]))
        .unwrap_or_default();
    let body = match &m.body {
        Body::Text(t) => t.clone(),
        Body::Structured(s) => {
            let valid = registry.validate_message(s).is_ok();
            let installed = builtin::renderers();
            let trust = TrustPolicy::default();
            match resolve_renderer(&s.contract.id, "tui", valid, &installed, &[], &trust) {
                Resolution::Installed(r) => match r.id.as_str() {
                    "nmesh.card" => card(&s.payload),
                    "nmesh.table" => table(&s.payload),
                    _ => generic(&s.payload),
                },
                Resolution::Trusted(..) | Resolution::Generic => generic(&s.payload),
                Resolution::AltText => format!("{} (unrendered {})", s.alt, s.contract.id),
            }
        }
        other => other.summary(),
    };
    format!("{who} ▸ {body}")
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or_default()
}

fn card(v: &Value) -> String {
    let mut out = format!("┌─ {}", s(v, "title"));
    if !s(v, "subtitle").is_empty() {
        out.push_str(&format!(" — {}", s(v, "subtitle")));
    }
    if !s(v, "body").is_empty() {
        for line in s(v, "body").lines() {
            out.push_str(&format!("\n│ {line}"));
        }
    }
    for f in v
        .get("fields")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        out.push_str(&format!("\n│ {}: {}", s(f, "label"), s(f, "value")));
    }
    let actions: Vec<String> = v
        .get("actions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|a| format!("[{}]", s(a, "label")))
        .collect();
    if !actions.is_empty() {
        out.push_str(&format!("\n│ {}", actions.join(" ")));
    }
    out.push_str("\n└─");
    out
}

fn table(v: &Value) -> String {
    let cols: Vec<(&str, &str)> = v
        .get("columns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|c| (s(c, "key"), s(c, "label")))
        .collect();
    let mut out = cols.iter().map(|(_, l)| *l).collect::<Vec<_>>().join(" | ");
    for row in v
        .get("rows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let cells: Vec<String> = cols
            .iter()
            .map(|(k, _)| match row.get(*k) {
                Some(Value::String(s)) => s.clone(),
                Some(other) => other.to_string(),
                None => String::new(),
            })
            .collect();
        out.push_str(&format!("\n{}", cells.join(" | ")));
    }
    out
}

fn generic(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}
