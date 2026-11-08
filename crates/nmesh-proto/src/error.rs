#[derive(Debug, thiserror::Error)]
pub enum ProtoError {
    #[error("unexpected event kind {found}, expected {expected}")]
    UnexpectedKind { expected: u16, found: u16 },
    #[error("missing required tag `{0}`")]
    MissingTag(&'static str),
    #[error("invalid tag `{tag}`: {reason}")]
    InvalidTag { tag: &'static str, reason: String },
    #[error("invalid JSON content: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Tag(#[from] nostr::event::tag::Error),
}
