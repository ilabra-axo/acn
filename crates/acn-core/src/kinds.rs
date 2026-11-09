//! Provisional kind block (spec §5.1, open decision D-1). Code MUST use these names, never literals.

pub const VERSION: &str = "acn/0.4";

pub const K_REQUEST: u16 = 7700;
pub const K_COUNTER: u16 = 7701;
pub const K_ACCEPT: u16 = 7702;
pub const K_PROMISE: u16 = 7703;
pub const K_DECLINE: u16 = 7704;
pub const K_WITHDRAW: u16 = 7705;
pub const K_REVOKE: u16 = 7706;
pub const K_PROGRESS: u16 = 7707;
pub const K_REPORT: u16 = 7708;
pub const K_SATISFY: u16 = 7709;
pub const K_REJECT: u16 = 7710;
pub const K_DISPUTE: u16 = 7711;
pub const K_RULING: u16 = 7712;
/// Settlement verifier releases the escrowed secret on pass (CR-002, verifier trigger).
pub const K_RELEASE: u16 = 7713;
/// Requester pays against the REPORT's hashlock (CR-002, contingent trigger).
pub const K_FUND: u16 = 7714;
/// Performer reveals the key it was paid for (CR-002, contingent trigger).
pub const K_CLAIM: u16 = 7715;

pub const K_ATTEST_MECH: u16 = 7720;
pub const K_ATTEST_GROUND: u16 = 7721;
pub const K_INTERP_REPORT: u16 = 7722;
pub const K_ENDORSE: u16 = 7723;
pub const K_SCORE: u16 = 7724;
/// Judged attestation (CR-001): pinned judges' scores for a `judged` condition.
pub const K_ATTEST_JUDGED: u16 = 7725;
/// Escrow holder (settlement verifier, or arbiter) confirms the escrowed secret hashes to `H`.
pub const K_ESCROW_ACK: u16 = 7726;
/// Terms template (CR-003): a signed, parameterized fragment of terms. Regular: its id pins it.
pub const K_TERMS_TEMPLATE: u16 = 7727;

pub const K_BIND: u16 = 7730;
pub const K_BIND_ACK: u16 = 7731;
pub const K_BIND_REVOKE: u16 = 7732;

/// Rumor-only envelope for private mode (spec §5.5). Never published signed.
pub const K_PRIVATE_ACT: u16 = 7739;
/// Service advertisement (addressable).
pub const K_SVC: u16 = 37700;

pub const BLOCK: std::ops::RangeInclusive<u16> = 7700..=7739;

/// Commitment acts (advancing + PROGRESS), in kind order.
pub const ACTS: &[u16] = &[
    K_REQUEST, K_COUNTER, K_ACCEPT, K_PROMISE, K_DECLINE, K_WITHDRAW, K_REVOKE, K_PROGRESS,
    K_REPORT, K_SATISFY, K_REJECT, K_DISPUTE, K_RULING, K_RELEASE, K_FUND, K_CLAIM,
];

pub fn is_known(kind: u16) -> bool {
    ACTS.contains(&kind)
        || matches!(
            kind,
            K_ATTEST_MECH
                | K_ATTEST_GROUND
                | K_ATTEST_JUDGED
                | K_ESCROW_ACK
                | K_TERMS_TEMPLATE
                | K_INTERP_REPORT
                | K_ENDORSE
                | K_SCORE
                | K_BIND
                | K_BIND_ACK
                | K_BIND_REVOKE
                | K_PRIVATE_ACT
                | K_SVC
        )
}

/// Human-readable act name (used in anomaly details and UIs).
pub fn name(kind: u16) -> &'static str {
    match kind {
        K_REQUEST => "REQUEST",
        K_COUNTER => "COUNTER",
        K_ACCEPT => "ACCEPT",
        K_PROMISE => "PROMISE",
        K_DECLINE => "DECLINE",
        K_WITHDRAW => "WITHDRAW",
        K_REVOKE => "REVOKE",
        K_PROGRESS => "PROGRESS",
        K_REPORT => "REPORT",
        K_SATISFY => "SATISFY",
        K_REJECT => "REJECT",
        K_DISPUTE => "DISPUTE",
        K_RULING => "RULING",
        K_RELEASE => "RELEASE",
        K_FUND => "FUND",
        K_CLAIM => "CLAIM",
        K_ATTEST_MECH => "ATTEST_MECH",
        K_ATTEST_GROUND => "ATTEST_GROUND",
        K_ATTEST_JUDGED => "ATTEST_JUDGED",
        K_ESCROW_ACK => "ESCROW_ACK",
        K_TERMS_TEMPLATE => "TERMS_TEMPLATE",
        K_INTERP_REPORT => "INTERP_REPORT",
        K_ENDORSE => "ENDORSE",
        K_SCORE => "SCORE",
        K_BIND => "BIND",
        K_BIND_ACK => "BIND_ACK",
        K_BIND_REVOKE => "BIND_REVOKE",
        K_PRIVATE_ACT => "PRIVATE_ACT",
        K_SVC => "SVC",
        _ => "UNKNOWN",
    }
}
