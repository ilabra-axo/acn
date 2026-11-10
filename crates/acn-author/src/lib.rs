//! `acn-author`: authoring aids for ACN terms (CR-003 §3). Core validates terms and never helps
//! write them; these aids live in bindings. Everything here produces or reads ordinary terms, so
//! the validator judges the result like any other.
//!
//! - [`lint`]: rules L1–L8 on any draft, from either side, before it is sent. Errors block
//!   sending; warnings go on the terms card.
//! - [`draft_request`]: the requester drafter. An intent, its requirements and optional templates
//!   in; terms out, matching a template where one fits and leaving what it can't close `open`.
//! - [`draft_counter`]: the performer drafter. A `REQUEST` with open conditions and the
//!   performer's templates in; a `COUNTER` that closes them, with one line per change.
//!
//! These are the reference binding's `TermsAuthor` (`draft` and `critique`) and the performer
//! side of `Executor.quote` returning `Counter` (docs/harness-binding-API.md).
//!
//! Pure: no I/O. What the aids would need to fetch (whether a rubric is retrievable, its text)
//! is passed in.

mod draft;
mod lint;

pub use draft::{
    draft_counter, draft_request, Capabilities, Change, CounterDraft, Intent, RequestDraft,
    RequesterConfig, TemplateCandidate,
};
pub use lint::{lint, lint_value, Finding, Level, LintContext};
