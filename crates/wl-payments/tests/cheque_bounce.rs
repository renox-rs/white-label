//! BL-PAY-09 (#102): a bounced cheque is recorded with the bank's return
//! reason and certificate and the part the bank paid; the honoured part is a
//! payment allocated to the invoice, the rest is the open amount, a bounce fee
//! charge is added when configured and `ChequeBounced` is emitted. An issued
//! cheque that bounces leaves the accountant a task to replace it.

mod support;

use renox::chrono::TimeDelta;
use wl_kernel::cheques::{ChequeId, ChequeStatus, RegisterCheque};
use wl_kernel::documents::today;
use wl_kernel::invoicing::VatCategory;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::payments::{Direction, PaymentMethod, PaymentSource};
use wl_kernel::Subject;
use wl_payments::cheque::ChequeReturn;
use wl_payments::followup::{FollowupKind, FollowupStatus};
use wl_payments::gateway::FakeGateway;
use wl_payments::notices::{CHEQUE_BOUNCED, CHEQUE_TASK};
use wl_payments::{Cheque, FollowupTask, Payment, WlPayments};

use support::{BLUE_DUNES, Booted, DUSK_HOLDINGS, boot, boot_with};

fn lease() -> Subject {
    Subject::new("property.lease", 9)
}

async fn cheque_payments(t: &Booted) -> Vec<Payment> {
    Payment::query()
        .where_(Payment::SOURCE.eq(PaymentSource::Cheque))
        .order_by(Payment::ID)
        .get(t.db())
        .await
        .unwrap()
}

/// A deposited received cheque of `amount` fils paying `rent`.
async fn deposited(
    t: &Booted,
    staff: &renox::auth::User,
    number: &str,
    amount: i64,
    rent: wl_kernel::invoicing::InvoiceId,
) -> Cheque {
    let cheque = t.received_cheque(BLUE_DUNES, number, amount, Some(rent)).await;
    t.deposit_cheque(staff, &cheque).await
}

/// AC1: AED 20,000.00 for rent, bounced for "insufficient funds" with the
/// certificate and AED 6,000.00 honoured: `bounced`, a payment of AED 6,000.00
/// allocated to the rent invoice, AED 14,000.00 open, `ChequeBounced`.
#[renox::test]
async fn a_bounce_with_a_honoured_part_gives_a_payment_the_open_amount_and_the_event() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    let accountant = t.accountant("Cara").await;
    let rent = t.invoices.add(BLUE_DUNES, "INV-1", 2_000_000);
    let cheque = deposited(&t, &staff, "1042", 2_000_000, rent).await;
    assert_eq!(cheque.status, ChequeStatus::Deposited);

    t.bounce_cheque(&staff, &cheque, "insufficient_funds", "CERT-77", "6000.00")
        .await
        .assert_ok();

    let bounced = t.cheque(cheque.id).await;
    assert_eq!(bounced.status, ChequeStatus::Bounced);
    assert_eq!(bounced.honoured_amount, 600_000);
    assert_eq!(bounced.open_amount, 1_400_000);
    assert_eq!(bounced.return_reason.as_deref(), Some("insufficient funds"));
    assert_eq!(bounced.return_certificate.as_deref(), Some("CERT-77"));
    assert_eq!(bounced.bounced_on, Some(today()));
    assert_eq!(bounced.presentations, 1);

    // The honoured part is a payment of the cheque, allocated to the rent.
    let payments = cheque_payments(&t).await;
    assert_eq!(payments.len(), 1);
    let payment = &payments[0];
    assert_eq!(payment.amount, 600_000);
    assert_eq!(payment.method, PaymentMethod::Cheque);
    assert_eq!(payment.direction, Direction::In);
    assert_eq!(payment.party_id, BLUE_DUNES.0);
    let allocations = t.allocations(payment).await;
    assert_eq!(allocations.len(), 1);
    assert_eq!(allocations[0].target().to_string(), rent.subject().to_string());
    assert_eq!(allocations[0].amount, 600_000);
    let calls = t.invoices.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!((calls[0].invoice, calls[0].amount), (rent, 600_000));
    assert_eq!(t.received.all().len(), 1, "the honoured part is a payment received");

    // The return is kept, with its payment.
    let returns = ChequeReturn::of_cheque(t.db(), cheque.id).await.unwrap();
    assert_eq!(returns.len(), 1);
    assert_eq!(returns[0].presentation, 1);
    assert_eq!(returns[0].honoured_amount, 600_000);
    assert_eq!(returns[0].honoured_payment_id, Some(payment.id));
    assert_eq!(returns[0].certificate.as_deref(), Some("CERT-77"));

    // ChequeBounced, for wl-sales.
    let events = t.cheque_bounced.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].cheque, ChequeId(cheque.id));
    assert_eq!(events[0].party, BLUE_DUNES);
    assert_eq!(
        (events[0].amount, events[0].honoured_amount, events[0].open_amount),
        (2_000_000, 600_000, 1_400_000)
    );
    assert_eq!(events[0].reason, "insufficient funds");
    assert_eq!(events[0].presentation, 1);
    assert_eq!(events[0].linked, vec![rent.subject()]);

    // FIN.RUL.04: the party is told, the accountant gets a task.
    let told = t.notices.of_type(CHEQUE_BOUNCED);
    assert_eq!(told.len(), 1);
    assert!(matches!(
        told[0].recipients[0],
        NotifyRecipient::Party { id } if id == BLUE_DUNES
    ));
    let data = told[0].data.to_string();
    assert!(data.contains("1042") && data.contains("insufficient funds"), "{data}");
    assert!(data.contains("AED 14,000.00"), "{data}");
    let tasks = FollowupTask::open_of_cheque(t.db(), cheque.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].kind, FollowupKind::FollowUpCheque);
    assert_eq!(tasks[0].role, "accountant");
    assert_eq!(tasks[0].amount, 1_400_000);
    let staff_told = t.notices.of_type(CHEQUE_TASK);
    assert_eq!(staff_told.len(), 1);
    assert!(staff_told[0].recipients.iter().any(
        |r| matches!(r, NotifyRecipient::User { id } if *id == accountant.id)
    ));

    // What the party owes on bounced cheques.
    let owed = wl_kernel::cheques::cheques(t.state())
        .open_bounced_amount(t.db(), BLUE_DUNES, "AED")
        .await
        .unwrap();
    assert_eq!(owed, 1_400_000);
}

/// A bounce with nothing honoured records no payment and leaves the whole
/// amount open.
#[renox::test]
async fn a_bounce_with_nothing_honoured_leaves_the_whole_amount_open() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    let rent = t.invoices.add(BLUE_DUNES, "INV-1", 2_000_000);
    let cheque = deposited(&t, &staff, "1043", 2_000_000, rent).await;
    t.bounce_cheque(&staff, &cheque, "account_closed", "CERT-78", "")
        .await
        .assert_ok();
    let bounced = t.cheque(cheque.id).await;
    assert_eq!(bounced.status, ChequeStatus::Bounced);
    assert_eq!((bounced.honoured_amount, bounced.open_amount), (0, 2_000_000));
    assert!(cheque_payments(&t).await.is_empty());
    assert!(t.invoices.calls().is_empty());
    assert_eq!(t.cheque_bounced.all().len(), 1);
}

/// AC2: with a fee charge type of AED 500.00 set, a charge of AED 500.00 plus
/// standard VAT is added, due on the bounce date, on the agreement the cheque
/// is linked to.
#[renox::test]
async fn a_bounce_fee_charge_is_added_on_the_agreement() {
    let t = boot_with(
        WlPayments::new().bounce_fee("bounce_fee", 50_000),
        FakeGateway::new(),
    )
    .await;
    let staff = t.cheque_staff("Amal").await;
    let rent = t.invoices.add(BLUE_DUNES, "INV-1", 2_000_000);
    let entity = t.entity("PGP").await;
    let cheque = t
        .register_cheque(
            RegisterCheque::new(BLUE_DUNES, entity.id, "ENBD", "1044", today(), 2_000_000)
                .link(rent.subject())
                .agreement(lease()),
        )
        .await;
    let cheque = t.deposit_cheque(&staff, &cheque).await;
    t.bounce_cheque(&staff, &cheque, "insufficient_funds", "CERT-79", "6000")
        .await
        .assert_ok();

    let charges = t.fees.charges();
    assert_eq!(charges.len(), 1);
    assert_eq!(charges[0].cheque, ChequeId(cheque.id));
    assert_eq!(charges[0].party, BLUE_DUNES);
    assert_eq!(charges[0].charge_type, "bounce_fee");
    assert_eq!(charges[0].amount, 50_000);
    assert_eq!(charges[0].vat_category, VatCategory::Standard);
    assert_eq!(charges[0].due, today());
    assert_eq!(charges[0].agreement, Some(lease()));
    let bounced = t.cheque(cheque.id).await;
    assert_eq!(bounced.bounce_fee_total, 50_000);
    let returns = ChequeReturn::of_cheque(t.db(), cheque.id).await.unwrap();
    assert_eq!(returns[0].fee_amount, 50_000);
    assert_eq!(
        returns[0].fee_charge().map(|c| c.to_string()).as_deref(),
        Some("invoicing.charge:1")
    );
    // The charge was made: no task for it.
    let tasks = FollowupTask::open_of_cheque(t.db(), cheque.id).await.unwrap();
    assert!(tasks.iter().all(|t| t.kind != FollowupKind::RaiseBounceFee));
}

/// Without an agreement the fee is a standalone charge for the party.
#[renox::test]
async fn a_cheque_without_an_agreement_gets_a_standalone_fee_charge() {
    let t = boot_with(
        WlPayments::new().bounce_fee("bounce_fee", 50_000),
        FakeGateway::new(),
    )
    .await;
    let staff = t.cheque_staff("Amal").await;
    let rent = t.invoices.add(BLUE_DUNES, "INV-1", 2_000_000);
    let cheque = deposited(&t, &staff, "1045", 2_000_000, rent).await;
    t.bounce_cheque(&staff, &cheque, "insufficient_funds", "CERT-80", "")
        .await
        .assert_ok();
    let charges = t.fees.charges();
    assert_eq!(charges.len(), 1);
    assert_eq!(charges[0].agreement, None);
    assert_eq!(charges[0].party, BLUE_DUNES);
}

/// Without a fee setting no charge is asked for; when invoicing cannot
/// schedule the charge, the accountant gets a task to raise it.
#[renox::test]
async fn no_fee_is_added_unless_set_and_a_charge_that_cannot_be_made_becomes_a_task() {
    let plain = boot().await;
    let staff = plain.cheque_staff("Amal").await;
    let rent = plain.invoices.add(BLUE_DUNES, "INV-1", 2_000_000);
    let cheque = deposited(&plain, &staff, "1046", 2_000_000, rent).await;
    plain
        .bounce_cheque(&staff, &cheque, "insufficient_funds", "C", "")
        .await
        .assert_ok();
    assert!(plain.fees.charges().is_empty());
    assert_eq!(plain.cheque(cheque.id).await.bounce_fee_total, 0);

    let t = boot_with(
        WlPayments::new().bounce_fee("bounce_fee", 50_000),
        FakeGateway::new(),
    )
    .await;
    t.fees.make_charges(false);
    let staff = t.cheque_staff("Amal").await;
    t.accountant("Cara").await;
    let rent = t.invoices.add(BLUE_DUNES, "INV-1", 2_000_000);
    let cheque = deposited(&t, &staff, "1047", 2_000_000, rent).await;
    t.bounce_cheque(&staff, &cheque, "insufficient_funds", "C", "")
        .await
        .assert_ok();
    let tasks = FollowupTask::open_of_cheque(t.db(), cheque.id).await.unwrap();
    let fee_task = tasks
        .iter()
        .find(|task| task.kind == FollowupKind::RaiseBounceFee)
        .expect("a task to raise the fee");
    assert_eq!(fee_task.amount, 50_000);
    assert!(fee_task.text.contains("AED 500.00"));
    let returns = ChequeReturn::of_cheque(t.db(), cheque.id).await.unwrap();
    assert_eq!(returns[0].fee_charge(), None);
    assert_eq!(returns[0].fee_amount, 50_000);
}

/// AC3: a received cheque cannot bounce without its certificate; nor without a
/// reason; nothing is written.
#[renox::test]
async fn a_received_cheque_cannot_bounce_without_the_certificate() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    let rent = t.invoices.add(BLUE_DUNES, "INV-1", 2_000_000);
    let cheque = deposited(&t, &staff, "1048", 2_000_000, rent).await;

    t.bounce_cheque(&staff, &cheque, "insufficient_funds", "", "6000")
        .await
        .assert_status(422);
    t.bounce_cheque(&staff, &cheque, "insufficient_funds", "   ", "")
        .await
        .assert_status(422);
    t.bounce_cheque(&staff, &cheque, "", "CERT-1", "")
        .await
        .assert_status(422);
    t.bounce_cheque(&staff, &cheque, "no_such_reason", "CERT-1", "")
        .await
        .assert_status(422);
    let still = t.cheque(cheque.id).await;
    assert_eq!(still.status, ChequeStatus::Deposited);
    assert_eq!(still.honoured_amount, 0);
    assert!(cheque_payments(&t).await.is_empty());
    assert!(t.cheque_bounced.all().is_empty());
    assert!(ChequeReturn::of_cheque(t.db(), cheque.id).await.unwrap().is_empty());
}

/// A honoured amount of the whole cheque is refused (that is `clear`), and so
/// is one that is not an amount.
#[renox::test]
async fn a_honoured_amount_must_stay_below_the_cheque() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    let rent = t.invoices.add(BLUE_DUNES, "INV-1", 2_000_000);
    let cheque = deposited(&t, &staff, "1049", 2_000_000, rent).await;
    t.bounce_cheque(&staff, &cheque, "insufficient_funds", "C", "20000")
        .await
        .assert_status(422);
    t.bounce_cheque(&staff, &cheque, "insufficient_funds", "C", "25000.50")
        .await
        .assert_status(422);
    t.bounce_cheque(&staff, &cheque, "insufficient_funds", "C", "plenty")
        .await
        .assert_status(422);
    assert_eq!(t.cheque(cheque.id).await.status, ChequeStatus::Deposited);
    // Just below is fine.
    t.bounce_cheque(&staff, &cheque, "insufficient_funds", "C", "19999.99")
        .await
        .assert_ok();
    assert_eq!(t.cheque(cheque.id).await.open_amount, 1);
}

/// Only a deposited cheque bounces, and only a user with `cheques.deposit`.
#[renox::test]
async fn only_a_deposited_cheque_bounces_and_only_for_a_user_with_the_right() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    let viewer = t
        .user("Vera", &[wl_payments::permissions::CHEQUES_VIEW])
        .await;
    let rent = t.invoices.add(BLUE_DUNES, "INV-1", 2_000_000);
    let held = t.received_cheque(BLUE_DUNES, "1050", 2_000_000, Some(rent)).await;
    t.bounce_cheque(&staff, &held, "insufficient_funds", "C", "")
        .await
        .assert_status(422);
    let cheque = t.deposit_cheque(&staff, &held).await;
    t.bounce_cheque(&viewer, &cheque, "insufficient_funds", "C", "")
        .await
        .assert_forbidden();
    assert_eq!(t.cheque(cheque.id).await.status, ChequeStatus::Deposited);
    t.bounce_cheque(&staff, &cheque, "insufficient_funds", "C", "")
        .await
        .assert_ok();
    // Not twice.
    t.bounce_cheque(&staff, &cheque, "insufficient_funds", "C", "")
        .await
        .assert_status(422);
    assert_eq!(t.cheque_bounced.all().len(), 1);
}

/// AC5: a deposited issued cheque of AED 10,500.00 for a supplier bill
/// bounces with a reason and no certificate: `bounced`, AED 10,500.00 open, no
/// fee, no `ChequeBounced`, a task to replace it; the bill is not touched.
#[renox::test]
async fn an_issued_cheque_that_bounces_leaves_a_task_to_replace_it() {
    let t = boot_with(
        WlPayments::new().bounce_fee("bounce_fee", 50_000),
        FakeGateway::new(),
    )
    .await;
    let staff = t.cheque_staff("Amal").await;
    let accountant = t.accountant("Cara").await;
    let entity = t.entity("PGP").await;
    let bill = Subject::new("invoicing.supplier_bill", 4);
    let cheque = t
        .register_cheque(
            RegisterCheque::new(BLUE_DUNES, entity.id, "ENBD", "9001", today(), 1_050_000)
                .issued()
                .link(bill),
        )
        .await;
    let cheque = t.deposit_cheque(&staff, &cheque).await;
    // No certificate is asked of an issued cheque.
    t.bounce_cheque(&staff, &cheque, "insufficient_funds", "", "")
        .await
        .assert_ok();

    let bounced = t.cheque(cheque.id).await;
    assert_eq!(bounced.status, ChequeStatus::Bounced);
    assert_eq!(bounced.open_amount, 1_050_000);
    assert_eq!(bounced.return_certificate, None);
    assert!(t.fees.charges().is_empty(), "no fee");
    assert!(t.cheque_bounced.all().is_empty(), "no ChequeBounced");
    assert!(t.notices.of_type(CHEQUE_BOUNCED).is_empty(), "the payee is not told");
    assert!(cheque_payments(&t).await.is_empty());
    let tasks = FollowupTask::open_of_cheque(t.db(), cheque.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].kind, FollowupKind::ReplaceCheque);
    assert_eq!(tasks[0].role, "accountant");
    assert_eq!(tasks[0].status, FollowupStatus::Open);
    assert_eq!(tasks[0].amount, 1_050_000);
    assert!(tasks[0].text.contains("9001"));
    let staff_told = t.notices.of_type(CHEQUE_TASK);
    assert_eq!(staff_told.len(), 1);
    assert!(staff_told[0].recipients.iter().any(
        |r| matches!(r, NotifyRecipient::User { id } if *id == accountant.id)
    ));
    // An issued cheque is not owed by the party.
    let owed = wl_kernel::cheques::cheques(t.state())
        .open_bounced_amount(t.db(), BLUE_DUNES, "AED")
        .await
        .unwrap();
    assert_eq!(owed, 0);
}

/// A blocked party is not told that its cheque bounced; the staff still are.
#[renox::test]
async fn a_blocked_party_is_not_told_of_the_bounce() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    t.accountant("Cara").await;
    let entity = t.entity("PGP").await;
    let cheque = t
        .register_cheque(RegisterCheque::new(
            DUSK_HOLDINGS,
            entity.id,
            "ENBD",
            "1051",
            today() - TimeDelta::days(1),
            500_000,
        ))
        .await;
    let cheque = t.deposit_cheque(&staff, &cheque).await;
    t.bounce_cheque(&staff, &cheque, "insufficient_funds", "C", "")
        .await
        .assert_ok();
    assert!(t.notices.of_type(CHEQUE_BOUNCED).is_empty());
    assert_eq!(t.notices.of_type(CHEQUE_TASK).len(), 1);
    assert_eq!(t.cheque_bounced.all().len(), 1, "the event is still emitted");
}
