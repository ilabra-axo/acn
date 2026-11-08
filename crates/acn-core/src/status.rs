//! Time-dependent status (spec §6.5). Kept out of the fold so the fold stays time-free (INV-2).

use serde::Serialize;

use crate::fold::{Commitment, Phase};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Party {
    Requester,
    Performer,
    Arbiter,
    /// The settlement verifier (CR-002 verifier trigger).
    Verifier,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    /// `None` while terms are pending.
    pub phase: Option<Phase>,
    pub terminal: bool,
    /// Who missed a deadline, if anyone (scored, never announced).
    pub lapsed: Option<Party>,
    /// Whose move it is, for queue views.
    pub awaiting: Option<Party>,
    /// Seconds until the relevant deadline (negative once passed).
    pub deadline_in: Option<i64>,
}

pub fn status_at(c: &Commitment, now: u64) -> Status {
    let (Some(st), Some(terms)) = (&c.state, &c.terms) else {
        return Status {
            phase: None,
            terminal: false,
            lapsed: None,
            awaiting: None,
            deadline_in: None,
        };
    };
    let now_i = now as i64;
    let verifier = terms.settlement_trigger == crate::terms::SettlementTrigger::Verifier;
    // A disqualifier hit counts as failing: the move is the requester's (REJECT), and nobody is
    // obliged to release, fund or satisfy (CR-003 §2.2).
    let failing = c.disqualified()
        || c.conditions
            .values()
            .any(|s| *s == crate::fold::ConditionStatus::Fail);
    // Contingent: the funding deadline, once every condition passed (and nothing was hit).
    let passed = c
        .passed_at
        .filter(|_| !c.disqualified())
        .zip(terms.fund_window)
        .map(|(p, w)| p.saturating_add(w));
    let rejected_final = st.ruled
        || terms.arbiter.is_none()
        || st
            .rejected_at
            .is_some_and(|r| now > r.saturating_add(terms.review_window));
    let terminal = match st.phase {
        Phase::Satisfied | Phase::Declined | Phase::Withdrawn | Phase::Revoked | Phase::Settled => {
            true
        }
        Phase::Rejected => rejected_final,
        _ => false,
    };
    let (awaiting, deadline) = match st.phase {
        Phase::Requested | Phase::Promised | Phase::Reworking => {
            (Some(Party::Performer), Some(terms.deadline))
        }
        Phase::Countered => {
            let by_requester = st.pending.as_ref().is_some_and(|p| p.by == c.requester);
            (
                Some(if by_requester {
                    Party::Performer
                } else {
                    Party::Requester
                }),
                None,
            )
        }
        // Verifier trigger: the verifier releases on pass; a failing attestation hands the
        // move to the requester (REJECT for rework).
        Phase::Reported if verifier && !failing => (
            Some(Party::Verifier),
            st.report_at.map(|r| r.saturating_add(terms.review_window)),
        ),
        Phase::Reported => (
            Some(Party::Requester),
            st.report_at.map(|r| r.saturating_add(terms.review_window)),
        ),
        // Contingent: the verifier attests; once everything passes, the requester funds within
        // fund_window; once funded, the performer claims before the funding lock expires.
        Phase::Offered if passed.is_some() => (Some(Party::Requester), passed),
        Phase::Offered if failing => (
            Some(Party::Requester),
            st.report_at.map(|r| r.saturating_add(terms.review_window)),
        ),
        Phase::Offered => (
            Some(Party::Verifier),
            st.report_at.map(|r| r.saturating_add(terms.review_window)),
        ),
        Phase::Funded => (Some(Party::Performer), st.fund_locktime),
        Phase::Disputed => (Some(Party::Arbiter), None),
        Phase::Rejected if !terminal => (
            Some(Party::Performer),
            st.rejected_at
                .map(|r| r.saturating_add(terms.review_window)),
        ),
        _ => (None, None),
    };
    let lapsed = match st.phase {
        Phase::Promised | Phase::Reworking if now > terms.deadline => Some(Party::Performer),
        Phase::Reported
            if st
                .report_at
                .is_some_and(|r| now > r.saturating_add(terms.review_window)) =>
        {
            // Verifier trigger: no RELEASE and no failing attestation in time is the verifier's
            // fault; the payment refunds at the locktime.
            Some(if verifier && !failing {
                Party::Verifier
            } else {
                Party::Requester
            })
        }
        // Unfunded passing offer: requester fault (it never got the key, so it gains nothing).
        Phase::Offered if passed.is_some_and(|d| now > d) => Some(Party::Requester),
        // No attestation in time: verifier fault.
        Phase::Offered
            if passed.is_none()
                && !failing
                && st
                    .report_at
                    .is_some_and(|r| now > r.saturating_add(terms.review_window)) =>
        {
            Some(Party::Verifier)
        }
        // Funded but never claimed: performer fault; the payment refunds at its locktime.
        Phase::Funded if st.fund_locktime.is_some_and(|l| now > l) => Some(Party::Performer),
        _ => None,
    };
    Status {
        phase: Some(st.phase),
        terminal,
        lapsed,
        awaiting: if terminal { None } else { awaiting },
        deadline_in: deadline.map(|d| d as i64 - now_i),
    }
}
