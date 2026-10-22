//! A package whose payment is reversed (BL-SAL-16, #135): the invoice reopens,
//! the package stays `active` with its used credits, the seller gets a task
//! to collect, the holder is told, and an optional setting suspends
//! `consume` until the invoice is paid.

mod support;

use std::sync::{Arc, Mutex};

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::invoicing::{InvoiceId, InvoicePaid, InvoiceReopened, InvoiceStatus, ReopenReason};
use wl_kernel::notify::{MessageId, Notifier, NotifyFuture, NotifyRecipient, NotifyRequest};
use wl_kernel::payments::{
    PaymentId, PaymentReversed, PaymentSource, RemovedAllocation, ReversalReason,
};
use wl_kernel::sales::{ConsumePackage, PackageConsumption, package_ledger};
use wl_kernel::tickets::{
    Assignee, OpenTicket, TaskComment, TaskCreator, TicketFuture, TicketId, TicketRef,
};
use wl_kernel::{PartyId, Subject};
use wl_sales::notices::PACKAGE_BALANCE_DUE;
use wl_sales::package_reopen::{refusals, task_key};
use wl_sales::packages::{self, NewPackage};
use wl_sales::{Package, PackageStatus, WlSales};

use support::{Booted, boot_configured};

const HOLDER: PartyId = PartyId(7);
const ITEM: &str = "physio_10";

/// The tasks opened and the comments added, as ticketing would keep them.
#[derive(Clone, Default)]
struct Desk {
    opened: Arc<Mutex<Vec<OpenTicket>>>,
    comments: Arc<Mutex<Vec<(TicketId, TaskComment)>>>,
}

impl Desk {
    fn opened(&self) -> Vec<OpenTicket> {
        self.opened.lock().unwrap().clone()
    }

    fn comments(&self) -> Vec<(TicketId, TaskComment)> {
        self.comments.lock().unwrap().clone()
    }
}

impl TaskCreator for Desk {
    fn open<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: OpenTicket,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move {
            let mut opened = self.opened.lock().unwrap();
            opened.push(ticket);
            let n = opened.len() as i64;
            Ok(Some(TicketRef {
                id: TicketId(n),
                number: format!("TKT-{n}"),
            }))
        })
    }

    fn find_open<'a>(
        &'a self,
        _state: &'a AppState,
        source_key: &'a str,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move {
            let opened = self.opened.lock().unwrap();
            Ok(opened
                .iter()
                .position(|t| t.source_key.as_deref() == Some(source_key))
                .map(|n| TicketRef {
                    id: TicketId(n as i64 + 1),
                    number: format!("TKT-{}", n + 1),
                }))
        })
    }

    fn comment<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: TicketId,
        comment: TaskComment,
    ) -> TicketFuture<'a, ()> {
        Box::pin(async move {
            self.comments.lock().unwrap().push((ticket, comment));
            Ok(())
        })
    }
}

/// Every notification asked for.
#[derive(Clone, Default)]
struct Mail(Arc<Mutex<Vec<NotifyRequest>>>);

impl Mail {
    fn balance_due(&self) -> Vec<NotifyRequest> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.type_key == PACKAGE_BALANCE_DUE)
            .cloned()
            .collect()
    }
}

impl Notifier for Mail {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(request);
            Ok(vec![MessageId(1)])
        })
    }
}

/// Provides the fakes.
struct Fakes {
    desk: Desk,
    mail: Mail,
}

impl Module for Fakes {
    fn name(&self) -> &'static str {
        "fakes"
    }

    fn register(&self, app: &mut Registry) {
        wl_kernel::tickets::provide_task_creator(app, self.desk.clone());
        wl_kernel::notify::provide_notifier(app, self.mail.clone());
    }
}

async fn boot_with_setting(sales: WlSales) -> (Booted, Desk, Mail) {
    let desk = Desk::default();
    let mail = Mail::default();
    let t = boot_configured(
        sales,
        Fakes {
            desk: desk.clone(),
            mail: mail.clone(),
        },
    )
    .await;
    (t, desk, mail)
}

async fn boot() -> (Booted, Desk, Mail) {
    boot_with_setting(WlSales::new()).await
}

/// An active package of 10 credits with 4 used, sold by `seller` and paid
/// with invoice 55.
async fn active_package(t: &Booted, seller: Option<i64>) -> Package {
    let mut new = NewPackage::new(HOLDER, ITEM, 10)
        .item_name("Physio, 10 sessions")
        .price(150_000)
        .invoice(InvoiceId(55));
    if let Some(seller) = seller {
        new = new.sold_by(seller);
    }
    let package = packages::sell(t.state(), new).await.unwrap();
    t.state()
        .emit(InvoicePaid {
            invoice: InvoiceId(55),
            party: HOLDER,
            source: package.subject(),
        })
        .await
        .unwrap();
    packages::consume(t.state(), package.id, 4, booking(1))
        .await
        .unwrap();
    reload(t, &package).await
}

async fn reload(t: &Booted, package: &Package) -> Package {
    Package::find_or_404(t.db(), package.id).await.unwrap()
}

fn booking(n: i64) -> Subject {
    Subject::new("scheduling.booking", n)
}

fn payment(n: i64) -> Subject {
    PaymentId(n).subject()
}

/// What `wl-invoicing` emits when the reversal of payment `n` reopens the
/// package's invoice.
fn reopened(package: &Package, n: i64, balance: i64) -> InvoiceReopened {
    InvoiceReopened {
        invoice: InvoiceId(55),
        party: HOLDER,
        source: package.subject(),
        money_source: payment(n),
        amount: balance,
        balance,
        new_status: InvoiceStatus::Issued,
        reason: ReopenReason::PaymentReversed,
        reversal_reason: Some("bank_return".into()),
    }
}

fn reversed(n: i64, prepaid: Option<Subject>, removed: Vec<RemovedAllocation>) -> PaymentReversed {
    PaymentReversed {
        payment: PaymentId(n),
        party: HOLDER,
        legal_entity_id: 1,
        reason: ReversalReason::BankReturn,
        source: PaymentSource::Manual,
        statement_line: None,
        removed,
        prepaid,
        reversed_on: NaiveDate::from_ymd_opt(2026, 11, 3).unwrap(),
    }
}

/// Done when 1: an `active` package (10 credits, 4 used) whose invoice is
/// reopened stays `active` with 6 left, the seller has a "collect before next
/// use" task, and the holder gets a balance-due message; the same reversal
/// delivered twice changes nothing more.
#[renox::test]
async fn a_reopened_invoice_leaves_the_package_active_and_asks_the_seller_to_collect() {
    let (t, desk, mail) = boot().await;
    let seller = t.user("Fatima", &[]).await;
    let package = active_package(&t, Some(seller.id)).await;
    assert_eq!(package.remaining(), 6);

    t.state()
        .emit(reopened(&package, 42, 150_000))
        .await
        .unwrap();

    let after = reload(&t, &package).await;
    assert_eq!(after.status, PackageStatus::Active);
    assert_eq!((after.credits_used, after.remaining()), (4, 6));
    assert_eq!(after.reopened_balance, 150_000);
    assert!(after.reopened_at.is_some());

    let tasks = desk.opened();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignee, Some(Assignee::user(seller.id)));
    assert!(
        tasks[0]
            .title
            .contains("Collect AED 1,500.00 before the next use")
    );
    assert!(tasks[0].title.contains("Physio, 10 sessions"));
    assert_eq!(tasks[0].subject, package.subject());
    assert_eq!(
        tasks[0].source_key.as_deref(),
        Some(task_key(package.id, &payment(42)).as_str())
    );
    assert_eq!(after.collect_ticket_id, Some(1));

    let notices = mail.balance_due();
    assert_eq!(notices.len(), 1);
    assert_eq!(
        notices[0].recipients,
        vec![NotifyRecipient::Party { id: HOLDER }]
    );
    assert_eq!(notices[0].data["balance"], "1,500.00");
    assert_eq!(notices[0].subject, package.subject());

    // The same event again: one task and one notice in all.
    t.state()
        .emit(reopened(&package, 42, 150_000))
        .await
        .unwrap();
    assert_eq!(desk.opened().len(), 1);
    assert_eq!(mail.balance_due().len(), 1);

    // A second reversal is a second task.
    t.state()
        .emit(reopened(&package, 43, 150_000))
        .await
        .unwrap();
    assert_eq!(desk.opened().len(), 2);
}

/// A package with no seller user gives the task to the holder of the
/// configured role; the payer is told as well as the holder.
#[renox::test]
async fn without_a_seller_user_the_role_is_asked_and_the_payer_is_told_too() {
    let sales = WlSales::new().package_seller_role("role_nadia");
    let (t, desk, mail) = boot_with_setting(sales).await;
    let nadia = t.user("Nadia", &[]).await;
    let new = NewPackage::new(HOLDER, ITEM, 10)
        .payer(PartyId(8))
        .invoice(InvoiceId(55));
    let package = packages::sell(t.state(), new).await.unwrap();
    t.state()
        .emit(InvoicePaid {
            invoice: InvoiceId(55),
            party: HOLDER,
            source: package.subject(),
        })
        .await
        .unwrap();

    t.state()
        .emit(reopened(&package, 42, 50_000))
        .await
        .unwrap();

    let tasks = desk.opened();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignee, Some(Assignee::user(nadia.id)));
    let told: Vec<_> = mail
        .balance_due()
        .into_iter()
        .flat_map(|r| r.recipients)
        .collect();
    assert_eq!(
        told,
        vec![
            NotifyRecipient::Party { id: HOLDER },
            NotifyRecipient::Party { id: PartyId(8) }
        ]
    );
}

/// "Package `completed`: same treatment"; a package never activated, or one
/// that was cancelled, has no credit to protect and is left alone.
#[renox::test]
async fn a_completed_package_gets_the_task_and_a_pending_one_does_not() {
    let (t, desk, mail) = boot().await;
    let package = active_package(&t, None).await;
    packages::consume(t.state(), package.id, 6, booking(2))
        .await
        .unwrap();
    assert_eq!(reload(&t, &package).await.status, PackageStatus::Completed);

    t.state()
        .emit(reopened(&package, 42, 150_000))
        .await
        .unwrap();
    assert_eq!(reload(&t, &package).await.status, PackageStatus::Completed);
    assert_eq!(desk.opened().len(), 1);
    assert_eq!(mail.balance_due().len(), 1);

    let pending = packages::sell(
        t.state(),
        NewPackage::new(HOLDER, "other", 3).invoice(InvoiceId(77)),
    )
    .await
    .unwrap();
    let mut event = reopened(&pending, 44, 10_000);
    event.invoice = InvoiceId(77);
    t.state().emit(event).await.unwrap();
    assert_eq!(desk.opened().len(), 1);
    assert_eq!(
        reload(&t, &pending).await.status,
        PackageStatus::PendingPayment
    );
}

/// Done when 2: with the setting off, the next `consume` succeeds.
#[renox::test]
async fn with_the_setting_off_the_next_consume_succeeds() {
    let (t, _, _) = boot().await;
    let package = active_package(&t, None).await;
    t.state()
        .emit(reopened(&package, 42, 150_000))
        .await
        .unwrap();

    let outcome = package_ledger(t.state())
        .consume(t.state(), ConsumePackage::new(HOLDER, ITEM, 1, booking(2)))
        .await
        .unwrap();
    assert_eq!(
        outcome,
        PackageConsumption::Consumed {
            package: package.package_id(),
            remaining: 5
        }
    );
    assert_eq!(reload(&t, &package).await.credits_used, 5);
}

/// Done when 3: with the setting on, `consume` is refused until the invoice
/// is paid, then succeeds; the refusal is on the seller's task and is not
/// replayed.
#[renox::test]
async fn with_the_setting_on_consume_waits_for_the_invoice_to_be_paid() {
    let sales = WlSales::new().suspend_package_consumption_while_unpaid(true);
    let (t, desk, _) = boot_with_setting(sales).await;
    let seller = t.user("Fatima", &[]).await;
    let package = active_package(&t, Some(seller.id)).await;
    t.state()
        .emit(reopened(&package, 42, 150_000))
        .await
        .unwrap();

    let ledger = package_ledger(t.state());
    let refused = ledger
        .consume(t.state(), ConsumePackage::new(HOLDER, ITEM, 1, booking(2)))
        .await
        .unwrap();
    assert_eq!(
        refused,
        PackageConsumption::Unpaid {
            package: package.package_id()
        }
    );
    let still = reload(&t, &package).await;
    assert_eq!(
        (still.status, still.credits_used),
        (PackageStatus::Active, 4)
    );
    assert!(
        still
            .movements(t.db())
            .await
            .unwrap()
            .iter()
            .all(|m| m.source() != Some(booking(2)))
    );

    // The refusal is recorded and listed on the seller's task.
    let rows = refusals(t.db(), package.id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].source_id, rows[0].quantity), (2, 1));
    let comments = desk.comments();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].0, TicketId(1));
    assert!(comments[0].1.text.contains("Credit not consumed: unpaid"));
    assert!(comments[0].1.text.contains("scheduling.booking:2"));

    // The invoice is paid: the suspension ends, the earlier refusal is not
    // replayed, a new consume goes through.
    t.state()
        .emit(InvoicePaid {
            invoice: InvoiceId(55),
            party: HOLDER,
            source: package.subject(),
        })
        .await
        .unwrap();
    let paid = reload(&t, &package).await;
    assert_eq!(paid.reopened_balance, 0);
    assert_eq!(paid.credits_used, 4, "the refused credit is not replayed");
    let outcome = ledger
        .consume(t.state(), ConsumePackage::new(HOLDER, ITEM, 1, booking(3)))
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        PackageConsumption::Consumed { remaining: 5, .. }
    ));
    // The seller consumes the refused one by hand.
    packages::consume(t.state(), package.id, 1, booking(2))
        .await
        .unwrap();
    assert_eq!(reload(&t, &package).await.credits_used, 6);
}

/// The reversal of a payment made for a package with an invoice is handled
/// from the reopened invoice, once; a package paid in advance with no invoice
/// is handled from the reversal itself.
#[renox::test]
async fn the_reversal_itself_counts_only_for_a_package_without_an_invoice() {
    let (t, desk, mail) = boot().await;
    let with_invoice = active_package(&t, None).await;
    let allocation = RemovedAllocation {
        allocation: 1,
        target: InvoiceId(55).subject(),
        amount: 150_000,
    };
    let event = reversed(42, Some(with_invoice.subject()), vec![allocation]);
    t.state().emit(event.clone()).await.unwrap();
    assert!(desk.opened().is_empty(), "wl-invoicing reopens the invoice");
    assert_eq!(
        reload(&t, &with_invoice).await.status,
        PackageStatus::Active
    );

    t.state()
        .emit(reopened(&with_invoice, 42, 150_000))
        .await
        .unwrap();
    t.state().emit(event).await.unwrap();
    assert_eq!(desk.opened().len(), 1);
    assert_eq!(mail.balance_due().len(), 1);

    // No invoice: a package paid by a payment link.
    let linked = packages::sell(
        t.state(),
        NewPackage::new(PartyId(9), "yoga", 5).price(40_000),
    )
    .await
    .unwrap();
    packages::activate(t.state(), linked.id, wl_kernel::documents::today())
        .await
        .unwrap();
    let event = reversed(50, Some(linked.subject()), Vec::new());
    t.state().emit(event.clone()).await.unwrap();
    t.state().emit(event).await.unwrap();
    let after = reload(&t, &linked).await;
    assert_eq!(after.status, PackageStatus::Active);
    assert_eq!(after.reopened_balance, 40_000);
    assert_eq!(desk.opened().len(), 2);
    assert_eq!(mail.balance_due().len(), 2);
}

/// Two invoices that name one package as their source: each reopening adds
/// its own balance (one invoice reopened twice keeps its latest balance),
/// and paying one invoice clears only its part.
#[renox::test]
async fn several_invoices_of_one_package_add_up_and_are_paid_one_by_one() {
    let (t, _, _) = boot().await;
    let package = active_package(&t, None).await;
    let on = |invoice: i64, n: i64, balance: i64| InvoiceReopened {
        invoice: InvoiceId(invoice),
        ..reopened(&package, n, balance)
    };

    t.state().emit(on(55, 42, 100_000)).await.unwrap();
    t.state().emit(on(56, 43, 30_000)).await.unwrap();
    assert_eq!(reload(&t, &package).await.reopened_balance, 130_000);

    // Invoice 55 reopened again with a larger balance replaces its own part.
    t.state().emit(on(55, 44, 120_000)).await.unwrap();
    assert_eq!(reload(&t, &package).await.reopened_balance, 150_000);

    // Invoice 56 paid: invoice 55 is still open, consumption stays suspended.
    t.state()
        .emit(InvoicePaid {
            invoice: InvoiceId(56),
            party: HOLDER,
            source: package.subject(),
        })
        .await
        .unwrap();
    let after = reload(&t, &package).await;
    assert_eq!(after.reopened_balance, 120_000);
    assert!(after.reopened_at.is_some());

    t.state()
        .emit(InvoicePaid {
            invoice: InvoiceId(55),
            party: HOLDER,
            source: package.subject(),
        })
        .await
        .unwrap();
    let after = reload(&t, &package).await;
    assert_eq!(after.reopened_balance, 0);
    assert!(after.reopened_at.is_none());
}

#[renox::test]
async fn the_new_tables_match_the_models() {
    let (t, _, _) = boot().await;
    t.app.assert_models_match_schema().await;
}
