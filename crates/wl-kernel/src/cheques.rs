//! The cheque contract (ARCHITECTURE §5, wl-payments §4 *Cheque*, §6): what
//! a vertical (01 Property) and the other modules may do with cheques
//! without depending on the `wl-payments` crate.
//!
//! `wl-payments` provides a [`Cheques`] with [`provide_cheques`] and emits
//! [`ChequeBounced`] and [`ChequeBounceResolved`]. A vertical registers the
//! post-dated cheques of a lease, one by one with [`RegisterCheque`] or all
//! at once with [`ChequeSchedule`], and reads them back per agreement.
//! [`cheques`] falls back to [`NoCheques`] when the `payments` module is
//! off:
//!
//! ```ignore
//! use wl_kernel::cheques::{self, ChequeSchedule, ScheduledCheque};
//!
//! let schedule = ChequeSchedule::new(lease, tenant, entity, "ENBD")
//!     .cheque(ScheduledCheque::new("1042", date, 2_000_000).link(rent_invoice));
//! let registered = cheques::cheques(&state).register_schedule(&state, schedule, None).await?;
//! ```
//!
//! A cheque has a direction: money in is a *received* cheque (a customer's),
//! money out an *issued* one (to a supplier); the kernel reuses
//! [`Direction`]. Money is `i64` in fils of the cheque's currency.
//!
//! The bounce fee is a charge of the client's charge catalogue
//! (wl-invoicing). `wl-invoicing` provides [`BounceFees`] when it can
//! schedule charges; without it [`bounce_fees`] gives [`NoBounceFees`], which
//! adds nothing, and `wl-payments` leaves the accountant a task to raise the
//! charge.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::invoicing::VatCategory;
use crate::parties::PartyId;
use crate::payments::Direction;
use crate::subject::Subject;

/// The subject type of a cheque, for [`Subject`] references.
pub const CHEQUE_SUBJECT_TYPE: &str = "payments.cheque";

/// The id of a cheque.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct ChequeId(pub i64);

impl ChequeId {
    /// The cheque as a [`Subject`]: `payments.cheque:42`.
    pub fn subject(self) -> Subject {
        Subject::new(CHEQUE_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for ChequeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for ChequeId {
    fn from(id: i64) -> Self {
        ChequeId(id)
    }
}

/// The lifecycle state of a cheque (wl-payments §4); the state of the
/// `wl-payments` Cheque machine.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ChequeStatus {
    /// A received cheque held, or an issued cheque handed to the payee.
    #[default]
    Received,
    /// A received cheque whose date is close: on the deposit list.
    DueForDeposit,
    /// Sent to the bank, or an issued cheque presented by the payee.
    Deposited,
    /// Funds moved; the payment is recorded (terminal).
    Cleared,
    /// Returned unpaid, in full or in part: the open amount is still owed.
    Bounced,
    /// The open amount is handed to a legal case.
    LegalCase,
    /// The open amount is settled by other cheques or payments (terminal).
    Replaced,
    /// The open amount is written off as bad debt (terminal).
    WrittenOff,
    /// Handed back undeposited, or an issued cheque cancelled (terminal).
    Returned,
}

impl ChequeStatus {
    /// Whether the cheque was returned and its open amount is still owed:
    /// `bounced` or `legal_case`.
    pub fn is_owed(self) -> bool {
        matches!(self, ChequeStatus::Bounced | ChequeStatus::LegalCase)
    }

    /// Whether the cheque has not cleared yet and is still expected to:
    /// `received`, `due_for_deposit` or `deposited` (the cash-flow forecast
    /// `FIN.CFL.01` counts these).
    pub fn is_pending(self) -> bool {
        matches!(
            self,
            ChequeStatus::Received | ChequeStatus::DueForDeposit | ChequeStatus::Deposited
        )
    }
}

/// A cheque another module asks to register: [`Cheques::register`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterCheque {
    /// Received (money in) or issued (money out).
    pub direction: Direction,
    /// The drawer of a received cheque, the payee of an issued one.
    pub party: PartyId,
    /// The legal entity whose books it belongs to.
    pub legal_entity_id: i64,
    /// The bank the cheque is drawn on.
    pub bank: String,
    /// The cheque number.
    pub number: String,
    /// The date on the cheque (post-dated cheques are in the future).
    pub date: NaiveDate,
    /// The amount in fils; above 0.
    pub amount: i64,
    /// The ISO currency code, e.g. `AED`.
    pub currency: String,
    /// The charges, invoices or supplier bills the cheque pays, oldest
    /// first; the honoured part is allocated to them in this order.
    pub linked: Vec<Subject>,
    /// The agreement it was received with (a lease), if any.
    pub agreement: Option<Subject>,
    /// The staff member who collected it.
    pub collected_by: Option<i64>,
    /// Whether it was collected in the field (a driver or sales rep): it can
    /// be deposited only after a cash handover confirms it (`FIN.ACL.20`).
    pub field_collected: bool,
    /// The bank account it is deposited in (received) or drawn on (issued),
    /// free text for now.
    pub bank_account: Option<String>,
    /// The approved owner drawing an issued cheque pays: the id of the
    /// drawing request in `wl-payments` (`FIN.ACL.21`). The cheque is then
    /// registered to the drawings category's owner party for the approved
    /// amount only, and its `clear` records a payment with purpose *owner
    /// drawings*.
    pub drawing: Option<i64>,
}

impl RegisterCheque {
    /// A received cheque of `amount` fils in AED from `party`.
    pub fn new(
        party: PartyId,
        legal_entity_id: i64,
        bank: &str,
        number: &str,
        date: NaiveDate,
        amount: i64,
    ) -> Self {
        RegisterCheque {
            direction: Direction::In,
            party,
            legal_entity_id,
            bank: bank.to_owned(),
            number: number.to_owned(),
            date,
            amount,
            currency: crate::payments::DEFAULT_CURRENCY.to_owned(),
            linked: Vec::new(),
            agreement: None,
            collected_by: None,
            field_collected: false,
            bank_account: None,
            drawing: None,
        }
    }

    /// Makes it the cheque of an approved owner drawing (an issued cheque).
    pub fn drawing(mut self, request: i64) -> Self {
        self.drawing = Some(request);
        self
    }

    /// Makes it an issued cheque (money out).
    pub fn issued(mut self) -> Self {
        self.direction = Direction::Out;
        self
    }

    /// Sets the currency.
    pub fn currency(mut self, currency: &str) -> Self {
        self.currency = currency.to_owned();
        self
    }

    /// Links a charge, invoice or supplier bill the cheque pays.
    pub fn link(mut self, document: Subject) -> Self {
        self.linked.push(document);
        self
    }

    /// Names the agreement it was received with.
    pub fn agreement(mut self, agreement: Subject) -> Self {
        self.agreement = Some(agreement);
        self
    }

    /// Names who collected it, and whether that was in the field.
    pub fn collected_by(mut self, user: i64, in_the_field: bool) -> Self {
        self.collected_by = Some(user);
        self.field_collected = in_the_field;
        self
    }

    /// Names the bank account.
    pub fn bank_account(mut self, account: &str) -> Self {
        self.bank_account = Some(account.to_owned());
        self
    }
}

/// One cheque of a [`ChequeSchedule`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledCheque {
    /// The cheque number.
    pub number: String,
    /// The date on the cheque.
    pub date: NaiveDate,
    /// The amount in fils; above 0.
    pub amount: i64,
    /// What it pays, oldest first.
    pub linked: Vec<Subject>,
}

impl ScheduledCheque {
    /// A cheque of `amount` fils dated `date`.
    pub fn new(number: &str, date: NaiveDate, amount: i64) -> Self {
        ScheduledCheque {
            number: number.to_owned(),
            date,
            amount,
            linked: Vec::new(),
        }
    }

    /// Links a charge or invoice it pays.
    pub fn link(mut self, document: Subject) -> Self {
        self.linked.push(document);
        self
    }
}

/// The post-dated cheques a party hands over with an agreement (a lease):
/// [`Cheques::register_schedule`] registers them all or none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChequeSchedule {
    /// The agreement they were received with, e.g. `property.lease:9`.
    pub agreement: Subject,
    /// The drawer.
    pub party: PartyId,
    /// The legal entity whose books they belong to.
    pub legal_entity_id: i64,
    /// The bank they are drawn on.
    pub bank: String,
    /// The ISO currency code.
    pub currency: String,
    /// The cheques, in date order.
    pub cheques: Vec<ScheduledCheque>,
}

impl ChequeSchedule {
    /// An empty schedule in AED for `agreement`.
    pub fn new(agreement: Subject, party: PartyId, legal_entity_id: i64, bank: &str) -> Self {
        ChequeSchedule {
            agreement,
            party,
            legal_entity_id,
            bank: bank.to_owned(),
            currency: crate::payments::DEFAULT_CURRENCY.to_owned(),
            cheques: Vec::new(),
        }
    }

    /// Adds a cheque.
    pub fn cheque(mut self, cheque: ScheduledCheque) -> Self {
        self.cheques.push(cheque);
        self
    }

    /// What the cheques add up to, in fils.
    pub fn total(&self) -> i64 {
        self.cheques.iter().map(|c| c.amount).sum()
    }
}

/// What other modules see of a cheque.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChequeSummary {
    /// The cheque.
    pub id: ChequeId,
    /// Received or issued.
    pub direction: Direction,
    /// The drawer or payee.
    pub party: PartyId,
    /// The legal entity whose books it belongs to.
    pub legal_entity_id: i64,
    /// The bank.
    pub bank: String,
    /// The cheque number.
    pub number: String,
    /// The date on the cheque.
    pub date: NaiveDate,
    /// The amount, in fils.
    pub amount: i64,
    /// What the bank paid of it when it bounced, in fils.
    pub honoured_amount: i64,
    /// What is still owed after a bounce, in fils.
    pub open_amount: i64,
    /// The ISO currency code.
    pub currency: String,
    /// How many times it was deposited.
    pub presentations: i32,
    /// The lifecycle state.
    pub status: ChequeStatus,
    /// The agreement it was received with, if any.
    pub agreement: Option<Subject>,
    /// What it pays, oldest first.
    pub linked: Vec<Subject>,
}

/// One document a cheque pays, with the part of the cheque applied to it:
/// [`OpenCheque::documents`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChequeDocument {
    /// The invoice, charge or supplier bill.
    pub document: Subject,
    /// The part of the cheque's open amount applied to it, in fils.
    pub amount: i64,
}

/// A cheque not cleared yet and what it pays: [`Cheques::not_cleared`]
/// (`FIN.CFL.01`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCheque {
    /// The cheque; its `open_amount` is what is still expected.
    pub cheque: ChequeSummary,
    /// What it pays, oldest first, with the open amount applied in that
    /// order: an invoice up to its balance, the first other document with
    /// what is left. A document that takes nothing is listed with 0.
    pub documents: Vec<ChequeDocument>,
}

/// Cheques handed back undeposited: [`Cheques::return_undeposited`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReturnCheques {
    /// The cheques to hand back.
    pub cheques: Vec<ChequeId>,
    /// Why (a cancelled lease, a handover to a new manager).
    pub reason: String,
}

impl ReturnCheques {
    /// Hands back `cheques` for `reason`.
    pub fn new(cheques: Vec<ChequeId>, reason: &str) -> Self {
        ReturnCheques {
            cheques,
            reason: reason.to_owned(),
        }
    }
}

/// What [`Cheques::return_undeposited`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReturnedCheques {
    /// The reference of the handover receipt, one for the whole call; the
    /// person collecting the cheques signs it.
    pub receipt: String,
    /// The cheques that were `received` or `due_for_deposit` and are now
    /// `returned`.
    pub returned: Vec<ChequeId>,
    /// The cheques left as they were, because they were deposited or had a
    /// final outcome already.
    pub skipped: Vec<ChequeId>,
}

/// The future a [`Cheques`] method returns.
pub type ChequeFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Registers and reads cheques for other modules. `wl-payments` provides it;
/// without that module, [`cheques`] gives [`NoCheques`].
pub trait Cheques: Send + Sync + 'static {
    /// Registers `cheque` (status `received`), by user `by` (`None` = the
    /// system). Refused (422) for an amount of 0 or less, a blank bank or
    /// number, an unknown party, an inactive legal entity or a currency that
    /// is not an ISO code. The calling module has authorized its user.
    fn register<'a>(
        &'a self,
        state: &'a AppState,
        cheque: RegisterCheque,
        by: Option<i64>,
    ) -> ChequeFuture<'a, ChequeSummary>;

    /// Registers every cheque of `schedule` in one transaction, or none
    /// (422 when one is refused), and returns them in the schedule's order.
    fn register_schedule<'a>(
        &'a self,
        state: &'a AppState,
        schedule: ChequeSchedule,
        by: Option<i64>,
    ) -> ChequeFuture<'a, Vec<ChequeSummary>>;

    /// The cheque with this id.
    fn find<'a>(&'a self, db: &'a Db, id: ChequeId) -> ChequeFuture<'a, Option<ChequeSummary>>;

    /// The cheques received with `agreement`, oldest date first.
    fn of_agreement<'a>(
        &'a self,
        db: &'a Db,
        agreement: &'a Subject,
    ) -> ChequeFuture<'a, Vec<ChequeSummary>>;

    /// What `party` still owes on its bounced received cheques (`bounced`
    /// and `legal_case`) in `currency`, in fils: the credit check of
    /// wl-sales reads it.
    fn open_bounced_amount<'a>(
        &'a self,
        db: &'a Db,
        party: PartyId,
        currency: &'a str,
    ) -> ChequeFuture<'a, i64>;

    /// The cheques of `legal_entity_id` in `direction` not cleared yet
    /// (`received`, `due_for_deposit` or `deposited`; a bounced cheque
    /// re-presented counts again once `deposited`), oldest cheque date
    /// first, each with its open amount and the documents it pays. Read
    /// only. The default finds none (`FIN.CFL.01`).
    fn not_cleared<'a>(
        &'a self,
        _state: &'a AppState,
        _legal_entity_id: i64,
        _direction: Direction,
    ) -> ChequeFuture<'a, Vec<OpenCheque>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// Hands back the cheques of `request` that are not deposited yet
    /// (`received` or `due_for_deposit`) against one handover receipt, as the
    /// system on behalf of the user `by`; the others are `skipped`, never
    /// touched. All or none. `None` when no cheque could be returned. The
    /// default refuses (422): payments is not enabled.
    fn return_undeposited<'a>(
        &'a self,
        _state: &'a AppState,
        _request: ReturnCheques,
        _by: Option<i64>,
    ) -> ChequeFuture<'a, Option<ReturnedCheques>> {
        Box::pin(async { Err(payments_off("the cheques cannot be returned")) })
    }

    /// Moves the link of cheque `cheque` from the document `from` to the
    /// document `to` (an amended lease: the cheque pays the new charge). The
    /// link keeps its place in the order the cheque's documents are paid in.
    /// Only a `received` or `due_for_deposit` cheque is relinked; refused
    /// (422) for another status, for a cheque not linked to `from`, or when
    /// it is linked to `to` already. The default refuses (422): payments is
    /// not enabled.
    fn relink<'a>(
        &'a self,
        _state: &'a AppState,
        cheque: ChequeId,
        _from: Subject,
        _to: Subject,
    ) -> ChequeFuture<'a, ()> {
        Box::pin(async move { Err(payments_off(&format!("cheque {cheque} cannot be relinked"))) })
    }

    /// Marks the cheques of `cheques` that are `received` or
    /// `due_for_deposit` as held as evidence (a lease ended by a forced
    /// termination while a legal case is open): they stay in the register,
    /// are never put on the deposit list and cannot be deposited. Returns
    /// the cheques that were marked; any other is left as it is. `note` says
    /// why. The default refuses (422): payments is not enabled.
    fn hold_as_evidence<'a>(
        &'a self,
        _state: &'a AppState,
        _cheques: Vec<ChequeId>,
        _note: String,
    ) -> ChequeFuture<'a, Vec<ChequeId>> {
        Box::pin(async { Err(payments_off("the cheques cannot be held as evidence")) })
    }

    /// The received cheques of `legal_entity_id` that bounced and are still
    /// owed (`bounced` or `legal_case`), oldest cheque date first, each with
    /// its open amount and the documents it pays: the forecast shows what
    /// they cover on a memo line, not as a receipt. Read only. The default
    /// finds none.
    fn bounced<'a>(
        &'a self,
        _state: &'a AppState,
        _legal_entity_id: i64,
    ) -> ChequeFuture<'a, Vec<OpenCheque>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// The 422 of a write while payments is off.
fn payments_off(what: &str) -> Error {
    Error::Status(
        StatusCode::UNPROCESSABLE_ENTITY,
        format!("{what}: payments is not enabled"),
    )
}

/// The cheques when the `payments` module is off: nothing is found and
/// nothing is owed, and a registration is refused.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoCheques;

impl Cheques for NoCheques {
    fn register<'a>(
        &'a self,
        _state: &'a AppState,
        _cheque: RegisterCheque,
        _by: Option<i64>,
    ) -> ChequeFuture<'a, ChequeSummary> {
        Box::pin(async { Err(payments_off("a cheque cannot be registered")) })
    }

    fn register_schedule<'a>(
        &'a self,
        _state: &'a AppState,
        _schedule: ChequeSchedule,
        _by: Option<i64>,
    ) -> ChequeFuture<'a, Vec<ChequeSummary>> {
        Box::pin(async { Err(payments_off("the cheques cannot be registered")) })
    }

    fn find<'a>(&'a self, _db: &'a Db, _id: ChequeId) -> ChequeFuture<'a, Option<ChequeSummary>> {
        Box::pin(async { Ok(None) })
    }

    fn of_agreement<'a>(
        &'a self,
        _db: &'a Db,
        _agreement: &'a Subject,
    ) -> ChequeFuture<'a, Vec<ChequeSummary>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn open_bounced_amount<'a>(
        &'a self,
        _db: &'a Db,
        _party: PartyId,
        _currency: &'a str,
    ) -> ChequeFuture<'a, i64> {
        Box::pin(async { Ok(0) })
    }
}

/// Makes `cheques` the app's [`Cheques`]; `wl-payments` calls it in
/// `register`.
pub fn provide_cheques(app: &mut Registry, cheques: impl Cheques) {
    let cheques: Arc<dyn Cheques> = Arc::new(cheques);
    app.provide(cheques);
}

/// The app's [`Cheques`], or [`NoCheques`] when none is provided.
pub fn cheques(state: &AppState) -> Arc<dyn Cheques> {
    match state.provided::<Arc<dyn Cheques>>() {
        Some(cheques) => Arc::clone(&cheques),
        None => Arc::new(NoCheques),
    }
}

/// The bounce fee a cheque `bounce` asks wl-invoicing to add:
/// [`BounceFees::add`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BounceFeeCharge {
    /// The cheque that bounced.
    pub cheque: ChequeId,
    /// The party charged: the drawer.
    pub party: PartyId,
    /// The legal entity issuing the charge.
    pub legal_entity_id: i64,
    /// The agreement the cheque is linked to: the charge is scheduled on it.
    /// `None`: a standalone charge for the party.
    pub agreement: Option<Subject>,
    /// The charge type of the client's charge catalogue.
    pub charge_type: String,
    /// The amount before VAT, in fils.
    pub amount: i64,
    /// The ISO currency code.
    pub currency: String,
    /// The VAT treatment (standard by default, `⚠ open: accountant A-05`).
    pub vat_category: VatCategory,
    /// The day the charge is due: the bounce date (Asia/Dubai).
    pub due: NaiveDate,
}

/// Adds the bounce fee charge of a bounced cheque. The module that keeps the
/// charge schedules provides it (01 Property for the cheques of its leases,
/// until wl-invoicing keeps charge schedules); without one [`bounce_fees`]
/// gives [`NoBounceFees`].
pub trait BounceFees: Send + Sync + 'static {
    /// Schedules `charge` and returns it as a [`Subject`] (e.g.
    /// `invoicing.charge:12`), or `None` when no charge was made (the
    /// default: invoicing cannot schedule charges), so the caller leaves the
    /// accountant a task to raise it.
    fn add<'a>(
        &'a self,
        _state: &'a AppState,
        _charge: BounceFeeCharge,
    ) -> ChequeFuture<'a, Option<Subject>> {
        Box::pin(async { Ok(None) })
    }
}

/// The bounce fees when nothing can schedule a charge: none is made.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoBounceFees;

impl BounceFees for NoBounceFees {}

/// Makes `fees` the app's [`BounceFees`]; the module that keeps the charge
/// schedules calls it in `register`.
pub fn provide_bounce_fees(app: &mut Registry, fees: impl BounceFees) {
    let fees: Arc<dyn BounceFees> = Arc::new(fees);
    app.provide(fees);
}

/// The app's [`BounceFees`], or [`NoBounceFees`] when none is provided.
pub fn bounce_fees(state: &AppState) -> Arc<dyn BounceFees> {
    match state.provided::<Arc<dyn BounceFees>>() {
        Some(fees) => Arc::clone(&fees),
        None => Arc::new(NoBounceFees),
    }
}

/// The subject type of a scheduled charge, for [`Subject`] references (the
/// bounce fee charge, a rent charge of a lease).
pub const CHARGE_SUBJECT_TYPE: &str = "invoicing.charge";

/// Invoices a scheduled charge that is not invoiced yet, for a cheque
/// `write_off` (wl-payments §4 *Cheque*: "a linked charge not yet invoiced is
/// invoiced first"): [`ChargeInvoicer::invoice`]. The module that owns the
/// charges provides it; without one, [`charge_invoicer`] gives
/// [`NoChargeInvoicer`], which invoices nothing.
pub trait ChargeInvoicer: Send + Sync + 'static {
    /// Whether `link` (a document a cheque is linked to) is a charge this
    /// provider owns. The default knows the charges of wl-invoicing
    /// ([`CHARGE_SUBJECT_TYPE`]); a vertical that keeps its own charges (01
    /// Property's lease charges) names their subject type.
    fn owns(&self, link: &Subject) -> bool {
        link.subject_type == CHARGE_SUBJECT_TYPE
    }

    /// Invoices the charge `charge` now and returns the issued invoice, or
    /// the invoice it already has. `None` when it cannot be invoiced (the
    /// default), so the caller refuses the write-off and asks for the charge
    /// to be invoiced first.
    fn invoice<'a>(
        &'a self,
        _state: &'a AppState,
        _charge: &'a Subject,
    ) -> ChequeFuture<'a, Option<crate::invoicing::InvoiceId>> {
        Box::pin(async { Ok(None) })
    }
}

/// The charge invoicer when nothing can invoice a charge.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoChargeInvoicer;

impl ChargeInvoicer for NoChargeInvoicer {}

/// Makes `invoicer` the app's [`ChargeInvoicer`].
pub fn provide_charge_invoicer(app: &mut Registry, invoicer: impl ChargeInvoicer) {
    let invoicer: Arc<dyn ChargeInvoicer> = Arc::new(invoicer);
    app.provide(invoicer);
}

/// The app's [`ChargeInvoicer`], or [`NoChargeInvoicer`] when none is
/// provided.
pub fn charge_invoicer(state: &AppState) -> Arc<dyn ChargeInvoicer> {
    match state.provided::<Arc<dyn ChargeInvoicer>>() {
        Some(invoicer) => Arc::clone(&invoicer),
        None => Arc::new(NoChargeInvoicer),
    }
}

/// A received cheque bounced (`bounce`): wl-sales puts the customer account
/// (and its credit group) on credit hold (`SAL.CRD.01`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChequeBounced {
    /// The cheque.
    pub cheque: ChequeId,
    /// The drawer.
    pub party: PartyId,
    /// The drawer's credit group. `wl-payments` does not know credit
    /// groups, so it is `None` and wl-sales resolves the group from the
    /// party.
    pub credit_group: Option<Subject>,
    /// The cheque amount, in fils.
    pub amount: i64,
    /// What the bank paid of it, in fils.
    pub honoured_amount: i64,
    /// What is still owed, in fils.
    pub open_amount: i64,
    /// The ISO currency code.
    pub currency: String,
    /// The bank's return reason.
    pub reason: String,
    /// Which presentation bounced: 1 for the first deposit.
    pub presentation: i32,
    /// What the cheque pays.
    pub linked: Vec<Subject>,
}

impl Event for ChequeBounced {}

/// A bounced received cheque was referred to a legal case (`refer_to_legal`):
/// a vertical whose agreement it was received with sets its legal case flag
/// (01 Property: a flag of type arrears on the lease, with its ticket).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChequeReferredToLegal {
    /// The cheque.
    pub cheque: ChequeId,
    /// The drawer.
    pub party: PartyId,
    /// The agreement the cheque was received with, if any.
    pub agreement: Option<Subject>,
    /// What is still owed on it, in fils.
    pub open_amount: i64,
    /// The ISO currency code.
    pub currency: String,
    /// The user who owns the case.
    pub case_owner: i64,
}

impl Event for ChequeReferredToLegal {}

/// How a bounced cheque was resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChequeOutcome {
    /// Re-presented and paid.
    Cleared,
    /// Replaced by other cheques or payments.
    Replaced,
    /// Written off on the linked invoices.
    WrittenOff,
}

/// A bounced cheque was resolved (`clear` after a re-presentation,
/// `replace`, `write_off`): wl-sales lifts the credit hold for `cleared` or
/// `replaced` when `others_open` is empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChequeBounceResolved {
    /// The cheque.
    pub cheque: ChequeId,
    /// The drawer.
    pub party: PartyId,
    /// How it was resolved.
    pub outcome: ChequeOutcome,
    /// The other bounced cheques of the party still open.
    pub others_open: Vec<ChequeId>,
}

impl Event for ChequeBounceResolved {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cheque_id_is_a_subject() {
        assert_eq!(ChequeId(42).subject().to_string(), "payments.cheque:42");
        assert_eq!(ChequeId::from(3), ChequeId(3));
        assert_eq!(ChequeId(5).to_string(), "5");
    }

    #[test]
    fn statuses_are_stored_in_snake_case() {
        assert_eq!(ChequeStatus::DueForDeposit.as_str(), "due_for_deposit");
        assert_eq!(ChequeStatus::LegalCase.as_str(), "legal_case");
        assert_eq!(ChequeStatus::WrittenOff.as_str(), "written_off");
        assert!(ChequeStatus::Bounced.is_owed() && ChequeStatus::LegalCase.is_owed());
        assert!(!ChequeStatus::Replaced.is_owed());
        assert!(ChequeStatus::Deposited.is_pending());
        assert!(!ChequeStatus::Cleared.is_pending());
    }

    #[test]
    fn a_registration_defaults_to_a_received_aed_cheque() {
        let date = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
        let cheque = RegisterCheque::new(PartyId(7), 2, "ENBD", "1042", date, 2_000_000);
        assert_eq!(cheque.direction, Direction::In);
        assert_eq!(cheque.currency, "AED");
        assert!(cheque.linked.is_empty() && !cheque.field_collected);
        let issued = cheque
            .clone()
            .issued()
            .link(Subject::new("invoicing.supplier_bill", 4))
            .collected_by(9, true);
        assert_eq!(issued.direction, Direction::Out);
        assert_eq!(issued.linked.len(), 1);
        assert_eq!(issued.collected_by, Some(9));
        assert!(issued.field_collected);
    }

    #[renox::test]
    async fn without_payments_cheques_cannot_be_returned() {
        let app = renox::testing::TestApp::new(renox::App::new()).await;
        let request = ReturnCheques::new(vec![ChequeId(1), ChequeId(2)], "lease cancelled");
        assert_eq!(request.cheques.len(), 2);
        let refused = NoCheques
            .return_undeposited(app.state(), request, None)
            .await;
        assert!(refused.is_err());
    }

    #[test]
    fn a_schedule_adds_up_its_cheques() {
        let date = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
        let schedule =
            ChequeSchedule::new(Subject::new("property.lease", 9), PartyId(7), 2, "ENBD")
                .cheque(ScheduledCheque::new("1", date, 2_000_000))
                .cheque(
                    ScheduledCheque::new("2", date, 2_000_000)
                        .link(Subject::new("invoicing.charge", 1)),
                );
        assert_eq!(schedule.total(), 4_000_000);
        assert_eq!(schedule.cheques[1].linked.len(), 1);
    }

    #[test]
    fn the_events_serialize_with_plain_ids() {
        let bounced = ChequeBounced {
            cheque: ChequeId(3),
            party: PartyId(7),
            credit_group: None,
            amount: 2_000_000,
            honoured_amount: 600_000,
            open_amount: 1_400_000,
            currency: "AED".into(),
            reason: "insufficient funds".into(),
            presentation: 1,
            linked: vec![Subject::new("invoicing.invoice", 5)],
        };
        let json = renox::serde_json::to_value(&bounced).unwrap();
        assert_eq!(json["cheque"], 3);
        assert_eq!(json["party"], 7);
        assert_eq!(json["open_amount"], 1_400_000);
        let resolved = ChequeBounceResolved {
            cheque: ChequeId(3),
            party: PartyId(7),
            outcome: ChequeOutcome::Replaced,
            others_open: vec![ChequeId(4)],
        };
        let json = renox::serde_json::to_value(&resolved).unwrap();
        assert_eq!(json["outcome"], "replaced");
        assert_eq!(json["others_open"][0], 4);
    }
}
