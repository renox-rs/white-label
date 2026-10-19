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
//! - **A change of the amount while the deposit is held.** A lease amendment or
//!   renewal (01) calls [`Deposits::adjust`] with the new amount and its
//!   version: an increase raises a top-up charge ([`DepositTopUps`], provided
//!   by the module that owns the charges), a decrease becomes a part refund
//!   through the Refund flow; [`DepositAdjusted`] says what was done.
//! - **Events.** [`DepositDeductionsPublished`], [`DepositDeductionsDisputed`]
//!   and [`DepositDisputeAnswered`] let the vertical's rule (01 `PRP.RUL.11`)
//!   count the window and escalate an unanswered dispute; [`DepositSettled`]
//!   says a deposit was refunded, forfeited or transferred.
//!
//! - **A forfeiture applied to the deductions.** When the deductions exceed
//!   the deposit (01 Property §4.3), the vertical asks [`Deposits::apply_to_deductions`]
//!   to forfeit the deposit with the reason [`APPLIED_TO_DEDUCTIONS`] and to
//!   apply it first to the unpaid invoices it names, oldest first, as payments
//!   of method *deposit applied*; what is left covers the damage lines. The
//!   vertical invoices the rest of the damages itself.
//!
//! Money is `i64` in fils of the deposit's currency.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::invoicing::InvoiceId;
use crate::parties::PartyId;
use crate::payments::{DepositId, RefundId};
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

/// A deposit's required amount changed while it is held: what to adjust it to
/// ([`Deposits::adjust`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdjustDeposit {
    /// What the deposit is held for (the lease).
    pub source: Subject,
    /// The new amount the source requires, in fils; above 0.
    pub new_amount: i64,
    /// The version of the source that changed it (the lease amendment or
    /// renewal version); above 0.
    pub source_version: i32,
    /// The day the change takes effect: the due date of a top-up charge.
    pub effective_on: NaiveDate,
}

/// What the module that owns the charges is asked to raise for an increase of
/// a deposit ([`DepositTopUps::request`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopUpCharge {
    /// The deposit.
    pub deposit: DepositId,
    /// Who pays: the deposit's party.
    pub party: PartyId,
    /// The legal entity issuing the charge.
    pub legal_entity_id: i64,
    /// What the deposit is held for: the charge is scheduled on it.
    pub source: Subject,
    /// The version of the source that raised the amount.
    pub source_version: i32,
    /// The increase, in fils.
    pub amount: i64,
    /// The ISO currency code.
    pub currency: String,
    /// The day the charge is due: the effective date.
    pub due: NaiveDate,
}

/// Raises the top-up charge of a deposit that grew. The module that owns the
/// charges (wl-invoicing) provides it; without one, [`deposit_top_ups`] gives
/// [`NoDepositTopUps`], which raises nothing, and `wl-payments` leaves the
/// accountant a task to raise it.
pub trait DepositTopUps: Send + Sync + 'static {
    /// Schedules `charge` and returns it as a [`Subject`], or `None` when no
    /// charge was made (the default).
    fn request<'a>(
        &'a self,
        _state: &'a AppState,
        _charge: TopUpCharge,
    ) -> DepositFuture<'a, Option<Subject>> {
        Box::pin(async { Ok(None) })
    }
}

/// The top-up charges when nothing can schedule a charge: none is made.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoDepositTopUps;

impl DepositTopUps for NoDepositTopUps {}

/// Makes `top_ups` the app's [`DepositTopUps`].
pub fn provide_deposit_top_ups(app: &mut Registry, top_ups: impl DepositTopUps) {
    let top_ups: Arc<dyn DepositTopUps> = Arc::new(top_ups);
    app.provide(top_ups);
}

/// The app's [`DepositTopUps`], or [`NoDepositTopUps`] when none is provided.
pub fn deposit_top_ups(state: &AppState) -> Arc<dyn DepositTopUps> {
    match state.provided::<Arc<dyn DepositTopUps>>() {
        Some(top_ups) => Arc::clone(&top_ups),
        None => Arc::new(NoDepositTopUps),
    }
}

/// A deposit held for a source: [`Deposits::held_for`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeldDeposit {
    /// The deposit.
    pub deposit: DepositId,
    /// Whose deposit it is.
    pub party: PartyId,
    /// What the source requires, in fils.
    pub required: i64,
    /// What is held, in fils: the money allocated to the deposit.
    pub held: i64,
    /// The ISO currency code.
    pub currency: String,
}

/// The settlement of the deposit of a cancelled agreement
/// ([`Deposits::settle_cancelled`]): the agreement never started, so the
/// deposit is refunded, or part of it forfeited where the signed terms allow
/// it. The part forfeited is published to the party as a deduction line with
/// the clause as its evidence; the refund of the rest waits for the dispute
/// window like any settlement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelSettlement {
    /// What the deposit is held for (the lease).
    pub source: Subject,
    /// The part forfeited, in fils; 0 for none.
    pub forfeit: i64,
    /// The clause of the signed terms that allows the forfeit; required when
    /// `forfeit` is above 0.
    pub clause: Option<String>,
}

impl CancelSettlement {
    /// A settlement of the deposit held for `source` with nothing forfeited.
    pub fn new(source: Subject) -> Self {
        CancelSettlement {
            source,
            forfeit: 0,
            clause: None,
        }
    }

    /// With `forfeit` fils kept under `clause`.
    pub fn forfeit(mut self, forfeit: i64, clause: &str) -> Self {
        self.forfeit = forfeit;
        self.clause = Some(clause.to_owned());
        self
    }
}

/// The reason of a forfeiture that applies the deposit to the deductions: the
/// whole deposit goes to the unpaid rent and the damage lines (01 Property
/// §4.3 row "Deductions exceed the deposit").
pub const APPLIED_TO_DEDUCTIONS: &str = "applied to deductions";

/// The part of a deposit applied to one unpaid invoice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyToInvoice {
    /// The invoice of the party.
    pub invoice: InvoiceId,
    /// In a request, what the invoice is open for; in a result, the amount
    /// applied to it. In fils, above 0, never above the invoice's balance.
    pub amount: i64,
}

/// A forfeiture of the deposit of `source` that applies it to the deductions
/// ([`Deposits::apply_to_deductions`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyDeposit {
    /// What the deposit is held for (the lease).
    pub source: Subject,
    /// The user whose approval the vertical vetted (a property manager); not
    /// the user who forfeits.
    pub approver: i64,
    /// The unpaid invoices of the party, oldest first, each with what it is
    /// open for. The deposit is applied to them in this order; what is left
    /// of it covers the damage lines of the published version.
    pub rent: Vec<ApplyToInvoice>,
}

/// What [`Deposits::apply_to_deductions`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepositApplied {
    /// The deposit, now `forfeited`.
    pub deposit: DepositId,
    /// Whose deposit it was.
    pub party: PartyId,
    /// The legal entity that held it.
    pub legal_entity_id: i64,
    /// What it held, in fils.
    pub held: i64,
    /// The part applied to invoices, in fils.
    pub to_invoices: i64,
    /// What was applied to each invoice, oldest first; invoices that got
    /// nothing are left out.
    pub shares: Vec<ApplyToInvoice>,
    /// The part kept for the damage lines, in fils: `held - to_invoices`.
    pub kept: i64,
    /// The published version of the forfeiture.
    pub version: i32,
}

/// What [`Deposits::settle_cancelled`] and [`Deposits::start_settlement`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlementStarted {
    /// The deposit, now `settlement_pending`.
    pub deposit: DepositId,
    /// What was held, in fils.
    pub held: i64,
    /// The part forfeited, in fils.
    pub forfeit: i64,
    /// The published version of the deductions: 1 after a cancellation, 0
    /// when nothing was published yet.
    pub version: i32,
}

/// What other modules do with deposits. `wl-payments` provides it; without
/// that module, [`deposits`] gives [`NoDeposits`].
pub trait Deposits: Send + Sync + 'static {
    /// The deposit held (`held`) for `source`, if there is one. The default
    /// finds none: payments is not enabled.
    fn held_for<'a>(
        &'a self,
        _db: &'a Db,
        _source: &'a Subject,
    ) -> DepositFuture<'a, Option<HeldDeposit>> {
        Box::pin(async { Ok(None) })
    }

    /// Starts the settlement of the held deposit of `request.source` because
    /// the agreement was cancelled, as the system on behalf of the user `by`,
    /// and publishes the first version of the deductions to the party: the
    /// forfeited part as a line with its clause, else no line. Refused (422)
    /// for a forfeit above the held amount, or above 0 without a clause.
    /// `None` when the source holds no deposit. The refund of the rest is the
    /// accountant's, after the dispute window (`deposits.refund`). The
    /// default finds no deposit: payments is not enabled.
    fn settle_cancelled<'a>(
        &'a self,
        _state: &'a AppState,
        _request: CancelSettlement,
        _by: Option<i64>,
    ) -> DepositFuture<'a, Option<SettlementStarted>> {
        Box::pin(async { Ok(None) })
    }

    /// Starts the settlement of the held deposit of `source` because its
    /// agreement ended (a lease terminated), as the system: the deposit
    /// becomes `settlement_pending` and nothing is published to the party yet
    /// (`version` of the result is 0); the deductions follow with the move-out
    /// inspection. `None` when the source holds no deposit. The default
    /// finds no deposit: payments is not enabled.
    fn start_settlement<'a>(
        &'a self,
        _state: &'a AppState,
        _source: &'a Subject,
    ) -> DepositFuture<'a, Option<SettlementStarted>> {
        Box::pin(async { Ok(None) })
    }

    /// Records that a legal case on the agreement `source` is open (`open`)
    /// or decided (`!open`): while it is open the deductions dispute flag of
    /// the deposit being settled stays, and its refund and forfeit wait
    /// (`dispute_legal_case`). Returns whether a deposit in settlement was
    /// found. The default finds none: payments is not enabled.
    fn set_legal_case<'a>(
        &'a self,
        _state: &'a AppState,
        _source: &'a Subject,
        _open: bool,
    ) -> DepositFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }

    /// Forfeits the deposit of `request.source` that is `settlement_pending`
    /// with the reason [`APPLIED_TO_DEDUCTIONS`], for the user `by` (who holds
    /// `deposits.refund`), approved by `request.approver`; the same waits as
    /// `forfeit` apply (no dispute open, the window of the published version
    /// passed). In the same step the deposit is applied to `request.rent`,
    /// oldest first, as payments of method *deposit applied* allocated to
    /// those invoices; the rest of the held amount is kept for the damage
    /// lines. Refused (422) when the source has no deposit being settled, the
    /// total deductions (the invoices' open amounts plus the damage lines of
    /// the published version) are below the deposit, or an invoice is not the
    /// party's or refuses the allocation. The default refuses (422):
    /// payments is not enabled.
    fn apply_to_deductions<'a>(
        &'a self,
        _state: &'a AppState,
        _by: &'a AuthUser,
        request: ApplyDeposit,
    ) -> DepositFuture<'a, DepositApplied> {
        Box::pin(async move {
            Err(Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!(
                    "the deposit of {} cannot be applied: payments is not enabled",
                    request.source
                ),
            ))
        })
    }

    /// Adjusts the held deposit of `request.source` to `request.new_amount`
    /// for the user `by` (`None` = the system, as a lease amendment or
    /// renewal `activate` does): an increase raises a top-up charge, a
    /// decrease asks for a part refund through the Refund flow. The deposit
    /// stays held. Returns the event, or `None` when the amount is the same
    /// and nothing was recorded. Refused (422) when the source has no held
    /// deposit, an earlier adjustment is still open, or the amount or the
    /// version is not above 0. The default refuses (422): payments is not
    /// enabled.
    fn adjust<'a>(
        &'a self,
        _state: &'a AppState,
        request: AdjustDeposit,
        _by: Option<i64>,
    ) -> DepositFuture<'a, Option<DepositAdjusted>> {
        Box::pin(async move {
            Err(Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!(
                    "the deposit of {} cannot be adjusted: payments is not enabled",
                    request.source
                ),
            ))
        })
    }
}

/// The deposits when the `payments` module is off.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoDeposits;

impl Deposits for NoDeposits {}

/// Makes `deposits` the app's [`Deposits`]; `wl-payments` calls it in
/// `register`.
pub fn provide_deposits(app: &mut Registry, deposits: impl Deposits) {
    let deposits: Arc<dyn Deposits> = Arc::new(deposits);
    app.provide(deposits);
}

/// The app's [`Deposits`], or [`NoDeposits`] when none is provided.
pub fn deposits(state: &AppState) -> Arc<dyn Deposits> {
    match state.provided::<Arc<dyn Deposits>>() {
        Some(deposits) => Arc::clone(&deposits),
        None => Arc::new(NoDeposits),
    }
}

/// A held deposit's amount was changed by its source (`adjust`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepositAdjusted {
    /// The deposit.
    pub deposit: DepositId,
    /// What it is held for.
    pub source: Subject,
    /// Whose deposit it is.
    pub party: PartyId,
    /// The version of the source that changed the amount.
    pub source_version: i32,
    /// The amount before, in fils.
    pub old_amount: i64,
    /// The amount now, in fils.
    pub new_amount: i64,
    /// The day the change takes effect.
    pub effective_on: NaiveDate,
    /// The top-up charge of an increase, when one was made.
    pub top_up_charge: Option<Subject>,
    /// The part refund of a decrease, when one was asked for.
    pub part_refund: Option<RefundId>,
}

impl Event for DepositAdjusted {}

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
    fn a_cancel_settlement_is_built_with_its_builder() {
        let source = Subject::new("property.lease", 9);
        let plain = CancelSettlement::new(source.clone());
        assert_eq!((plain.forfeit, plain.clause.as_deref()), (0, None));
        let kept = plain.forfeit(200_000, "Reservation form §4");
        assert_eq!(kept.forfeit, 200_000);
        assert_eq!(kept.clause.as_deref(), Some("Reservation form §4"));
        assert_eq!(kept.source, source);
    }

    #[renox::test]
    async fn without_payments_no_deposit_is_held_or_settled() {
        let app = renox::testing::TestApp::new(renox::App::new()).await;
        let source = Subject::new("property.lease", 9);
        let none = NoDeposits.held_for(app.db(), &source).await.unwrap();
        assert!(none.is_none());
        let settled = NoDeposits
            .settle_cancelled(app.state(), CancelSettlement::new(source), None)
            .await
            .unwrap();
        assert!(settled.is_none());
    }

    #[test]
    fn an_application_of_a_deposit_keeps_its_invoices_in_order() {
        let request = ApplyDeposit {
            source: Subject::new("property.lease", 9),
            approver: 4,
            rent: vec![
                ApplyToInvoice {
                    invoice: InvoiceId(11),
                    amount: 350_000,
                },
                ApplyToInvoice {
                    invoice: InvoiceId(12),
                    amount: 150_000,
                },
            ],
        };
        let json = renox::serde_json::to_value(&request).unwrap();
        assert_eq!(json["rent"][1]["amount"], 150_000);
        let back: ApplyDeposit = renox::serde_json::from_value(json).unwrap();
        assert_eq!(back, request);
        assert_eq!(APPLIED_TO_DEDUCTIONS, "applied to deductions");
    }

    #[test]
    fn a_kind_and_an_answer_have_names() {
        assert_eq!(DeductionKind::Forfeiture.as_str(), "forfeiture");
        assert_eq!(DeductionKind::default(), DeductionKind::Deductions);
        let json = renox::serde_json::to_value(DisputeAnswer::Upheld).unwrap();
        assert_eq!(json, "upheld");
    }
}
