//! The stand-ins of the money side of a visit: the invoice book, the payer
//! bills, the prepaid packages and the recorder that holds the deposits.

use std::sync::{Arc, Mutex};

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::invoicing::{
    InvoiceDraft, InvoiceFuture, InvoiceId, InvoiceStatus, InvoiceSummary, Invoicer,
};
use wl_kernel::payer_bills::{
    NewPayerBill, PayerAnswer, PayerBillFuture, PayerBillId, PayerBillLineSummary, PayerBillStatus,
    PayerBillSummary, PayerBills, Recoupment, RejectionDeadline, Remittance, SubmissionError,
};
use wl_kernel::payments::{
    AllocationTarget, Direction, PaymentFuture, PaymentId, PaymentMethod, PaymentRecorder,
    PaymentStatus, PaymentSummary, RecordPayment, RefundId, RefundRequest, ReversalReason,
};
use wl_kernel::sales::{ConsumePackage, PackageConsumption, PackageId, PackageLedger, SalesFuture};
use wl_kernel::{PartyId, Subject};

use super::scheduling::Refunds;

/// One invoice the clinic drafted.
#[derive(Debug, Clone)]
pub struct Invoiced {
    /// The id it answered with.
    pub id: i64,
    /// What it was asked for.
    pub draft: InvoiceDraft,
    /// Whether the system issued it.
    pub issued: bool,
}

impl Invoiced {
    /// The total with VAT, in fils.
    pub fn gross(&self) -> i64 {
        self.draft.lines.iter().map(|l| l.gross()).sum()
    }
}

/// The invoices drafted and issued so far.
#[derive(Clone, Default)]
pub struct Books(Arc<Mutex<Vec<Invoiced>>>);

impl Books {
    /// Every invoice, oldest first.
    pub fn all(&self) -> Vec<Invoiced> {
        self.0.lock().unwrap().clone()
    }
}

impl Invoicer for Books {
    fn draft<'a>(
        &'a self,
        _db: &'a Db,
        draft: InvoiceDraft,
    ) -> InvoiceFuture<'a, Option<InvoiceId>> {
        let mut all = self.0.lock().unwrap();
        let id = 100 + i64::try_from(all.len()).unwrap();
        all.push(Invoiced {
            id,
            draft,
            issued: false,
        });
        Box::pin(async move { Ok(Some(InvoiceId(id))) })
    }

    fn find<'a>(&'a self, _db: &'a Db, id: InvoiceId) -> InvoiceFuture<'a, Option<InvoiceSummary>> {
        let found = self
            .0
            .lock()
            .unwrap()
            .iter()
            .find(|i| i.id == id.0)
            .map(|i| InvoiceSummary {
                id,
                number: i.issued.then(|| format!("INV-{}", i.id)),
                status: if i.issued {
                    InvoiceStatus::Issued
                } else {
                    InvoiceStatus::Draft
                },
                party: i.draft.party,
                legal_entity_id: None,
                currency: i.draft.currency.clone(),
                gross_total: i.gross(),
                balance: i.gross(),
                due_date: None,
                disputed: false,
                chasing_paused_until: None,
            });
        Box::pin(async move { Ok(found) })
    }

    fn allocate<'a>(
        &'a self,
        _state: &'a AppState,
        _id: InvoiceId,
        _source: Subject,
        _amount: i64,
    ) -> InvoiceFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn issue<'a>(
        &'a self,
        _state: &'a AppState,
        id: InvoiceId,
        _by: Option<&'a AuthUser>,
    ) -> InvoiceFuture<'a, ()> {
        for invoice in self.0.lock().unwrap().iter_mut() {
            if invoice.id == id.0 {
                invoice.issued = true;
            }
        }
        Box::pin(async { Ok(()) })
    }
}

/// One payer bill the stand-in keeps.
#[derive(Debug, Clone)]
pub struct Bill {
    /// The id it answered with.
    pub id: i64,
    /// What it was asked for.
    pub request: NewPayerBill,
    /// Where it stands.
    pub status: PayerBillStatus,
    /// Its number, once issued.
    pub number: Option<String>,
    /// How often it was sent again after a remittance.
    pub resubmissions: i64,
    /// Whether a remittance was recorded.
    pub has_remittance: bool,
    /// The day of the last remittance.
    pub last_remittance_on: Option<NaiveDate>,
    /// Its lines.
    pub lines: Vec<PayerBillLineSummary>,
    /// The errors of each technical rejection.
    pub rejections: Vec<Vec<SubmissionError>>,
    /// How many times it was issued.
    pub issued: i64,
    /// The corrections of the last resubmission.
    pub corrections: Option<String>,
}

impl Bill {
    fn summary(&self) -> PayerBillSummary {
        let claimed: i64 = self.lines.iter().map(|l| l.claimed).sum();
        let paid: i64 = self.lines.iter().map(|l| l.paid).sum();
        let denied: i64 = self.lines.iter().map(|l| l.denied).sum();
        PayerBillSummary {
            id: PayerBillId(self.id),
            number: self.number.clone(),
            status: self.status,
            payer: PartyId(900),
            customer: self.request.customer,
            legal_entity_id: 1,
            source: self.request.source.clone(),
            currency: self.request.currency.clone(),
            claimed,
            paid,
            denied,
            open: claimed - paid,
            resubmission_count: self.resubmissions,
            has_remittance: self.has_remittance,
            not_submitted_in_time: false,
        }
    }
}

/// The payer bills the clinic made, with the lifecycle of the real ones
/// (issue, technical rejection, resubmission, a simple remittance and
/// recoupment), kept in memory. An advice that pays more than a line has
/// open is answered as waiting for approval (request 77) and records
/// nothing; the approval itself is the real wl-invoicing's, tested in
/// `claims_real.rs`.
#[derive(Clone, Default)]
pub struct Claims(Arc<Mutex<Vec<Bill>>>);

impl Claims {
    /// Every payer bill asked for, oldest first.
    pub fn all(&self) -> Vec<NewPayerBill> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|b| b.request.clone())
            .collect()
    }

    /// What the payer is claimed for, over all bills, in fils.
    pub fn claimed(&self) -> i64 {
        self.all()
            .iter()
            .flat_map(|bill| bill.lines.iter())
            .map(|line| line.claimed)
            .sum()
    }

    /// Payer bill `id` as the stand-in keeps it.
    pub fn bill(&self, id: i64) -> Bill {
        self.0
            .lock()
            .unwrap()
            .iter()
            .find(|b| b.id == id)
            .cloned()
            .expect("the payer bill exists")
    }

    /// Changes payer bill `id` as a test needs it (a state the stand-in's own
    /// lifecycle reaches only through steps the test does not take).
    pub fn force(&self, id: i64, change: impl FnOnce(&mut Bill)) {
        let mut bills = self.0.lock().unwrap();
        let bill = bills
            .iter_mut()
            .find(|b| b.id == id)
            .expect("the payer bill exists");
        change(bill);
    }

    fn with<T>(&self, id: PayerBillId, f: impl FnOnce(&mut Bill) -> Result<T>) -> Result<T> {
        let mut bills = self.0.lock().unwrap();
        match bills.iter_mut().find(|b| b.id == id.0) {
            Some(bill) => f(bill),
            None => Err(Error::NotFound),
        }
    }
}

fn refused(message: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.to_owned())
}

impl PayerBills for Claims {
    fn find<'a>(
        &'a self,
        _db: &'a Db,
        id: PayerBillId,
    ) -> PayerBillFuture<'a, Option<PayerBillSummary>> {
        let found = self
            .0
            .lock()
            .unwrap()
            .iter()
            .find(|b| b.id == id.0)
            .map(Bill::summary);
        Box::pin(async move { Ok(found) })
    }

    fn lines<'a>(
        &'a self,
        _db: &'a Db,
        id: PayerBillId,
    ) -> PayerBillFuture<'a, Vec<PayerBillLineSummary>> {
        let lines = self
            .0
            .lock()
            .unwrap()
            .iter()
            .find(|b| b.id == id.0)
            .map(|b| b.lines.clone())
            .unwrap_or_default();
        Box::pin(async move { Ok(lines) })
    }

    fn create<'a>(
        &'a self,
        _state: &'a AppState,
        request: NewPayerBill,
        _by: Option<&'a AuthUser>,
    ) -> PayerBillFuture<'a, PayerBillId> {
        let mut all = self.0.lock().unwrap();
        let id = 500 + i64::try_from(all.len() + 1).unwrap();
        let lines = request
            .lines
            .iter()
            .zip(1_i64..)
            .map(|(line, position)| PayerBillLineSummary {
                id: id * 10 + position,
                position,
                description: line.description.clone(),
                claimed: line.claimed,
                paid: 0,
                denied: 0,
                written_off: 0,
                transferred: 0,
                denial_code: None,
                denial_reason: None,
                recoupment_code: None,
            })
            .collect();
        all.push(Bill {
            id,
            request,
            status: PayerBillStatus::Draft,
            number: None,
            resubmissions: 0,
            has_remittance: false,
            last_remittance_on: None,
            lines,
            rejections: Vec::new(),
            issued: 0,
            corrections: None,
        });
        Box::pin(async move { Ok(PayerBillId(id)) })
    }

    fn issue<'a>(
        &'a self,
        _state: &'a AppState,
        id: PayerBillId,
        _by: Option<&'a AuthUser>,
    ) -> PayerBillFuture<'a, ()> {
        let done = self.with(id, |bill| {
            if bill.status != PayerBillStatus::Draft {
                return Err(refused("the payer bill is not a draft"));
            }
            bill.status = PayerBillStatus::Issued;
            bill.issued += 1;
            if bill.number.is_none() {
                bill.number = Some(format!("PB-2026-{:04}", bill.id));
            }
            Ok(())
        });
        Box::pin(async move { done })
    }

    fn submission_rejected<'a>(
        &'a self,
        _state: &'a AppState,
        id: PayerBillId,
        errors: Vec<SubmissionError>,
        _by: Option<&'a AuthUser>,
    ) -> PayerBillFuture<'a, RejectionDeadline> {
        let done = self.with(id, |bill| {
            if bill.status != PayerBillStatus::Issued {
                return Err(refused("the payer bill is not issued"));
            }
            bill.status = PayerBillStatus::Draft;
            bill.rejections.push(errors);
            Ok(RejectionDeadline { deadline: None })
        });
        Box::pin(async move { done })
    }

    fn resubmit<'a>(
        &'a self,
        _state: &'a AppState,
        id: PayerBillId,
        corrections: &'a str,
        _by: Option<&'a AuthUser>,
    ) -> PayerBillFuture<'a, ()> {
        let done = self.with(id, |bill| {
            if !matches!(
                bill.status,
                PayerBillStatus::PartiallyPaid | PayerBillStatus::Denied
            ) {
                return Err(refused("the payer bill cannot be resubmitted"));
            }
            bill.status = PayerBillStatus::Issued;
            bill.resubmissions += 1;
            bill.corrections = Some(corrections.to_owned());
            for line in &mut bill.lines {
                line.denied = 0;
            }
            Ok(())
        });
        Box::pin(async move { done })
    }

    fn record_remittance<'a>(
        &'a self,
        _state: &'a AppState,
        id: PayerBillId,
        remittance: Remittance,
        _by: &'a AuthUser,
    ) -> PayerBillFuture<'a, PayerAnswer> {
        let done = self.with(id, |bill| {
            if bill.status != PayerBillStatus::Issued {
                return Err(refused("the payer bill is not issued"));
            }
            for entry in &remittance.lines {
                let Some(line) = bill.lines.iter().find(|l| l.id == entry.line) else {
                    return Err(refused("a line is not on the payer bill"));
                };
                if entry.paid > line.claimed - line.paid - entry.denied {
                    return Ok(PayerAnswer::Pending { request: 77 });
                }
            }
            for entry in &remittance.lines {
                let line = bill
                    .lines
                    .iter_mut()
                    .find(|l| l.id == entry.line)
                    .expect("checked above");
                line.paid += entry.paid;
                line.denied = entry.denied;
                if entry.denied > 0 {
                    line.denial_code = entry.code.clone();
                    line.denial_reason = entry.reason.clone();
                }
            }
            bill.has_remittance = true;
            bill.last_remittance_on = Some(remittance.date);
            bill.status = if bill.lines.iter().all(|l| l.paid == l.claimed) {
                PayerBillStatus::Paid
            } else if bill.lines.iter().all(|l| l.paid == 0) {
                PayerBillStatus::Denied
            } else {
                PayerBillStatus::PartiallyPaid
            };
            Ok(PayerAnswer::Done)
        });
        Box::pin(async move { done })
    }

    fn record_recoupment<'a>(
        &'a self,
        _state: &'a AppState,
        id: PayerBillId,
        recoupment: Recoupment,
        _by: &'a AuthUser,
    ) -> PayerBillFuture<'a, PayerAnswer> {
        let done = self.with(id, |bill| {
            for entry in &recoupment.lines {
                let Some(line) = bill.lines.iter().find(|l| l.id == entry.line) else {
                    return Err(refused("a line is not on the payer bill"));
                };
                if entry.amount <= 0 || entry.amount > line.paid {
                    return Err(refused("more than the line was paid"));
                }
            }
            for entry in &recoupment.lines {
                let line = bill
                    .lines
                    .iter_mut()
                    .find(|l| l.id == entry.line)
                    .expect("checked above");
                line.paid -= entry.amount;
                line.denied += entry.amount;
                line.recoupment_code = Some(recoupment.code.clone());
            }
            bill.status = if bill.lines.iter().all(|l| l.paid == 0) {
                PayerBillStatus::Denied
            } else {
                PayerBillStatus::PartiallyPaid
            };
            Ok(PayerAnswer::Done)
        });
        Box::pin(async move { done })
    }
}

/// The prepaid packages of the patients, by item, and the sessions used.
#[derive(Clone, Default)]
pub struct Packages {
    credits: Arc<Mutex<Vec<(i64, String, i64)>>>,
    used: Arc<Mutex<Vec<(String, i64, Subject)>>>,
}

impl Packages {
    /// `party` holds a package with `credits` of `item`.
    pub fn give(&self, party: PartyId, item: &str, credits: i64) {
        self.credits
            .lock()
            .unwrap()
            .push((party.0, item.to_owned(), credits));
    }

    /// The sessions consumed, with what consumed them.
    pub fn used(&self) -> Vec<(String, i64, Subject)> {
        self.used.lock().unwrap().clone()
    }
}

impl PackageLedger for Packages {
    fn consume<'a>(
        &'a self,
        _state: &'a AppState,
        request: ConsumePackage,
    ) -> SalesFuture<'a, PackageConsumption> {
        let mut credits = self.credits.lock().unwrap();
        let answer = match credits.iter_mut().find(|(party, item, left)| {
            *party == request.party.0 && *item == request.item && *left >= request.quantity
        }) {
            Some(found) => {
                found.2 -= request.quantity;
                self.used.lock().unwrap().push((
                    request.item.clone(),
                    request.quantity,
                    request.source.clone(),
                ));
                PackageConsumption::Consumed {
                    package: PackageId(9),
                    remaining: found.2,
                }
            }
            None => PackageConsumption::NoCredit,
        };
        Box::pin(async move { Ok(answer) })
    }
}

/// An allocation of a payment the clinic asked for.
pub type Allocated = (PaymentId, AllocationTarget, i64);

/// The payment recorder with deposits held for bookings: it answers
/// `payments_for` with them, records the allocations and delegates the
/// refunds to wl-scheduling's own stand-in.
#[derive(Clone, Default)]
pub struct Deposits {
    refunds: Refunds,
    held: Arc<Mutex<Vec<(Subject, PaymentSummary)>>>,
    allocated: Arc<Mutex<Vec<Allocated>>>,
}

impl Deposits {
    /// Over the refunds stand-in.
    pub fn new(refunds: Refunds) -> Self {
        Deposits {
            refunds,
            ..Deposits::default()
        }
    }

    /// `party` paid `amount` fils as the deposit of `booking`.
    pub fn paid(&self, booking: i64, party: PartyId, amount: i64) {
        let mut held = self.held.lock().unwrap();
        let id = 700 + i64::try_from(held.len()).unwrap();
        held.push((
            wl_kernel::scheduling::BookingId(booking).subject(),
            PaymentSummary {
                id: PaymentId(id),
                number: Some(format!("RCT-{id}")),
                party,
                legal_entity_id: 1,
                direction: Direction::In,
                method: PaymentMethod::Card,
                amount,
                allocated: 0,
                unallocated: amount,
                currency: "AED".to_owned(),
                status: PaymentStatus::Received,
                date: NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
            },
        ));
    }

    /// The allocations asked so far.
    pub fn allocated(&self) -> Vec<Allocated> {
        self.allocated.lock().unwrap().clone()
    }

    /// The amount allocated to invoices so far, in fils.
    pub fn allocated_total(&self) -> i64 {
        self.allocated().iter().map(|(_, _, amount)| amount).sum()
    }
}

impl PaymentRecorder for Deposits {
    fn record<'a>(
        &'a self,
        state: &'a AppState,
        payment: RecordPayment,
        by: Option<i64>,
    ) -> PaymentFuture<'a, PaymentSummary> {
        self.refunds.record(state, payment, by)
    }

    fn allocate<'a>(
        &'a self,
        _state: &'a AppState,
        payment: PaymentId,
        target: AllocationTarget,
        amount: i64,
    ) -> PaymentFuture<'a, ()> {
        for (_, summary) in self.held.lock().unwrap().iter_mut() {
            if summary.id == payment {
                summary.unallocated -= amount;
                summary.allocated += amount;
            }
        }
        self.allocated
            .lock()
            .unwrap()
            .push((payment, target, amount));
        Box::pin(async { Ok(()) })
    }

    fn reverse<'a>(
        &'a self,
        state: &'a AppState,
        payment: PaymentId,
        reason: ReversalReason,
        note: Option<String>,
        by: Option<i64>,
    ) -> PaymentFuture<'a, ()> {
        self.refunds.reverse(state, payment, reason, note, by)
    }

    fn find<'a>(&'a self, db: &'a Db, id: PaymentId) -> PaymentFuture<'a, Option<PaymentSummary>> {
        self.refunds.find(db, id)
    }

    fn credit_balance<'a>(
        &'a self,
        db: &'a Db,
        party: PartyId,
        currency: &'a str,
    ) -> PaymentFuture<'a, i64> {
        self.refunds.credit_balance(db, party, currency)
    }

    fn payments_for<'a>(
        &'a self,
        _db: &'a Db,
        prepaid: &'a Subject,
    ) -> PaymentFuture<'a, Vec<PaymentSummary>> {
        let found: Vec<PaymentSummary> = self
            .held
            .lock()
            .unwrap()
            .iter()
            .filter(|(subject, _)| subject == prepaid)
            .map(|(_, summary)| summary.clone())
            .collect();
        Box::pin(async move { Ok(found) })
    }

    fn request_refund<'a>(
        &'a self,
        state: &'a AppState,
        request: RefundRequest,
        by: Option<i64>,
    ) -> PaymentFuture<'a, RefundId> {
        self.refunds.request_refund(state, request, by)
    }
}
