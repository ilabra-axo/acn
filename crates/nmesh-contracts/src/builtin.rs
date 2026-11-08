//! Built-in contracts and renderers.
//!
//! Contract definitions live as plain JSON in `contracts/registry/` so non-Rust clients
//! (web, Kotlin) can consume the same files. They are embedded here at compile time.

use nmesh_proto::manifest::{ContractDefinition, RendererManifest, RendererRuntime};

pub const CARD: &str = "nmesh.card/v1";
pub const TABLE: &str = "nmesh.table/v1";
pub const FORM: &str = "nmesh.form/v1";
pub const FORM_SUBMIT: &str = "nmesh.form.submit/v1";
pub const LOCATION: &str = "nmesh.location/v1";

const SOURCES: &[&str] = &[
    include_str!("../../../contracts/registry/nmesh.card.v1.json"),
    include_str!("../../../contracts/registry/nmesh.table.v1.json"),
    include_str!("../../../contracts/registry/nmesh.form.v1.json"),
    include_str!("../../../contracts/registry/nmesh.form.submit.v1.json"),
    include_str!("../../../contracts/registry/nmesh.location.v1.json"),
];

pub fn contracts() -> Vec<ContractDefinition> {
    SOURCES
        .iter()
        .map(|s| serde_json::from_str(s).expect("built-in contract JSON is valid"))
        .collect()
}

/// Renderer id for the generic fallback (pretty JSON / key-value tree). Handles any contract.
pub const GENERIC_RENDERER: &str = "nmesh.generic";

/// Built-in Tier 0 renderers every nmesh client is expected to ship.
pub fn renderers() -> Vec<RendererManifest> {
    let r = |id: &str, title: &str, contracts: &[&str], actions: &[&str]| RendererManifest {
        id: id.into(),
        title: title.into(),
        contracts: contracts.iter().map(|s| s.to_string()).collect(),
        targets: vec!["web".into(), "android".into(), "tui".into()],
        runtime: RendererRuntime::Declarative,
        entry: None,
        actions: actions.iter().map(|s| s.to_string()).collect(),
        permissions: vec![],
    };
    vec![
        r("nmesh.card", "Card", &[CARD], &[FORM_SUBMIT]),
        r("nmesh.table", "Table", &[TABLE], &[]),
        r("nmesh.form", "Form", &[FORM], &[FORM_SUBMIT]),
        r("nmesh.location", "Location", &[LOCATION], &[]),
        r(GENERIC_RENDERER, "Generic", &[], &[]),
    ]
}
