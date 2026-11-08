//! Marmot (MLS over Nostr) E2E groups — **spike, feature `mls`**.
//!
//! nmesh does not implement MLS itself. It uses MDK (`mdk-core`), the reference Marmot
//! implementation used by White Noise, so groups interoperate with other Marmot clients.
//! NIP-EE is superseded by Marmot; see docs/adr/0003-groups-mls.md.
//!
//! Group application messages are rumors (exactly like NIP-17), so nmesh structured messages
//! and actions travel inside MLS groups unchanged — `Inbound::from_rumor` handles both paths.
//!
//! Status: only KeyPackage creation is wired up. Group create / welcome / message processing is
//! roadmap Phase 2 (docs/ROADMAP.md). Storage is in-memory; production needs `mdk-sqlite-storage`.

use mdk_core::MDK;
use mdk_memory_storage::MdkMemoryStorage;
use nostr_sdk::prelude::*;

use crate::{CoreError, Session};

/// Marmot KeyPackage (addressable, kind 30443). MDK also emits legacy kind 443 tags.
pub const KEY_PACKAGE_KIND: u16 = 30443;

pub struct Groups {
    mdk: MDK<MdkMemoryStorage>,
}

impl Default for Groups {
    fn default() -> Self {
        Self {
            mdk: MDK::new(MdkMemoryStorage::default()),
        }
    }
}

impl Groups {
    pub fn mdk(&self) -> &MDK<MdkMemoryStorage> {
        &self.mdk
    }

    /// Create and publish a KeyPackage so others can add this identity to groups.
    pub async fn publish_key_package(&self, session: &Session) -> Result<EventId, CoreError> {
        let kp = self
            .mdk
            .create_key_package_for_event(&session.public_key(), session.relays().to_vec())
            .map_err(|e| CoreError::Other(format!("mls: {e}")))?;
        let builder =
            EventBuilder::new(Kind::Custom(KEY_PACKAGE_KIND), kp.content).tags(kp.tags_30443);
        let out = session.client().send_event_builder(builder).await?;
        Ok(*out.id())
    }
}
