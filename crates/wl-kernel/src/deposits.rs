//! The deposit contract (ARCHITECTURE §5, wl-payments §4 *Deposit*, §5.4, §6):
//! what the portal and the verticals (01 Property) may do with deposits
//! without depending on the `wl-payments` crate.
//!
//! - **Deductions shown to the party.** When a deposit's settlement starts,
//!   staff itemise the deductions with evidence and publish them in numbered
//!   versions. Where the vertical allows it (a setting, off by default, on for
//!   01), the party may dispute lines in the portal within the dispute window.
//!   `wl-payments` provides [`DepositDeductions`] with
//!   [`provide_deposit_deductions`]; the portal reads what to show with
//!   [`DepositDeductions::published_for`] and sends a dispute with
//!   [`DepositDeductions::dispute`]. [`deposit_deductions`] falls back to
//!   [`NoDepositDeductions`] when `payments` is off: nothing is shown and a
//!   dispute is refused.
//! - **Events.** [`DepositDeductionsPublished`], [`DepositDeductionsDisputed`]
//!   and [`DepositDisputeAnswered`] let the vertical's rule (01 `PRP.RUL.11`)
//!   count the window and escalate an unanswered dispute; [`DepositSettled`]
//!   says a deposit was refunded, forfeited or transferred.
//!
//! Money is `i64` in fils of the deposit's currency.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::parties::PartyId;
use crate::payments::DepositId;
use crate::subject::Subject;

/// What a published version of the deductions is.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DeductionKind {
    /// The itemised deductions from the deposit.
    #[default]
    Deductions,
    /// A forfeiture, published as the deductions it is applied to or as one
    /// line for the whole deposit with the reason.
    Forfeiture,
}

/// One line of a published version, as the party sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeductionLineView {
    /// The line's id, as [`DisputeDeductions::lines`] names it.
    pub id: i64,
    /// What the deduction is for, e.g. "Repainting".
    pub description: String,
    /// The amount, in fils.
    pub amount: i64,
    /// The evidence reference (inspection photos, a quote).
    pub evidence: String,
}

/// The deductions of a deposit as published to the party: the latest version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedDeductions {
    /// The deposit.
    pub deposit: DepositId,
    /// What it is held for (the lease).
    pub source: Subject,
    /// The version, from 1; a revision publishes the next one.
    pub version: i32,
    /// Deductions, or a forfeiture.
    pub kind: DeductionKind,
    /// The reason of a forfeiture.
    pub reason: Option<String>,
    /// The ISO currency code.
    pub currency: String,
    /// The day it was published (Asia/Dubai).
    pub published_on: NaiveDate,
    /// The last day the party may dispute (23:59 Asia/Dubai); `None` when the
    /// vertical does not allow a dispute.
    pub window_ends_on: Option<NaiveDate>,
    /// The lines.
    pub lines: Vec<DeductionLineView>,
    /// The ids of the lines the party disputed and staff have not answered.
    pub disputed_lines: Vec<i64>,
    /// Whether the party may dispute now: allowed, the window is open and the
    /// settlement is pending. The portal shows its dispute button only then.
    pub can_dispute: bool,
}

/// A dispute of deduction lines by the party.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisputeDeductions {
    /// The deposit.
    pub deposit: DepositId,
    /// The version the party saw; a dispute of an older version is refused.
    pub version: i32,
    /// The lines disputed.
    pub lines: Vec<i64>,
    /// Why, in the party's words; required.
    pub comment: String,
}

/// The future a [`DepositDeductions`] method returns.
pub type DepositFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Shows the published deductions to the portal and takes the party's dispute.
/// `wl-payments` provides it; without that module, [`deposit_deductions`] gives
/// [`NoDepositDeductions`].
pub trait DepositDeductions: Send + Sync + 'static {
    /// The latest published deductions of the deposits of `party` that are
    /// being settled, newest first. The default has none.
    fn published_for<'a>(
        &'a self,
        _state: &'a AppState,
        _party: PartyId,
    ) -> DepositFuture<'a, Vec<PublishedDeductions>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// Records `party`'s dispute of lines of the latest published version.
    /// Refused (422) when the vertical does not allow a dispute, the window
    /// has passed, the version is not the latest, a line is not on it or is
    /// disputed already, the comment is blank, or the deposit is not the
    /// party's. The default refuses (422): payments is not enabled.
    fn dispute<'a>(
        &'a self,
        _state: &'a AppState,
        _party: PartyId,
        request: DisputeDeductions,
    ) -> DepositFuture<'a, ()> {
        Box::pin(async move {
            Err(Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!(
                    "the deductions of deposit {} cannot be disputed: payments is not enabled",
                    request.deposit
                ),
            ))
        })
    }
}

/// The deductions when the `payments` module is off: nothing is published
/// and a dispute is refused.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoDepositDeductions;

impl DepositDeductions for NoDepositDeductions {}

/// Makes `deductions` the app's [`DepositDeductions`]; `wl-payments` calls it
/// in `register`.
pub fn provide_deposit_deductions(app: &mut Registry, deductions: impl DepositDeductions) {
    let deductions: Arc<dyn DepositDeductions> = Arc::new(deductions);
    app.provide(deductions);
}

/// The app's [`DepositDeductions`], or [`NoDepositDeductions`] when none is
/// provided.
pub fn deposit_deductions(state: &AppState) -> Arc<dyn DepositDeductions> {
    match state.provided::<Arc<dyn DepositDeductions>>() {
        Some(deductions) => Arc::clone(&deductions),
        None => Arc::new(NoDepositDeductions),
    }
}

/// Deductions (or a forfeiture) were published to the party: a version, and
/// the window in which it may dispute starts (01 `PRP.RUL.11` counts N days
/// from here).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepositDeductionsPublished {
    /// The deposit.
    pub deposit: DepositId,
    /// What it is held for.
    pub source: Subject,
    /// Whose deposit it is.
    pub party: PartyId,
    /// The version.
    pub version: i32,
    /// Deductions, or a forfeiture.
    pub kind: DeductionKind,
    /// What the version deducts in all, in fils.
    pub total: i64,
    /// The day it was published.
    pub published_on: NaiveDate,
    /// The last day of the dispute window; `None` without a dispute setting.
    pub window_ends_on: Option<NaiveDate>,
}

impl Event for DepositDeductionsPublished {}

/// The party disputed lines of the published deductions: the flag is set and
/// `refund` and `forfeit` wait (01 `PRP.RUL.11` escalates an unanswered
/// dispute).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepositDeductionsDisputed {
    /// The deposit.
    pub deposit: DepositId,
    /// What it is held for.
    pub source: Subject,
    /// Whose deposit it is.
    pub party: PartyId,
    /// The version disputed.
    pub version: i32,
    /// The ids of the lines.
    pub lines: Vec<i64>,
    /// The day it was disputed.
    pub raised_on: NaiveDate,
}

impl Event for DepositDeductionsDisputed {}

/// How staff answered a dispute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisputeAnswer {
    /// The deductions were revised into a new published version; the window
    /// starts again.
    Revised,
    /// The deductions stand; the window does not restart.
    Upheld,
}

/// Staff answered disputed lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepositDisputeAnswered {
    /// The deposit.
    pub deposit: DepositId,
    /// What it is held for.
    pub source: Subject,
    /// Whose deposit it is.
    pub party: PartyId,
    /// The version that was disputed.
    pub version: i32,
    /// How it was answered.
    pub answer: DisputeAnswer,
    /// The new version, when the deductions were revised.
    pub new_version: Option<i32>,
    /// Whether any line of the deposit is still disputed.
    pub still_open: bool,
}

impl Event for DepositDisputeAnswered {}

/// How a deposit was settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepositOutcome {
    /// Paid back, possibly after deductions.
    Refunded,
    /// Kept in full, with a reason.
    Forfeited,
    /// Handed over to another holder with the source agreement.
    Transferred,
}

/// A deposit was settled (`refund`, `forfeit` or `transfer`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepositSettled {
    /// The deposit.
    pub deposit: DepositId,
    /// What it was held for.
    pub source: Subject,
    /// Whose deposit it was.
    pub party: PartyId,
    /// How it was settled.
    pub outcome: DepositOutcome,
    /// What the latest published version deducts, in fils; 0 without.
    pub deductions: i64,
    /// What is paid back, in fils: the refund of a `refund`, else 0.
    pub refund_amount: i64,
    /// The new holder of a transferred deposit.
    pub new_holder: Option<PartyId>,
}

impl Event for DepositSettled {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deposit_outcome_is_stored_in_snake_case() {
        let settled = DepositSettled {
            deposit: DepositId(3),
            source: Subject::new("property.lease", 9),
            party: PartyId(7),
            outcome: DepositOutcome::Transferred,
            deductions: 0,
            refund_amount: 0,
            new_holder: Some(PartyId(11)),
        };
        let json = renox::serde_json::to_value(&settled).unwrap();
        assert_eq!(json["deposit"], 3);
        assert_eq!(json["outcome"], "transferred");
        assert_eq!(json["new_holder"], 11);
    }

    #[test]
    fn a_kind_and_an_answer_have_names() {
        assert_eq!(DeductionKind::Forfeiture.as_str(), "forfeiture");
        assert_eq!(DeductionKind::default(), DeductionKind::Deductions);
        let json = renox::serde_json::to_value(DisputeAnswer::Upheld).unwrap();
        assert_eq!(json, "upheld");
    }
}
