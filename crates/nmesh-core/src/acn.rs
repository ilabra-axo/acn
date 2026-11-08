//! ACN transport glue: sign `acn_core` drafts with the session key and carry signed acts
//! privately (Core.md §5.5: unsigned `K_PRIVATE_ACT` rumor whose content is the signed act).

use acn_core::draft::Draft;
use nostr_sdk::prelude::*;

use crate::{CoreError, Session};

impl Session {
    /// Sign an ACN draft with this session's key.
    ///
    /// `not_before` should be the `created_at` of the act's `prev`: V-15 rejects acts that
    /// precede their predecessor, so this guards against clock skew between parties.
    pub fn sign_act(
        &self,
        draft: &Draft,
        not_before: Option<u64>,
    ) -> Result<acn_core::Event, CoreError> {
        let now = Timestamp::now().as_secs();
        let at = Timestamp::from(not_before.map_or(now, |nb| now.max(nb)));
        let tags = draft
            .tags
            .iter()
            .map(|t| Tag::parse(t.clone()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| CoreError::Other(format!("acn tag: {e}")))?;
        let ev = EventBuilder::new(Kind::from_u16(draft.kind), &draft.content)
            .tags(tags)
            .custom_created_at(at)
            .sign_with_keys(self.keys())
            .map_err(|e| CoreError::Other(format!("acn sign: {e}")))?;
        acn_core::Event::from_json(&ev.as_json()).map_err(|e| CoreError::Other(e.to_string()))
    }

    /// Send a signed act to `to` inside a gift-wrapped `K_PRIVATE_ACT` rumor.
    pub async fn send_act(
        &self,
        to: PublicKey,
        act: &acn_core::Event,
    ) -> Result<EventId, CoreError> {
        let rumor = EventBuilder::new(
            Kind::from_u16(nmesh_proto::kinds::ACN_PRIVATE_ACT),
            act.to_json(),
        )
        .tags([
            Tag::public_key(to),
            Tag::alt(format!("ACN {} act", acn_core::kinds::name(act.kind))),
        ]);
        self.send_rumor(to, rumor).await
    }

    /// NIP-44 encrypt `plaintext` to `to` (used for the arbiter escrow, Core.md §8.4).
    pub fn nip44_encrypt(&self, to: &PublicKey, plaintext: &str) -> Result<String, CoreError> {
        nostr_sdk::nips::nip44::encrypt(
            self.keys().secret_key(),
            to,
            plaintext,
            nostr_sdk::nips::nip44::Version::V2,
        )
        .map_err(|e| CoreError::Other(format!("nip44: {e}")))
    }

    /// Publish a signed ACN event as-is (public acts, bindings, `K_SVC`).
    pub async fn publish_act(&self, act: &acn_core::Event) -> Result<EventId, CoreError> {
        let ev = Event::from_json(act.to_json()).map_err(|e| CoreError::Other(e.to_string()))?;
        Ok(*self.client().send_event(&ev).await?.id())
    }

    /// Authors of `K_SVC` advertisements with the given `d` identifiers (e.g. the demo cast).
    pub async fn find_acn_services(
        &self,
        ids: &[&str],
        timeout: std::time::Duration,
    ) -> Result<Vec<PublicKey>, CoreError> {
        let filter = Filter::new()
            .kind(Kind::from_u16(acn_core::kinds::K_SVC))
            .identifiers(ids.iter().map(|s| s.to_string()));
        let events = self.client().fetch_events(filter, timeout).await?;
        let mut out: Vec<(usize, PublicKey)> = events
            .iter()
            .filter_map(|e| {
                let d = e.tags.identifier()?;
                ids.iter().position(|i| *i == d).map(|p| (p, e.pubkey))
            })
            .collect();
        out.sort_by_key(|(p, _)| *p);
        out.dedup_by_key(|(p, _)| *p);
        Ok(out.into_iter().map(|(_, pk)| pk).collect())
    }

    /// Fetch an agent's public ACN metadata: its `K_SVC` and operator binding events.
    pub async fn acn_public(
        &self,
        agent: PublicKey,
        timeout: std::time::Duration,
    ) -> Result<(Option<serde_json::Value>, Vec<acn_core::Event>), CoreError> {
        use acn_core::kinds::{K_BIND, K_BIND_ACK, K_BIND_REVOKE, K_SVC};
        let k = |n: u16| Kind::from_u16(n);
        let first = self
            .client()
            .fetch_events(
                Filter::new().author(agent).kinds([k(K_SVC), k(K_BIND_ACK)]),
                timeout,
            )
            .await?;
        let binds = self
            .client()
            .fetch_events(Filter::new().kind(k(K_BIND)).pubkey(agent), timeout)
            .await?;
        let operators: Vec<PublicKey> = binds.iter().map(|e| e.pubkey).collect();
        let revokes = if operators.is_empty() {
            Events::default()
        } else {
            self.client()
                .fetch_events(
                    Filter::new().kind(k(K_BIND_REVOKE)).authors(operators),
                    timeout,
                )
                .await?
        };

        let svc = first
            .iter()
            .filter(|e| e.kind == k(K_SVC))
            .max_by_key(|e| e.created_at)
            .and_then(|e| serde_json::from_str(&e.content).ok());
        let to_acn = |e: &Event| acn_core::Event::from_json(&e.as_json()).ok();
        let bindings = first
            .iter()
            .filter(|e| e.kind == k(K_BIND_ACK))
            .chain(binds.iter())
            .chain(revokes.iter())
            .filter_map(to_acn)
            .collect();
        Ok((svc, bindings))
    }
}
