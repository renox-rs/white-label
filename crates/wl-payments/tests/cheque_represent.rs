//! BL-PAY-10 (#103): a bounced received cheque is presented again, at most
//! twice in all and only when the bank paid nothing; the party is told the
//! new date, a new fee is added at each bounce, and a cheque that clears
//! after a re-presentation emits `ChequeBounceResolved` (cleared).

mod support;

use renox::prelude::*;
use renox::chrono::TimeDelta;
use wl_kernel::cheques::{ChequeId, ChequeOutcome, ChequeStatus, RegisterCheque};
use wl_kernel::documents::today;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::payments::{PaymentMethod, PaymentSource};
use wl_payments::cheque::ChequeReturn;
use wl_payments::followup::{FollowupKind, FollowupStatus};
use wl_payments::gateway::FakeGateway;
use wl_payments::notices::CHEQUE_PRESENTED;
use wl_payments::{FollowupTask, Payment, WlPayments};

use support::{BLUE_DUNES, Booted, QUIET_SANDS, boot, boot_with};

fn in_days(days: i64) -> String {
    (today() + TimeDelta::days(days)).format("%Y-%m-%d").to_string()
}

async fn cheque_payments(t: &Booted) -> Vec<Payment> {
    Payment::query()
        .where_(Payment::SOURCE.eq(PaymentSource::Cheque))
        .order_by(Payment::ID)
        .get(t.db())
        .await
        .unwrap()
}

/// AC1: a bounced cheque (presentations 1, honoured 0), presented again with
/// a date and the note "funds confirmed by phone", is `deposited` with
/// presentations 2, and the party is told the new date.
#[renox::test]
async fn a_bounced_cheque_is_presented_again_and_the_party_is_told() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    t.accountant("Cara").await;
    let rent = t.invoices.add(BLUE_DUNES, "INV-1", 2_000_000);
    let bounced = t
        .bounced_cheque(&staff, BLUE_DUNES, "2001", 2_000_000, Some(rent), "")
        .await;
    assert_eq!((bounced.presentations, bounced.honoured_amount), (1, 0));
    assert_eq!(
        FollowupTask::open_of_cheque(t.db(), bounced.id).await.unwrap().len(),
        1
    );

    let date = in_days(30);
    t.represent_cheque(&staff, &bounced, &date, false, "funds confirmed by phone")
        .await
        .assert_ok();

    let again = t.cheque(bounced.id).await;
    assert_eq!(again.status, ChequeStatus::Deposited);
    assert_eq!(again.presentations, 2);
    assert_eq!(again.deposit_date.map(|d| d.to_string()), Some(date.clone()));
    assert_eq!(again.open_amount, 2_000_000);

    let told = t.notices.of_type(CHEQUE_PRESENTED);
    assert_eq!(told.len(), 1);
    assert!(matches!(
        told[0].recipients[0],
        NotifyRecipient::Party { id } if id == BLUE_DUNES
    ));
    let data = told[0].data.to_string();
    assert!(data.contains("2001") && data.contains("AED 20,000.00"), "{data}");

    // The bounce's follow-up task is done: the cheque is with the bank.
    let tasks = FollowupTask::of_cheque(t.db(), bounced.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].kind, FollowupKind::FollowUpCheque);
    assert_eq!(tasks[0].status, FollowupStatus::Done);
}

/// The party's confirmation alone is enough, without a note.
#[renox::test]
async fn the_confirmation_of_funds_alone_is_enough() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    let bounced = t
        .bounced_cheque(&staff, BLUE_DUNES, "2002", 500_000, None, "")
        .await;
    t.represent_cheque(&staff, &bounced, &in_days(3), true, "")
        .await
        .assert_ok();
    assert_eq!(t.cheque(bounced.id).await.status, ChequeStatus::Deposited);
}

/// A date, and a confirmation or a note, are needed; the date is not in the
/// past.
#[renox::test]
async fn a_re_presentation_needs_a_date_and_a_confirmation_or_a_note() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    let bounced = t
        .bounced_cheque(&staff, BLUE_DUNES, "2003", 500_000, None, "")
        .await;
    t.represent_cheque(&staff, &bounced, &in_days(3), false, "")
        .await
        .assert_status(422);
    t.represent_cheque(&staff, &bounced, &in_days(3), false, "   ")
        .await
        .assert_status(422);
    t.represent_cheque(&staff, &bounced, &in_days(-1), true, "")
        .await
        .assert_status(422);
    t.represent_cheque(&staff, &bounced, "", true, "")
        .await
        .assert_status(422);
    let still = t.cheque(bounced.id).await;
    assert_eq!(still.status, ChequeStatus::Bounced);
    assert_eq!(still.presentations, 1);
    assert!(t.notices.of_type(CHEQUE_PRESENTED).is_empty());
    // Today is fine.
    t.represent_cheque(&staff, &bounced, &in_days(0), true, "")
        .await
        .assert_ok();
}

/// AC2: a second re-presentation is refused (maximum 2 allows one), and the
/// second bounce added a new fee.
#[renox::test]
async fn a_second_re_presentation_is_refused_and_each_bounce_adds_a_fee() {
    let t = boot_with(
        WlPayments::new().bounce_fee("bounce_fee", 50_000),
        FakeGateway::new(),
    )
    .await;
    let staff = t.cheque_staff("Amal").await;
    let rent = t.invoices.add(BLUE_DUNES, "INV-1", 2_000_000);
    let bounced = t
        .bounced_cheque(&staff, BLUE_DUNES, "2004", 2_000_000, Some(rent), "")
        .await;
    assert_eq!(t.fees.charges().len(), 1);
    t.represent_cheque(&staff, &bounced, &in_days(2), true, "")
        .await
        .assert_ok();
    let again = t.cheque(bounced.id).await;
    // The bank returns it a second time.
    t.bounce_cheque(&staff, &again, "account_closed", "CERT-2", "")
        .await
        .assert_ok();
    let second = t.cheque(bounced.id).await;
    assert_eq!(second.status, ChequeStatus::Bounced);
    assert_eq!(second.presentations, 2);
    assert_eq!(second.return_reason.as_deref(), Some("account closed"));
    assert_eq!(t.fees.charges().len(), 2, "a new fee at each bounce");
    assert_eq!(second.bounce_fee_total, 100_000);
    let returns = ChequeReturn::of_cheque(t.db(), bounced.id).await.unwrap();
    assert_eq!(
        returns.iter().map(|r| r.presentation).collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(returns[1].certificate.as_deref(), Some("CERT-2"));
    assert_eq!(t.cheque_bounced.all().len(), 2);
    // The follow-up task of the second bounce is open again.
    let open = FollowupTask::open_of_cheque(t.db(), bounced.id).await.unwrap();
    assert_eq!(open.len(), 1);

    t.represent_cheque(&staff, &second, &in_days(5), true, "")
        .await
        .assert_status(422);
    let after = t.cheque(bounced.id).await;
    assert_eq!(after.status, ChequeStatus::Bounced);
    assert_eq!(after.presentations, 2);
}

/// AC3: with AED 6,000.00 honoured, the cheque cannot be presented again; an
/// issued cheque neither.
#[renox::test]
async fn a_partly_honoured_or_an_issued_cheque_is_not_presented_again() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    let rent = t.invoices.add(BLUE_DUNES, "INV-1", 2_000_000);
    let bounced = t
        .bounced_cheque(&staff, BLUE_DUNES, "2005", 2_000_000, Some(rent), "6000")
        .await;
    assert_eq!(bounced.honoured_amount, 600_000);
    t.represent_cheque(&staff, &bounced, &in_days(3), true, "")
        .await
        .assert_status(422);
    assert_eq!(t.cheque(bounced.id).await.status, ChequeStatus::Bounced);

    let entity = t.entity("PGP").await;
    let issued = t
        .register_cheque(
            RegisterCheque::new(BLUE_DUNES, entity.id, "ENBD", "2006", today(), 1_050_000).issued(),
        )
        .await;
    let issued = t.deposit_cheque(&staff, &issued).await;
    t.bounce_cheque(&staff, &issued, "insufficient_funds", "", "")
        .await
        .assert_ok();
    let issued = t.cheque(issued.id).await;
    t.represent_cheque(&staff, &issued, &in_days(3), true, "")
        .await
        .assert_status(422);
}

/// Only `cheques.deposit` re-presents, and only from `bounced`.
#[renox::test]
async fn only_a_bounced_cheque_is_presented_again_by_a_user_with_the_right() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    let viewer = t
        .user("Vera", &[wl_payments::permissions::CHEQUES_VIEW])
        .await;
    let held = t.received_cheque(BLUE_DUNES, "2007", 500_000, None).await;
    t.represent_cheque(&staff, &held, &in_days(3), true, "")
        .await
        .assert_status(422);
    let bounced = t
        .bounced_cheque(&staff, BLUE_DUNES, "2008", 500_000, None, "")
        .await;
    t.represent_cheque(&viewer, &bounced, &in_days(3), true, "")
        .await
        .assert_forbidden();
    assert_eq!(t.cheque(bounced.id).await.status, ChequeStatus::Bounced);
}

/// AC4: `clear` after a re-presentation records a payment for the open amount
/// and emits `ChequeBounceResolved` (cleared), listing the party's other
/// bounced cheques still open.
#[renox::test]
async fn clearing_after_a_re_presentation_resolves_the_bounce() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    let rent = t.invoices.add(BLUE_DUNES, "INV-1", 2_000_000);
    let other_rent = t.invoices.add(BLUE_DUNES, "INV-2", 300_000);
    let bounced = t
        .bounced_cheque(&staff, BLUE_DUNES, "2009", 2_000_000, Some(rent), "")
        .await;
    let other = t
        .bounced_cheque(&staff, BLUE_DUNES, "2010", 300_000, Some(other_rent), "")
        .await;
    let elsewhere = t
        .bounced_cheque(&staff, QUIET_SANDS, "2011", 100_000, None, "")
        .await;

    t.represent_cheque(&staff, &bounced, &in_days(2), true, "")
        .await
        .assert_ok();
    let again = t.cheque(bounced.id).await;
    assert!(t.cheque_resolved.all().is_empty(), "not resolved yet");
    t.cheque_action(&staff, &again, "clear", &[]).await.assert_ok();

    let cleared = t.cheque(bounced.id).await;
    assert_eq!(cleared.status, ChequeStatus::Cleared);
    assert_eq!(cleared.open_amount, 0);
    let payments = cheque_payments(&t).await;
    assert_eq!(payments.len(), 1);
    assert_eq!(payments[0].amount, 2_000_000);
    assert_eq!(payments[0].method, PaymentMethod::Cheque);
    let calls = t.invoices.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!((calls[0].invoice, calls[0].amount), (rent, 2_000_000));

    let resolved = t.cheque_resolved.all();
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].cheque, ChequeId(bounced.id));
    assert_eq!(resolved[0].party, BLUE_DUNES);
    assert_eq!(resolved[0].outcome, ChequeOutcome::Cleared);
    assert_eq!(resolved[0].others_open, vec![ChequeId(other.id)]);
    assert_ne!(elsewhere.id, other.id);
    // The party still owes the other cheque.
    let owed = wl_kernel::cheques::cheques(t.state())
        .open_bounced_amount(t.db(), BLUE_DUNES, "AED")
        .await
        .unwrap();
    assert_eq!(owed, 300_000);
}
