//! BL-LOG-18 (#316): flag and resolve a COD statement dispute, 03 §4.1 row
//! *Client disputes a COD statement*, §7.1 rows `dispute` and
//! `resolve_dispute`, `LOG.ACL.12`. The remittance and its payout are real
//! (wl-payments); the ticket is recorded by the test desk.

mod support;

use logistics::dispute::StatementDispute;
use logistics::{AdjustmentLine, AdjustmentStatus, Remittance, RemittanceStatus};
use renox::prelude::*;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::tickets::{Assignee, TicketId};

use logistics::permissions as perm;
use support::{SOUQ, World, boot_with_payments};

const ACCOUNT: &str = "AE070331234567890123456";

/// R-14: AED 500 of COD for Souq Express, drafted, approved by Bashar and
/// paid by Amal; Amal holds the accountant role (the ticket goes to her).
async fn paid() -> (World, Remittance) {
    let w = boot_with_payments().await;
    renox::auth::permissions::define_role(
        w.db(),
        perm::INVOICE_OWNER_ROLE,
        &[perm::REMITTANCES_VIEW],
    )
    .await
    .unwrap();
    w.amal
        .assign_role(w.db(), perm::INVOICE_OWNER_ROLE)
        .await
        .unwrap();
    w.cod_terms(SOUQ, 0, Some(ACCOUNT)).await;
    w.delivered(SOUQ, "ORD-1", "500").await;
    w.rule().await;
    let draft = w.the_remittance_of(SOUQ).await;
    approve_and_pay(&w, &draft, "TRF-1").await;
    let paid = w.remittance(&draft).await;
    assert_eq!(paid.status, RemittanceStatus::Paid);
    (w, paid)
}

async fn approve_and_pay(w: &World, draft: &Remittance, reference: &str) {
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
        &[("payment_reference", reference)],
    )
    .await
    .assert_status(303);
}

fn dispute_path(r: &Remittance) -> String {
    format!("/remittances/{}/dispute", r.id)
}

fn resolve_path(r: &Remittance) -> String {
    format!("/remittances/{}/resolve-dispute", r.id)
}

/// A second delivered shipment and the weekly rule: the next draft.
async fn next_draft(w: &World) -> Remittance {
    w.delivered(SOUQ, "ORD-2", "200").await;
    w.rule().await;
    w.remittances_of(SOUQ)
        .await
        .into_iter()
        .find(|r| r.status == RemittanceStatus::Draft)
        .expect("the next draft")
}

/// Customer service: views remittances and flags a dispute.
async fn customer_service(w: &World) -> User {
    w.user_with("Cora", &[perm::REMITTANCES_VIEW, perm::REMITTANCES_DISPUTE])
        .await
}

/// AC1: given remittance R-14 `paid`, when the client user disputes with
/// reason "shipment S-3001 missing", R-14 has `disputed` set and the
/// accountant has a ticket.
#[renox::test]
async fn the_client_disputes_a_paid_statement_and_the_accountant_gets_a_ticket() {
    let (w, r14) = paid().await;
    w.post(
        &w.noor,
        &dispute_path(&r14),
        &[("reason", "shipment S-3001 missing")],
    )
    .await
    .assert_status(303);

    let flagged = w.remittance(&r14).await;
    assert!(flagged.disputed);
    assert_eq!(flagged.status, RemittanceStatus::Paid, "no state changes");
    let opened = w.tickets.opened();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].subject, flagged.subject());
    assert_eq!(
        opened[0].source_key.as_deref(),
        Some(format!("logistics.statement_dispute:{}", r14.id).as_str())
    );
    assert!(opened[0].description.contains("shipment S-3001 missing"));
    assert_eq!(opened[0].assignee, Some(Assignee::user(w.amal.id)));
    assert_eq!(flagged.dispute_ticket_id, Some(1));
    let kept = StatementDispute::query().get(w.db()).await.unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].via, "portal");
    assert_eq!(kept[0].raised_by, w.noor.id);

    // The client's statement shows the flag, and no second dispute is offered.
    w.get(&w.noor, &format!("/remittances/{}/statement", r14.id))
        .await
        .assert_ok()
        .assert_see("\"disputed\":true")
        .assert_see("\"dispute_offered\":false");
}

/// Only the people who may dispute can: another client's user, hub staff
/// and a client user on an `approved` or `draft` remittance cannot; customer
/// service flags an `approved` one on the client's call.
#[renox::test]
async fn who_may_dispute_which_remittance() {
    let (w, r14) = paid().await;
    w.post(&w.farah, &dispute_path(&r14), &[("reason", "not ours")])
        .await
        .assert_forbidden();
    w.post(&w.hub, &dispute_path(&r14), &[("reason", "not ours")])
        .await
        .assert_forbidden();
    w.post(&w.noor, &dispute_path(&r14), &[("reason", " ")])
        .await
        .assert_status(422);
    assert!(!w.remittance(&r14).await.disputed);

    // The next remittance: a draft cannot be disputed, an approved one only
    // by customer service on the client's call.
    let next = next_draft(&w).await;
    let service = customer_service(&w).await;
    w.post(&service, &dispute_path(&next), &[("reason", "call")])
        .await
        .assert_status(422)
        .assert_see("no statement to dispute yet");
    w.post(&w.bashar, &format!("/remittances/{}/approve", next.id), &[])
        .await
        .assert_status(303);
    w.post(&w.noor, &dispute_path(&next), &[("reason", "early")])
        .await
        .assert_status(422)
        .assert_see("published when the remittance is paid");
    w.post(
        &service,
        &dispute_path(&next),
        &[("reason", "The client called about the total")],
    )
    .await
    .assert_status(303);
    let flagged = w.remittance(&next).await;
    assert!(flagged.disputed);
    assert_eq!(flagged.status, RemittanceStatus::Approved);
    let kept = StatementDispute::query()
        .where_(StatementDispute::REMITTANCE_ID.eq(next.id))
        .first(w.db())
        .await
        .unwrap()
        .expect("the dispute is kept");
    assert_eq!(kept.via, "staff");
}

/// A second dispute of a disputed remittance adds its message to the open
/// ticket; no second flag, no second ticket.
#[renox::test]
async fn a_second_dispute_adds_a_message_to_the_open_ticket() {
    let (w, r14) = paid().await;
    w.post(
        &w.noor,
        &dispute_path(&r14),
        &[("reason", "shipment S-3001 missing")],
    )
    .await
    .assert_status(303);
    w.post(
        &w.nadia,
        &dispute_path(&r14),
        &[("reason", "and S-3002 too")],
    )
    .await
    .assert_status(303);
    assert_eq!(w.tickets.opened().len(), 1);
    let comments = w.tickets.comments();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].0, TicketId(1));
    assert_eq!(comments[0].1.text, "and S-3002 too");
    assert_eq!(StatementDispute::query().count(w.db()).await.unwrap(), 1);
}

/// `dispute` is not offered while the transfer is returned: the client's
/// question is a normal ticket.
#[renox::test]
async fn a_dispute_is_not_offered_while_the_transfer_is_returned() {
    let (w, r14) = paid().await;
    w.post(
        &w.amal,
        &format!("/remittances/{}/return-payment", r14.id),
        &[("advice", "BANK-ADV-1")],
    )
    .await
    .assert_status(303);
    assert_eq!(
        w.remittance_status(&r14).await,
        RemittanceStatus::PaymentReturned
    );
    let service = customer_service(&w).await;
    for who in [&w.noor, &service] {
        w.post(who, &dispute_path(&r14), &[("reason", "where is my money")])
            .await
            .assert_status(422)
            .assert_see("not offered while the transfer is returned");
    }
    assert!(!w.remittance(&r14).await.disputed);
    assert!(w.tickets.opened().is_empty());
    w.get(&w.noor, &format!("/remittances/{}/statement", r14.id))
        .await
        .assert_ok()
        .assert_see("\"dispute_offered\":false");
}

/// AC2: when accountant B resolves *adjusted* with +AED 150, the flag
/// clears, a +AED 150 line is on the next draft awaiting another user's
/// approval, and the client is told.
#[renox::test]
async fn resolving_adjusted_puts_a_line_on_the_next_draft_for_another_user_to_approve() {
    let (w, r14) = paid().await;
    w.post(
        &w.noor,
        &dispute_path(&r14),
        &[("reason", "shipment S-3001 missing")],
    )
    .await
    .assert_status(303);
    let next = next_draft(&w).await;

    // Only a holder of `remittances.approve` resolves; the outcome and the
    // note are checked.
    let service = customer_service(&w).await;
    let resolve = resolve_path(&r14);
    let fields = [
        ("outcome", "adjusted"),
        ("amount", "150"),
        ("note", "Shipment S-3001 is confirmed missing"),
    ];
    w.post(&service, &resolve, &fields).await.assert_forbidden();
    w.post(
        &w.bashar,
        &resolve,
        &[("outcome", "adjusted"), ("amount", "150"), ("note", " ")],
    )
    .await
    .assert_status(422);
    w.post(
        &w.bashar,
        &resolve,
        &[("outcome", "adjusted"), ("amount", ""), ("note", "x")],
    )
    .await
    .assert_status(422);
    w.post(&w.bashar, &resolve, &[("outcome", "maybe"), ("note", "x")])
        .await
        .assert_status(422);
    assert!(w.remittance(&r14).await.disputed);

    w.post(&w.bashar, &resolve, &fields)
        .await
        .assert_status(303);
    let cleared = w.remittance(&r14).await;
    assert!(!cleared.disputed);
    assert_eq!(cleared.dispute_ticket_id, None);
    let lines = AdjustmentLine::query().get(w.db()).await.unwrap();
    assert_eq!(lines.len(), 1);
    let line = &lines[0];
    assert_eq!(line.amount, 15_000);
    assert_eq!(line.source, "statement_dispute");
    assert_eq!(line.corrects_remittance_id, Some(r14.id));
    assert_eq!(line.remittance_id, Some(next.id), "on the next draft");
    assert_eq!(
        line.prepared_by,
        Some(w.bashar.id),
        "prepared by the resolver"
    );
    assert_eq!(line.status, AdjustmentStatus::Pending);
    // The resolver cannot approve it; another user can, and the draft counts it.
    w.post(
        &w.bashar,
        &format!("/remittances/adjustments/{}/approve", line.id),
        &[],
    )
    .await
    .assert_status(422);
    assert_eq!(w.remittance(&next).await.adjustment_total, 0);
    w.post(
        &w.amal,
        &format!("/remittances/adjustments/{}/approve", line.id),
        &[],
    )
    .await
    .assert_status(303);
    assert_eq!(w.remittance(&next).await.adjustment_total, 15_000);

    // The outcome is kept, the ticket closed and the client told.
    let kept = StatementDispute::query()
        .first(w.db())
        .await
        .unwrap()
        .expect("kept");
    assert_eq!(kept.outcome.as_deref(), Some("adjusted"));
    assert_eq!(kept.outcome_amount, Some(15_000));
    assert_eq!(kept.resolved_by, Some(w.bashar.id));
    assert_eq!(kept.adjustment_line_id, Some(line.id));
    assert_eq!(w.tickets.completed().len(), 1);
    assert_eq!(w.tickets.completed()[0].0, TicketId(1));
    let told: Vec<_> = w
        .notices
        .to_clients()
        .into_iter()
        .filter(|request| {
            request.data["title"]
                .as_str()
                .is_some_and(|title| title.contains("Your question about statement"))
        })
        .collect();
    assert_eq!(told.len(), 1);
    assert!(
        told[0]
            .recipients
            .contains(&NotifyRecipient::Party { id: SOUQ })
    );
    assert!(
        told[0].data["body"]
            .as_str()
            .unwrap()
            .contains("AED 150.00")
    );
    // The disputed remittance itself was not edited.
    assert_eq!(cleared.net, r14.net);
    assert_eq!(cleared.status, RemittanceStatus::Paid);
}

/// AC3: resolving *upheld* clears the flag with no line and tells the
/// client; a remittance that is not disputed has nothing to resolve.
#[renox::test]
async fn resolving_upheld_clears_the_flag_without_a_line_and_tells_the_client() {
    let (w, r14) = paid().await;
    let resolve = resolve_path(&r14);
    w.post(
        &w.bashar,
        &resolve,
        &[("outcome", "upheld"), ("note", "It stands")],
    )
    .await
    .assert_status(422)
    .assert_see("no open dispute");
    w.post(
        &w.noor,
        &dispute_path(&r14),
        &[("reason", "shipment S-3001 missing")],
    )
    .await
    .assert_status(303);
    w.post(
        &w.bashar,
        &resolve,
        &[
            ("outcome", "upheld"),
            ("note", "S-3001 was delivered on 3 November, POD attached"),
        ],
    )
    .await
    .assert_status(303);
    assert!(!w.remittance(&r14).await.disputed);
    assert_eq!(AdjustmentLine::query().count(w.db()).await.unwrap(), 0);
    let told: Vec<_> = w
        .notices
        .to_clients()
        .into_iter()
        .filter(|request| {
            request.data["body"]
                .as_str()
                .is_some_and(|body| body.contains("it stands"))
        })
        .collect();
    assert_eq!(told.len(), 1);
    // It can be disputed again, with a new ticket.
    w.post(&w.noor, &dispute_path(&r14), &[("reason", "still missing")])
        .await
        .assert_status(303);
    assert_eq!(w.tickets.opened().len(), 2);
}

/// AC4: while R-14 is disputed, the next Monday's draft is still created
/// and can be approved and paid.
#[renox::test]
async fn a_dispute_pauses_nothing() {
    let (w, r14) = paid().await;
    w.post(
        &w.noor,
        &dispute_path(&r14),
        &[("reason", "shipment S-3001 missing")],
    )
    .await
    .assert_status(303);
    assert!(w.remittance(&r14).await.disputed);

    let next = next_draft(&w).await;
    assert_eq!(next.status, RemittanceStatus::Draft);
    assert_eq!(next.cod_total, 20_000);
    approve_and_pay(&w, &next, "TRF-2").await;
    assert_eq!(w.remittance_status(&next).await, RemittanceStatus::Paid);
    // Still disputed, and not edited.
    let still = w.remittance(&r14).await;
    assert!(still.disputed);
    assert_eq!(still.net, r14.net);
    assert_eq!(still.payment_reference.as_deref(), Some("TRF-1"));
}
