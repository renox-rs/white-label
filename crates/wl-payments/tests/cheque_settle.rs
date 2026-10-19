//! BL-PAY-11 (#104): the party may pay part of a bounced cheque's open amount
//! in cash, by transfer or by card; replacement cheques or payments that
//! cover the open amount make the cheque `replaced` (`FIN.ACL.02`), and the
//! original is handed back with a receipt once the replacement clears. An
//! issued cheque is replaced the same way, by a new issued cheque or an
//! outgoing payment.

mod support;

use wl_kernel::cheques::{ChequeId, ChequeOutcome, ChequeStatus, RegisterCheque};
use wl_kernel::documents::today;
use wl_kernel::payments::{Direction, PaymentMethod, PaymentSource, RecordPayment};
use wl_payments::cheque_settle::{ChequeReplacement, HAND_BACK_SERIES};
use wl_payments::followup::{FollowupKind, FollowupStatus};
use wl_payments::{Cheque, FollowupTask, Payment};

use support::{BLUE_DUNES, Booted, QUIET_SANDS, boot, year};

async fn cheque_payments(t: &Booted) -> Vec<Payment> {
    Payment::query()
        .where_(Payment::SOURCE.eq(PaymentSource::Cheque))
        .order_by(Payment::ID)
        .get(t.db())
        .await
        .unwrap()
}

async fn tasks_of(t: &Booted, cheque: &Cheque, kind: FollowupKind) -> Vec<FollowupTask> {
    FollowupTask::of_cheque(t.db(), cheque.id)
        .await
        .unwrap()
        .into_iter()
        .filter(|task| task.kind == kind)
        .collect()
}

/// AC1: a bounced cheque with AED 14,000.00 open, a cash part payment of AED
/// 4,000.00 is recorded and allocated; AED 10,000.00 stay open and the cheque
/// stays `bounced`.
#[renox::test]
async fn a_cash_part_payment_is_recorded_allocated_and_reduces_the_open_amount() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    let rent = t.invoices.add(BLUE_DUNES, "INV-1", 2_000_000);
    let cheque = t
        .bounced_cheque(&staff, BLUE_DUNES, "3001", 2_000_000, Some(rent), "6000")
        .await;
    assert_eq!(cheque.open_amount, 1_400_000);

    t.settle_cheque(&staff, &cheque, "cash", "4000.00", "")
        .await
        .assert_ok();

    let after = t.cheque(cheque.id).await;
    assert_eq!(after.status, ChequeStatus::Bounced);
    assert_eq!(after.settled_amount, 400_000);
    assert_eq!(after.open_amount, 1_000_000);
    assert_eq!(after.honoured_amount, 600_000);
    let payments = cheque_payments(&t).await;
    assert_eq!(payments.len(), 2, "the honoured part and the part payment");
    let part = &payments[1];
    assert_eq!(part.amount, 400_000);
    assert_eq!(part.method, PaymentMethod::Cash);
    assert_eq!(part.direction, Direction::In);
    assert_eq!(part.collected_by, Some(staff.id));
    assert_eq!(part.party_id, BLUE_DUNES.0);
    let calls = t.invoices.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!((calls[1].invoice, calls[1].amount), (rent, 400_000));
    assert_eq!(calls[1].source, part.subject());
    assert_eq!(t.allocations(part).await.len(), 1);
    assert_eq!(t.received.all().len(), 2);
    // The party owes what is open.
    let owed = wl_kernel::cheques::cheques(t.state())
        .open_bounced_amount(t.db(), BLUE_DUNES, "AED")
        .await
        .unwrap();
    assert_eq!(owed, 1_000_000);
}

/// AC2: a part payment equal to (or above) the open amount is refused; so is
/// one with no amount, by a cheque, or a transfer without a reference.
#[renox::test]
async fn a_part_payment_must_be_below_the_open_amount_and_complete() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    let rent = t.invoices.add(BLUE_DUNES, "INV-1", 1_000_000);
    let cheque = t
        .bounced_cheque(&staff, BLUE_DUNES, "3002", 1_000_000, Some(rent), "")
        .await;

    t.settle_cheque(&staff, &cheque, "cash", "10000.00", "")
        .await
        .assert_status(422);
    t.settle_cheque(&staff, &cheque, "cash", "10000.01", "")
        .await
        .assert_status(422);
    t.settle_cheque(&staff, &cheque, "cash", "0", "")
        .await
        .assert_status(422);
    t.settle_cheque(&staff, &cheque, "cash", "", "")
        .await
        .assert_status(422);
    t.settle_cheque(&staff, &cheque, "cheque", "100.00", "")
        .await
        .assert_status(422);
    t.settle_cheque(&staff, &cheque, "", "100.00", "")
        .await
        .assert_status(422);
    // A transfer needs its reference; nothing is written when it is missing.
    t.settle_cheque(&staff, &cheque, "transfer", "100.00", "")
        .await
        .assert_status(422);
    let unchanged = t.cheque(cheque.id).await;
    assert_eq!((unchanged.settled_amount, unchanged.open_amount), (0, 1_000_000));
    assert!(cheque_payments(&t).await.is_empty());

    t.settle_cheque(&staff, &cheque, "transfer", "100.00", "TRX-5")
        .await
        .assert_ok();
    t.settle_cheque(&staff, &cheque, "card", "50.00", "SLIP-8")
        .await
        .assert_ok();
    let after = t.cheque(cheque.id).await;
    assert_eq!((after.settled_amount, after.open_amount), (15_000, 985_000));
    assert_eq!(cheque_payments(&t).await.len(), 2);
    // The last fils below the open amount is fine, the open amount is not.
    t.settle_cheque(&staff, &after, "cash", "9850.00", "")
        .await
        .assert_status(422);
    t.settle_cheque(&staff, &after, "cash", "9849.99", "")
        .await
        .assert_ok();
    assert_eq!(t.cheque(cheque.id).await.open_amount, 1);
}

/// Only `cheques.manage` settles, replaces and links; only a received cheque
/// takes a part payment; only a bounced one is settled.
#[renox::test]
async fn settling_needs_the_right_the_direction_and_a_bounced_cheque() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    let depositor = t
        .user(
            "Dina",
            &[
                wl_payments::permissions::CHEQUES_VIEW,
                wl_payments::permissions::CHEQUES_DEPOSIT,
            ],
        )
        .await;
    let bounced = t
        .bounced_cheque(&staff, BLUE_DUNES, "3003", 1_000_000, None, "")
        .await;
    t.settle_cheque(&depositor, &bounced, "cash", "100", "")
        .await
        .assert_forbidden();
    t.replace_cheque(&depositor, &bounced).await.assert_forbidden();
    t.link_replacement(&depositor, &bounced, "payment", 1)
        .await
        .assert_forbidden();

    let held = t.received_cheque(BLUE_DUNES, "3004", 1_000_000, None).await;
    t.settle_cheque(&staff, &held, "cash", "100", "")
        .await
        .assert_status(422);

    let entity = t.entity("PGP").await;
    let issued = t
        .register_cheque(
            RegisterCheque::new(BLUE_DUNES, entity.id, "ENBD", "3005", today(), 1_050_000).issued(),
        )
        .await;
    let issued = t.deposit_cheque(&staff, &issued).await;
    t.bounce_cheque(&staff, &issued, "insufficient_funds", "", "")
        .await
        .assert_ok();
    let issued = t.cheque(issued.id).await;
    t.settle_cheque(&staff, &issued, "cash", "100", "")
        .await
        .assert_status(422);
}

/// AC4: replacements that cover less than the open amount do not allow
/// `replace` (`FIN.ACL.02`); once they cover it, it does.
#[renox::test]
async fn a_cheque_is_replaced_only_when_the_replacements_cover_the_open_amount() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    let rent = t.invoices.add(BLUE_DUNES, "INV-1", 1_000_000);
    let original = t
        .bounced_cheque(&staff, BLUE_DUNES, "3010", 1_000_000, Some(rent), "")
        .await;

    // Nothing linked.
    t.replace_cheque(&staff, &original).await.assert_status(422);
    // AED 9,000.00 against AED 10,000.00.
    let short = t.received_cheque(BLUE_DUNES, "3011", 900_000, Some(rent)).await;
    t.link_replacement(&staff, &original, "cheque", short.id)
        .await
        .assert_ok();
    t.replace_cheque(&staff, &original).await.assert_status(422);
    assert_eq!(t.cheque(original.id).await.status, ChequeStatus::Bounced);
    assert!(t.cheque_resolved.all().is_empty());

    // AED 1,000.00 more.
    let rest = t.received_cheque(BLUE_DUNES, "3012", 100_000, Some(rent)).await;
    t.link_replacement(&staff, &original, "cheque", rest.id)
        .await
        .assert_ok();
    t.replace_cheque(&staff, &original).await.assert_ok();
    assert_eq!(t.cheque(original.id).await.status, ChequeStatus::Replaced);
    let links = ChequeReplacement::of_cheque(t.db(), original.id).await.unwrap();
    assert_eq!(links.iter().map(|l| l.amount).sum::<i64>(), 1_000_000);
}

/// A replacement belongs to the same party, direction and currency, is not
/// the cheque itself, and replaces one cheque only.
#[renox::test]
async fn a_replacement_is_checked() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    let original = t
        .bounced_cheque(&staff, BLUE_DUNES, "3020", 1_000_000, None, "")
        .await;
    let other_original = t
        .bounced_cheque(&staff, BLUE_DUNES, "3021", 500_000, None, "")
        .await;

    t.link_replacement(&staff, &original, "cheque", original.id)
        .await
        .assert_status(422);
    let stranger = t.received_cheque(QUIET_SANDS, "3022", 1_000_000, None).await;
    t.link_replacement(&staff, &original, "cheque", stranger.id)
        .await
        .assert_status(422);
    t.link_replacement(&staff, &original, "cheque", other_original.id)
        .await
        .assert_status(422);
    t.link_replacement(&staff, &original, "cheque", 9_999)
        .await
        .assert_status(404);
    t.link_replacement(&staff, &original, "payment", 9_999)
        .await
        .assert_status(404);
    t.link_replacement(&staff, &original, "loan", 1)
        .await
        .assert_status(422);

    let strangers_payment = t.receive(QUIET_SANDS, 1_000_000).await;
    t.link_replacement(&staff, &original, "payment", strangers_payment.id)
        .await
        .assert_status(422);
    let entity = t.entity("PGP").await;
    let outgoing = t
        .receive_request(
            RecordPayment::new(BLUE_DUNES, entity.id, PaymentMethod::Transfer, 1_000_000)
                .outgoing()
                .reference("TRF-OUT"),
        )
        .await;
    t.link_replacement(&staff, &original, "payment", outgoing.id)
        .await
        .assert_status(422);

    // One replacement replaces one cheque.
    let good = t.received_cheque(BLUE_DUNES, "3023", 1_000_000, None).await;
    t.link_replacement(&staff, &original, "cheque", good.id)
        .await
        .assert_ok();
    t.link_replacement(&staff, &original, "cheque", good.id)
        .await
        .assert_status(422);
    let third = t
        .bounced_cheque(&staff, BLUE_DUNES, "3024", 1_000_000, None, "")
        .await;
    t.link_replacement(&staff, &third, "cheque", good.id)
        .await
        .assert_status(422);
    // Only a bounced cheque takes replacements.
    let held = t.received_cheque(BLUE_DUNES, "3025", 1_000_000, None).await;
    t.link_replacement(&staff, &held, "cheque", good.id)
        .await
        .assert_status(422);
}

/// AC3 (replacement cheque): linking a new cheque for the whole open amount
/// and replacing gives `replaced` and `ChequeBounceResolved` (replaced), and
/// the bounce tasks are done; the original is handed back with a receipt only
/// once the new cheque clears.
#[renox::test]
async fn the_original_is_handed_back_with_a_receipt_when_the_replacement_clears() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    t.accountant("Cara").await;
    let rent = t.invoices.add(BLUE_DUNES, "INV-1", 2_000_000);
    let original = t
        .bounced_cheque(&staff, BLUE_DUNES, "3030", 2_000_000, Some(rent), "6000")
        .await;
    t.settle_cheque(&staff, &original, "cash", "4000.00", "")
        .await
        .assert_ok();
    let original = t.cheque(original.id).await;
    assert_eq!(original.open_amount, 1_000_000);
    assert_eq!(tasks_of(&t, &original, FollowupKind::FollowUpCheque).await.len(), 1);

    let new = t.received_cheque(BLUE_DUNES, "3031", 1_000_000, Some(rent)).await;
    t.link_replacement(&staff, &original, "cheque", new.id)
        .await
        .assert_ok();
    t.replace_cheque(&staff, &original).await.assert_ok();

    let replaced = t.cheque(original.id).await;
    assert_eq!(replaced.status, ChequeStatus::Replaced);
    let resolved = t.cheque_resolved.all();
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].cheque, ChequeId(original.id));
    assert_eq!(resolved[0].outcome, ChequeOutcome::Replaced);
    assert!(resolved[0].others_open.is_empty());
    let follow_up = tasks_of(&t, &replaced, FollowupKind::FollowUpCheque).await;
    assert_eq!(follow_up[0].status, FollowupStatus::Done);
    // Kept until the replacement clears.
    assert!(tasks_of(&t, &replaced, FollowupKind::HandBack).await.is_empty());

    let new = t.deposit_cheque(&staff, &new).await;
    t.cheque_action(&staff, &new, "clear", &[]).await.assert_ok();
    let hand_back = tasks_of(&t, &replaced, FollowupKind::HandBack).await;
    assert_eq!(hand_back.len(), 1);
    assert_eq!(hand_back[0].status, FollowupStatus::Open);
    assert_eq!(hand_back[0].role, "accountant");
    let receipt = format!("{}-{}-0001", HAND_BACK_SERIES.prefix, year());
    assert_eq!(hand_back[0].evidence.0["receipt"], receipt.as_str());
    assert!(hand_back[0].text.contains(&receipt));
    assert!(hand_back[0].text.contains("3030"));
    // The original stays replaced.
    assert_eq!(t.cheque(original.id).await.status, ChequeStatus::Replaced);
    assert_eq!(t.cheque(new.id).await.status, ChequeStatus::Cleared);
}

/// A replacement made of payments is already in: the original is handed back
/// at once.
#[renox::test]
async fn a_replacement_payment_has_the_original_handed_back_at_once() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    t.accountant("Cara").await;
    let original = t
        .bounced_cheque(&staff, BLUE_DUNES, "3040", 1_000_000, None, "")
        .await;
    let payment = t.receive(BLUE_DUNES, 1_000_000).await;
    t.link_replacement(&staff, &original, "payment", payment.id)
        .await
        .assert_ok();
    t.replace_cheque(&staff, &original).await.assert_ok();
    assert_eq!(t.cheque(original.id).await.status, ChequeStatus::Replaced);
    let hand_back = tasks_of(&t, &original, FollowupKind::HandBack).await;
    assert_eq!(hand_back.len(), 1);
    assert!(hand_back[0].evidence.0["receipt"].as_str().is_some());
    // The payment replaces one cheque only.
    let another = t
        .bounced_cheque(&staff, BLUE_DUNES, "3041", 1_000_000, None, "")
        .await;
    t.link_replacement(&staff, &another, "payment", payment.id)
        .await
        .assert_status(422);
}

/// If a replacement cheque bounces, the original stays `replaced`, is kept
/// with a note and is not handed back; the new cheque follows its own bounce
/// flow.
#[renox::test]
async fn a_replacement_that_bounces_keeps_the_original_with_a_note() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    t.accountant("Cara").await;
    let original = t
        .bounced_cheque(&staff, BLUE_DUNES, "3050", 1_000_000, None, "")
        .await;
    let new = t.received_cheque(BLUE_DUNES, "3051", 1_000_000, None).await;
    t.link_replacement(&staff, &original, "cheque", new.id)
        .await
        .assert_ok();
    t.replace_cheque(&staff, &original).await.assert_ok();

    let new = t.deposit_cheque(&staff, &new).await;
    t.bounce_cheque(&staff, &new, "insufficient_funds", "CERT-9", "")
        .await
        .assert_ok();
    let kept = t.cheque(original.id).await;
    assert_eq!(kept.status, ChequeStatus::Replaced);
    let note = kept.custody_note.expect("a note");
    assert!(note.contains("3051"), "{note}");
    assert!(tasks_of(&t, &kept, FollowupKind::HandBack).await.is_empty());
    // The new cheque is bounced on its own and ChequeBounced was emitted.
    assert_eq!(t.cheque(new.id).await.status, ChequeStatus::Bounced);
    assert_eq!(t.cheque_bounced.all().len(), 2);
    assert_eq!(tasks_of(&t, &new, FollowupKind::FollowUpCheque).await.len(), 1);
}

/// AC5: a bounced issued cheque of AED 10,500.00 is replaced by an outgoing
/// transfer of the same amount; a part payment is refused (received cheques
/// only), and no hand-back is made.
#[renox::test]
async fn an_issued_cheque_is_replaced_by_an_outgoing_payment() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    t.accountant("Cara").await;
    let entity = t.entity("PGP").await;
    let issued = t
        .register_cheque(
            RegisterCheque::new(BLUE_DUNES, entity.id, "ENBD", "3060", today(), 1_050_000)
                .issued()
                .link(wl_kernel::Subject::new("invoicing.supplier_bill", 4)),
        )
        .await;
    let issued = t.deposit_cheque(&staff, &issued).await;
    t.bounce_cheque(&staff, &issued, "insufficient_funds", "", "")
        .await
        .assert_ok();
    let issued = t.cheque(issued.id).await;
    assert_eq!(tasks_of(&t, &issued, FollowupKind::ReplaceCheque).await.len(), 1);

    t.settle_cheque(&staff, &issued, "cash", "100", "")
        .await
        .assert_status(422);
    // An incoming payment cannot replace it; an outgoing transfer of the
    // whole amount does.
    let incoming = t.receive(BLUE_DUNES, 1_050_000).await;
    t.link_replacement(&staff, &issued, "payment", incoming.id)
        .await
        .assert_status(422);
    t.replace_cheque(&staff, &issued).await.assert_status(422);
    let transfer = t
        .receive_request(
            RecordPayment::new(BLUE_DUNES, entity.id, PaymentMethod::Transfer, 1_050_000)
                .outgoing()
                .reference("TRF-2"),
        )
        .await;
    t.link_replacement(&staff, &issued, "payment", transfer.id)
        .await
        .assert_ok();
    t.replace_cheque(&staff, &issued).await.assert_ok();

    let replaced = t.cheque(issued.id).await;
    assert_eq!(replaced.status, ChequeStatus::Replaced);
    let tasks = tasks_of(&t, &replaced, FollowupKind::ReplaceCheque).await;
    assert_eq!(tasks[0].status, FollowupStatus::Done);
    assert!(tasks_of(&t, &replaced, FollowupKind::HandBack).await.is_empty());
    assert!(t.cheque_resolved.all().is_empty(), "an issued cheque holds no credit");
}
