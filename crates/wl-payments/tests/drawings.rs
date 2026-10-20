//! Owner drawings (wl-payments §3 *Payment* purpose, §4 `record`, §5.3 step 5,
//! `FIN.ACL.21`, `FIN.ACL.23`): the acceptance checks of #119. wl-invoicing is
//! not part of this app: the approval is the test's `DrawingsDesk`, and a
//! decision is the kernel event `OwnerDrawingDecided` the test emits. The bank
//! statement line marked as a drawing is tested at the end of this file.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::bank_statements::bank_statements;
use wl_kernel::cheques::RegisterCheque;
use wl_kernel::documents::today;
use wl_kernel::drawings::{
    DRAWING_REQUEST_SUBJECT_TYPE, DrawingPaymentSource, OwnerDrawingDecided,
};
use wl_kernel::payments::{Direction, PaymentStatus};
use wl_kernel::{LegalEntity, Subject};
use wl_payments::allocate::allocate;
use wl_payments::bank_statement::{BankStatementImport, ImportStatus};
use wl_payments::bank_statement_line::{BankStatementLine, LineStatus};
use wl_payments::drawings::{DrawingStatus, PaymentDrawing, PaymentDrawings, record_approved};
use wl_payments::expense_category::{self, ExpenseCategory, VatTreatment};
use wl_payments::notices::BANK_LINE_DRAWING_REJECTED;
use wl_payments::{Payment, permissions as perm};

use support::{BLUE_DUNES, BLUE_DUNES_CODE, Booted, QUIET_SANDS, QUIET_SANDS_CODE, boot};

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2027, m, d).unwrap()
}

/// `POST /payments` as `user`: an owner drawing (`out`) or repayment (`in`) of
/// `amount` dirhams by transfer to or from the party `code`, dated `date`.
async fn post_drawing(
    t: &Booted,
    user: &User,
    entity: &LegalEntity,
    code: &str,
    direction: &str,
    amount: &str,
    date: &str,
) -> renox::testing::TestResponse {
    let entity = entity.id.to_string();
    t.app.acting_as(user);
    t.app
        .htmx()
        .post(
            "/payments",
            &[
                ("legal_entity_id", entity.as_str()),
                ("party_code", code),
                ("method", "transfer"),
                ("amount", amount),
                ("reference", "TRX-DRAW"),
                ("payment_date", date),
                ("purpose", "owner_drawings"),
                ("drawing_category", "owner_drawings"),
                ("drawing_direction", direction),
            ],
        )
        .await
}

/// The drawing requests, oldest first.
async fn drawings(t: &Booted) -> Vec<PaymentDrawing> {
    PaymentDrawing::query()
        .order_by(PaymentDrawing::ID)
        .get(t.db())
        .await
        .unwrap()
}

/// Every payment, oldest first.
async fn payments(t: &Booted) -> Vec<Payment> {
    Payment::query()
        .order_by(Payment::ID)
        .get(t.db())
        .await
        .unwrap()
}

/// The approvers decide the request `drawing`: `approver` says yes, or
/// nobody does and `reason` says why.
async fn decide(
    t: &Booted,
    drawing: &PaymentDrawing,
    approver: Option<&User>,
    reason: Option<&str>,
) {
    t.state()
        .emit(OwnerDrawingDecided {
            source: Subject::new(DRAWING_REQUEST_SUBJECT_TYPE, drawing.id),
            legal_entity_id: drawing.legal_entity_id,
            approved: approver.is_some(),
            approved_by: approver.map(|u| u.id),
            reason: reason.map(str::to_owned),
        })
        .await
        .expect("the decision is handled");
}

/// Accountant A saves an outgoing transfer of AED 15,000 to owner party O with
/// purpose *owner drawings*: no payment exists and the approval is asked for.
/// A cannot approve it; owner B approves it and the payment is recorded
/// `received` with the details A entered; `PaymentReceived`.
#[renox::test]
async fn a_drawing_is_recorded_only_after_the_final_approval() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let pgp = t.entity("PGP").await;
    post_drawing(
        &t,
        &a,
        &pgp,
        QUIET_SANDS_CODE,
        "out",
        "15000.00",
        "2027-03-10",
    )
    .await
    .assert_status(303);

    assert!(
        payments(&t).await.is_empty(),
        "nothing is recorded before the approval"
    );
    let saved = drawings(&t).await;
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].status, DrawingStatus::Requested);
    let asked = t.drawings.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!((asked[0].amount, asked[0].requested_by), (1_500_000, a.id));
    assert_eq!(asked[0].category, "owner_drawings");
    assert_eq!(
        asked[0].source,
        Subject::new(DRAWING_REQUEST_SUBJECT_TYPE, saved[0].id)
    );
    assert!(t.received.all().is_empty());

    decide(&t, &saved[0], Some(&b), None).await;
    let recorded = payments(&t).await;
    assert_eq!(recorded.len(), 1);
    let payment = &recorded[0];
    assert_eq!(payment.status, PaymentStatus::Received);
    assert_eq!(payment.direction, Direction::Out);
    assert_eq!(
        (payment.amount, payment.party_id),
        (1_500_000, QUIET_SANDS.0)
    );
    assert_eq!(payment.purpose, "owner_drawings");
    assert_eq!(payment.drawing_category.as_deref(), Some("owner_drawings"));
    assert_eq!(payment.payment_date, day(3, 10));
    assert_eq!(payment.reference.as_deref(), Some("TRX-DRAW"));
    assert_eq!(payment.recorded_by, Some(a.id));
    assert!(!payment.self_approved);
    assert!(payment.number.is_some(), "it has its voucher number");
    assert_eq!(t.received.all().len(), 1, "PaymentReceived");
    let done = drawings(&t).await;
    assert_eq!(
        (done[0].status, done[0].payment_id),
        (DrawingStatus::Recorded, Some(payment.id))
    );
    assert_eq!(done[0].approved_by, Some(b.id));

    // A repeated decision records nothing twice.
    decide(&t, &done[0], Some(&b), None).await;
    assert_eq!(payments(&t).await.len(), 1);
}

/// B rejects another request, or it is withdrawn or expires: no payment is
/// recorded and the request keeps the reason.
#[renox::test]
async fn a_rejection_records_nothing() {
    let t = boot().await;
    let a = t.user_a().await;
    let pgp = t.entity("PGP").await;
    post_drawing(
        &t,
        &a,
        &pgp,
        QUIET_SANDS_CODE,
        "out",
        "2000.00",
        "2027-03-11",
    )
    .await
    .assert_status(303);
    let saved = drawings(&t).await;
    decide(&t, &saved[0], None, Some("not this month")).await;
    assert!(payments(&t).await.is_empty());
    let after = drawings(&t).await;
    assert_eq!(after[0].status, DrawingStatus::Rejected);
    assert_eq!(after[0].reason.as_deref(), Some("not this month"));
    assert!(t.received.all().is_empty());
}

/// The entity's only holder of `drawings.approve` records their own drawing at
/// once, flagged *self-approved* (`FIN.ACL.23`).
#[renox::test]
async fn a_sole_owner_drawing_is_recorded_at_once_and_flagged() {
    let t = boot().await;
    let a = t.user_a().await;
    let pgp = t.entity("PGP").await;
    t.drawings.approves_at_once(true);
    post_drawing(
        &t,
        &a,
        &pgp,
        QUIET_SANDS_CODE,
        "out",
        "900.00",
        "2027-03-12",
    )
    .await
    .assert_status(303);
    let recorded = payments(&t).await;
    assert_eq!(recorded.len(), 1);
    assert!(recorded[0].self_approved);
    assert_eq!(recorded[0].purpose, "owner_drawings");
    assert_eq!(drawings(&t).await[0].status, DrawingStatus::Recorded);
}

/// An outgoing drawing to supplier party S (not the owner party) is refused,
/// and so is a category with no owner party; nothing is saved or asked.
#[renox::test]
async fn a_drawing_to_another_party_is_refused() {
    let t = boot().await;
    let a = t.user_a().await;
    let pgp = t.entity("PGP").await;
    post_drawing(&t, &a, &pgp, BLUE_DUNES_CODE, "out", "100.00", "2027-03-12")
        .await
        .assert_status(422);
    assert!(drawings(&t).await.is_empty());
    assert!(t.drawings.asked().is_empty());
    assert!(payments(&t).await.is_empty());

    t.drawings.owner(None);
    post_drawing(
        &t,
        &a,
        &pgp,
        QUIET_SANDS_CODE,
        "out",
        "100.00",
        "2027-03-12",
    )
    .await
    .assert_status(422);
    assert!(drawings(&t).await.is_empty());

    // Not a drawings category.
    t.drawings.owner(Some(QUIET_SANDS));
    let entity = pgp.id.to_string();
    t.app.acting_as(&a);
    t.app
        .htmx()
        .post(
            "/payments",
            &[
                ("legal_entity_id", entity.as_str()),
                ("party_code", QUIET_SANDS_CODE),
                ("method", "transfer"),
                ("amount", "100.00"),
                ("reference", "TRX"),
                ("purpose", "owner_drawings"),
                ("drawing_category", "marketing"),
            ],
        )
        .await
        .assert_status(422);
    assert!(drawings(&t).await.is_empty());
}

/// An incoming payment of AED 5,000 from O dated 2027-03-20 with purpose *owner
/// drawings* is recorded without approval and reduces the March drawings; from
/// another party it is refused.
#[renox::test]
async fn a_repayment_by_the_owner_needs_no_approval() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let pgp = t.entity("PGP").await;
    post_drawing(
        &t,
        &a,
        &pgp,
        QUIET_SANDS_CODE,
        "out",
        "15000.00",
        "2027-03-10",
    )
    .await
    .assert_status(303);
    decide(&t, &drawings(&t).await[0], Some(&b), None).await;
    let asked = t.drawings.asked().len();

    post_drawing(
        &t,
        &a,
        &pgp,
        QUIET_SANDS_CODE,
        "in",
        "5000.00",
        "2027-03-20",
    )
    .await
    .assert_status(303);
    assert_eq!(t.drawings.asked().len(), asked, "no approval is asked for");
    let all = payments(&t).await;
    assert_eq!(all.len(), 2);
    assert_eq!(all[1].direction, Direction::In);
    assert_eq!(all[1].purpose, "owner_drawings");
    assert_eq!(all[1].payment_date, day(3, 20));

    // What budget vs actual reads: a drawing of AED 15,000 and a repayment of
    // AED 5,000 in March.
    let read = PaymentDrawings
        .payments(t.db(), &[pgp.id], day(3, 1), day(3, 31))
        .await
        .unwrap();
    assert_eq!(read.len(), 2);
    assert_eq!(
        (read[0].direction, read[0].amount),
        (Direction::Out, 1_500_000)
    );
    assert_eq!(
        (read[1].direction, read[1].amount),
        (Direction::In, 500_000)
    );
    assert_eq!(read[0].category, "owner_drawings");
    let none = PaymentDrawings
        .payments(t.db(), &[pgp.id], day(4, 1), day(4, 30))
        .await
        .unwrap();
    assert!(none.is_empty());

    // From another party it is refused.
    post_drawing(&t, &a, &pgp, BLUE_DUNES_CODE, "in", "100.00", "2027-03-21")
        .await
        .assert_status(422);
    assert_eq!(payments(&t).await.len(), 2);

    // The repayment is no credit of the owner party and takes no allocation.
    let repayment = &all[1];
    assert!(
        allocate(
            t.state(),
            repayment.payment_id(),
            wl_kernel::payments::AllocationTarget::Invoice(wl_kernel::invoicing::InvoiceId(1)),
            100,
            None,
        )
        .await
        .is_err()
    );
    let credit = wl_kernel::payments::payment_recorder(t.state())
        .credit_balance(t.db(), QUIET_SANDS, "AED")
        .await
        .unwrap();
    assert_eq!(credit, 0);
}

/// After B approves a drawing of AED 4,000 paid by cheque, registering the
/// issued cheque for AED 4,500 or to another payee is refused; a cheque of
/// AED 4,000 to the owner party is accepted whatever its date.
#[renox::test]
async fn the_cheque_of_an_approved_drawing_is_for_the_owner_and_the_amount() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let pgp = t.entity("PGP").await;
    let entity = pgp.id.to_string();
    t.app.acting_as(&a);
    t.app
        .htmx()
        .post(
            "/payments",
            &[
                ("legal_entity_id", entity.as_str()),
                ("party_code", QUIET_SANDS_CODE),
                ("method", "cheque"),
                ("amount", "4000.00"),
                ("purpose", "owner_drawings"),
                ("drawing_direction", "out"),
            ],
        )
        .await
        .assert_status(303);
    let saved = drawings(&t).await;
    assert!(payments(&t).await.is_empty());
    // Not approved yet: no cheque.
    let cheque = |party, amount, number: &str| {
        RegisterCheque::new(party, pgp.id, "ENBD", number, day(4, 15), amount)
            .issued()
            .drawing(saved[0].id)
    };
    let refused = |request: RegisterCheque| {
        let state = t.state().clone();
        async move {
            wl_kernel::cheques::cheques(&state)
                .register(&state, request, None)
                .await
        }
    };
    assert!(refused(cheque(QUIET_SANDS, 400_000, "9001")).await.is_err());

    decide(&t, &saved[0], Some(&b), None).await;
    let approved = drawings(&t).await;
    assert_eq!(approved[0].status, DrawingStatus::Approved);
    assert!(
        payments(&t).await.is_empty(),
        "a cheque drawing waits for its cheque"
    );
    assert!(refused(cheque(QUIET_SANDS, 450_000, "9002")).await.is_err());
    assert!(refused(cheque(BLUE_DUNES, 400_000, "9003")).await.is_err());
    // A received cheque cannot carry a drawing.
    let received = RegisterCheque::new(QUIET_SANDS, pgp.id, "ENBD", "9004", day(4, 15), 400_000)
        .drawing(saved[0].id);
    assert!(refused(received).await.is_err());

    let registered = refused(cheque(QUIET_SANDS, 400_000, "9005")).await.unwrap();
    assert_eq!(registered.amount, 400_000);
    assert_eq!(drawings(&t).await[0].cheque_id, Some(registered.id.0));
    // One cheque per drawing.
    assert!(refused(cheque(QUIET_SANDS, 400_000, "9006")).await.is_err());
}

/// The issued cheque of an approved drawing, once cleared, records the payment
/// with purpose *owner drawings* on the clear date, without a second request.
#[renox::test]
async fn clearing_the_cheque_records_the_drawing() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let staff = t.cheque_staff("Chequer").await;
    let pgp = t.entity("PGP").await;
    let entity = pgp.id.to_string();
    t.app.acting_as(&a);
    t.app
        .htmx()
        .post(
            "/payments",
            &[
                ("legal_entity_id", entity.as_str()),
                ("party_code", QUIET_SANDS_CODE),
                ("method", "cheque"),
                ("amount", "4000.00"),
                ("purpose", "owner_drawings"),
                ("drawing_direction", "out"),
            ],
        )
        .await
        .assert_status(303);
    let saved = drawings(&t).await;
    decide(&t, &saved[0], Some(&b), None).await;
    let asked = t.drawings.asked().len();

    let cheque = t
        .register_cheque(
            RegisterCheque::new(QUIET_SANDS, pgp.id, "ENBD", "9100", today(), 400_000)
                .issued()
                .drawing(saved[0].id),
        )
        .await;
    let cheque = t.deposit_cheque(&staff, &cheque).await;
    t.cheque_action(&staff, &cheque, "clear", &[])
        .await
        .assert_ok();

    let recorded = payments(&t).await;
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].direction, Direction::Out);
    assert_eq!(recorded[0].purpose, "owner_drawings");
    assert_eq!(
        recorded[0].drawing_category.as_deref(),
        Some("owner_drawings")
    );
    assert_eq!(
        (recorded[0].amount, recorded[0].party_id),
        (400_000, QUIET_SANDS.0)
    );
    assert_eq!(recorded[0].payment_date, today());
    assert_eq!(t.drawings.asked().len(), asked, "no second request");
    let done = drawings(&t).await;
    assert_eq!(
        (done[0].status, done[0].payment_id),
        (DrawingStatus::Recorded, Some(recorded[0].id))
    );
    assert_eq!(t.received.all().len(), 1);
}

/// A cheque that is not for a drawing records no purpose: an ordinary issued
/// cheque to the owner party is an ordinary payment.
#[renox::test]
async fn an_ordinary_cheque_to_the_owner_is_not_a_drawing() {
    let t = boot().await;
    let staff = t.cheque_staff("Chequer").await;
    let pgp = t.entity("PGP").await;
    let cheque = t
        .register_cheque(
            RegisterCheque::new(QUIET_SANDS, pgp.id, "ENBD", "9200", today(), 100_000).issued(),
        )
        .await;
    let cheque = t.deposit_cheque(&staff, &cheque).await;
    t.cheque_action(&staff, &cheque, "clear", &[])
        .await
        .assert_ok();
    let recorded = payments(&t).await;
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].purpose, "none");
}

/// Reversing a recorded drawing (reason *entered in error*) needs no drawings
/// approval and tells the holders of `drawings.approve`; the reversed payment
/// is no longer read as a drawing.
#[renox::test]
async fn reversing_a_drawing_tells_the_approvers() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let owner = t.user("Owner", &[perm::VIEW, "drawings.approve"]).await;
    let pgp = t.entity("PGP").await;
    post_drawing(
        &t,
        &a,
        &pgp,
        QUIET_SANDS_CODE,
        "out",
        "1000.00",
        "2027-03-10",
    )
    .await
    .assert_status(303);
    decide(&t, &drawings(&t).await[0], Some(&b), None).await;
    let payment = payments(&t).await.remove(0);

    t.reverse(&a, &payment, "entered_in_error", "wrong amount")
        .await
        .assert_ok();
    let notices = t.notices.of_type("payments.drawing_reversed");
    assert_eq!(notices.len(), 1);
    assert!(notices[0].recipients.iter().any(|r| matches!(
        r,
        wl_kernel::notify::NotifyRecipient::User { id } if *id == owner.id
    )));
    let read = PaymentDrawings
        .payments(t.db(), &[pgp.id], day(3, 1), day(3, 31))
        .await
        .unwrap();
    assert!(read.is_empty());
}

/// The *Owner drawings* expense category is seeded with VAT blocked, no cost
/// object and the owner drawings budget category; seeding twice changes
/// nothing.
#[renox::test]
async fn the_owner_drawings_expense_category_is_seeded() {
    let t = boot().await;
    assert_eq!(expense_category::install(t.db()).await.unwrap(), 7);
    assert_eq!(expense_category::install(t.db()).await.unwrap(), 0);
    let drawings = ExpenseCategory::by_code(t.db(), "owner_drawings")
        .await
        .unwrap()
        .expect("seeded");
    assert_eq!(drawings.vat_treatment, VatTreatment::Blocked);
    assert!(!drawings.cost_object_allowed);
    assert!(drawings.is_owner_drawings());
    let fuel = ExpenseCategory::by_code(t.db(), "fuel")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fuel.vat_treatment, VatTreatment::Recoverable);
    assert!(fuel.cost_object_allowed);
}

/// The form offers the purpose, and the request page says where a drawing
/// stands.
#[renox::test]
async fn the_form_and_the_request_page_show_the_drawing() {
    let t = boot().await;
    let a = t.user_a().await;
    let pgp = t.entity("PGP").await;
    t.app.acting_as(&a);
    t.app
        .get("/payments/new")
        .await
        .assert_ok()
        .assert_see("Owner drawings");
    post_drawing(
        &t,
        &a,
        &pgp,
        QUIET_SANDS_CODE,
        "out",
        "250.00",
        "2027-03-10",
    )
    .await
    .assert_status(303);
    let id = drawings(&t).await[0].id;
    t.app
        .get(&format!("/payments/drawings/{id}"))
        .await
        .assert_ok()
        .assert_see("Waiting for approval")
        .assert_see("Quiet Sands LLC");
}

// ---------------------------------------------------------------------------
// §5.7 step 3: a bank statement line marked as an owner drawing.

/// An import of PGP's account `ENBD-1` for March 2027 with the lines of `csv`,
/// read and in review; the user who reconciles it acts.
async fn reviewed(t: &Booted, csv: &str) -> (User, BankStatementImport) {
    let dina = t.user("Dina", &[perm::VIEW, perm::BANK_RECONCILE]).await;
    t.app.acting_as(&dina);
    let entity = t.entity("PGP").await.id.to_string();
    t.app
        .htmx()
        .post(
            "/bank-statements",
            &[
                ("legal_entity_id", entity.as_str()),
                ("bank_account", "ENBD-1"),
                ("file_name", "march.csv"),
                ("format", "csv"),
                ("period_start", "2027-03-01"),
                ("period_end", "2027-03-31"),
                ("closing_balance", "10,000"),
                ("content", csv),
            ],
        )
        .await
        .assert_status(303);
    let import = BankStatementImport::query()
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    t.app
        .htmx()
        .post(&format!("/bank-statements/{}/parse", import.id), &[])
        .await
        .assert_ok();
    (dina, import)
}

const ATM: &str = "date,description,reference,amount\n\
    2027-03-08,ATM Marina,ATM-1,-1200.00\n\
    2027-03-09,Rent received,RENT-1,3000.00\n";

async fn lines(t: &Booted, import: &BankStatementImport) -> Vec<BankStatementLine> {
    BankStatementLine::of_import(t.db(), import.id)
        .await
        .unwrap()
}

async fn mark(t: &Booted, line: &BankStatementLine) -> renox::testing::TestResponse {
    t.app
        .htmx()
        .post(
            &format!("/bank-statement-lines/{}/drawing", line.id),
            &[("category", "owner_drawings")],
        )
        .await
}

async fn line_of(t: &Booted, line: &BankStatementLine) -> BankStatementLine {
    BankStatementLine::find_or_404(t.db(), line.id)
        .await
        .unwrap()
}

async fn complete(t: &Booted, import: &BankStatementImport) -> renox::testing::TestResponse {
    t.app
        .htmx()
        .post(&format!("/bank-statements/{}/complete", import.id), &[])
        .await
}

/// An outgoing line of AED 1,200.00 (ATM) marked as a drawing stays
/// `suggested`; the import cannot complete while the request is open; after
/// the approval the payment is recorded with the line's date and amount and
/// the line is matched, and the import can complete.
#[renox::test]
async fn a_marked_line_is_matched_when_the_drawing_is_approved() {
    let t = boot().await;
    let b = t.user_b().await;
    let (dina, import) = reviewed(&t, ATM).await;
    let all = lines(&t, &import).await;
    let (atm, rent) = (&all[0], &all[1]);
    assert_eq!((atm.amount, rent.amount), (-120_000, 300_000));

    mark(&t, atm).await.assert_ok();
    let marked = line_of(&t, atm).await;
    assert_eq!(marked.status, LineStatus::Suggested);
    assert!(payments(&t).await.is_empty(), "nothing before the approval");
    let asked = t.drawings.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!((asked[0].amount, asked[0].requested_by), (120_000, dina.id));
    let request = drawings(&t).await;
    assert_eq!(request.len(), 1);
    assert_eq!(marked.drawing_request_id, Some(request[0].id));
    assert_eq!(request[0].statement_line_id, Some(atm.id));

    // The read of the marked lines (FIN.BUD.01): waiting, not recorded.
    let entity = t.entity("PGP").await;
    let read = bank_statements(t.state())
        .drawing_lines(t.db(), &[entity.id], day(3, 1), day(3, 31))
        .await
        .unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(
        (read[0].line_id, read[0].amount, read[0].date),
        (atm.id, 120_000, day(3, 8))
    );
    assert_eq!(read[0].category, "owner_drawings");
    assert!(!read[0].recorded);

    // The rent line is ignored; the drawing still holds the import back.
    t.app
        .htmx()
        .post(
            &format!("/bank-statement-lines/{}/ignore", rent.id),
            &[("reason", "booked in the rent run")],
        )
        .await
        .assert_ok();
    complete(&t, &import).await.assert_status(422);
    assert_eq!(
        BankStatementImport::find_or_404(t.db(), import.id)
            .await
            .unwrap()
            .status,
        ImportStatus::InReview
    );

    decide(&t, &request[0], Some(&b), None).await;
    let recorded = payments(&t).await;
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].direction, Direction::Out);
    assert_eq!(
        (recorded[0].amount, recorded[0].payment_date),
        (120_000, day(3, 8))
    );
    assert_eq!(recorded[0].purpose, "owner_drawings");
    assert_eq!(recorded[0].reference.as_deref(), Some("ATM-1"));
    assert_eq!(recorded[0].bank_account.as_deref(), Some("ENBD-1"));
    let matched = line_of(&t, atm).await;
    assert_eq!(matched.status, LineStatus::Matched);
    assert_eq!(matched.match_id, Some(recorded[0].id));
    let read = bank_statements(t.state())
        .drawing_lines(t.db(), &[entity.id], day(3, 1), day(3, 31))
        .await
        .unwrap();
    assert!(read[0].recorded);
    // Outside its dates, or for another entity, nothing is read.
    assert!(
        bank_statements(t.state())
            .drawing_lines(t.db(), &[entity.id], day(4, 1), day(4, 30))
            .await
            .unwrap()
            .is_empty()
    );

    complete(&t, &import).await.assert_ok();
    assert_eq!(
        BankStatementImport::find_or_404(t.db(), import.id)
            .await
            .unwrap()
            .status,
        ImportStatus::Completed
    );
}

/// A rejection (or a withdrawal or an expiry) puts the line back to
/// `unmatched` with the reason and tells the accountant who marked it; the
/// line can be marked again; the drawing is no longer read.
#[renox::test]
async fn a_rejected_drawing_puts_the_line_back_with_the_reason() {
    let t = boot().await;
    let (dina, import) = reviewed(&t, ATM).await;
    let atm = lines(&t, &import).await.remove(0);
    mark(&t, &atm).await.assert_ok();
    let request = drawings(&t).await.remove(0);

    decide(&t, &request, None, Some("expired after 7 days")).await;
    assert!(payments(&t).await.is_empty());
    let back = line_of(&t, &atm).await;
    assert_eq!(back.status, LineStatus::Unmatched);
    assert_eq!(back.reason.as_deref(), Some("expired after 7 days"));
    assert_eq!(back.drawing_request_id, None);
    let told = t.notices.of_type(BANK_LINE_DRAWING_REJECTED);
    assert_eq!(told.len(), 1);
    assert!(matches!(
        told[0].recipients[0],
        wl_kernel::notify::NotifyRecipient::User { id } if id == dina.id
    ));
    let data = told[0].data.to_string();
    assert!(
        data.contains("expired after 7 days") && data.contains("1,200.00"),
        "{data}"
    );
    let entity = t.entity("PGP").await;
    assert!(
        bank_statements(t.state())
            .drawing_lines(t.db(), &[entity.id], day(3, 1), day(3, 31))
            .await
            .unwrap()
            .is_empty()
    );

    // Marked again, it is a new request.
    mark(&t, &atm).await.assert_ok();
    assert_eq!(line_of(&t, &atm).await.status, LineStatus::Suggested);
    assert_eq!(drawings(&t).await.len(), 2);
    assert_eq!(line_of(&t, &atm).await.reason, None);
}

/// The sole owner's drawing is approved at once: the line is matched when it
/// is marked.
#[renox::test]
async fn a_sole_owner_marks_a_line_and_it_is_matched_at_once() {
    let t = boot().await;
    t.drawings.approves_at_once(true);
    let (_dina, import) = reviewed(&t, ATM).await;
    let atm = lines(&t, &import).await.remove(0);
    mark(&t, &atm).await.assert_ok();
    let matched = line_of(&t, &atm).await;
    assert_eq!(matched.status, LineStatus::Matched);
    let recorded = payments(&t).await;
    assert_eq!(recorded.len(), 1);
    assert!(recorded[0].self_approved);
    assert_eq!(matched.match_id, Some(recorded[0].id));
}

/// Only money out, of an unmatched line, in a known drawings category, can be
/// marked; an import in review with a drawing waiting cannot be reverted.
#[renox::test]
async fn only_an_outgoing_unmatched_line_is_marked_and_a_waiting_drawing_blocks_the_revert() {
    let t = boot().await;
    let (_dina, import) = reviewed(&t, ATM).await;
    let all = lines(&t, &import).await;
    mark(&t, &all[1]).await.assert_status(422);
    t.app
        .htmx()
        .post(
            &format!("/bank-statement-lines/{}/drawing", all[0].id),
            &[("category", "no_such_category")],
        )
        .await
        .assert_status(422);
    assert_eq!(line_of(&t, &all[0]).await.status, LineStatus::Unmatched);
    t.drawings.owner(None);
    mark(&t, &all[0]).await.assert_status(422);
    t.drawings.owner(Some(QUIET_SANDS));
    assert!(t.drawings.asked().is_empty());

    mark(&t, &all[0]).await.assert_ok();
    mark(&t, &all[0]).await.assert_status(422);
    let revert = format!("/bank-statements/{}/revert", import.id);
    t.app.htmx().post(&revert, &[]).await.assert_status(422);
    let request = drawings(&t).await.remove(0);
    decide(&t, &request, None, Some("withdrawn")).await;
    t.app.htmx().post(&revert, &[]).await.assert_ok();
    assert!(lines(&t, &import).await.is_empty());
}

/// Requester cannot approve: the decision of the user who saved the drawing
/// records nothing; another approver's does.
#[renox::test]
async fn the_requester_cannot_approve_their_own_drawing() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let pgp = t.entity("PGP").await;
    post_drawing(
        &t,
        &a,
        &pgp,
        QUIET_SANDS_CODE,
        "out",
        "15000.00",
        "2027-03-10",
    )
    .await
    .assert_status(303);
    let saved = drawings(&t).await.remove(0);

    let refused = t
        .state()
        .emit(OwnerDrawingDecided {
            source: Subject::new(DRAWING_REQUEST_SUBJECT_TYPE, saved.id),
            legal_entity_id: saved.legal_entity_id,
            approved: true,
            approved_by: Some(a.id),
            reason: None,
        })
        .await;
    assert!(refused.is_err(), "the requester's approval is refused");
    assert!(payments(&t).await.is_empty());
    assert_eq!(drawings(&t).await[0].status, DrawingStatus::Requested);

    decide(&t, &saved, Some(&b), None).await;
    assert_eq!(payments(&t).await.len(), 1);
    assert_eq!(drawings(&t).await[0].status, DrawingStatus::Recorded);
}

/// Money out to a blocked owner party waits under `PTY.ACL.03`: the approval
/// stands but the payment is not recorded until the party is free; a
/// repayment from the same party is money in and never refused.
#[renox::test]
async fn a_drawing_to_a_blocked_owner_waits_for_the_unblock() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let pgp = t.entity("PGP").await;
    post_drawing(
        &t,
        &a,
        &pgp,
        QUIET_SANDS_CODE,
        "out",
        "900.00",
        "2027-03-12",
    )
    .await
    .assert_status(303);
    let saved = drawings(&t).await.remove(0);

    t.blocked.block(QUIET_SANDS);
    decide(&t, &saved, Some(&b), None).await;
    assert!(payments(&t).await.is_empty(), "no money goes out");
    let waiting = drawings(&t).await.remove(0);
    assert_eq!(waiting.status, DrawingStatus::Approved);
    assert!(
        waiting
            .reason
            .as_deref()
            .is_some_and(|r| r.contains("hold")),
        "{:?}",
        waiting.reason
    );
    let again = record_approved(t.state(), &waiting).await.unwrap_err();
    assert!(again.to_string().contains("hold"), "{again}");

    // Money in from the blocked party is received.
    post_drawing(&t, &a, &pgp, QUIET_SANDS_CODE, "in", "100.00", "2027-03-13")
        .await
        .assert_status(303);
    assert_eq!(payments(&t).await.len(), 1);

    t.blocked.free(QUIET_SANDS);
    record_approved(t.state(), &waiting).await.unwrap();
    let recorded = payments(&t).await;
    assert_eq!(recorded.len(), 2);
    assert_eq!(drawings(&t).await[0].status, DrawingStatus::Recorded);
}
