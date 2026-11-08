//! ACN projections for the UI (docs/Core.md §12 "thread view").
//!
//! The heavy lifting is `acn_view::project` (shared with the in-browser presenter mode); this
//! module only adds nmesh rendering (contract validation + renderer choice for the result) and
//! npub formatting. Nothing here affects commitment semantics (INV-12).

use std::collections::BTreeMap;

use acn_core::kinds::*;
use acn_core::BindingStatus;
use acn_view::Projection;
use nmesh_core::{PublicKey, ToBech32};
use nmesh_proto::{ContractRef, StructuredMessage};
use serde::Serialize;
use serde_json::Value;

use crate::view::{BodyView, RenderContext, RendererView};

#[derive(Debug, Clone, Serialize)]
pub struct BindingView {
    pub bound: bool,
    pub operator: Option<String>,
    pub detail: String,
}

impl From<&BindingStatus> for BindingView {
    fn from(b: &BindingStatus) -> Self {
        match b {
            BindingStatus::Bound { operator, .. } => Self {
                bound: true,
                operator: PublicKey::parse(operator)
                    .ok()
                    .and_then(|p| p.to_bech32().ok()),
                detail: "bound to an accountable operator key".into(),
            },
            BindingStatus::Unbound => Self {
                bound: false,
                operator: None,
                detail: "unaccountable: no operator binding".into(),
            },
            BindingStatus::OutOfScope { reason } => Self {
                bound: false,
                operator: None,
                detail: format!("binding out of scope: {reason}"),
            },
            BindingStatus::Revoked { .. } => Self {
                bound: false,
                operator: None,
                detail: "binding revoked".into(),
            },
        }
    }
}

/// `acn_view::Projection` plus nmesh rendering.
#[derive(Debug, Clone, Serialize)]
pub struct CommitmentView {
    pub conversation: String,
    #[serde(flatten)]
    pub projection: Projection,
    /// The current REPORT's result, ready for the renderer registry.
    pub result: Option<BodyView>,
    pub performer_binding: BindingView,
}

impl std::ops::Deref for CommitmentView {
    type Target = Projection;
    fn deref(&self) -> &Projection {
        &self.projection
    }
}

/// `extra`: public events the fold also needs (the performer's operator bindings, V-8).
pub fn project(
    conversation: &str,
    me: &str,
    events: &BTreeMap<String, acn_core::Event>,
    extra: &[acn_core::Event],
    ctx: &RenderContext<'_>,
    now: u64,
) -> Vec<CommitmentView> {
    let list: Vec<acn_core::Event> = events.values().chain(extra).cloned().collect();
    acn_view::project(&list, me, now)
        .into_iter()
        .map(|p| CommitmentView {
            conversation: conversation.to_string(),
            result: p
                .report
                .as_ref()
                .map(|r| render(ctx, &r.payload, r.contract.as_deref())),
            performer_binding: BindingView::from(&p.binding),
            projection: p,
        })
        .collect()
}

fn render(ctx: &RenderContext<'_>, payload: &Value, contract: Option<&str>) -> BodyView {
    match contract {
        Some(contract) => {
            let cref = ctx
                .registry
                .pinned_ref(contract)
                .unwrap_or_else(|| ContractRef::new(contract));
            let alt = payload
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("Result")
                .to_string();
            ctx.body(&nmesh_core::Body::Structured(StructuredMessage::new(
                cref,
                payload.clone(),
                alt,
            )))
        }
        None => match payload.get("text").and_then(Value::as_str) {
            Some(t) if payload.as_object().is_some_and(|o| o.len() == 1) => {
                BodyView::Text { text: t.into() }
            }
            _ => BodyView::Structured {
                contract: "(none)".into(),
                schema_hash: None,
                alt: "Result".into(),
                payload: payload.clone(),
                valid: true,
                error: None,
                renderer: RendererView::Generic,
                reply_to: None,
            },
        },
    }
}

/// Commitment id an ACN event belongs to.
pub fn commitment_of(e: &acn_core::Event) -> String {
    if e.kind == K_REQUEST {
        return e.id.clone();
    }
    e.tags
        .iter()
        .find(|t| t.first().map(String::as_str) == Some("c"))
        .and_then(|t| t.get(1).cloned())
        .unwrap_or_default()
}
