//! The counter sale base (wl-sales §4.3, §5.4; base for BL-SAL-22): a cashier
//! opens a cart, adds lines, completes it with the tenders, and
//! `CounterSaleCompleted` follows the commit. A completed sale is never
//! edited (`SAL.ACL.04`).

mod support;

use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::invoicing::{
    InvoiceDraft, InvoiceFuture, InvoiceId, InvoiceSummary, Invoicer, provide_invoicer,
};
use wl_kernel::Subject;
use wl_kernel::sales::CounterSaleCompleted;
use wl_sales::permissions as perm;
use wl_sales::{CounterSale, CounterSalePayment, CounterSaleStatus, WlSales};

use support::{Booted, boot_configured};

/// Records the drafts it was asked for.
struct Books(Arc<Mutex<Vec<InvoiceDraft>>>);

impl Invoicer for Books {
    fn draft<'a>(
        &'a self,
        _db: &'a Db,
        draft: InvoiceDraft,
    ) -> InvoiceFuture<'a, Option<InvoiceId>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(draft);
            Ok(Some(InvoiceId(9)))
        })
    }

    fn find<'a>(
        &'a self,
        _db: &'a Db,
        _id: InvoiceId,
    ) -> InvoiceFuture<'a, Option<InvoiceSummary>> {
        Box::pin(async { Ok(None) })
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
}

/// The events and the drafts a test saw.
#[derive(Clone, Default)]
struct Seen {
    completed: Arc<Mutex<Vec<CounterSaleCompleted>>>,
    drafts: Arc<Mutex<Vec<InvoiceDraft>>>,
}

impl Module for Seen {
    fn name(&self) -> &'static str {
        "counter_events"
    }

    fn register(&self, app: &mut Registry) {
        let log = self.completed.clone();
        app.listen(move |event: CounterSaleCompleted, _state: AppState| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push(event);
                Ok(())
            }
        });
        provide_invoicer(app, Books(self.drafts.clone()));
    }
}

async fn boot_with_sales(sales: WlSales) -> (Booted, Seen) {
    let seen = Seen::default();
    let t = boot_configured(sales, seen.clone()).await;
    (t, seen)
}

async fn boot() -> (Booted, Seen) {
    boot_with_sales(WlSales::new()).await
}

async fn cashier(t: &Booted) -> User {
    t.user("Cora", &[perm::POS_SELL]).await
}

/// A cart for the customer `party` (empty for a walk-in), opened by the
/// signed-in cashier through the page.
async fn open(t: &Booted, party: &str) -> CounterSale {
    let fields: Vec<(&str, &str)> = if party.is_empty() {
        vec![("x", "")]
    } else {
        vec![("party_id", party)]
    };
    t.app.post("/counter-sales", &fields).await.assert_status(303);
    CounterSale::query()
        .order_by_desc(CounterSale::ID)
        .first(t.db())
        .await
        .unwrap()
        .expect("the cart was opened")
}

async fn add_line(t: &Booted, sale: &CounterSale, price: &str) {
    t.app
        .post(
            &format!("/counter-sales/{}/lines", sale.id),
            &[("item_code", "tee"), ("unit_price", price)],
        )
        .await
        .assert_status(303);
}

async fn reload(t: &Booted, sale: &CounterSale) -> CounterSale {
    CounterSale::find_or_404(t.db(), sale.id).await.unwrap()
}

#[renox::test]
async fn a_cashier_completes_a_sale_with_a_card_and_the_event_follows() {
    let (t, seen) = boot().await;
    t.app.acting_as(&cashier(&t).await);
    let sale = open(&t, "7").await;
    assert_eq!(sale.status, CounterSaleStatus::Open);
    assert!(sale.number.starts_with("RCPT-"), "{}", sale.number);
    add_line(&t, &sale, "100.00").await;
    let sale = reload(&t, &sale).await;
    assert_eq!(
        (sale.net_total, sale.vat_total, sale.gross_total),
        (10_000, 500, 10_500)
    );

    t.app
        .post(
            &format!("/counter-sales/{}/complete", sale.id),
            &[("card", "105.00"), ("card_payment", "77")],
        )
        .await
        .assert_status(303);

    let done = reload(&t, &sale).await;
    assert_eq!(done.status, CounterSaleStatus::Completed);
    assert!(done.completed_at.is_some());
    let paid: Vec<CounterSalePayment> = done.payments(t.db()).await.unwrap();
    assert_eq!(paid.len(), 1);
    assert_eq!((paid[0].method.as_str(), paid[0].amount), ("card", 10_500));
    assert_eq!(paid[0].payment_id, Some(77));
    assert_eq!(
        CounterSale::of_payment(t.db(), 77).await.unwrap().map(|s| s.id),
        Some(sale.id)
    );

    let events = seen.completed.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].sale.0, sale.id);
    assert_eq!(events[0].gross_total, 10_500);
    assert_eq!(events[0].by_method, vec![("card".to_owned(), 10_500)]);
    assert_eq!(events[0].party.map(|p| p.0), Some(7));

    // The sale named a customer, so its invoice was drafted for the sale.
    assert_eq!(done.invoice_id, Some(9));
    let drafts = seen.drafts.lock().unwrap().clone();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].source, sale.subject());
    assert_eq!(drafts[0].lines.len(), 1);

    t.app
        .get(&format!("/counter-sales/{}", sale.id))
        .await
        .assert_ok()
        .assert_see(&sale.number)
        .assert_see("AED 105.00");
}

#[renox::test]
async fn a_walk_in_sale_has_no_invoice() {
    let (t, seen) = boot().await;
    t.app.acting_as(&cashier(&t).await);
    let sale = open(&t, "").await;
    add_line(&t, &sale, "10.00").await;
    t.app
        .post(
            &format!("/counter-sales/{}/complete", sale.id),
            &[("cash", "10.50")],
        )
        .await
        .assert_status(303);
    let done = reload(&t, &sale).await;
    assert_eq!(done.status, CounterSaleStatus::Completed);
    assert_eq!(done.invoice_id, None);
    assert!(seen.drafts.lock().unwrap().is_empty());
    assert_eq!(seen.completed.lock().unwrap().len(), 1);
}

#[renox::test]
async fn tenders_that_do_not_add_up_and_an_empty_cart_are_refused() {
    let (t, seen) = boot().await;
    t.app.acting_as(&cashier(&t).await);
    let sale = open(&t, "").await;
    let url = format!("/counter-sales/{}/complete", sale.id);
    // No lines yet.
    t.app.post(&url, &[("cash", "10.00")]).await.assert_status(422);
    add_line(&t, &sale, "100.00").await;
    // Short, over and mixed amounts.
    t.app.post(&url, &[("cash", "100.00")]).await.assert_status(422);
    t.app.post(&url, &[("cash", "200.00")]).await.assert_status(422);
    t.app
        .post(&url, &[("cash", "50.00"), ("card", "55.00")])
        .await
        .assert_status(303);
    assert_eq!(reload(&t, &sale).await.status, CounterSaleStatus::Completed);
    assert_eq!(seen.completed.lock().unwrap().len(), 1);
}

#[renox::test]
async fn charging_the_customer_account_needs_the_setting_and_a_customer() {
    let (t, _) = boot().await;
    t.app.acting_as(&cashier(&t).await);
    let sale = open(&t, "7").await;
    add_line(&t, &sale, "100.00").await;
    let url = format!("/counter-sales/{}/complete", sale.id);
    // The default setting is off.
    t.app.post(&url, &[("account", "105.00")]).await.assert_status(422);
    assert_eq!(reload(&t, &sale).await.status, CounterSaleStatus::Open);

    let (t, _) = boot_with_sales(WlSales::new().counter_sales_on_account(true)).await;
    t.app.acting_as(&cashier(&t).await);
    let walk_in = open(&t, "").await;
    add_line(&t, &walk_in, "100.00").await;
    let url = format!("/counter-sales/{}/complete", walk_in.id);
    t.app.post(&url, &[("account", "105.00")]).await.assert_status(422);
    let named = open(&t, "7").await;
    add_line(&t, &named, "100.00").await;
    t.app
        .post(
            &format!("/counter-sales/{}/complete", named.id),
            &[("account", "105.00")],
        )
        .await
        .assert_status(303);
    assert_eq!(reload(&t, &named).await.status, CounterSaleStatus::Completed);
}

#[renox::test]
async fn a_completed_sale_is_never_edited_and_the_till_needs_pos_sell() {
    let (t, _) = boot().await;
    let cora = cashier(&t).await;
    t.app.acting_as(&cora);
    let sale = open(&t, "").await;
    add_line(&t, &sale, "10.00").await;
    t.app
        .post(
            &format!("/counter-sales/{}/complete", sale.id),
            &[("cash", "10.50")],
        )
        .await
        .assert_status(303);
    // SAL.ACL.04: no line after completion, no second completion.
    t.app
        .post(
            &format!("/counter-sales/{}/lines", sale.id),
            &[("item_code", "tee"), ("unit_price", "5.00")],
        )
        .await
        .assert_status(422);
    t.app
        .post(
            &format!("/counter-sales/{}/complete", sale.id),
            &[("cash", "10.50")],
        )
        .await
        .assert_status(422);
    assert_eq!(reload(&t, &sale).await.gross_total, 1_050);

    let nobody = t.user("Nina", &[]).await;
    t.app.acting_as(&nobody);
    t.app.get("/counter-sales").await.assert_forbidden();
    t.app.acting_as(&cora);
    t.app.get("/counter-sales").await.assert_ok().assert_see("RCPT-");
}
