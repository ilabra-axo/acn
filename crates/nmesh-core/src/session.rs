use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use nmesh_proto::{kinds, StructuredMessage};
use nostr_sdk::prelude::*;
use tokio::sync::{mpsc, Mutex};

use crate::{CoreError, IdentityKind, Inbound};

/// NIP-59 gift wraps back-date `created_at` by up to two days, so subscriptions must look back
/// at least that far and rely on de-duplication.
const GIFT_WRAP_LOOKBACK: Duration = Duration::from_secs(2 * 24 * 60 * 60 + 60 * 60);

#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub relays: Vec<String>,
    pub kind: IdentityKind,
    pub display_name: Option<String>,
    pub about: Option<String>,
    /// Publish kind 0 metadata and the NIP-17 inbox relay list (kind 10050) on connect.
    pub announce: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            relays: vec![],
            kind: IdentityKind::Human,
            display_name: None,
            about: None,
            announce: true,
        }
    }
}

/// Options for [`Session::inbox_with`].
#[derive(Debug, Clone)]
pub struct InboxOptions {
    /// Also yield our own messages (self-copies sent from this or other devices).
    /// Agents must leave this off to avoid reacting to their own replies.
    pub include_own: bool,
    /// How far back to fetch history. Never less than the NIP-59 back-dating window.
    pub lookback: Duration,
}

impl Default for InboxOptions {
    fn default() -> Self {
        Self {
            include_own: false,
            lookback: GIFT_WRAP_LOOKBACK,
        }
    }
}

/// One identity connected to a set of relays.
#[derive(Clone)]
pub struct Session {
    client: Client,
    keys: Keys,
    relays: Vec<RelayUrl>,
}

impl Session {
    pub async fn connect(keys: Keys, config: SessionConfig) -> Result<Self, CoreError> {
        if config.relays.is_empty() {
            return Err(CoreError::Other("at least one relay is required".into()));
        }
        let client = Client::builder().signer(keys.clone()).build();
        let mut relays = Vec::with_capacity(config.relays.len());
        for r in &config.relays {
            let url = RelayUrl::parse(r)?;
            client.add_relay(url.clone()).await?;
            relays.push(url);
        }
        client.connect().await;
        client.wait_for_connection(Duration::from_secs(5)).await;

        let session = Self {
            client,
            keys,
            relays,
        };
        if config.announce {
            // Not fatal: relays may be temporarily unreachable; we'll still receive once they are.
            if let Err(e) = session.announce(&config).await {
                tracing::warn!(error = %e, "failed to publish profile / inbox relays");
            }
        }
        Ok(session)
    }

    pub fn public_key(&self) -> PublicKey {
        self.keys.public_key()
    }

    pub fn keys(&self) -> &Keys {
        &self.keys
    }

    pub fn relays(&self) -> &[RelayUrl] {
        &self.relays
    }

    /// Escape hatch for features not yet wrapped (manifests, slash command definitions...).
    pub fn client(&self) -> &Client {
        &self.client
    }

    async fn announce(&self, config: &SessionConfig) -> Result<(), CoreError> {
        let mut md = Metadata::new();
        if let Some(n) = &config.display_name {
            md = md.name(n).display_name(n);
        }
        if let Some(a) = &config.about {
            md = md.about(a);
        }
        if config.kind == IdentityKind::Agent {
            md = md.custom_field("bot", true);
        }
        self.client.set_metadata(&md).await?;

        // NIP-17: advertise where we want to receive DMs.
        let inbox = EventBuilder::new(kinds::kind(kinds::INBOX_RELAYS), "")
            .tags(self.relays.iter().cloned().map(Tag::relay));
        self.client.send_event_builder(inbox).await?;
        Ok(())
    }

    /// Send a plain-text NIP-17 DM.
    pub async fn send_text(&self, to: PublicKey, text: &str) -> Result<EventId, CoreError> {
        let rumor = EventBuilder::private_msg_rumor(to, text);
        self.wrap_and_send(to, rumor).await
    }

    /// Send a structured message as a gift-wrapped rumor (end-to-end encrypted).
    pub async fn send_structured(
        &self,
        to: PublicKey,
        msg: &StructuredMessage,
    ) -> Result<EventId, CoreError> {
        // `p` tag on the rumor so multi-party DMs (NIP-17 "rooms") know the participants.
        let rumor = msg.to_builder().tag(Tag::public_key(to));
        self.wrap_and_send(to, rumor).await
    }

    /// Send an action (button press / form submit) back to the author of a structured message.
    pub async fn send_action(
        &self,
        to: PublicKey,
        action: &nmesh_proto::Action,
    ) -> Result<EventId, CoreError> {
        let rumor = action.to_builder().tag(Tag::public_key(to));
        self.wrap_and_send(to, rumor).await
    }

    /// Gift-wrap a rumor to the recipient *and* to ourselves (so our other devices see it).
    /// Returns the rumor id, which is what replies and actions reference.
    pub(crate) async fn send_rumor(
        &self,
        to: PublicKey,
        rumor: EventBuilder,
    ) -> Result<EventId, CoreError> {
        self.wrap_and_send(to, rumor).await
    }

    async fn wrap_and_send(
        &self,
        to: PublicKey,
        rumor: EventBuilder,
    ) -> Result<EventId, CoreError> {
        let mut rumor = rumor.build(self.public_key());
        let id = rumor.id();
        let out = self.client.gift_wrap(&to, rumor.clone(), []).await?;
        if out.success.is_empty() {
            // Relays answer OK=false (e.g. rate limits) without an error; don't lose it silently.
            tracing::warn!(failed = ?out.failed, "no relay accepted the gift wrap");
        }
        if to != self.public_key() {
            self.client.gift_wrap(&self.public_key(), rumor, []).await?;
        }
        Ok(id)
    }

    /// Subscribe to our inbox and return a stream of decrypted messages from others.
    pub async fn inbox(&self) -> Result<mpsc::Receiver<Inbound>, CoreError> {
        self.inbox_with(InboxOptions::default()).await
    }

    /// Subscribe to our inbox with explicit options.
    pub async fn inbox_with(
        &self,
        opts: InboxOptions,
    ) -> Result<mpsc::Receiver<Inbound>, CoreError> {
        let (tx, rx) = mpsc::channel(256);
        let lookback = opts.lookback.max(GIFT_WRAP_LOOKBACK);
        let include_own = opts.include_own;
        let filter = Filter::new()
            .kind(Kind::GiftWrap)
            .pubkey(self.public_key())
            .since(Timestamp::now() - lookback);

        let mut notifications = self.client.notifications();
        self.client.subscribe(filter, None).await?;

        let client = self.client.clone();
        let me = self.public_key();
        let seen: Arc<Mutex<HashSet<EventId>>> = Arc::default();
        tokio::spawn(async move {
            loop {
                let n = match notifications.recv().await {
                    Ok(n) => n,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "inbox notification stream lagged");
                        continue;
                    }
                    Err(_) => break,
                };
                let RelayPoolNotification::Event { event, .. } = n else {
                    continue;
                };
                if event.kind != Kind::GiftWrap {
                    continue;
                }
                let unwrapped = match client.unwrap_gift_wrap(&event).await {
                    Ok(u) => u,
                    Err(e) => {
                        tracing::debug!(error = %e, "failed to unwrap gift wrap");
                        continue;
                    }
                };
                if unwrapped.sender == me && !include_own {
                    continue;
                }
                let msg = Inbound::from_rumor(unwrapped.sender, unwrapped.rumor);
                if !seen.lock().await.insert(msg.id) {
                    continue;
                }
                if tx.send(msg).await.is_err() {
                    break;
                }
            }
        });
        Ok(rx)
    }

    pub async fn shutdown(&self) {
        self.client.shutdown().await;
    }
}
