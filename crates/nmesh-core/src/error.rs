#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error(transparent)]
    Client(#[from] nostr_sdk::client::Error),
    #[error(transparent)]
    Proto(#[from] nmesh_proto::ProtoError),
    #[error(transparent)]
    Contract(#[from] nmesh_contracts::ContractError),
    #[error("key error: {0}")]
    Key(#[from] nostr_sdk::prelude::key::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("relay url error: {0}")]
    RelayUrl(#[from] nostr_sdk::prelude::types::url::Error),
    #[error("{0}")]
    Other(String),
}
