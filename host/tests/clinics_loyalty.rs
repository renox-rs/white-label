//! BL-CLN-15 (#363): the clinic's loyalty programme (05 §7.2, `SAL.RUL.07`,
//! `SAL.RUL.08`, `SAL.ACL.07`) through the real host with real invoices, payer
//! bills, counter sales, one-time codes and the real wl-notify: a self-pay
//! invoice of AED 200 with an insurer share earns 20 points, 100 points are
//! redeemed with the code as an AED 10 line, the expiry and its reminder run
//! as the module runs them, and an adjustment of 600 points waits for a
//! manager. Only the SMS and e-mail providers are stand-ins.

mod support;

use host::roles::{ADMIN, CLINIC_MANAGER, RECEIVABLES, RECEPTIONIST};
use renox::chrono::{Duration, Months, NaiveDate};
use renox::db::DateTime;
use renox::prelude::*;
use renox::testing::TestApp;
use std::sync::{Arc, Mutex};
use support::{
    Wire, clinics_booted_with, company, draft_invoice, entity, person, tenant,
};
use wl_invoicing::payer_agreement::{PayerAgreement, VatRecipient};
use wl_kernel::approvals;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceId, VatCategory, invoicer};
use wl_kernel::notify::NotifyChannel;
use wl_kernel::payer_bills::{NewPayerBill, NewPayerBillLine, payer_bills};
use wl_kernel::{BusinessCalendar, PartyId, Subject};
use wl_notify::{Clock, Message, WlNotify};
use wl_sales::expiry::run_daily;
use wl_sales::{
    AdjustmentStatus, CounterSale, LoyaltyAccount, LoyaltyProgramme, PointsAdjustment, PointsKind,
    PointsMovement, RedemptionStatus, Redemption,
};

/// The clock of wl-notify: Dubai noon of the day the test sets.
#[derive(Clone)]
struct Moving(Arc<Mutex<DateTime>>);

impl Moving {
    fn day(&self, day: NaiveDate) {
        let local = day.and_hms_opt(12, 0, 0).expect("a time");
        *self.0.lock().unwrap() = BusinessCalendar::default().from_local(local);
    }
}

struct Rig(Moving);

impl Module for Rig {
    fn name(&self) -> &'static str {
        "clinics_loyalty_rig"
    }

    fn register(&self, app: &mut Registry) {
        let at = (self.0).0.clone();
        app.provide(Clock::from_fn(move || *at.lock().unwrap()));
    }
}

struct World {
    app: TestApp,
    sms: Wire,
    mail: Wire,
    clock: Moving,
}

async fn world() -> World {
    let sms = Wire::new(NotifyChannel::Sms);
    let mail = Wire::new(NotifyChannel::Email);
    let clock = Moving(Arc::new(Mutex::new(renox::db::now())));
    clock.day(today());
    let (s, m) = (sms.clone(), mail.clone());
    let app = clinics_booted_with(move |n: WlNotify| n.provider(s).provider(m), Rig(clock.clone()))
        .await;
    World {
        app,
        sms,
        mail,
        clock,
    }
}

async fn account(w: &World, party: PartyId) -> LoyaltyAccount {
    LoyaltyAccount::of_party(w.app.db(), party)
        .await
        .unwrap()
        .expect("the member is enrolled")
}

/// AED text of `fils`, as a form field.
fn aed(fils: i64) -> String {
    format!("{}.{:02}", fils / 100, fils % 100)
}

/// A payer agreement of the clinic's settings, and a draft claim of `claimed`
/// fils for `visit` of `patient`.
async fn claim(app: &TestApp, patient: PartyId, visit: i64, claimed: i64) {
    let payer = company(app, "Daman Insurance").await;
    let legal = entity(app, "PGC").await;
    let agreement = PayerAgreement::create(
        app.state(),
        PayerAgreement {
            legal_entity_id: legal.id,
            payer_party_id: payer.0,
            name: "Daman Gold".to_owned(),
            active: true,
            vat_recipient: VatRecipient::Customer,
            first_submission_days: 30,
            resubmission_days: 30,
            resubmission_limit: 2,
            ..PayerAgreement::default()
        },
        None,
    )
    .await
    .unwrap();
    let request = NewPayerBill::new(
        agreement.id,
        patient,
        Subject::new("clinics.visit", visit),
        today(),
    )
    .void_before_first_submission()
    .line(NewPayerBillLine::new("Consultation x 1", claimed));
    payer_bills(app.state())
        .create(app.state(), request, None)
        .await
        .expect("the payer bill is made");
}

/// Done when (loyalty), 1 to 4: a self-pay invoice of AED 200 with the
/// insurer's AED 800 earns 20 points; 100 points are redeemed with the code
/// sent to the patient's mobile as an AED 10 line; the points expire 12
/// months after they were earned with a reminder 30 days before; an
/// adjustment of 600 points waits for a manager who is not the requester.
#[renox::test]
async fn the_loyalty_programme_earns_redeems_expires_and_asks_a_manager_above_500() {
    let w = world().await;
    let app = &w.app;
    let programme = LoyaltyProgramme::first_active(app.db())
        .await
        .unwrap()
        .expect("the clinic starts with its programme");
    let rita = person(app, "Rita", RECEPTIONIST).await;
    let mona = person(app, "Mona", CLINIC_MANAGER).await;
    let ada = person(app, "Ada", ADMIN).await;
    let cara = person(app, "Cara", RECEIVABLES).await;
    let pat = tenant(app, "Pat Lane").await;

    // The desk enrols the patient with her mobile.
    app.acting_as(&rita);
    app.htmx()
        .post(
            "/loyalty/accounts",
            &[
                ("party_id", pat.0.to_string().as_str()),
                ("mobile", "0501234567"),
                ("terms", "1"),
            ],
        )
        .await
        .assert_status(303);
    let member = account(&w, pat).await;

    // A visit of AED 1,000: the insurer is billed AED 800, the patient's
    // invoice is paid; only the patient's AED 200 earns, 20 points.
    let invoice = draft_invoice(
        app,
        pat,
        Subject::new("clinics.visit", 5),
        100_000,
        VatCategory::ZeroRated,
        today(),
    )
    .await;
    app.acting_as(&cara);
    app.htmx()
        .post(&format!("/invoices/{}/issue", invoice.id), &[])
        .await
        .assert_redirect(&format!("/invoices/{}", invoice.id));
    claim(app, pat, 5, 80_000).await;
    invoicer(app.state())
        .allocate(
            app.state(),
            InvoiceId(invoice.id),
            Subject::new("payments.payment", 1),
            100_000,
        )
        .await
        .expect("the payment is allocated");
    assert_eq!(account(&w, pat).await.points_balance, 20);
    assert_eq!(programme.points_for(20_000), 20);

    // More visits to reach 100 points: a counter sale of AED 800 earns 80.
    app.acting_as(&rita);
    let first = ring_up(app, &member, "Skin care", 80_000).await;
    assert_eq!(account(&w, pat).await.points_balance, 100);
    assert_eq!(first.net_total, 80_000);

    // 100 points are an AED 10 discount line, confirmed by the code the
    // patient is sent.
    let sale = open_cart(app, &member, "Check-up", 4_000).await;
    app.htmx()
        .post(
            &format!("/counter-sales/{}/redeem", sale.id),
            &[("points", "100")],
        )
        .await
        .assert_status(303);
    app.run_all_jobs().await;
    let sms = w.sms.sent().pop().expect("the code was sent");
    assert_eq!(sms.address, "+971501234567");
    let code: String = sms.body.chars().take_while(char::is_ascii_digit).collect();
    assert_eq!(code.len(), 6);
    app.htmx()
        .post(
            &format!("/counter-sales/{}/redeem/confirm", sale.id),
            &[("code", code.as_str())],
        )
        .await
        .assert_status(303);
    let redemption = Redemption::of_sale(app.db(), &sale.subject())
        .await
        .unwrap()
        .remove(0);
    assert_eq!(redemption.status, RedemptionStatus::Held);
    let held = CounterSale::find_or_404(app.db(), sale.id).await.unwrap();
    app.htmx()
        .post(
            &format!("/counter-sales/{}/complete", sale.id),
            &[("cash", aed(held.gross_total).as_str())],
        )
        .await
        .assert_status(303);
    let after = account(&w, pat).await;
    let redeemed: i64 = PointsMovement::of_account(app.db(), after.id)
        .await
        .unwrap()
        .iter()
        .filter(|m| m.kind == PointsKind::Redeem)
        .map(|m| m.points)
        .sum();
    assert_eq!(redeemed, -100);
    // AED 40 before VAT is AED 42; the AED 10 line leaves AED 32 to pay (the
    // VAT of the discounted supply is open item A-15).
    assert_eq!(held.gross_total, 3_200);

    // The points earned today expire 12 months later, reminded 30 days
    // before (the module's own daily run).
    let expiry = today().checked_add_months(Months::new(12)).unwrap();
    let at = |day: NaiveDate| -> DateTime {
        BusinessCalendar::default().from_local(day.and_hms_opt(2, 0, 0).unwrap())
    };
    let day = expiry - Duration::days(31);
    w.clock.day(day);
    let early = run_daily(app.state(), day, at(day)).await.unwrap();
    assert_eq!(early.reminded, 0, "31 days before: too early");
    let day = expiry - Duration::days(30);
    w.clock.day(day);
    let reminder = run_daily(app.state(), day, at(day)).await.unwrap();
    assert_eq!(reminder.reminded, 1, "30 days before: the reminder");
    app.run_all_jobs().await;
    let told = Message::query()
        .where_(Message::TYPE_KEY.eq(wl_sales::notices::LOYALTY_EXPIRY))
        .count(app.db())
        .await
        .unwrap();
    assert_eq!(told, 1);
    assert!(!w.mail.sent().is_empty(), "the reminder reached the mailbox");
    w.clock.day(expiry);
    let last = run_daily(app.state(), expiry, at(expiry)).await.unwrap();
    assert_eq!(last.expired_points, 0, "still redeemable on the last day");
    let next = expiry + Duration::days(1);
    let gone = run_daily(app.state(), next, at(next)).await.unwrap();
    assert_eq!(gone.expired_points, after.points_balance);
    assert_eq!(account(&w, pat).await.points_balance, 0);

    // 600 points added by hand wait for a manager: the requester cannot
    // approve it, another holder of `loyalty.manage` can.
    app.acting_as(&mona);
    app.request()
        .post(
            &format!("/loyalty/accounts/{}/adjust", member.id),
            &[("points", "600"), ("reason", "Goodwill after a complaint")],
        )
        .await
        .assert_status(303);
    let adjustment = PointsAdjustment::of_account(app.db(), member.id)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(adjustment.status, AdjustmentStatus::PendingApproval);
    assert_eq!(account(&w, pat).await.points_balance, 0);
    let request = adjustment.approval_request_id.expect("an approval request");
    assert!(
        approvals::approve(app.state(), request, &mona, "")
            .await
            .is_err()
    );
    approvals::approve(app.state(), request, &ada, "agreed")
        .await
        .unwrap();
    assert_eq!(account(&w, pat).await.points_balance, 600);
}

/// A counter sale of `price` fils before VAT (one line), the member attached;
/// it stays open.
async fn open_cart(app: &TestApp, member: &LoyaltyAccount, item: &str, price: i64) -> CounterSale {
    app.htmx()
        .post("/counter-sales", &[("x", "")])
        .await
        .assert_status(303);
    let sale = CounterSale::query()
        .order_by_desc(CounterSale::ID)
        .first(app.db())
        .await
        .unwrap()
        .expect("the cart was opened");
    app.htmx()
        .post(
            &format!("/counter-sales/{}/lines", sale.id),
            &[
                ("item_code", item),
                ("unit_price", aed(price).as_str()),
                ("quantity", "1"),
            ],
        )
        .await
        .assert_status(303);
    app.htmx()
        .post(
            &format!("/counter-sales/{}/member", sale.id),
            &[("code", member.code.as_str())],
        )
        .await
        .assert_status(303);
    CounterSale::find_or_404(app.db(), sale.id).await.unwrap()
}

/// Rings up and completes a counter sale paid in cash.
async fn ring_up(
    app: &TestApp,
    member: &LoyaltyAccount,
    item: &str,
    price: i64,
) -> CounterSale {
    let sale = open_cart(app, member, item, price).await;
    app.htmx()
        .post(
            &format!("/counter-sales/{}/complete", sale.id),
            &[("cash", aed(sale.gross_total).as_str())],
        )
        .await
        .assert_status(303);
    CounterSale::find_or_404(app.db(), sale.id).await.unwrap()
}
