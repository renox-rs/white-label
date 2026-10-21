//! BL-LOG-15 (#313), 03 §4.1 *Remittance wrong after payment*, §7.1 *COD
//! remittance*, `LOG.ACL.12`: a paid remittance is never edited; a
//! correction is a signed adjustment line with a reason on the client's next
//! draft, prepared by one user and approved by another with
//! `remittances.approve`. A rejected line stays unapproved and is not
//! counted, and a remittance with an unapproved line cannot be approved.

mod support;

use logistics::adjustment::{self, AdjustmentLine, AdjustmentStatus};
use logistics::{Remittance, RemittanceStatus, Shipment};
use renox::prelude::*;

use support::{SOUQ, World, boot_with_payments};

const ACCOUNT: &str = "AE070331234567890123456";

/// R-10: a paid remittance of Souq Express with the delivered shipment
/// `ORD-1` (COD AED 100).
async fn paid_remittance(w: &World) -> (Remittance, Shipment) {
    w.cod_terms(SOUQ, 0, Some(ACCOUNT)).await;
    let shipment = w.delivered(SOUQ, "ORD-1", "100").await;
    w.rule().await;
    let draft = w.the_remittance_of(SOUQ).await;
    w.post(
        &w.bashar,
        &format!("/remittances/{}/approve", draft.id),
        &[],
    )
    .await
    .assert_status(303);
    w.post(
        &w.amal,
        &format!("/remittances/{}/pay", draft.id),
        &[("payment_reference", "TRF-0010")],
    )
    .await
    .assert_status(303);
    let paid = w.remittance(&draft).await;
    assert_eq!(paid.status, RemittanceStatus::Paid);
    (paid, shipment)
}

/// The adjustment lines of Souq Express, oldest first.
async fn lines(w: &World) -> Vec<AdjustmentLine> {
    AdjustmentLine::query()
        .where_(AdjustmentLine::CLIENT_PARTY_ID.eq(SOUQ.0))
        .order_by(AdjustmentLine::ID)
        .get(w.db())
        .await
        .unwrap()
}

/// Amal adds a line of `amount` dirhams with `reason`.
async fn add_line(w: &World, amount: &str, reason: &str, more: &[(&str, &str)]) -> AdjustmentLine {
    let mut fields = vec![
        ("client_party_id", "501"),
        ("amount", amount),
        ("reason", reason),
    ];
    fields.extend_from_slice(more);
    w.post(&w.amal, "/remittances/adjustments", &fields)
        .await
        .assert_status(303);
    lines(w).await.pop().expect("the line is added")
}

fn path(line: &AdjustmentLine, action: &str) -> String {
    format!("/remittances/adjustments/{}/{action}", line.id)
}

async fn reload(w: &World, line: &AdjustmentLine) -> AdjustmentLine {
    AdjustmentLine::find_or_404(w.db(), line.id).await.unwrap()
}

/// AC1: R-10 paid with a COD line wrong by AED 70: accountant A adds a
/// +AED 70 line with the reason "COD under-remitted on ORD-1", accountant B
/// approves it, the next draft shows +AED 70 and the next statement shows the
/// line and its reason (not the approver's internal note).
#[renox::test]
async fn a_line_added_after_payment_is_approved_by_another_user_and_goes_on_the_next_draft() {
    let w = boot_with_payments().await;
    let (paid, shipment) = paid_remittance(&w).await;
    let shipment_id = shipment.id.to_string();
    let corrects = paid.id.to_string();

    let line = add_line(
        &w,
        "70",
        "COD under-remitted on ORD-1",
        &[
            ("shipment_id", shipment_id.as_str()),
            ("corrects_remittance_id", corrects.as_str()),
        ],
    )
    .await;
    assert_eq!(line.amount, 7_000);
    assert_eq!(line.status, AdjustmentStatus::Pending);
    assert_eq!(line.prepared_by, Some(w.amal.id));
    assert_eq!(line.remittance_id, None, "no draft yet: the line is open");
    // Adding a line starts no draft before the weekly run.
    assert_eq!(w.remittances_of(SOUQ).await.len(), 1);

    w.post(
        &w.bashar,
        &path(&line, "approve"),
        &[("note", "internal: checked the scale slip")],
    )
    .await
    .assert_status(303);
    let approved = reload(&w, &line).await;
    assert_eq!(approved.status, AdjustmentStatus::Approved);
    assert_eq!(approved.decided_by, Some(w.bashar.id));

    // The next Monday: a new shipment and the open line make the next draft.
    w.delivered(SOUQ, "ORD-2", "200").await;
    w.rule().await;
    let all = w.remittances_of(SOUQ).await;
    assert_eq!(all.len(), 2);
    let next = &all[1];
    assert_eq!(next.status, RemittanceStatus::Draft);
    assert_eq!(next.adjustment_total, 7_000);
    assert_eq!(next.net, 27_000);
    assert_eq!(reload(&w, &line).await.remittance_id, Some(next.id));

    // The statement: date, amount, reason and what it corrects.
    let statement = adjustment::statement_lines(w.db(), next).await.unwrap();
    assert_eq!(statement.len(), 1);
    assert_eq!(statement[0].amount, 7_000);
    assert_eq!(statement[0].reason, "COD under-remitted on ORD-1");
    assert_eq!(statement[0].corrects, shipment.number);
    assert!(!statement[0].date.is_empty());
    assert!(!format!("{statement:?}").contains("internal"));
    w.get(&w.amal, &format!("/remittances/{}", next.id))
        .await
        .assert_ok()
        .assert_see("COD under-remitted on ORD-1");
}

/// AC2: the preparer cannot approve the line, and the remittance with the
/// unapproved line cannot be approved; once the line is approved by the other
/// user, it can.
#[renox::test]
async fn the_preparer_cannot_approve_the_line_and_an_unapproved_line_blocks_the_remittance() {
    let w = boot_with_payments().await;
    w.delivered(SOUQ, "ORD-1", "100").await;
    w.rule().await;
    let draft = w.the_remittance_of(SOUQ).await;

    let line = add_line(&w, "70", "COD under-remitted on ORD-1", &[]).await;
    assert_eq!(
        line.remittance_id,
        Some(draft.id),
        "the client's draft takes it"
    );

    w.post(&w.amal, &path(&line, "approve"), &[])
        .await
        .assert_status(422)
        .assert_see("You prepared this adjustment line");
    assert_eq!(reload(&w, &line).await.status, AdjustmentStatus::Pending);
    let approve = format!("/remittances/{}/approve", draft.id);
    w.post(&w.bashar, &approve, &[])
        .await
        .assert_status(422)
        .assert_see("1 adjustment line(s)");
    assert_eq!(w.remittance_status(&draft).await, RemittanceStatus::Draft);
    // A pending line is not counted.
    assert_eq!(w.remittance(&draft).await.adjustment_total, 0);

    w.post(&w.bashar, &path(&line, "approve"), &[])
        .await
        .assert_status(303);
    assert_eq!(w.remittance(&draft).await.adjustment_total, 7_000);
    assert_eq!(w.remittance(&draft).await.net, 17_000);
    w.post(&w.bashar, &approve, &[]).await.assert_status(303);
    assert_eq!(
        w.remittance_status(&draft).await,
        RemittanceStatus::Approved
    );
}

/// AC3: when B rejects the line, it stays unapproved and is not counted; the
/// remittance still cannot be approved; the preparer edits and resubmits it,
/// and another user's approval makes it count. Another user cannot resubmit
/// it and an approved line cannot be edited or deleted.
#[renox::test]
async fn a_rejected_line_is_not_counted_until_it_is_edited_resubmitted_and_approved() {
    let w = boot_with_payments().await;
    w.delivered(SOUQ, "ORD-1", "100").await;
    w.rule().await;
    let draft = w.the_remittance_of(SOUQ).await;
    let line = add_line(&w, "70", "COD under-remitted on ORD-1", &[]).await;

    w.post(
        &w.bashar,
        &path(&line, "reject"),
        &[("note", "The amount is wrong")],
    )
    .await
    .assert_status(303);
    let rejected = reload(&w, &line).await;
    assert_eq!(rejected.status, AdjustmentStatus::Rejected);
    assert!(!rejected.counts());
    assert_eq!(
        rejected.decision_note.as_deref(),
        Some("The amount is wrong")
    );
    assert_eq!(w.remittance(&draft).await.adjustment_total, 0);
    assert_eq!(w.remittance(&draft).await.net, 10_000);
    w.post(
        &w.bashar,
        &format!("/remittances/{}/approve", draft.id),
        &[],
    )
    .await
    .assert_status(422);
    // The next run attaches no rejected line a second time and keeps it as it is.
    w.rule().await;
    assert_eq!(reload(&w, &line).await.status, AdjustmentStatus::Rejected);

    // Only the preparer edits and resubmits.
    w.post(
        &w.bashar,
        &path(&line, "resubmit"),
        &[("amount", "50"), ("reason", "COD under-remitted by AED 50")],
    )
    .await
    .assert_status(422)
    .assert_see("Only the user who prepared the line");
    w.post(
        &w.amal,
        &path(&line, "resubmit"),
        &[("amount", "50"), ("reason", "COD under-remitted by AED 50")],
    )
    .await
    .assert_status(303);
    let again = reload(&w, &line).await;
    assert_eq!(again.status, AdjustmentStatus::Pending);
    assert_eq!(again.amount, 5_000);
    assert_eq!(again.reason, "COD under-remitted by AED 50");
    w.post(&w.bashar, &path(&line, "approve"), &[])
        .await
        .assert_status(303);
    assert_eq!(w.remittance(&draft).await.adjustment_total, 5_000);

    // An approved line is never edited or deleted.
    w.post(
        &w.amal,
        &path(&line, "resubmit"),
        &[("amount", "90"), ("reason", "Changed")],
    )
    .await
    .assert_status(422);
    w.post(&w.amal, &path(&line, "delete"), &[])
        .await
        .assert_status(422)
        .assert_see("never deleted or edited");
    let kept = reload(&w, &line).await;
    assert_eq!(kept.amount, 5_000);
    assert_eq!(kept.status, AdjustmentStatus::Approved);
}

/// The preparer deletes an unapproved line; the draft is worked out again.
#[renox::test]
async fn the_preparer_deletes_an_unapproved_line() {
    let w = boot_with_payments().await;
    w.delivered(SOUQ, "ORD-1", "100").await;
    w.rule().await;
    let draft = w.the_remittance_of(SOUQ).await;
    let line = add_line(&w, "-30", "Delivery reversed", &[]).await;

    w.post(&w.bashar, &path(&line, "delete"), &[])
        .await
        .assert_status(422);
    w.post(&w.amal, &path(&line, "delete"), &[])
        .await
        .assert_status(303);
    assert!(lines(&w).await.is_empty());
    w.post(
        &w.bashar,
        &format!("/remittances/{}/approve", draft.id),
        &[],
    )
    .await
    .assert_status(303);
}

/// AC4: R-10 cannot be edited: a line cannot be put on it, it cannot be
/// approved, reopened, paid or rebuilt, and nothing about it changes. An
/// approved remittance is reopened first.
#[renox::test]
async fn a_paid_remittance_cannot_be_edited() {
    let w = boot_with_payments().await;
    let (paid, _shipment) = paid_remittance(&w).await;
    let target = paid.id.to_string();

    w.post(
        &w.amal,
        "/remittances/adjustments",
        &[
            ("client_party_id", "501"),
            ("amount", "70"),
            ("reason", "COD under-remitted"),
            ("remittance_id", target.as_str()),
        ],
    )
    .await
    .assert_status(422)
    .assert_see("is paid and is never edited");
    for action in ["approve", "reopen"] {
        w.post(
            &w.bashar,
            &format!("/remittances/{}/{action}", paid.id),
            &[],
        )
        .await
        .assert_status(422);
    }
    w.post(
        &w.amal,
        &format!("/remittances/{}/pay", paid.id),
        &[("payment_reference", "TRF-0011")],
    )
    .await
    .assert_status(422);
    w.post(&w.amal, "/remittances/build", &[("client_party_id", "501")])
        .await
        .assert_status(422);
    // A rebuild by the weekly rule leaves it alone as well.
    w.rule().await;

    let after = w.remittance(&paid).await;
    assert_eq!(after.status, RemittanceStatus::Paid);
    assert_eq!(after.net, paid.net);
    assert_eq!(after.cod_total, paid.cod_total);
    assert_eq!(after.adjustment_total, 0);
    assert_eq!(after.payment_reference, paid.payment_reference);
    assert_eq!(after.lines(w.db()).await.unwrap().len(), 1);
    assert!(lines(&w).await.is_empty());
    assert_eq!(w.remittances_of(SOUQ).await.len(), 1);
}

/// A line cannot be put on an approved remittance either: it is reopened to a
/// draft first, and then takes it.
#[renox::test]
async fn an_approved_remittance_is_reopened_before_a_line_goes_on_it() {
    let w = boot_with_payments().await;
    w.delivered(SOUQ, "ORD-1", "100").await;
    w.rule().await;
    let draft = w.the_remittance_of(SOUQ).await;
    w.post(
        &w.bashar,
        &format!("/remittances/{}/approve", draft.id),
        &[],
    )
    .await
    .assert_status(303);
    let target = draft.id.to_string();
    let fields = [
        ("client_party_id", "501"),
        ("amount", "-20"),
        ("reason", "Delivery reversed"),
        ("remittance_id", target.as_str()),
    ];

    w.post(&w.amal, "/remittances/adjustments", &fields)
        .await
        .assert_status(422)
        .assert_see("reopen it to a draft");
    w.post(&w.bashar, &format!("/remittances/{}/reopen", draft.id), &[])
        .await
        .assert_status(303);
    w.post(&w.amal, "/remittances/adjustments", &fields)
        .await
        .assert_status(303);
    assert_eq!(lines(&w).await[0].remittance_id, Some(draft.id));
}

/// Who may prepare and who may decide: a dispatcher does neither, the line
/// needs a reason and an amount, and a client's shipment is the only one it
/// can correct.
#[renox::test]
async fn the_permissions_and_the_inputs_of_a_line_are_checked() {
    let w = boot_with_payments().await;
    let other = w.delivered(support::OTHER_CLIENT, "ORD-9", "100").await;
    let fields = [
        ("client_party_id", "501"),
        ("amount", "70"),
        ("reason", "COD under-remitted"),
    ];
    w.post(&w.dispatcher, "/remittances/adjustments", &fields)
        .await
        .assert_forbidden();
    w.post(
        &w.amal,
        "/remittances/adjustments",
        &[("client_party_id", "501"), ("amount", "0"), ("reason", "x")],
    )
    .await
    .assert_status(422)
    .assert_see("Give the amount");
    w.post(
        &w.amal,
        "/remittances/adjustments",
        &[
            ("client_party_id", "501"),
            ("amount", "70"),
            ("reason", " "),
        ],
    )
    .await
    .assert_status(422)
    .assert_see("Give the reason");
    let other_id = other.id.to_string();
    w.post(
        &w.amal,
        "/remittances/adjustments",
        &[
            ("client_party_id", "501"),
            ("amount", "70"),
            ("reason", "x"),
            ("shipment_id", other_id.as_str()),
        ],
    )
    .await
    .assert_status(422)
    .assert_see("not one of the client");
    w.post(
        &w.amal,
        "/remittances/adjustments",
        &[
            ("client_party_id", "501"),
            ("amount", "abc"),
            ("reason", "x"),
        ],
    )
    .await
    .assert_status(422);
    assert!(lines(&w).await.is_empty());

    let line = add_line(&w, "70", "COD under-remitted", &[]).await;
    w.post(&w.dispatcher, &path(&line, "approve"), &[])
        .await
        .assert_forbidden();
    w.post(&w.dispatcher, &path(&line, "reject"), &[])
        .await
        .assert_forbidden();
}
