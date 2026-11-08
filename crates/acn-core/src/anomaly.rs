//! Anomaly codes and severities (spec §10, INV-11).

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Retained, not applied.
    Reject,
    /// Applied, noted.
    Flag,
    /// A dependency is missing; re-evaluated when more events/documents arrive.
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    BadSig,
    BadVersion,
    UnknownKind,
    Malformed,
    ForeignPrev,
    PendingRef,
    Unauthorized,
    IllegalTransition,
    Stale,
    Unbound,
    OutOfScope,
    Revoked,
    TermsMismatch,
    PendingTerms,
    BadPreimage,
    WrongEvidenceClass,
    UnlistedVerifier,
    Equivocation,
    PendingEndorsement,
    Unendorsed,
    TimeRegress,
    ContradictedRejection,
    Late,
    // V-17 judged evidence (CR-001).
    SelfJudged,
    JudgeMismatch,
    RubricMismatch,
    InjectionDetected,
    BelowThreshold,
    // Settlement triggers (CR-002).
    InvalidSettlementCombo,
    MissingEscrowAck,
    BadRelease,
    // Contingent settlement (CR-002 Phase C).
    ContingentNondeterministic,
    AttestationMismatch,
    FundMismatch,
    PrematureFund,
    BadKey,
}

impl Code {
    pub const ALL: &'static [Code] = &[
        Code::BadSig,
        Code::BadVersion,
        Code::UnknownKind,
        Code::Malformed,
        Code::ForeignPrev,
        Code::PendingRef,
        Code::Unauthorized,
        Code::IllegalTransition,
        Code::Stale,
        Code::Unbound,
        Code::OutOfScope,
        Code::Revoked,
        Code::TermsMismatch,
        Code::PendingTerms,
        Code::BadPreimage,
        Code::WrongEvidenceClass,
        Code::UnlistedVerifier,
        Code::Equivocation,
        Code::PendingEndorsement,
        Code::Unendorsed,
        Code::TimeRegress,
        Code::ContradictedRejection,
        Code::Late,
        Code::SelfJudged,
        Code::JudgeMismatch,
        Code::RubricMismatch,
        Code::InjectionDetected,
        Code::BelowThreshold,
        Code::InvalidSettlementCombo,
        Code::MissingEscrowAck,
        Code::BadRelease,
        Code::ContingentNondeterministic,
        Code::AttestationMismatch,
        Code::FundMismatch,
        Code::PrematureFund,
        Code::BadKey,
    ];

    pub fn severity(self) -> Severity {
        use Code::*;
        match self {
            PendingRef | PendingTerms | PendingEndorsement => Severity::Pending,
            Unbound
            | WrongEvidenceClass
            | UnlistedVerifier
            | Equivocation
            | ContradictedRejection
            | Late
            | SelfJudged
            | JudgeMismatch
            | RubricMismatch
            | InjectionDetected
            | BelowThreshold
            | AttestationMismatch
            | PrematureFund => Severity::Flag,
            _ => Severity::Reject,
        }
    }

    pub fn as_str(self) -> &'static str {
        use Code::*;
        match self {
            BadSig => "bad_sig",
            BadVersion => "bad_version",
            UnknownKind => "unknown_kind",
            Malformed => "malformed",
            ForeignPrev => "foreign_prev",
            PendingRef => "pending_ref",
            Unauthorized => "unauthorized",
            IllegalTransition => "illegal_transition",
            Stale => "stale",
            Unbound => "unbound",
            OutOfScope => "out_of_scope",
            Revoked => "revoked",
            TermsMismatch => "terms_mismatch",
            PendingTerms => "pending_terms",
            BadPreimage => "bad_preimage",
            WrongEvidenceClass => "wrong_evidence_class",
            UnlistedVerifier => "unlisted_verifier",
            Equivocation => "equivocation",
            PendingEndorsement => "pending_endorsement",
            Unendorsed => "unendorsed",
            TimeRegress => "time_regress",
            ContradictedRejection => "contradicted_rejection",
            Late => "late",
            SelfJudged => "self_judged",
            JudgeMismatch => "judge_mismatch",
            RubricMismatch => "rubric_mismatch",
            InjectionDetected => "injection_detected",
            BelowThreshold => "below_threshold",
            InvalidSettlementCombo => "invalid_settlement_combo",
            MissingEscrowAck => "missing_escrow_ack",
            BadRelease => "bad_release",
            ContingentNondeterministic => "contingent_nondeterministic",
            AttestationMismatch => "attestation_mismatch",
            FundMismatch => "fund_mismatch",
            PrematureFund => "premature_fund",
            BadKey => "bad_key",
        }
    }
}

/// One finding about one event. Ordered so output is independent of arrival order.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Anomaly {
    pub event: String,
    pub code: Code,
    pub severity: Severity,
    pub detail: String,
}

impl Anomaly {
    pub fn new(event: impl Into<String>, code: Code, detail: impl Into<String>) -> Self {
        Self {
            event: event.into(),
            code,
            severity: code.severity(),
            detail: detail.into(),
        }
    }
}
