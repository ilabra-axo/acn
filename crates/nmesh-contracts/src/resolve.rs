//! Renderer resolution.
//!
//! Order (docs/spec/0003-renderers.md):
//! 1. installed renderers (user-chosen; includes built-ins) that support the target;
//! 2. trusted third-party renderers (per [`TrustPolicy`]) that support the target;
//! 3. the generic built-in renderer, if the payload validated;
//! 4. the `alt` text.
//!
//! Among candidates in the same tier, the least-privileged runtime wins
//! (Declarative < SandboxedWeb < Native), then fewest permissions.

use nmesh_proto::manifest::{RendererManifest, RendererRuntime};
use nostr::PublicKey;

use crate::builtin::GENERIC_RENDERER;
use crate::trust::{TrustDecision, TrustPolicy};

#[derive(Debug, Clone, PartialEq)]
pub enum Resolution<'a> {
    Installed(&'a RendererManifest),
    Trusted(&'a RendererManifest, TrustDecision),
    Generic,
    AltText,
}

fn supports(r: &RendererManifest, contract_id: &str, target: &str) -> bool {
    r.contracts.iter().any(|c| c == contract_id) && r.targets.iter().any(|t| t == target)
}

fn privilege(r: &RendererManifest) -> (u8, usize) {
    let tier = match r.runtime {
        RendererRuntime::Declarative => 0,
        RendererRuntime::SandboxedWeb => 1,
        RendererRuntime::Native => 2,
    };
    (tier, r.permissions.len())
}

/// `installed`: renderers the user has installed (built-ins included).
/// `discovered`: renderer manifests seen on relays, with their publisher.
/// `payload_valid`: whether the payload validated against its contract. Invalid payloads never
/// reach a renderer; they fall back to alt text.
pub fn resolve_renderer<'a>(
    contract_id: &str,
    target: &str,
    payload_valid: bool,
    installed: &'a [RendererManifest],
    discovered: &'a [(PublicKey, RendererManifest)],
    trust: &TrustPolicy,
) -> Resolution<'a> {
    if !payload_valid {
        return Resolution::AltText;
    }

    if let Some(r) = installed
        .iter()
        .filter(|r| supports(r, contract_id, target))
        .min_by_key(|r| privilege(r))
    {
        return Resolution::Installed(r);
    }

    if let Some((r, d)) = discovered
        .iter()
        .filter(|(_, r)| supports(r, contract_id, target))
        .map(|(pk, r)| (r, trust.evaluate(pk)))
        .filter(|(_, d)| d.is_trusted())
        .min_by_key(|(r, _)| privilege(r))
    {
        return Resolution::Trusted(r, d);
    }

    if installed.iter().any(|r| r.id == GENERIC_RENDERER) {
        return Resolution::Generic;
    }
    Resolution::AltText
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin;
    use nostr::Keys;

    fn third_party(id: &str, contract: &str) -> RendererManifest {
        RendererManifest {
            id: id.into(),
            title: id.into(),
            contracts: vec![contract.into()],
            targets: vec!["web".into()],
            runtime: RendererRuntime::SandboxedWeb,
            entry: None,
            actions: vec![],
            permissions: vec![],
        }
    }

    #[test]
    fn prefers_installed_builtin() {
        let installed = builtin::renderers();
        let trust = TrustPolicy::new(Keys::generate().public_key());
        let res = resolve_renderer(builtin::CARD, "web", true, &installed, &[], &trust);
        assert!(matches!(res, Resolution::Installed(r) if r.id == "nmesh.card"));
    }

    #[test]
    fn untrusted_third_party_falls_back_to_generic() {
        let installed = builtin::renderers();
        let stranger = Keys::generate().public_key();
        let discovered = vec![(stranger, third_party("x.chart", "x.chart/v1"))];
        let trust = TrustPolicy::new(Keys::generate().public_key());
        let res = resolve_renderer("x.chart/v1", "web", true, &installed, &discovered, &trust);
        assert_eq!(res, Resolution::Generic);
    }

    #[test]
    fn allow_listed_third_party_is_used() {
        let installed = builtin::renderers();
        let friend = Keys::generate().public_key();
        let discovered = vec![(friend, third_party("x.chart", "x.chart/v1"))];
        let mut trust = TrustPolicy::new(Keys::generate().public_key());
        trust.allow(friend);
        let res = resolve_renderer("x.chart/v1", "web", true, &installed, &discovered, &trust);
        assert!(
            matches!(res, Resolution::Trusted(r, TrustDecision::AllowListed) if r.id == "x.chart")
        );
    }

    #[test]
    fn invalid_payload_uses_alt() {
        let installed = builtin::renderers();
        let trust = TrustPolicy::default();
        let res = resolve_renderer(builtin::CARD, "web", false, &installed, &[], &trust);
        assert_eq!(res, Resolution::AltText);
    }
}
