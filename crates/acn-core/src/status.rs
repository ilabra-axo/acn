//! Time-dependent status (spec §6.5). Kept out of the fold so the fold stays time-free (INV-2).

use serde::Serialize;

use crate::fold::{Commitment, Phase};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Party {
    Requester,
    Performer,
    Arbiter,
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
    let rejected_final = st.ruled
        || terms.arbiter.is_none()
        || st
            .rejected_at
            .is_some_and(|r| now > r.saturating_add(terms.review_window));
    let terminal = match st.phase {
        Phase::Satisfied | Phase::Declined | Phase::Withdrawn | Phase::Revoked => true,
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
        Phase::Reported => (
            Some(Party::Requester),
            st.report_at.map(|r| r.saturating_add(terms.review_window)),
        ),
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
            Some(Party::Requester)
        }
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
