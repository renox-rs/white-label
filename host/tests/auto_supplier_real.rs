//! BL-AUT-11 (#407) against the REAL shared modules of the host (wl-ticketing,
//! wl-inventory, wl-notify, wl-parties): a warranty ticket that closes gives
//! the purchaser a real task, one that only resolves gives none; raising the
//! claim queues a real email to the supplier and puts the kept part on a real
//! draft Return to supplier with outcome credit; a return the supplier refused
//! leaves the claim raised and the purchaser a real write-off task; the
//! write-off waits for the branch manager. The app's own tests
//! (`apps/auto/tests/aut_11_supplier.rs`) show the same points on stand-ins.

mod support;

use auto::supplier_claims::{self, NewSupplierClaim};
use auto::{PartLine, SupplierClaim, SupplierClaimStatus};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use support::autofx::{World, approved_job, boot, sent_of, shop};
use support::company;
use wl_inventory::movement::StockMovement;
use wl_inventory::return_credit::{CreditBasis, CreditOutcome, credit};
use wl_inventory::return_dispatch::{CollectionProof, DispatchOutcome, dispatch};
use wl_inventory::supplier_return::{ReturnStatus, SupplierReturn, WantedOutcome};
use wl_inventory::{Item, Location};
use wl_kernel::documents::today;
use wl_kernel::tickets::{OpenTicket, task_creator};
use wl_kernel::{PartyId, Subject};

/// The tasks of the ticketing module whose source key starts with `prefix`.
async fn tasks(w: &World, prefix: &str) -> Vec<wl_ticketing::Ticket> {
    wl_ticketing::Ticket::query()
        .get(w.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|t| {
            t.source_key
                .as_deref()
                .is_some_and(|k| k.starts_with(prefix))
        })
        .collect()
}

#[renox::test]
async fn aut_11_real_the_claim_is_raised_from_a_closed_ticket_and_the_kept_part_is_drafted_for_return()
 {
    let w = boot().await;
    let s = shop(&w, None).await;
    let job = approved_job(&w, &s, &[("Panel", 1_000, 10_000)], &[]).await;
    let supplier: PartyId = company(&w.app, "Gulf Auto Spares").await;
    // The workshop's store holds the alternator that failed.
    let workshop = Subject::new("scheduling.location", 1);
    let mut item = Item::new("ALT-9", "Alternator", "pcs");
    item.save(w.db()).await.unwrap();
    let mut store = Location::new("WORKSHOP-1", "Workshop 1").in_scope(&workshop);
    store.save(w.db()).await.unwrap();
    let mut received = StockMovement::receive(
        item.id,
        None,
        store.id,
        1_000,
        8_000,
        45_000,
        &Subject::new("inventory.goods_receipt", 900),
        None,
    );
    received.save(w.db()).await.unwrap();
    let mut fitted = PartLine {
        job_card_id: job.card.id,
        description: "Alternator".to_owned(),
        item_code: Some("ALT-9".to_owned()),
        quantity_milli: 1_000,
        unit_price: 45_000,
        approval: auto::LineStatus::Approved,
        fitted_milli: 1_000,
        ..PartLine::default()
    };
    fitted.save(w.db()).await.unwrap();

    // Two warranty tickets of the job card: one is closed, one only resolved.
    let creator = task_creator(w.state());
    let first = creator
        .open(
            w.state(),
            OpenTicket::new(job.card.subject(), "warranty", "Alternator failed"),
        )
        .await
        .unwrap()
        .expect("ticketing is on");
    let second = creator
        .open(
            w.state(),
            OpenTicket::new(job.card.subject(), "warranty", "Starter failed"),
        )
        .await
        .unwrap()
        .expect("ticketing is on");
    assert!(
        creator
            .resolve_from_source(w.state(), second.id, "Accepted")
            .await
            .unwrap()
    );
    assert!(
        tasks(&w, "auto.supplier_claim_task:").await.is_empty(),
        "resolved is not closed"
    );
    assert!(
        creator
            .complete(w.state(), first.id, "Accepted: replace the alternator")
            .await
            .unwrap()
    );
    let made = tasks(&w, "auto.supplier_claim_task:").await;
    assert_eq!(made.len(), 1, "the purchaser has a task on closed");
    assert_eq!(
        made[0].source_key.as_deref(),
        Some(format!("auto.supplier_claim_task:{}", first.id.0).as_str())
    );

    // The purchaser raises the claim.
    let claim = supplier_claims::raise(
        w.state(),
        &s.pia,
        NewSupplierClaim::new(
            first.id.0,
            job.card.subject(),
            supplier.0,
            "Alternator",
            45_000,
            today() - TimeDelta::days(100),
            12,
            today() - TimeDelta::days(5),
        )
        .item("ALT-9", "Bosch"),
    )
    .await
    .unwrap();
    assert_eq!(claim.status, SupplierClaimStatus::Raised);
    let mails = sent_of(&w, "auto.supplier_claim").await;
    assert_eq!(mails.len(), 1, "the claim is emailed to the supplier");
    assert!(mails[0].rendered_body.contains("Alternator"));
    let rts_id = claim.supplier_return_id.expect("a draft return");
    let rts = SupplierReturn::find_or_404(w.db(), rts_id).await.unwrap();
    assert_eq!(rts.status, ReturnStatus::Draft);
    assert_eq!(rts.supplier_party_id, supplier.0);
    assert_eq!(rts.source(), claim.subject());
    let lines = wl_inventory::supplier_return::lines_of(w.db(), rts.id)
        .await
        .unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].wanted_outcome, WantedOutcome::Credit);
    assert_eq!(lines[0].item_id, item.id);

    // The supplier refuses: the return is closed by hand and the claim stays
    // raised, with a write-off task for the purchaser.
    let sent = dispatch(
        w.state(),
        &s.pia,
        rts.id,
        &CollectionProof::DriverSignature("SIG-1".to_owned()),
    )
    .await
    .unwrap();
    assert!(matches!(sent, DispatchOutcome::Dispatched(_)));
    let closed = credit(
        w.state(),
        &s.pia,
        rts.id,
        CreditBasis::SupplierRefused,
        "supplier refused: water damage",
    )
    .await
    .unwrap();
    assert!(matches!(closed, CreditOutcome::Credited(_)));
    let after = SupplierClaim::find_or_404(w.db(), claim.id).await.unwrap();
    assert_eq!(after.status, SupplierClaimStatus::Raised, "not recovered");
    assert_eq!(
        tasks(&w, &format!("auto.supplier_claim_write_off:{}", claim.id))
            .await
            .len(),
        1
    );

    // The write-off waits for the branch manager.
    let asked = supplier_claims::write_off(w.state(), &s.pia, claim.id, "Supplier refused")
        .await
        .unwrap();
    assert_eq!(asked.claim.status, SupplierClaimStatus::Raised);
    let request = asked.approval_request.expect("an approval request");
    let basma = wl_kernel::approvals::awaiting(w.db(), &basma_user(&w).await)
        .await
        .unwrap();
    assert!(basma.iter().any(|r| r.id == request));
}

/// The branch manager Basma, as the user the approvals list for.
async fn basma_user(w: &World) -> User {
    User::find_by_email(w.db(), "basma@example.com")
        .await
        .unwrap()
        .expect("the branch manager exists")
}
