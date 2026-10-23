//! BL-AUT-11 (#407): raise and recover a supplier warranty claim (D-06, D-07;
//! `AUT.WAR.02`, `AUT.RUL.07`, `AUT.RUL.08`, 08 §4.6 step 4, §7.1 *Supplier
//! warranty claim*), with the stand-ins of the ticket book, the notifier and
//! the Return to supplier drafts. The same points against the real ticketing,
//! inventory and invoicing are in `host/tests/auto_supplier_real.rs`.
//!
//! D-06 and D-07 are open items built with the default the issue states: the
//! task comes on a ticket `closed`, never on `resolved`, and the only states are
//! `raised`, `recovered` and `written_off`.

mod support;

use auto::supplier_claims::{self, NewSupplierClaim};
use auto::supplier_rules;
use auto::{JobCardStatus, PartLine, SupplierClaim, SupplierClaimStatus};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use support::{Garage, Messages, at, boot, day};
use wl_kernel::approvals;
use wl_kernel::inventory::SupplierReturnCredited;
use wl_kernel::tickets::{TicketClosed, TicketId};
use wl_kernel::{PartyId, Subject};

/// The purchaser Pia: raises and recovers supplier claims.
async fn pia(t: &Garage) -> AuthUser {
    let user = t
        .user(
            "Pia",
            &[
                auto::permissions::SUPPLIER_CLAIMS_MANAGE,
                auto::permissions::JOB_CARDS_VIEW,
            ],
        )
        .await;
    t.auth(&user).await
}

/// The delivered job card the warranty tickets are about, with an alternator
/// fitted on it.
async fn delivered_with_alternator(t: &Garage) -> Subject {
    t.people.add(40, "Gulf Auto Spares");
    t.stock.with_store();
    let vehicle = t.hilux().await;
    let card = t.card_in(&vehicle, JobCardStatus::Delivered).await;
    let part = t
        .approved_part(&card, "Alternator", Some("ALT-9"), 1_000, 45_000)
        .await;
    PartLine::query()
        .where_(PartLine::ID.eq(part.id))
        .update(t.db(), &[("fitted_milli", &1_000_i64)])
        .await
        .unwrap();
    card.subject()
}

/// An alternator (item ALT-9, AED 450) bought on `bought` with 12 months of
/// warranty that failed ten days ago.
fn alternator(ticket: i64, about: &Subject, bought: NaiveDate) -> NewSupplierClaim {
    NewSupplierClaim::new(
        ticket,
        about.clone(),
        40,
        "Alternator",
        45_000,
        bought,
        12,
        day(-10),
    )
    .item("ALT-9", "Bosch")
}

/// The claim was raised `days` days ago.
async fn raised_days_ago(t: &Garage, id: i64, days: i64) {
    SupplierClaim::query()
        .where_(SupplierClaim::ID.eq(id))
        .update(t.db(), &[("raised_at", &Some(at(day(-days), 10, 0)))])
        .await
        .unwrap();
}

fn closed(ticket: i64) -> TicketClosed {
    TicketClosed {
        ticket: TicketId(ticket),
        resolution_minutes: None,
        within_sla: true,
    }
}

/// Warranty ticket T-1 closes: the purchaser gets a task to raise the supplier
/// claim. On T-2 `resolved` nothing happens; a job card with no part fitted
/// has nothing to claim.
#[renox::test]
async fn aut_11_the_purchaser_gets_a_task_when_a_warranty_ticket_closes_not_when_it_resolves() {
    let t = boot().await;
    let about = delivered_with_alternator(&t).await;
    t.tickets
        .add(&about, "warranty", 1, "TKT-2026-0001", "closed");
    t.tickets
        .add(&about, "warranty", 2, "TKT-2026-0002", "resolved");
    t.tickets
        .add(&about, "complaint", 3, "TKT-2026-0003", "closed");

    t.state().emit(closed(1)).await.unwrap();
    let tasks = t.tickets.with_key("auto.supplier_claim_task:1");
    assert_eq!(tasks.len(), 1, "the purchaser has a task");
    assert_eq!(tasks[0].ticket.subject, about);
    assert!(tasks[0].ticket.title.contains("TKT-2026-0001"));

    // A ticket that is only resolved, and one of another category: nothing.
    t.state().emit(closed(2)).await.unwrap();
    t.state().emit(closed(3)).await.unwrap();
    t.state().emit(closed(99)).await.unwrap();
    assert!(t.tickets.with_key("auto.supplier_claim_task:2").is_empty());
    assert!(t.tickets.with_key("auto.supplier_claim_task:3").is_empty());
    assert_eq!(t.tickets.tasks().len(), 1);

    // A job card without a fitted part has nothing to claim.
    let vehicle = t.hilux().await;
    let empty = t.card_in(&vehicle, JobCardStatus::Delivered).await;
    t.tickets
        .add(&empty.subject(), "warranty", 4, "TKT-2026-0004", "closed");
    t.state().emit(closed(4)).await.unwrap();
    assert!(t.tickets.with_key("auto.supplier_claim_task:4").is_empty());
}

/// Purchaser P raises the claim: it is `raised`, the claim is emailed to the
/// supplier, and a draft Return to supplier (outcome *credit*) exists for the
/// kept part; a part that was not kept has none.
#[renox::test]
async fn aut_11_raising_the_claim_emails_the_supplier_and_drafts_the_return_of_the_kept_part() {
    let t = boot().await;
    let about = delivered_with_alternator(&t).await;
    t.tickets
        .add(&about, "warranty", 1, "TKT-2026-0001", "closed");
    let pia = pia(&t).await;

    let claim = supplier_claims::raise(t.state(), &pia, alternator(1, &about, day(-100)))
        .await
        .unwrap();
    assert_eq!(claim.status, SupplierClaimStatus::Raised);
    let mails = t.messages.of_type("auto.supplier_claim");
    assert_eq!(mails.len(), 1, "the claim document is emailed");
    assert!(Messages::told_party(&mails[0], 40));
    let text = Messages::text(&mails[0], wl_kernel::notify::NotifyChannel::Email);
    assert!(text.contains("Alternator"), "{text}");
    assert!(text.contains("AED 450.00"), "{text}");
    assert!(text.contains(&claim.number()), "{text}");
    assert!(claim.claim_sent_at.is_some());
    assert!(claim.claim_document.is_some());

    let drafted = t.returns.all();
    assert_eq!(drafted.len(), 1, "a draft return for the kept part");
    assert_eq!(drafted[0].item_code, "ALT-9");
    assert_eq!(drafted[0].supplier, PartyId(40));
    assert_eq!(drafted[0].source, claim.subject());
    assert_eq!(drafted[0].quantity_milli, 1_000);
    assert_eq!(drafted[0].location, Subject::new("inventory.location", 1));
    assert_eq!(claim.supplier_return_id, Some(901));

    // The customer discarded this one: no return.
    t.tickets
        .add(&about, "warranty", 2, "TKT-2026-0002", "closed");
    let discarded =
        supplier_claims::raise(t.state(), &pia, alternator(2, &about, day(-100)).not_kept())
            .await
            .unwrap();
    assert_eq!(t.returns.all().len(), 1);
    assert_eq!(discarded.supplier_return_id, None);
}

/// When the Return is credited by a settled debit note the claim is recovered
/// automatically; a second credit does nothing.
#[renox::test]
async fn aut_11_the_claim_is_recovered_when_the_return_is_credited_by_a_settled_debit_note() {
    let t = boot().await;
    let about = delivered_with_alternator(&t).await;
    t.tickets
        .add(&about, "warranty", 1, "TKT-2026-0001", "closed");
    let pia = pia(&t).await;
    let claim = supplier_claims::raise(t.state(), &pia, alternator(1, &about, day(-100)))
        .await
        .unwrap();
    let rts = Subject::new(
        "inventory.supplier_return",
        claim.supplier_return_id.unwrap(),
    );

    t.state()
        .emit(SupplierReturnCredited {
            supplier_return: rts,
            supplier: PartyId(40),
            basis: "debit_note".to_owned(),
            debit_note: Some(wl_kernel::invoicing::SupplierDebitNoteId(21)),
            reason: None,
            wastage: 0,
        })
        .await
        .unwrap();
    let recovered = SupplierClaim::find_or_404(t.db(), claim.id).await.unwrap();
    assert_eq!(recovered.status, SupplierClaimStatus::Recovered);
    assert_eq!(recovered.recovery_basis.as_deref(), Some("credit_note"));
}

/// A claim raised on 2026-11-02 and not recovered: on 2027-01-01, 60 days
/// later, the purchaser and the branch manager are told, once. Not a day
/// before.
#[renox::test]
async fn aut_11_the_purchaser_and_the_branch_manager_are_told_at_sixty_days_once() {
    let t = boot().await;
    let about = delivered_with_alternator(&t).await;
    t.tickets
        .add(&about, "warranty", 1, "TKT-2026-0001", "closed");
    let pia = pia(&t).await;
    let claim = supplier_claims::raise(t.state(), &pia, alternator(1, &about, day(-100)))
        .await
        .unwrap();
    raised_days_ago(&t, claim.id, 59).await;
    assert_eq!(supplier_rules::chase(t.state(), day(0)).await.unwrap(), 0);
    assert!(t.tickets.with_key("auto.supplier_chase:").is_empty());

    raised_days_ago(&t, claim.id, 60).await;
    assert_eq!(supplier_rules::chase(t.state(), day(0)).await.unwrap(), 1);
    let purchaser = t
        .tickets
        .with_key(&format!("auto.supplier_chase:{}:purchaser", claim.id));
    let manager = t
        .tickets
        .with_key(&format!("auto.supplier_chase:{}:manager", claim.id));
    assert_eq!(purchaser.len(), 1);
    assert_eq!(manager.len(), 1);
    assert!(
        SupplierClaim::find_or_404(t.db(), claim.id)
            .await
            .unwrap()
            .overdue_notified_at
            .is_some()
    );

    // Once: no repeat on the next days.
    assert_eq!(supplier_rules::chase(t.state(), day(1)).await.unwrap(), 0);
    assert_eq!(t.tickets.with_key("auto.supplier_chase:").len(), 2);

    // A recovered claim is not chased.
    t.tickets
        .add(&about, "warranty", 2, "TKT-2026-0002", "closed");
    let second = supplier_claims::raise(t.state(), &pia, alternator(2, &about, day(-100)))
        .await
        .unwrap();
    supplier_claims::recover(t.state(), &pia, second.id, Some("CN-1"), false)
        .await
        .unwrap();
    raised_days_ago(&t, second.id, 90).await;
    assert_eq!(supplier_rules::chase(t.state(), day(0)).await.unwrap(), 0);
}

/// An RTS credited manually with the reason "supplier refused": the claim stays
/// `raised` and the purchaser has a write-off task.
#[renox::test]
async fn aut_11_a_credit_the_supplier_refused_leaves_the_claim_raised_with_a_write_off_task() {
    let t = boot().await;
    let about = delivered_with_alternator(&t).await;
    t.tickets
        .add(&about, "warranty", 1, "TKT-2026-0001", "closed");
    let pia = pia(&t).await;
    let claim = supplier_claims::raise(t.state(), &pia, alternator(1, &about, day(-100)))
        .await
        .unwrap();
    let rts = Subject::new(
        "inventory.supplier_return",
        claim.supplier_return_id.unwrap(),
    );

    t.state()
        .emit(SupplierReturnCredited {
            supplier_return: rts,
            supplier: PartyId(40),
            basis: "supplier_refused".to_owned(),
            debit_note: None,
            reason: Some("supplier refused".to_owned()),
            wastage: 45_000,
        })
        .await
        .unwrap();
    let after = SupplierClaim::find_or_404(t.db(), claim.id).await.unwrap();
    assert_eq!(after.status, SupplierClaimStatus::Raised, "not recovered");
    let tasks = t
        .tickets
        .with_key(&format!("auto.supplier_claim_write_off:{}", claim.id));
    assert_eq!(tasks.len(), 1, "the purchaser has a write-off task");
}

/// Terms of 12 months and 20,000 km, a part fitted at 40,000 km: failing at
/// 65,000 km `raise` is refused with "mileage out of terms"; at 58,000 km it is
/// allowed.
#[renox::test]
async fn aut_11_the_mileage_of_the_terms_decides_whether_the_warranty_applies() {
    let t = boot().await;
    let about = delivered_with_alternator(&t).await;
    t.tickets
        .add(&about, "warranty", 1, "TKT-2026-0001", "closed");
    let pia = pia(&t).await;
    let vehicle = t.hilux().await;
    let fitted = t.card_in(&vehicle, JobCardStatus::Delivered).await;

    let refused = supplier_claims::raise(
        t.state(),
        &pia,
        alternator(1, &about, day(-100))
            .mileage(20_000)
            .fitted_on(fitted.id, 40_000, 65_000),
    )
    .await
    .unwrap_err();
    assert!(
        refused.to_string().contains("mileage out of terms"),
        "{refused}"
    );
    assert!(t.messages.of_type("auto.supplier_claim").is_empty());
    let allowed = supplier_claims::raise(
        t.state(),
        &pia,
        alternator(1, &about, day(-100))
            .mileage(20_000)
            .fitted_on(fitted.id, 40_000, 58_000),
    )
    .await
    .unwrap();
    assert_eq!(allowed.status, SupplierClaimStatus::Raised);
}

/// *Labour included* on and AED 180 of labour on the replacement job card: the
/// claim amount is the part cost plus AED 180; without it, the part only.
#[renox::test]
async fn aut_11_labour_included_adds_the_labour_of_the_replacement_to_the_claim_amount() {
    let t = boot().await;
    let about = delivered_with_alternator(&t).await;
    t.tickets
        .add(&about, "warranty", 1, "TKT-2026-0001", "closed");
    t.tickets
        .add(&about, "warranty", 2, "TKT-2026-0002", "closed");
    let pia = pia(&t).await;
    let vehicle = t.hilux().await;
    let replacement = t.card_in(&vehicle, JobCardStatus::Delivered).await;
    t.approved_labour(&replacement, "Replace the alternator", 1_000, 18_000)
        .await;

    let with = supplier_claims::raise(
        t.state(),
        &pia,
        alternator(1, &about, day(-100)).with_labour_of(replacement.id),
    )
    .await
    .unwrap();
    assert_eq!(with.labour_amount, 18_000);
    assert_eq!(with.claim_amount, 45_000 + 18_000);
    let without = supplier_claims::raise(t.state(), &pia, alternator(2, &about, day(-100)))
        .await
        .unwrap();
    assert_eq!(without.claim_amount, 45_000, "only the part is claimed");
}

/// A write-off waits for a branch manager other than the purchaser who asked:
/// the claim stays `raised`; the requester cannot approve it; on the manager's
/// approval it is `written_off`; a rejected request leaves it `raised`.
#[renox::test]
async fn aut_11_a_write_off_needs_the_approval_of_a_branch_manager_who_did_not_ask() {
    let t = boot().await;
    let about = delivered_with_alternator(&t).await;
    t.tickets
        .add(&about, "warranty", 1, "TKT-2026-0001", "closed");
    t.tickets
        .add(&about, "warranty", 2, "TKT-2026-0002", "closed");
    let pam_user = t
        .user(
            "Pam",
            &[
                auto::permissions::SUPPLIER_CLAIMS_MANAGE,
                auto::permissions::SUPPLIER_CLAIMS_APPROVE,
                auto::permissions::JOB_CARDS_VIEW,
            ],
        )
        .await;
    let pam = t.auth(&pam_user).await;
    let basma = t
        .user(
            "Basma",
            &[
                auto::permissions::SUPPLIER_CLAIMS_APPROVE,
                auto::permissions::JOB_CARDS_VIEW,
            ],
        )
        .await;
    let claim = supplier_claims::raise(t.state(), &pam, alternator(1, &about, day(-100)))
        .await
        .unwrap();

    let blank = supplier_claims::write_off(t.state(), &pam, claim.id, " ")
        .await
        .unwrap_err();
    assert!(
        blank.to_string().to_lowercase().contains("reason"),
        "{blank}"
    );
    let asked = supplier_claims::write_off(t.state(), &pam, claim.id, "Supplier refused")
        .await
        .unwrap();
    assert_eq!(asked.claim.status, SupplierClaimStatus::Raised);
    let request = asked.approval_request.expect("an approval request waits");
    let waiting = approvals::awaiting(t.db(), &basma).await.unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].action, "auto.supplier_claim_write_off");

    let own = approvals::approve(t.state(), request, &pam_user, "").await;
    assert!(own.is_err(), "not the requester");
    approvals::approve(t.state(), request, &basma, "")
        .await
        .expect("the branch manager approves");
    let done = SupplierClaim::find_or_404(t.db(), claim.id).await.unwrap();
    assert_eq!(done.status, SupplierClaimStatus::WrittenOff);
    assert_eq!(done.write_off_reason.as_deref(), Some("Supplier refused"));

    // Rejected: it stays raised.
    let second = supplier_claims::raise(t.state(), &pam, alternator(2, &about, day(-100)))
        .await
        .unwrap();
    let asked = supplier_claims::write_off(t.state(), &pam, second.id, "Too old")
        .await
        .unwrap();
    approvals::reject(
        t.state(),
        asked.approval_request.unwrap(),
        &basma,
        "Chase the supplier first",
    )
    .await
    .unwrap();
    assert_eq!(
        SupplierClaim::find_or_404(t.db(), second.id)
            .await
            .unwrap()
            .status,
        SupplierClaimStatus::Raised
    );
}
