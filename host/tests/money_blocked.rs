//! BL-PRP-26 (#276) items 21 to 24 with the real modules of the host: blocking
//! a tenant leaves its lease active and opens no review task; blocking a
//! vendor puts its service contracts in review for the property manager;
//! money out to a blocked tenant (deposit refund) or owner (payout) waits for
//! an approved exception (`PTY.ACL.03`) and runs once it is given; rent from
//! a blocked tenant is recorded and allocated as usual.

mod support;

use host::roles::{ACCOUNTANT, FACILITY_MANAGER, FINANCE_MANAGER, PROPERTY_MANAGER};
use property::UnitUse;
use renox::auth::permissions::Scope;
use renox::prelude::*;
use support::*;
use wl_kernel::block_effects::{BlockEffect, RuleSource, effect_for};
use wl_kernel::invoicing::{InvoiceStatus, VatCategory};
use wl_parties::review_tasks::{ReviewTask, ReviewTaskStatus};

/// Items 21 and 22: the effects the Property client registers.
#[renox::test]
async fn the_block_effects_of_leases_and_service_contracts_are_the_property_managers() {
    let _app = booted().await;
    let lease = effect_for("lease", &Scope::global());
    assert_eq!(lease.effect, BlockEffect::Continue);
    assert_eq!(lease.decider_role.as_deref(), Some(PROPERTY_MANAGER));
    assert_eq!(lease.source, RuleSource::Vertical);
    let contract = effect_for("service_contract", &Scope::global());
    assert_eq!(contract.effect, BlockEffect::Review);
    assert_eq!(contract.decider_role.as_deref(), Some(PROPERTY_MANAGER));
    assert_eq!(
        contract.source,
        RuleSource::Vertical,
        "not the facility manager wl-ticketing names"
    );
}

/// Item 21: blocking a tenant leaves its lease active and creates no review
/// task for it.
#[renox::test]
async fn blocking_a_tenant_leaves_the_lease_active_and_opens_no_review_task() {
    let app = booted().await;
    let tenant = tenant(&app, "Tara Nasser").await;
    let unit = unit_of(&app, "TWR-A", "1204", UnitUse::Residential).await;
    let lease = active_lease(&app, &unit, tenant, false).await;

    block(&app, tenant).await;

    let after = property::Lease::find_or_404(app.db(), lease.id)
        .await
        .unwrap();
    assert_eq!(after.status, property::LeaseStatus::Active);
    assert!(
        ReviewTask::query().get(app.db()).await.unwrap().is_empty(),
        "no review task for a lease"
    );
}

/// Item 22: blocking a vendor puts its service contracts in review: one task
/// for the property manager (the decider), who sees it; the facility manager
/// of wl-ticketing does not.
#[renox::test]
async fn blocking_a_vendor_opens_a_review_task_for_the_property_manager() {
    use wl_ticketing::{NewContract, ServiceContracts};

    let app = booted().await;
    let admin = person(&app, "Alia", host::roles::ADMIN).await;
    let manager = person(&app, "Pam", PROPERTY_MANAGER).await;
    let facility = person(&app, "Fiona", FACILITY_MANAGER).await;
    let vendor = company(&app, "CoolAir LLC").await;
    let contract = ServiceContracts::create_draft(
        app.state(),
        NewContract {
            vendor,
            title: "AC annual maintenance".into(),
            start_date: day(0),
            end_date: day(365),
            renewal_notice_days: 60,
            sla_policy_id: None,
        },
    )
    .await
    .unwrap();
    ServiceContracts::add_coverage(
        app.db(),
        &contract,
        wl_kernel::Subject::new("property.building", 1),
        "general",
    )
    .await
    .unwrap();
    app.acting_as(&admin);
    app.htmx()
        .post(
            &format!("/service-contracts/{}/activate", contract.id),
            &[("x", "y")],
        )
        .await
        .assert_ok();

    block(&app, vendor).await;

    let tasks = ReviewTask::query().get(app.db()).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].agreement_kind, "service_contract");
    assert_eq!(tasks[0].party_id, vendor.0);
    assert_eq!(tasks[0].status, ReviewTaskStatus::Open);
    assert_eq!(tasks[0].decider_role.as_deref(), Some(PROPERTY_MANAGER));

    // The property manager opens it; the facility manager may not.
    app.acting_as(&manager);
    app.get(&format!("/parties/review-tasks/{}", tasks[0].id))
        .await
        .assert_ok();
    app.acting_as(&facility);
    app.get(&format!("/parties/review-tasks/{}", tasks[0].id))
        .await
        .assert_forbidden();
}

/// Item 23, deposit: with the tenant blocked, the deposit refund does not run;
/// it becomes an exception request, and runs once the finance manager
/// approves it.
#[renox::test]
async fn a_blocked_tenants_deposit_refund_waits_for_an_exception() {
    use wl_kernel::BLOCK_EXCEPTION_ACTION;
    use wl_kernel::approvals::{ApprovalRequest, approve};
    use wl_payments::Deposit;
    use wl_payments::deposit::DepositStatus;

    let app = booted().await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    let finance = person(&app, "Fiona", FINANCE_MANAGER).await;
    let tenant = tenant(&app, "Tara Nasser").await;
    let unit = unit_of(&app, "TWR-A", "1204", UnitUse::Residential).await;
    let lease = draft_lease(&app, &unit, tenant, -300).await;
    let deposit = settling_deposit(&app, &accountant, tenant, &lease, 500_000).await;
    app.acting_as(&accountant);
    app.htmx()
        .post(
            &format!("/deposits/{}/deductions", deposit.id),
            &[("lines", ""), ("forfeiture_reason", "")],
        )
        .await
        .assert_ok();
    block(&app, tenant).await;
    app.travel(std::time::Duration::from_secs(15 * 24 * 3600));

    app.acting_as(&accountant);
    app.htmx()
        .post(
            &format!("/deposits/{}/refund", deposit.id),
            &[("method", "transfer")],
        )
        .await
        .assert_ok();
    let waiting = Deposit::find_or_404(app.db(), deposit.id).await.unwrap();
    assert_eq!(waiting.status, DepositStatus::SettlementPending);
    assert_eq!(waiting.refund_id, None, "no refund yet");
    let requests = ApprovalRequest::pending_about(app.db(), &deposit.subject())
        .await
        .unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].action, BLOCK_EXCEPTION_ACTION);

    approve(app.state(), requests[0].id, &finance, "approved")
        .await
        .unwrap()
        .expect("the exception is decided");
    let refunded = Deposit::find_or_404(app.db(), deposit.id).await.unwrap();
    assert_eq!(refunded.status, DepositStatus::Refunded);
    assert!(refunded.refund_id.is_some(), "the refund runs once");
}

/// Item 23, owner payout: with the owner blocked, the payout of a published
/// statement is refused with the neutral text; the accountant's exception
/// request waits and makes no payout; the finance manager's approval lets one
/// payout run (a second approver, a property manager, approves the payout).
#[renox::test]
async fn a_blocked_owners_payout_waits_for_an_exception() {
    use property::agreement::{AgreementStatus, ManagementAgreement};
    use property::statement::{self, OwnerStatement, StatementStatus};
    use renox::state_machine::StateMachine;
    use wl_kernel::BLOCK_EXCEPTION_ACTION;
    use wl_kernel::approvals::{ApprovalRequest, approve};

    let app = booted().await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    let manager = person(&app, "Pam", PROPERTY_MANAGER).await;
    let finance = person(&app, "Fiona", FINANCE_MANAGER).await;
    let owner = tenant(&app, "Hessa Al Mansoori").await;
    let mut agreement = ManagementAgreement {
        owner_party_id: owner.0,
        fee_bp: 500,
        payout_frequency: "monthly".to_owned(),
        notice_days: 90,
        start_date: day(-400),
        status: AgreementStatus::Active,
        ..Default::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    agreement
        .insert_in_state(&mut tx, AgreementStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let draft = statement::generate(app.state(), agreement.id, day(-60), day(-31))
        .await
        .unwrap()
        .expect("a draft");
    app.acting_as(&accountant);
    app.htmx()
        .post(
            &format!("/property/statements/{}/lines", draft.id),
            &[
                ("kind", "collected"),
                ("description", "Rent"),
                ("amount", "10,000"),
            ],
        )
        .await
        .assert_ok();
    app.acting_as(&manager);
    app.post(
        &format!("/property/statements/{}/transitions/approve", draft.id),
        &[],
    )
    .await
    .assert_status(303);
    app.acting_as(&accountant);
    app.post(
        &format!("/property/statements/{}/transitions/publish", draft.id),
        &[],
    )
    .await
    .assert_status(303);

    block(&app, owner).await;
    let path = format!("/property/statements/{}/pay-out", draft.id);
    let refused = app.htmx().post(&path, &[]).await;
    refused.assert_status(422);
    assert!(refused.text().contains("on hold"), "{}", refused.text());
    assert!(
        ApprovalRequest::pending_about(app.db(), &draft.subject())
            .await
            .unwrap()
            .is_empty()
    );

    app.htmx()
        .post(&path, &[("block_exception", "1")])
        .await
        .assert_ok();
    let requests = ApprovalRequest::pending_about(app.db(), &draft.subject())
        .await
        .unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].action, BLOCK_EXCEPTION_ACTION);
    let waiting = OwnerStatement::find_or_404(app.db(), draft.id)
        .await
        .unwrap();
    assert_eq!(waiting.status, StatementStatus::Published);
    assert!(waiting.payout_payment_id.is_none(), "no payout yet");

    // The request has two steps: the payout's own (a property manager other
    // than the requester) and then the exception (the finance manager).
    approve(app.state(), requests[0].id, &finance, "too early")
        .await
        .expect_err("the exception step comes after the payout's own");
    approve(app.state(), requests[0].id, &manager, "ok")
        .await
        .unwrap();
    assert!(
        OwnerStatement::find_or_404(app.db(), draft.id)
            .await
            .unwrap()
            .payout_payment_id
            .is_none(),
        "still no payout after the first step"
    );
    approve(app.state(), requests[0].id, &finance, "approved")
        .await
        .unwrap()
        .expect("the exception is decided");
    let settled = OwnerStatement::find_or_404(app.db(), draft.id)
        .await
        .unwrap();
    assert_eq!(settled.status, StatementStatus::Settled);
    assert_eq!(waiting.balance(), 1_000_000);
    assert_eq!(settled.payout_amount, Some(1_000_000));
    let payout = wl_payments::Payment::find_or_404(
        app.db(),
        settled
            .payout_payment_id
            .expect("the payout was recorded once"),
    )
    .await
    .unwrap();
    // A payout to a landlord is money out, but not an owner drawing
    // (`PRP.OWN.01`, `FIN.BUD.01`).
    assert_eq!(payout.direction, wl_kernel::payments::Direction::Out);
    assert_ne!(payout.purpose, wl_kernel::drawings::PURPOSE);
}

/// Item 24: a rent payment from a blocked tenant is recorded and allocated as
/// usual: money in continues.
#[renox::test]
async fn rent_from_a_blocked_tenant_is_recorded_and_allocated() {
    use wl_kernel::invoicing::InvoiceId;
    use wl_kernel::payments::{AllocationTarget, PaymentMethod, RecordPayment, payment_recorder};

    let app = booted().await;
    let clerk = person(&app, "Clara", host::roles::RECEIVABLES).await;
    let tenant = tenant(&app, "Tara Nasser").await;
    let invoice = issued_invoice(&app, &clerk, tenant, 100_000, VatCategory::Exempt, day(10)).await;
    block(&app, tenant).await;

    let entity = entity(&app, "PGP").await;
    let mut request = RecordPayment::new(tenant, entity.id, PaymentMethod::Transfer, 100_000);
    request.reference = Some("TRF-1".to_owned());
    let payment = payment_recorder(app.state())
        .record(app.state(), request, None)
        .await
        .expect("the rent of a blocked tenant is recorded");
    wl_payments::allocate::allocate(
        app.state(),
        payment.id,
        AllocationTarget::Invoice(InvoiceId(invoice.id)),
        100_000,
        None,
    )
    .await
    .expect("and allocated");
    assert_eq!(reload(&app, &invoice).await.status, InvoiceStatus::Paid);
}
