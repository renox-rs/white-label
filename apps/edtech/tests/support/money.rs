//! The stand-ins the enrolments talk to through the kernel's contracts: the
//! party directory (students, guardians, payers), the invoice book, the
//! subscriptions and the prepaid packages. They keep what they were asked in
//! memory so a test can read it back.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::invoicing::{
    CreditLine, CreditNoteId, CreditNoteStatus, CreditNoteSummary, InvoiceDraft, InvoiceFuture,
    InvoiceId, InvoiceStatus, InvoiceSummary, Invoicer,
};
use wl_kernel::notify::{
    MessageId, NotificationTypeDef, Notifier, NotifyChannel, NotifyFuture, NotifyRequest,
};
use wl_kernel::payments::{
    AllocationTarget, PaymentFuture, PaymentId, PaymentLinkRef, PaymentLinkRequest, PaymentLinks,
    PaymentRecorder, PaymentStatus, PaymentSummary, RecordPayment, RefundId, RefundRequest,
    ReversalReason,
};
use wl_kernel::parties::{
    PartyDirectory, PartyFuture, PartyKind, PartyStatus, PartySummary,
};
use wl_kernel::sales::{
    PackageConsumption, PackageId, PackageLedger, PackageSales, SalesFuture, SellPackage,
};
use wl_kernel::subscriptions::{
    CancelSubscription, PauseCause, PauseSubscription, StartSubscription, SubscriptionFuture,
    SubscriptionId, SubscriptionPaused, SubscriptionResumed, SubscriptionStatus,
    SubscriptionSummary, Subscriptions,
};
use wl_kernel::{PartyId, Subject};

/// The parties the centre knows, with their guardians and the ones blocked.
#[derive(Clone, Default)]
pub struct People {
    names: Arc<Mutex<BTreeMap<i64, String>>>,
    guardians: Arc<Mutex<BTreeMap<i64, Vec<PartyId>>>>,
    blocked: Arc<Mutex<Vec<i64>>>,
}

impl People {
    /// Party `id` is called `name`.
    pub fn add(&self, id: i64, name: &str) {
        self.names.lock().unwrap().insert(id, name.to_owned());
    }

    /// `guardian` is a guardian of `child`.
    pub fn guardian(&self, child: i64, guardian: i64) {
        self.guardians
            .lock()
            .unwrap()
            .entry(child)
            .or_default()
            .push(PartyId(guardian));
    }

    /// Party `id` is blocked (`true`) or active again (`false`). The test then
    /// emits `PartyBlocked` or `PartyUnblocked`, as wl-parties does.
    pub fn set_blocked(&self, id: i64, blocked: bool) {
        let mut all = self.blocked.lock().unwrap();
        all.retain(|party| *party != id);
        if blocked {
            all.push(id);
        }
    }

    fn status_of(&self, id: i64) -> PartyStatus {
        if self.blocked.lock().unwrap().contains(&id) {
            PartyStatus::Blocked
        } else {
            PartyStatus::Active
        }
    }

    fn summary(&self, id: i64) -> Option<PartySummary> {
        let status = self.status_of(id);
        self.names.lock().unwrap().get(&id).map(|name| PartySummary {
            id: PartyId(id),
            code: format!("P-{id:05}"),
            kind: PartyKind::Person,
            display_name: name.clone(),
            status,
            display_block_reason: (status == PartyStatus::Blocked).then(|| "on_hold".to_owned()),
        })
    }
}

impl PartyDirectory for People {
    fn find<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartySummary>> {
        let found = self.summary(party.0);
        Box::pin(async move { Ok(found) })
    }

    fn find_by_code<'a>(
        &'a self,
        _db: &'a Db,
        code: &'a str,
    ) -> PartyFuture<'a, Option<PartySummary>> {
        let id = code
            .strip_prefix("P-")
            .and_then(|digits| digits.parse::<i64>().ok());
        let found = id.and_then(|id| self.summary(id));
        Box::pin(async move { Ok(found) })
    }

    fn search<'a>(
        &'a self,
        _db: &'a Db,
        text: &'a str,
        limit: u64,
    ) -> PartyFuture<'a, Vec<PartySummary>> {
        let needle = text.to_lowercase();
        let ids: Vec<i64> = self.names.lock().unwrap().keys().copied().collect();
        let found: Vec<PartySummary> = ids
            .into_iter()
            .filter_map(|id| self.summary(id))
            .filter(|p| {
                p.display_name.to_lowercase().contains(&needle)
                    || p.code.to_lowercase().contains(&needle)
            })
            .take(usize::try_from(limit).unwrap_or(usize::MAX))
            .collect();
        Box::pin(async move { Ok(found) })
    }

    fn display_name<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
        _lang: &'a str,
    ) -> PartyFuture<'a, Option<String>> {
        let name = self.names.lock().unwrap().get(&party.0).cloned();
        Box::pin(async move { Ok(name) })
    }

    fn status<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartyStatus>> {
        let known = self.names.lock().unwrap().contains_key(&party.0);
        let status = self.status_of(party.0);
        Box::pin(async move { Ok(known.then_some(status)) })
    }

    fn guardians<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Vec<PartyId>> {
        let found = self
            .guardians
            .lock()
            .unwrap()
            .get(&party.0)
            .cloned()
            .unwrap_or_default();
        Box::pin(async move { Ok(found) })
    }
}

/// One invoice the centre drafted.
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

/// One credit note the book issued.
#[derive(Debug, Clone)]
pub struct Noted {
    /// The id it answered with.
    pub id: i64,
    /// The invoice it credits.
    pub invoice: i64,
    /// The lines credited: position and net.
    pub lines: Vec<(i64, i64)>,
    /// The reason given.
    pub reason: String,
    /// The total with VAT, in fils.
    pub gross: i64,
    /// Whether the invoice was credited in full.
    pub in_full: bool,
}

/// The VAT of a net amount at 5%, half-up.
fn vat_of(net: i64) -> i64 {
    edtech::refund::ratio(net, 500, 10_000)
}

/// The invoice book: the invoices drafted, what was paid on them and the
/// credit notes issued.
#[derive(Clone, Default)]
pub struct Books {
    invoices: Arc<Mutex<Vec<Invoiced>>>,
    payments: Arc<Mutex<Vec<(i64, i64)>>>,
    notes: Arc<Mutex<Vec<Noted>>>,
    discarded: Arc<Mutex<Vec<i64>>>,
}

impl Books {
    /// `amount` fils were paid on invoice `id`.
    pub fn pay(&self, id: i64, amount: i64) {
        self.payments.lock().unwrap().push((id, amount));
    }

    /// `amount` fils paid on invoice `id` were reversed (a chargeback).
    pub fn reverse(&self, id: i64, amount: i64) {
        self.payments.lock().unwrap().push((id, -amount));
    }

    /// What invoice `id` was paid so far, in fils.
    pub fn paid_of(&self, id: i64) -> i64 {
        self.payments
            .lock()
            .unwrap()
            .iter()
            .filter(|(invoice, _)| *invoice == id)
            .map(|(_, amount)| amount)
            .sum()
    }

    /// Every invoice, oldest first.
    pub fn all(&self) -> Vec<Invoiced> {
        self.invoices.lock().unwrap().clone()
    }

    /// The credit notes issued, oldest first.
    pub fn notes(&self) -> Vec<Noted> {
        self.notes.lock().unwrap().clone()
    }

    /// The drafts discarded, by invoice id.
    pub fn discarded(&self) -> Vec<i64> {
        self.discarded.lock().unwrap().clone()
    }

    /// Whether invoice `id` was credited in full.
    pub fn credited_in_full(&self, id: i64) -> bool {
        self.notes
            .lock()
            .unwrap()
            .iter()
            .any(|n| n.invoice == id && n.in_full)
    }

    /// The summary of credit note `note`.
    fn note_summary(&self, note: &Noted) -> CreditNoteSummary {
        let party = self
            .invoices
            .lock()
            .unwrap()
            .iter()
            .find(|i| i.id == note.invoice)
            .map(|i| i.draft.party)
            .unwrap_or(PartyId(0));
        CreditNoteSummary {
            id: CreditNoteId(note.id),
            number: Some(format!("CN-{}", note.id)),
            status: CreditNoteStatus::Issued,
            invoice: InvoiceId(note.invoice),
            party,
            legal_entity_id: Some(1),
            currency: "AED".to_owned(),
            gross_total: note.gross,
            remaining: note.gross,
        }
    }

    /// Records a credit note of `lines` (position, net) on `invoice`.
    fn credit(&self, invoice: i64, lines: Vec<(i64, i64)>, reason: &str, in_full: bool) -> i64 {
        let gross = lines.iter().map(|(_, net)| net + vat_of(*net)).sum();
        let mut notes = self.notes.lock().unwrap();
        let id = 500 + i64::try_from(notes.len()).unwrap();
        notes.push(Noted {
            id,
            invoice,
            lines,
            reason: reason.to_owned(),
            gross,
            in_full,
        });
        id
    }
}

impl Invoicer for Books {
    fn draft<'a>(
        &'a self,
        _db: &'a Db,
        draft: InvoiceDraft,
    ) -> InvoiceFuture<'a, Option<InvoiceId>> {
        let mut all = self.invoices.lock().unwrap();
        let id = 100 + i64::try_from(all.len()).unwrap();
        all.push(Invoiced {
            id,
            draft,
            issued: false,
        });
        Box::pin(async move { Ok(Some(InvoiceId(id))) })
    }

    fn discard_draft<'a>(&'a self, _db: &'a Db, id: InvoiceId) -> InvoiceFuture<'a, bool> {
        let is_draft = self
            .invoices
            .lock()
            .unwrap()
            .iter()
            .any(|i| i.id == id.0 && !i.issued);
        if is_draft {
            self.discarded.lock().unwrap().push(id.0);
        }
        Box::pin(async move { Ok(is_draft) })
    }

    fn find<'a>(&'a self, _db: &'a Db, id: InvoiceId) -> InvoiceFuture<'a, Option<InvoiceSummary>> {
        let found = self
            .invoices
            .lock()
            .unwrap()
            .iter()
            .find(|i| i.id == id.0)
            .map(|i| {
                let credited = self.credited_in_full(i.id);
                let discarded = self.discarded.lock().unwrap().contains(&i.id);
                let balance = if credited || discarded {
                    0
                } else {
                    i.gross() - self.paid_of(i.id)
                };
                let status = if credited {
                    InvoiceStatus::Credited
                } else if !i.issued {
                    InvoiceStatus::Draft
                } else if balance == 0 && i.gross() > 0 {
                    InvoiceStatus::Paid
                } else if balance < i.gross() {
                    InvoiceStatus::PartiallyPaid
                } else {
                    InvoiceStatus::Issued
                };
                InvoiceSummary {
                    id,
                    number: i.issued.then(|| format!("INV-{}", i.id)),
                    status,
                    party: i.draft.party,
                    legal_entity_id: Some(1),
                    currency: i.draft.currency.clone(),
                    gross_total: i.gross(),
                    balance,
                    due_date: None,
                    disputed: false,
                    chasing_paused_until: None,
                }
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
        for invoice in self.invoices.lock().unwrap().iter_mut() {
            if invoice.id == id.0 {
                invoice.issued = true;
            }
        }
        Box::pin(async { Ok(()) })
    }

    fn credit_in_full<'a>(
        &'a self,
        _state: &'a AppState,
        invoice: InvoiceId,
        reason: String,
        _by: Option<i64>,
    ) -> InvoiceFuture<'a, CreditNoteId> {
        let lines: Vec<(i64, i64)> = self
            .invoices
            .lock()
            .unwrap()
            .iter()
            .find(|i| i.id == invoice.0)
            .map(|i| {
                i.draft
                    .lines
                    .iter()
                    .enumerate()
                    .map(|(n, line)| (i64::try_from(n).unwrap() + 1, line.net()))
                    .collect()
            })
            .unwrap_or_default();
        let id = self.credit(invoice.0, lines, &reason, true);
        Box::pin(async move { Ok(CreditNoteId(id)) })
    }

    fn credit_partly<'a>(
        &'a self,
        _state: &'a AppState,
        invoice: InvoiceId,
        lines: Vec<CreditLine>,
        reason: String,
        _by: Option<i64>,
    ) -> InvoiceFuture<'a, CreditNoteId> {
        let lines = lines.iter().map(|l| (l.invoice_line, l.net)).collect();
        let id = self.credit(invoice.0, lines, &reason, false);
        Box::pin(async move { Ok(CreditNoteId(id)) })
    }

    fn find_credit_note<'a>(
        &'a self,
        _db: &'a Db,
        id: CreditNoteId,
    ) -> InvoiceFuture<'a, Option<CreditNoteSummary>> {
        let found = self
            .notes
            .lock()
            .unwrap()
            .iter()
            .find(|n| n.id == id.0)
            .map(|n| self.note_summary(n));
        Box::pin(async move { Ok(found) })
    }

    fn credit_notes_for<'a>(
        &'a self,
        _db: &'a Db,
        invoice: InvoiceId,
    ) -> InvoiceFuture<'a, Vec<CreditNoteSummary>> {
        let found: Vec<CreditNoteSummary> = self
            .notes
            .lock()
            .unwrap()
            .iter()
            .filter(|n| n.invoice == invoice.0)
            .map(|n| self.note_summary(n))
            .collect();
        Box::pin(async move { Ok(found) })
    }
}

/// A payment a test puts in front of the payment recorder.
#[derive(Debug, Clone)]
pub struct Paid {
    /// What it is for.
    pub prepaid: Subject,
    /// Where it stands.
    pub status: PaymentStatus,
    /// The amount, in fils.
    pub amount: i64,
    /// The id it answers with.
    pub id: i64,
}

/// A payment link the centre asked for.
#[derive(Debug, Clone)]
pub struct Linked {
    /// What was asked.
    pub request: PaymentLinkRequest,
    /// The id it answered with.
    pub id: i64,
    /// Whether it was cancelled.
    pub cancelled: bool,
}

/// The payments module as the enrolments see it: the payments made for a
/// record, the refunds asked, the payment links made and cancelled.
#[derive(Clone, Default)]
pub struct Cash {
    payments: Arc<Mutex<Vec<Paid>>>,
    refunds: Arc<Mutex<Vec<(RefundRequest, Option<i64>)>>>,
    links: Arc<Mutex<Vec<Linked>>>,
    no_links: Arc<Mutex<bool>>,
    refusing: Arc<Mutex<bool>>,
}

impl Cash {
    /// A payment of `amount` fils for `prepaid`, in `status`.
    pub fn add(&self, prepaid: Subject, status: PaymentStatus, amount: i64) -> i64 {
        let mut all = self.payments.lock().unwrap();
        let id = 700 + i64::try_from(all.len()).unwrap();
        all.push(Paid {
            prepaid,
            status,
            amount,
            id,
        });
        id
    }

    /// Payment `id` is now in `status`.
    pub fn set_status(&self, id: i64, status: PaymentStatus) {
        for payment in self.payments.lock().unwrap().iter_mut() {
            if payment.id == id {
                payment.status = status;
            }
        }
    }

    /// The refunds asked, with the user who asked (`None`: the system).
    pub fn refunds(&self) -> Vec<(RefundRequest, Option<i64>)> {
        self.refunds.lock().unwrap().clone()
    }

    /// The payment links made, oldest first.
    pub fn links(&self) -> Vec<Linked> {
        self.links.lock().unwrap().clone()
    }

    /// From now on no link can be made (no gateway).
    pub fn without_gateway(&self) {
        *self.no_links.lock().unwrap() = true;
    }

    /// From now on a refund is refused (`true`), or taken again (`false`).
    pub fn refuse_refunds(&self, refuse: bool) {
        *self.refusing.lock().unwrap() = refuse;
    }
}

impl PaymentRecorder for Cash {
    fn record<'a>(
        &'a self,
        _state: &'a AppState,
        _payment: RecordPayment,
        _by: Option<i64>,
    ) -> PaymentFuture<'a, PaymentSummary> {
        Box::pin(async {
            Err(Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                "not part of these tests".to_owned(),
            ))
        })
    }

    fn allocate<'a>(
        &'a self,
        _state: &'a AppState,
        _payment: PaymentId,
        _target: AllocationTarget,
        _amount: i64,
    ) -> PaymentFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn reverse<'a>(
        &'a self,
        _state: &'a AppState,
        _payment: PaymentId,
        _reason: ReversalReason,
        _note: Option<String>,
        _by: Option<i64>,
    ) -> PaymentFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn find<'a>(
        &'a self,
        _db: &'a Db,
        _id: PaymentId,
    ) -> PaymentFuture<'a, Option<PaymentSummary>> {
        Box::pin(async { Ok(None) })
    }

    fn credit_balance<'a>(
        &'a self,
        _db: &'a Db,
        _party: PartyId,
        _currency: &'a str,
    ) -> PaymentFuture<'a, i64> {
        Box::pin(async { Ok(0) })
    }

    fn payments_for<'a>(
        &'a self,
        _db: &'a Db,
        prepaid: &'a Subject,
    ) -> PaymentFuture<'a, Vec<PaymentSummary>> {
        let found: Vec<PaymentSummary> = self
            .payments
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.prepaid == *prepaid)
            .map(|p| PaymentSummary {
                id: PaymentId(p.id),
                number: None,
                party: PartyId(0),
                legal_entity_id: 1,
                direction: wl_kernel::payments::Direction::In,
                method: wl_kernel::payments::PaymentMethod::Gateway,
                amount: p.amount,
                allocated: 0,
                unallocated: p.amount,
                currency: "AED".to_owned(),
                status: p.status,
                date: wl_kernel::documents::today(),
            })
            .collect();
        Box::pin(async move { Ok(found) })
    }

    fn request_refund<'a>(
        &'a self,
        _state: &'a AppState,
        request: RefundRequest,
        by: Option<i64>,
    ) -> PaymentFuture<'a, RefundId> {
        if *self.refusing.lock().unwrap() {
            return Box::pin(async {
                Err(Error::Status(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "a refund cannot be requested: payments is not enabled".to_owned(),
                ))
            });
        }
        let mut all = self.refunds.lock().unwrap();
        all.push((request, by));
        let id = i64::try_from(all.len()).unwrap();
        Box::pin(async move { Ok(RefundId(id)) })
    }
}

impl PaymentLinks for Cash {
    fn create<'a>(
        &'a self,
        _state: &'a AppState,
        request: PaymentLinkRequest,
    ) -> PaymentFuture<'a, Option<PaymentLinkRef>> {
        if *self.no_links.lock().unwrap() {
            return Box::pin(async { Ok(None) });
        }
        let mut all = self.links.lock().unwrap();
        let id = 900 + i64::try_from(all.len()).unwrap();
        all.push(Linked {
            request,
            id,
            cancelled: false,
        });
        let link = PaymentLinkRef {
            link: Subject::new(wl_kernel::payments::PAYMENT_LINK_SUBJECT_TYPE, id),
            path: format!("/pay/link-{id}"),
        };
        Box::pin(async move { Ok(Some(link)) })
    }

    fn cancel<'a>(&'a self, _state: &'a AppState, link: &'a Subject) -> PaymentFuture<'a, bool> {
        let mut open = false;
        for made in self.links.lock().unwrap().iter_mut() {
            if made.id == link.subject_id && !made.cancelled {
                made.cancelled = true;
                open = true;
            }
        }
        Box::pin(async move { Ok(open) })
    }
}

/// The notifier: the notifications asked of it. It can be made to fail, except
/// for the types a test says always work (the kernel's alert to the owner is a
/// different channel from the centre's).
#[derive(Clone, Default)]
pub struct Messages(
    Arc<Mutex<Vec<NotifyRequest>>>,
    Arc<Mutex<bool>>,
    Arc<Mutex<Vec<String>>>,
);

impl Messages {
    /// Makes every further notification fail (or work again).
    pub fn fail(&self, failing: bool) {
        *self.1.lock().unwrap() = failing;
    }

    /// Notifications of the type `key` work even while the notifier fails.
    pub fn always_works(&self, key: &str) {
        self.2.lock().unwrap().push(key.to_owned());
    }

    /// The notifications sent so far, oldest first.
    pub fn sent(&self) -> Vec<NotifyRequest> {
        self.0.lock().unwrap().clone()
    }

    /// The notifications of `type_key` sent so far.
    pub fn of_type(&self, type_key: &str) -> Vec<NotifyRequest> {
        self.sent()
            .into_iter()
            .filter(|n| n.type_key == type_key)
            .collect()
    }

    /// The English title and body of `request`, joined: what a person reads.
    pub fn text(request: &NotifyRequest) -> String {
        let def: Arc<NotificationTypeDef> =
            wl_kernel::notify::notification_type(&request.type_key)
                .expect("the type is registered");
        let template = def
            .template_for(NotifyChannel::Email, "en")
            .expect("an English e-mail text");
        let subject = template
            .subject
            .as_deref()
            .map(|s| wl_kernel::notify::fill(s, &request.data))
            .unwrap_or_default();
        format!("{subject} {}", wl_kernel::notify::fill(&template.body, &request.data))
    }

    /// Whether a notification went to the user `id`.
    pub fn told_user(request: &NotifyRequest, id: i64) -> bool {
        request
            .recipients
            .iter()
            .any(|r| matches!(r, wl_kernel::notify::NotifyRecipient::User { id: user } if *user == id))
    }

    /// Whether a notification went to the party `id`.
    pub fn told_party(request: &NotifyRequest, id: i64) -> bool {
        request
            .recipients
            .iter()
            .any(|r| matches!(r, wl_kernel::notify::NotifyRecipient::Party { id: party } if party.0 == id))
    }
}

impl Notifier for Messages {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        let exempt = self.2.lock().unwrap().contains(&request.type_key);
        if *self.1.lock().unwrap() && !exempt {
            return Box::pin(async { Err(Error::permanent_message("the notifier is down")) });
        }
        let mut sent = self.0.lock().unwrap();
        sent.push(request);
        let id = i64::try_from(sent.len()).unwrap();
        Box::pin(async move { Ok(vec![MessageId(id)]) })
    }
}

/// One subscription of the stand-in.
#[derive(Clone)]
struct Sub {
    id: i64,
    start: StartSubscription,
    status: SubscriptionStatus,
    causes: Vec<PauseCause>,
    cancelled_on: Option<NaiveDate>,
}

impl Sub {
    fn summary(&self) -> SubscriptionSummary {
        SubscriptionSummary {
            id: SubscriptionId(self.id),
            party: self.start.party,
            source: self.start.source.clone(),
            status: self.status,
            cycle: self.start.cycle,
            next_cycle_date: self.start.start,
            pause_causes: self.causes.clone(),
            cancelled_on: self.cancelled_on,
            final_credit_note: None,
            final_invoice: None,
        }
    }

    fn live(&self) -> bool {
        self.status != SubscriptionStatus::Cancelled
    }
}

/// The subscriptions as wl-invoicing keeps them: started, paused and resumed
/// per cause (a cause already open is not added twice, a subscription resumes
/// when none is left) and cancelled, with the events it emits after a pause
/// cause is added and removed (`SubscriptionPaused`, `SubscriptionResumed`).
/// The calls through the contract are recorded; `user_pauses`, `user_resumes`
/// and `block_pauses` do what the guardian, staff and wl-parties do on the
/// other side.
#[derive(Clone, Default)]
pub struct Subs {
    all: Arc<Mutex<Vec<Sub>>>,
    cancelled: Arc<Mutex<Vec<(Subject, NaiveDate)>>>,
    pauses: Arc<Mutex<Vec<(Subject, PauseCause)>>>,
    resumes: Arc<Mutex<Vec<(Subject, PauseCause)>>>,
}

impl Subs {
    /// The subscriptions started, oldest first.
    pub fn started(&self) -> Vec<StartSubscription> {
        self.all
            .lock()
            .unwrap()
            .iter()
            .map(|sub| sub.start.clone())
            .collect()
    }

    /// The agreements whose subscriptions were cancelled, with the day.
    pub fn cancelled(&self) -> Vec<(Subject, NaiveDate)> {
        self.cancelled.lock().unwrap().clone()
    }

    /// The pauses the enrolment asked of the contract, with their cause.
    pub fn pauses(&self) -> Vec<(Subject, PauseCause)> {
        self.pauses.lock().unwrap().clone()
    }

    /// The resumes the enrolment asked of the contract, with their cause.
    pub fn resumes(&self) -> Vec<(Subject, PauseCause)> {
        self.resumes.lock().unwrap().clone()
    }

    /// Where the live subscription of `agreement` stands.
    pub fn status_of(&self, agreement: &Subject) -> Option<SubscriptionStatus> {
        self.all
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|sub| sub.live() && sub.start.source == *agreement)
            .map(|sub| sub.status)
    }

    /// The pause causes open on the live subscription of `agreement`.
    pub fn causes_of(&self, agreement: &Subject) -> Vec<PauseCause> {
        self.all
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|sub| sub.live() && sub.start.source == *agreement)
            .map(|sub| sub.causes.clone())
            .unwrap_or_default()
    }

    /// Adds `cause` to the live subscription of `agreement`; the event to emit
    /// when it was added.
    fn add_cause(
        &self,
        agreement: &Subject,
        cause: PauseCause,
        until: Option<NaiveDate>,
    ) -> Option<SubscriptionPaused> {
        let mut all = self.all.lock().unwrap();
        let sub = all
            .iter_mut()
            .rev()
            .find(|sub| sub.live() && sub.start.source == *agreement)?;
        if sub.status == SubscriptionStatus::Suspended || sub.causes.contains(&cause) {
            return None;
        }
        let first = sub.status != SubscriptionStatus::Paused;
        sub.causes.push(cause);
        sub.status = SubscriptionStatus::Paused;
        Some(SubscriptionPaused {
            subscription: SubscriptionId(sub.id),
            party: sub.start.party,
            source: sub.start.source.clone(),
            cause,
            until,
            first,
        })
    }

    /// Removes `cause` from the live subscription of `agreement`; the event to
    /// emit when it was open. `past_due` says where it goes back to.
    fn remove_cause(
        &self,
        agreement: &Subject,
        cause: PauseCause,
        past_due: bool,
    ) -> Option<SubscriptionResumed> {
        let mut all = self.all.lock().unwrap();
        let sub = all
            .iter_mut()
            .rev()
            .find(|sub| sub.live() && sub.start.source == *agreement)?;
        if !sub.causes.contains(&cause) {
            return None;
        }
        sub.causes.retain(|open| *open != cause);
        let resumed = sub.causes.is_empty() && sub.status == SubscriptionStatus::Paused;
        if resumed {
            sub.status = if past_due {
                SubscriptionStatus::PastDue
            } else {
                SubscriptionStatus::Active
            };
        }
        Some(SubscriptionResumed {
            subscription: SubscriptionId(sub.id),
            party: sub.start.party,
            source: sub.start.source.clone(),
            cause,
            resumed,
            to_past_due: resumed && past_due,
        })
    }

    /// The guardian (or staff) pauses the membership of `agreement` until
    /// `until`: wl-invoicing adds the cause `user` and tells the vertical.
    pub async fn user_pauses(&self, state: &AppState, agreement: &Subject, until: NaiveDate) {
        if let Some(event) = self.add_cause(agreement, PauseCause::User, Some(until)) {
            state.emit(event).await.expect("the pause is heard");
        }
    }

    /// The user's pause of the membership of `agreement` ends; it goes back to
    /// `past_due` instead of `active` when a cycle invoice is still overdue.
    pub async fn user_resumes(&self, state: &AppState, agreement: &Subject, past_due: bool) {
        if let Some(event) = self.remove_cause(agreement, PauseCause::User, past_due) {
            state.emit(event).await.expect("the resume is heard");
        }
    }

    /// wl-parties blocks the payer: wl-invoicing pauses the membership of
    /// `agreement` with the cause `party_block` by itself.
    pub async fn block_pauses(&self, state: &AppState, agreement: &Subject) {
        if let Some(event) = self.add_cause(agreement, PauseCause::PartyBlock, None) {
            state.emit(event).await.expect("the pause is heard");
        }
    }
}

impl Subscriptions for Subs {
    fn start<'a>(
        &'a self,
        _state: &'a AppState,
        start: StartSubscription,
    ) -> SubscriptionFuture<'a, Option<SubscriptionId>> {
        let mut all = self.all.lock().unwrap();
        let id = i64::try_from(all.len()).unwrap() + 1;
        all.push(Sub {
            id,
            start,
            status: SubscriptionStatus::Active,
            causes: Vec::new(),
            cancelled_on: None,
        });
        Box::pin(async move { Ok(Some(SubscriptionId(id))) })
    }

    fn of_agreement<'a>(
        &'a self,
        _db: &'a Db,
        agreement: &'a Subject,
    ) -> SubscriptionFuture<'a, Vec<SubscriptionSummary>> {
        let found: Vec<SubscriptionSummary> = self
            .all
            .lock()
            .unwrap()
            .iter()
            .filter(|sub| sub.start.source == *agreement)
            .map(Sub::summary)
            .collect();
        Box::pin(async move { Ok(found) })
    }

    fn pause<'a>(
        &'a self,
        state: &'a AppState,
        agreement: &'a Subject,
        pause: PauseSubscription,
    ) -> SubscriptionFuture<'a, Vec<SubscriptionSummary>> {
        self.pauses
            .lock()
            .unwrap()
            .push((agreement.clone(), pause.cause));
        let event = self.add_cause(agreement, pause.cause, pause.until);
        Box::pin(async move {
            if let Some(event) = event {
                state.emit(event).await?;
            }
            Ok(self.of_agreement_now(agreement))
        })
    }

    fn resume<'a>(
        &'a self,
        state: &'a AppState,
        agreement: &'a Subject,
        cause: PauseCause,
        _on: NaiveDate,
    ) -> SubscriptionFuture<'a, Vec<SubscriptionSummary>> {
        self.resumes
            .lock()
            .unwrap()
            .push((agreement.clone(), cause));
        let event = self.remove_cause(agreement, cause, false);
        Box::pin(async move {
            if let Some(event) = event {
                state.emit(event).await?;
            }
            Ok(self.of_agreement_now(agreement))
        })
    }

    fn cancel<'a>(
        &'a self,
        _state: &'a AppState,
        agreement: &'a Subject,
        cancel: CancelSubscription,
    ) -> SubscriptionFuture<'a, Vec<SubscriptionSummary>> {
        self.cancelled
            .lock()
            .unwrap()
            .push((agreement.clone(), cancel.on));
        for sub in self.all.lock().unwrap().iter_mut() {
            if sub.start.source == *agreement && sub.live() {
                sub.status = SubscriptionStatus::Cancelled;
                sub.cancelled_on = Some(cancel.on);
            }
        }
        Box::pin(async { Ok(Vec::new()) })
    }
}

impl Subs {
    fn of_agreement_now(&self, agreement: &Subject) -> Vec<SubscriptionSummary> {
        self.all
            .lock()
            .unwrap()
            .iter()
            .filter(|sub| sub.start.source == *agreement)
            .map(Sub::summary)
            .collect()
    }
}

/// The prepaid packages: sold, with credits a test gives.
#[derive(Clone, Default)]
pub struct Packages {
    sold: Arc<Mutex<Vec<SellPackage>>>,
    credits: Arc<Mutex<Vec<(i64, String, i64)>>>,
    ends: Arc<Mutex<BTreeMap<i64, NaiveDate>>>,
    extended: Arc<Mutex<Vec<(i64, i64, String)>>>,
    used: Arc<Mutex<Vec<(Subject, i64, String)>>>,
    asked: Arc<Mutex<Vec<Subject>>>,
}

impl Packages {
    /// The sources that hold a credit now: the attendance rows (or bookings)
    /// whose credit was consumed and not given back.
    pub fn consumed_by(&self) -> Vec<Subject> {
        self.used
            .lock()
            .unwrap()
            .iter()
            .map(|(source, _, _)| source.clone())
            .collect()
    }

    /// Every source `consume` was asked for, whatever the answer.
    pub fn asked(&self) -> Vec<Subject> {
        self.asked.lock().unwrap().clone()
    }

    /// The credits `party` has left of `item`.
    pub fn left(&self, party: PartyId, item: &str) -> i64 {
        self.credits
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(holder, code, _)| *holder == party.0 && code == item)
            .map_or(0, |(_, _, credits)| *credits)
    }

    /// The packages sold, oldest first.
    pub fn sold(&self) -> Vec<SellPackage> {
        self.sold.lock().unwrap().clone()
    }

    /// Package `id` is valid through `end`.
    pub fn set_end(&self, id: i64, end: NaiveDate) {
        self.ends.lock().unwrap().insert(id, end);
    }

    /// The last day package `id` is valid, if it has one.
    pub fn end_of(&self, id: i64) -> Option<NaiveDate> {
        self.ends.lock().unwrap().get(&id).copied()
    }

    /// The extensions asked of the ledger: package, days, reason.
    pub fn extended(&self) -> Vec<(i64, i64, String)> {
        self.extended.lock().unwrap().clone()
    }

    /// `party` holds `credits` of `item` now.
    pub fn give(&self, party: PartyId, item: &str, credits: i64) {
        self.credits
            .lock()
            .unwrap()
            .push((party.0, item.to_owned(), credits));
    }
}

impl PackageSales for Packages {
    fn sell<'a>(
        &'a self,
        _state: &'a AppState,
        sale: SellPackage,
    ) -> SalesFuture<'a, Option<PackageId>> {
        let mut all = self.sold.lock().unwrap();
        all.push(sale);
        let id = i64::try_from(all.len()).unwrap();
        Box::pin(async move { Ok(Some(PackageId(id))) })
    }
}

impl PackageLedger for Packages {
    /// As wl-sales does: a source that holds a credit consumes nothing more
    /// (`Already`); otherwise the holder's package for the item gives one while
    /// it has any, else there is `NoCredit`.
    fn consume<'a>(
        &'a self,
        _state: &'a AppState,
        request: wl_kernel::sales::ConsumePackage,
    ) -> SalesFuture<'a, PackageConsumption> {
        self.asked.lock().unwrap().push(request.source.clone());
        let mut used = self.used.lock().unwrap();
        if used.iter().any(|(source, _, _)| *source == request.source) {
            return Box::pin(async { Ok(PackageConsumption::Already { package: PackageId(1) }) });
        }
        let mut credits = self.credits.lock().unwrap();
        let answer = match credits
            .iter_mut()
            .rev()
            .find(|(holder, code, left)| {
                *holder == request.party.0 && *code == request.item && *left >= request.quantity
            }) {
            Some((_, _, left)) => {
                *left -= request.quantity;
                used.push((request.source.clone(), request.party.0, request.item.clone()));
                PackageConsumption::Consumed {
                    package: PackageId(1),
                    remaining: *left,
                }
            }
            None => PackageConsumption::NoCredit,
        };
        Box::pin(async move { Ok(answer) })
    }

    /// As wl-sales does: the credit the source consumed goes back to its
    /// package.
    fn reverse<'a>(&'a self, _state: &'a AppState, source: Subject) -> SalesFuture<'a, bool> {
        let mut used = self.used.lock().unwrap();
        let found = used.iter().position(|(held, _, _)| *held == source);
        let gave_back = match found {
            Some(index) => {
                let (_, party, item) = used.remove(index);
                let mut credits = self.credits.lock().unwrap();
                if let Some((_, _, left)) = credits
                    .iter_mut()
                    .rev()
                    .find(|(holder, code, _)| *holder == party && *code == item)
                {
                    *left += 1;
                }
                true
            }
            None => false,
        };
        Box::pin(async move { Ok(gave_back) })
    }

    fn extend_days<'a>(
        &'a self,
        _state: &'a AppState,
        package: PackageId,
        days: i64,
        why: &'a str,
    ) -> SalesFuture<'a, Option<NaiveDate>> {
        self.extended
            .lock()
            .unwrap()
            .push((package.0, days, why.to_owned()));
        let mut ends = self.ends.lock().unwrap();
        let moved = ends.get_mut(&package.0).map(|end| {
            *end += renox::chrono::TimeDelta::days(days);
            *end
        });
        Box::pin(async move { Ok(moved) })
    }

    fn balance<'a>(&'a self, _db: &'a Db, party: PartyId, item: &'a str) -> SalesFuture<'a, i64> {
        let left = self
            .credits
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(holder, code, _)| *holder == party.0 && code == item)
            .map_or(0, |(_, _, credits)| *credits);
        Box::pin(async move { Ok(left) })
    }
}
