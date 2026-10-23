//! BL-MFG-14 (#395), 07 §3.3, §4.1, §4.2, §4.4, §5, §7.1, §7.2, §9, `X-01`,
//! `X-02`, `X-05`, `X-08`, `X-09`, `X-10`, `X-11`, `X-13`, `X-14`, `X-18`,
//! `X-19`: the 07 deployment's values for the shared mechanisms are in force
//! in a fresh 07 deployment (`wl:install` and the seeds), read from the real
//! modules, not from constants. A test names the Done-when point it covers;
//! the mechanisms themselves are run end to end by the tests of the modules
//! and of `apps/manufacturing`.

mod support;

use host::roles::{
    ACCOUNTANT, ADMIN, LINE_OPERATOR, MEMBER, OWNER, PRODUCTION_MANAGER, PRODUCTION_PLANNER,
    QC_INSPECTOR, RECEIVABLES, SALES, SALES_MANAGER, STOREKEEPER, SUPERVISOR,
};
use manufacturing::conformity::{DISPATCH, ECAS, EQM, FOOD_CONTACT, PICK};
use renox::chrono::{Datelike, Days, NaiveDate, Weekday};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use wl_documents::expiry;
use wl_documents::{DocumentKind, DocumentStatus, Sop, SopReader};
use wl_hr::OnboardingTemplate;
use wl_inventory::WlInventory;
use wl_invoicing::cash_alert::{self, BELOW_THRESHOLD};
use wl_invoicing::cash_balance::{self, EnterBalance};
use wl_invoicing::cash_forecast::{self, Forecast};
use wl_invoicing::cash_forecast_load;
use wl_invoicing::supplier_bill::{SupplierBill, SupplierBillStatus};
use wl_kernel::LegalEntity;
use wl_kernel::approvals;
use wl_kernel::documents::{BlockMode, OverrideMode, today};
use wl_kernel::invoicing::{InvoiceStatus, VatCategory};
use wl_kernel::late_entries::{Assessment, assess, limits_for};
use wl_kernel::notify::{GiveUpReaction, Urgency, notification_type};
use wl_kernel::{PartyId, Subject};
use wl_sales::LeadSource;

use support::{company, entity, issued_invoice, manufacturing_booted, messages_to, person};

fn this_monday() -> NaiveDate {
    let today = today();
    today - Days::new(u64::from(today.weekday().num_days_from_monday()))
}

/// The first day of week `n` (from 1) of the forecast.
fn week(n: u64) -> NaiveDate {
    this_monday() + Days::new(7 * (n - 1))
}

fn holds(role: &str, permission: &str) -> bool {
    host::roles::ROLES
        .iter()
        .find(|(name, _)| *name == role)
        .is_some_and(|(_, grants)| grants.contains(&permission))
}

// ------------------------------------------------------------------ void

/// Done when "Void": a production report can be voided (while its work order
/// is open, MFG.ACL.10; the page tests are `manufacturing_void_screens`), and
/// no invoice, delivery note or certificate is a voidable record: those are
/// corrected by a credit note or a new version.
#[renox::test]
async fn only_production_reports_are_voidable() {
    let _app = manufacturing_booted().await;
    assert!(wl_kernel::voids::get("manufacturing.production_report").is_some());
    for subject_type in [
        "invoicing.invoice",
        "inventory.dispatch",
        "documents.document",
    ] {
        assert!(
            wl_kernel::voids::get(subject_type).is_none(),
            "{subject_type} is never voided"
        );
    }
}

// ---------------------------------------------------------- cancellation

/// Done when "Cancellation": the proposed outcome for the started goods of a
/// cancelled line follows the item's customer-specific flag: *finish to
/// stock* for a standard item, *scrap* once the flag is on; the fee is 0%, its
/// VAT category standard until open item A-04 is answered, and the three
/// outcomes are offered.
#[renox::test]
async fn the_proposed_outcome_follows_the_customer_specific_flag() {
    use wl_kernel::sales::{LineOutcomeKind, SalesOrderId, outcome_proposals};
    let app = manufacturing_booted().await;
    let manager = person(&app, "Mona", PRODUCTION_MANAGER).await;
    let mut label = wl_inventory::Item::new("LABEL-1", "Printed label", "pcs");
    label.save(app.db()).await.unwrap();
    let mut lid = wl_inventory::Item::new("LID-1", "Lid", "pcs");
    lid.save(app.db()).await.unwrap();
    let proposed = |code: &'static str| {
        let state = app.state().clone();
        async move {
            outcome_proposals(&state)
                .proposed(&state, SalesOrderId(1), 1, code)
                .await
                .unwrap()
        }
    };
    assert_eq!(
        proposed("LABEL-1").await,
        Some(LineOutcomeKind::FinishToStock)
    );
    app.acting_as(&manager);
    app.htmx()
        .post(
            "/manufacturing/conformity",
            &[("code", "LABEL-1"), ("customer_specific", "1")],
        )
        .await
        .assert_status(303);
    assert_eq!(proposed("LABEL-1").await, Some(LineOutcomeKind::Scrap));
    assert_eq!(
        proposed("LID-1").await,
        Some(LineOutcomeKind::FinishToStock)
    );

    let sales = host::sales_for("manufacturing");
    let config = sales.config();
    assert_eq!(
        config.cancel_outcomes,
        ["finish_to_stock", "scrap", "return_to_stock"]
    );
    assert_eq!(config.cancellation_fee_bps, 0);
    assert_eq!(config.cancellation_fee_amount, 0);
    assert_eq!(config.cancellation_fee_vat, "standard");
}

// ------------------------------------------------- credit and cheques

/// Done when "Credit and cheques": an invoice more than 30 days overdue holds
/// the next order, the credit is checked again at each dispatch, a bounced
/// cheque holds every open order on account of the customer, no bounce fee is
/// invoiced and a cheque is presented twice at most (the mechanisms run in
/// the tests of wl-sales and wl-payments with these values).
#[renox::test]
async fn the_credit_and_cheque_values_are_the_07_ones() {
    let _app = manufacturing_booted().await;
    let sales = host::sales_for("manufacturing");
    let config = sales.config();
    assert!(config.credit_check && config.hold_on_overdue && config.hold_on_limit);
    assert_eq!(config.credit_grace_days, 30);
    assert!(config.credit_recheck_at_dispatch);
    assert!(config.bounced_cheque_hold_group);
    assert_eq!(config.manual_discount_limit_bps, 500, "SAL.ACL.02: 5%");
    assert!(config.unique_customer_po);
    assert!(
        config.invoice_on_delivery,
        "the invoice covers what was accepted"
    );
    assert!(config.over_budget_approval, "SAL.ACL.07");
    assert!(config.campaign_required_sources.is_empty());

    let payments = host::payments_for("manufacturing");
    let config = payments.config();
    assert!(config.bounce_fee.is_none(), "no bounce fee");
    assert_eq!(
        config.cheque_max_presentations, 2,
        "a third presentation is refused"
    );
    assert_eq!(config.cheque_reminder_days, 3);
    assert_eq!(config.cheque_decision_days, 14);
}

// ------------------------------------------------------------- documents

/// Done when "Documents": `blocks_for("dispatch", "pick")` and
/// `blocks_for("dispatch", "dispatch")` return the three conformity kinds
/// with mode `block` and override `never`; the industrial licence has no
/// `blocks`.
#[renox::test]
async fn the_conformity_kinds_block_pick_and_dispatch_and_the_licence_blocks_nothing() {
    let app = manufacturing_booted().await;
    for guarded in [PICK, DISPATCH] {
        let entries = wl_documents::blocks_for(app.db(), guarded).await.unwrap();
        let mut kinds: Vec<&str> = entries.iter().map(|e| e.kind.as_str()).collect();
        kinds.sort_unstable();
        let mut expected = [ECAS, EQM, FOOD_CONTACT];
        expected.sort_unstable();
        assert_eq!(kinds, expected, "{guarded}");
        assert!(
            entries.iter().all(|e| e.mode == BlockMode::Block
                && e.override_mode == OverrideMode::Never
                && e.locked),
            "{guarded}"
        );
    }
    for key in [ECAS, EQM, FOOD_CONTACT] {
        let kind = DocumentKind::by_key(app.db(), key).await.unwrap().unwrap();
        assert!(kind.legal_block && kind.compliance_critical, "{key}");
        assert_eq!(kind.look_ahead_days, 30, "{key}");
        assert_eq!(
            kind.responsible_role.as_deref(),
            Some("qc_inspector"),
            "{key}"
        );
        // The company admin cannot loosen it.
        let refused =
            wl_documents::kinds::set_override_mode(app.db(), key, OverrideMode::WithApproval)
                .await
                .unwrap()
                .expect_err("the override mode stays never");
        assert_eq!(refused.reason, wl_documents::RefusalReason::LegalBlock);
    }
    for key in [
        "industrial_licence",
        "industrial_registry",
        "icv_certificate",
    ] {
        let kind = DocumentKind::by_key(app.db(), key).await.unwrap().unwrap();
        assert!(!kind.legal_block, "{key}");
        assert!(kind.blocks(app.db()).await.unwrap().is_empty(), "{key}");
        assert_eq!(kind.responsible_role.as_deref(), Some("owner"), "{key}");
    }
    let icv = DocumentKind::by_key(app.db(), "icv_certificate")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        icv.reminder_offsets.0,
        [60, 30, 7],
        "the ICV reminds at 60 days"
    );
    for key in ["customs_exit_certificate", "bill_of_lading", "airway_bill"] {
        let kind = DocumentKind::by_key(app.db(), key).await.unwrap().unwrap();
        assert_eq!(
            kind.responsible_role.as_deref(),
            Some("accountant"),
            "{key}"
        );
    }
}

// ------------------------------------------------------ responsible roles

/// Two plants with a line each; item X has stock at plant 1 only.
struct Plants {
    p1: manufacturing::Plant,
    p2: manufacturing::Plant,
    x: wl_inventory::Item,
    y: wl_inventory::Item,
}

async fn plants(app: &TestApp) -> Plants {
    let mut made = Vec::new();
    for (name, store, finished) in [("P1", "P1-RM", "P1-FG"), ("P2", "P2-RM", "P2-FG")] {
        let plant = manufacturing::Plant::create(
            app.db(),
            manufacturing::Plant {
                name: name.into(),
                active: true,
                ..manufacturing::Plant::default()
            },
        )
        .await
        .unwrap();
        let mut rm = wl_inventory::Location::new(store, store);
        rm.save(app.db()).await.unwrap();
        let mut fg = wl_inventory::Location::new(finished, finished);
        fg.save(app.db()).await.unwrap();
        manufacturing::Line::create(
            app.db(),
            manufacturing::Line {
                plant_id: plant.id,
                code: format!("{name}-L1"),
                name: format!("{name} line"),
                store_location_id: Some(rm.id),
                finished_location_id: Some(fg.id),
                active: true,
                ..manufacturing::Line::default()
            },
        )
        .await
        .unwrap();
        made.push((plant, fg));
    }
    let mut x = wl_inventory::Item::new("LED-X", "LED fitting", "pcs");
    x.average_cost = 500;
    x.save(app.db()).await.unwrap();
    let mut y = wl_inventory::Item::new("LED-Y", "LED strip", "pcs");
    y.save(app.db()).await.unwrap();
    wl_inventory::StockMovement::receive(
        x.id,
        None,
        made[0].1.id,
        10_000,
        500,
        5_000,
        &Subject::new("inventory.goods_receipt", 1),
        None,
    )
    .save(app.db())
    .await
    .unwrap();
    let (p2, _) = made.pop().unwrap();
    let (p1, _) = made.pop().unwrap();
    Plants { p1, p2, x, y }
}

async fn certificate(app: &TestApp, item: &wl_inventory::Item, kind: &str, days: i64) {
    let kind = DocumentKind::by_key(app.db(), kind).await.unwrap().unwrap();
    wl_documents::Document::create(
        app.db(),
        wl_documents::Document {
            subject_type: "inventory.item".into(),
            subject_id: item.id,
            kind_id: kind.id,
            title: format!("{} certificate", item.code),
            expiry_date: Some(today() + Days::new(days as u64)),
            current_version: 1,
            status: DocumentStatus::Accepted,
            ..Default::default()
        },
    )
    .await
    .unwrap();
}

async fn told(app: &TestApp, user: &User) -> usize {
    messages_to(app, expiry::run::NOTIFICATION_TYPE, user)
        .await
        .len()
}

/// Done when "Responsible roles": plants P1 (stock of item X) and P2 (none,
/// no work order for X): an expiring ECAS certificate of X notifies the QC
/// inspectors of P1 only; item Y, with no stock or work order anywhere, has
/// its expiring certificate notify the QC inspectors of P1 and P2; an
/// expiring ICV certificate notifies the general manager / owner.
#[renox::test]
async fn an_item_certificate_notifies_the_qc_inspectors_of_the_plants_that_hold_it() {
    let app = manufacturing_booted().await;
    let w = plants(&app).await;
    let qc1 = support::scoped_person(&app, "Qamar", QC_INSPECTOR, &w.p1.scope()).await;
    let qc2 = support::scoped_person(&app, "Qadir", QC_INSPECTOR, &w.p2.scope()).await;
    certificate(&app, &w.x, ECAS, 30).await;
    let report = expiry::run(app.state(), today()).await.unwrap();
    assert_eq!(report.failed, 0, "{report:?}");
    assert_eq!(told(&app, &qc1).await, 1, "P1 holds X");
    assert_eq!(told(&app, &qc2).await, 0, "P2 does not");

    certificate(&app, &w.y, EQM, 30).await;
    expiry::run(app.state(), today()).await.unwrap();
    assert_eq!(told(&app, &qc1).await, 2, "Y is nowhere: every plant");
    assert_eq!(told(&app, &qc2).await, 1);

    // The ICV certificate of the company goes to the general manager / owner.
    let owner = person(&app, "Omar", OWNER).await;
    let nec = entity(&app, "AWP").await;
    let kind = DocumentKind::by_key(app.db(), "icv_certificate")
        .await
        .unwrap()
        .unwrap();
    wl_documents::Document::create(
        app.db(),
        wl_documents::Document {
            subject_type: "kernel.legal_entity".into(),
            subject_id: nec.id,
            kind_id: kind.id,
            title: "ICV".into(),
            expiry_date: Some(today() + Days::new(60)),
            current_version: 1,
            status: DocumentStatus::Accepted,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    expiry::run(app.state(), today()).await.unwrap();
    assert_eq!(told(&app, &owner).await, 1, "60 days before the expiry");
}

/// An open work order for item X in plant 2 makes plant 2's QC inspectors
/// responsible too.
#[renox::test]
async fn an_open_work_order_makes_the_plant_responsible_for_the_certificate() {
    let app = manufacturing_booted().await;
    let w = plants(&app).await;
    let qc1 = support::scoped_person(&app, "Qamar", QC_INSPECTOR, &w.p1.scope()).await;
    let qc2 = support::scoped_person(&app, "Qadir", QC_INSPECTOR, &w.p2.scope()).await;
    let mut order = manufacturing::WorkOrder {
        number: "WO-X-1".into(),
        plant_id: w.p2.id,
        item_id: w.x.id,
        quantity_milli: 10_000,
        status: manufacturing::WorkOrderStatus::Planned,
        ..manufacturing::WorkOrder::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    order
        .insert_in_state(&mut tx, manufacturing::WorkOrderStatus::Planned, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    certificate(&app, &w.x, FOOD_CONTACT, 7).await;
    expiry::run(app.state(), today()).await.unwrap();
    assert_eq!(told(&app, &qc1).await, 1);
    assert_eq!(told(&app, &qc2).await, 1);
}

// --------------------------------------------------------- notifications

/// Done when "Notifications": `mfg.recall_notice` is registered with no group
/// and urgency `urgent` (a notice that gives up gives the recall owner a
/// task); the staff notice (`MFG.RUL.02` and the other alerts) has no group
/// and gives up with `none`; no type is statutory.
#[renox::test]
async fn the_recall_notice_is_urgent_and_the_staff_notices_have_no_group() {
    let _app = manufacturing_booted().await;
    let notice = notification_type("mfg.recall_notice").expect("registered");
    assert_eq!(notice.group, "");
    assert_eq!(notice.urgency, Urgency::Urgent);
    assert_eq!(notice.give_up, [GiveUpReaction::StaffTask]);
    assert!(!notice.statutory);
    let staff = notification_type("manufacturing.notice").expect("registered");
    assert_eq!(staff.group, "");
    assert_eq!(staff.give_up, [GiveUpReaction::None]);
    assert!(!staff.statutory);
    let inventory = host::manufacturing_seed::inventory(WlInventory::new());
    assert_eq!(
        inventory.config().recall_notice_key.as_deref(),
        Some("mfg.recall_notice")
    );
}

// ----------------------------------------------------------- party block

/// Done when "Party block": the work orders of a blocked customer continue
/// and the planner decides what to do with each; the sales orders not yet
/// dispatched are paused, decided by the sales manager.
#[renox::test]
async fn blocking_a_customer_pauses_its_orders_and_lets_its_work_orders_continue() {
    use wl_kernel::block_effects::{BlockEffect, effect_for};
    let _app = manufacturing_booted().await;
    let scope = renox::auth::permissions::Scope::global();
    let orders = effect_for("sales_order", &scope);
    assert_eq!(orders.effect, BlockEffect::Pause);
    assert_eq!(orders.decider_role.as_deref(), Some(SALES_MANAGER));
    let work = effect_for("work_order", &scope);
    assert_eq!(work.effect, BlockEffect::Continue);
    assert_eq!(work.decider_role.as_deref(), Some(PRODUCTION_PLANNER));
    let money_in = effect_for("money_in", &scope);
    assert_eq!(money_in.effect, BlockEffect::Continue);
}

// ------------------------------------------------- integrations, late entry

/// Done when "Integrations and late entry": the limits of a production
/// report are in force: 5 hours late is flagged with a reason, 30 hours late
/// needs the production manager (the report entry itself runs in
/// `apps/manufacturing/tests/late_report.rs`).
#[renox::test]
async fn a_production_report_is_flagged_after_30_minutes_and_approved_after_24_hours() {
    let app = manufacturing_booted().await;
    let limits = limits_for(app.db(), manufacturing::late_report::SUBJECT_TYPE)
        .await
        .unwrap();
    assert_eq!(limits.flag_after_minutes, 30);
    assert_eq!(limits.approval_after_hours, 24);
    let period = wl_kernel::late_entries::Period::Open;
    let now = renox::db::now();
    let five = assess(
        limits,
        now - renox::chrono::Duration::hours(5),
        now,
        &period,
    );
    assert!(matches!(five, Assessment::Flagged { .. }), "{five:?}");
    let thirty = assess(
        limits,
        now - renox::chrono::Duration::hours(30),
        now,
        &period,
    );
    assert!(
        matches!(thirty, Assessment::NeedsApproval { .. }),
        "{thirty:?}"
    );
    let manager = person(&app, "Mona", PRODUCTION_MANAGER).await;
    let approvers = renox::auth::permissions::users_with_permission(
        app.db(),
        wl_kernel::permissions::LATE_ENTRIES_APPROVE,
    )
    .await
    .unwrap();
    assert!(approvers.iter().any(|u| u.id == manager.id));
}

// ----------------------------------------------------------- compliance

/// Done when "Compliance": `MFG.RUL.08` and `DOC.RUL.03` are
/// compliance-critical; a failed run alerts the general manager / owner role
/// and the company admin and opens an owner task (the failing run itself is
/// `manufacturing_exports`).
#[renox::test]
async fn the_export_rule_and_the_certificate_rule_are_compliance_critical() {
    use wl_kernel::compliance::service::recipients;
    use wl_kernel::compliance::{AlertKind, Raised, is_critical, raise, schedule_subject};
    let app = manufacturing_booted().await;
    for rule in ["MFG.RUL.08", "DOC.RUL.03"] {
        assert!(is_critical(rule), "{rule}");
    }
    let owner = person(&app, "Omar", OWNER).await;
    let admin = person(&app, "Ada", ADMIN).await;
    let alert = raise(
        app.state(),
        Raised::new(
            "MFG.RUL.08",
            schedule_subject(),
            "MFG.RUL.08:2026-11-10",
            AlertKind::Failed,
        )
        .error("the evidence check failed"),
    )
    .await
    .unwrap()
    .expect("an alert");
    assert_eq!(alert.owner_role, "owner");
    let told = recipients(app.db(), &alert.owner_role).await.unwrap();
    assert!(told.contains(&admin.id) && told.contains(&owner.id));
}

// ----------------------------------------------------------------- money

/// Done when "Money": every invoice write-off creates an approval request
/// (AED 0), for the general manager / owner or the company admin, never the
/// requester; a customer buyer disputes an invoice in the portal and an open
/// dispute escalates on day 14.
#[renox::test]
async fn every_write_off_needs_the_owner_or_the_admin() {
    let app = manufacturing_booted().await;
    let clerk = person(&app, "Clara", RECEIVABLES).await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    let owner = person(&app, "Omar", OWNER).await;
    let admin = person(&app, "Ada", ADMIN).await;
    let storekeeper = person(&app, "Karim", STOREKEEPER).await;
    let customer = company(&app, "Al Noor Trading LLC").await;
    let due = today() - Days::new(30);
    let invoice = issued_invoice(&app, &clerk, customer, 100, VatCategory::Standard, due).await;

    app.acting_as(&accountant);
    app.htmx()
        .post(
            &format!("/invoices/{}/write-off", invoice.id),
            &[("reason_code", "insolvent"), ("reason_text", "")],
        )
        .await
        .assert_ok();
    let pending = approvals::pending_for(app.db(), &invoice.subject())
        .await
        .unwrap();
    assert_eq!(pending.len(), 1, "AED 0.01 is above the limit of AED 0");
    let waiting = |user: &User| {
        let (user, db) = (user.clone(), app.db().clone());
        async move {
            approvals::awaiting(&db, &user)
                .await
                .unwrap()
                .iter()
                .any(|r| r.action == "invoicing.write_off")
        }
    };
    assert!(waiting(&owner).await);
    assert!(waiting(&admin).await);
    assert!(!waiting(&accountant).await, "the requester cannot approve");
    assert!(!waiting(&storekeeper).await);
    let own = approvals::approve(app.state(), pending[0].id, &accountant, "").await;
    assert!(matches!(own, Err(Error::Forbidden)), "{own:?}");
    approvals::approve(app.state(), pending[0].id, &owner, "agreed")
        .await
        .unwrap();
    let after = wl_invoicing::Invoice::find_or_404(app.db(), invoice.id)
        .await
        .unwrap();
    assert_eq!(after.status, InvoiceStatus::WrittenOff);

    let invoicing = host::invoicing_for("manufacturing", &["AWP".to_owned()]);
    let config = invoicing.config();
    assert!(config.portal_disputes);
    assert_eq!(config.dispute_escalation_days, 14);
    assert!(config.bad_debt_relief, "FIN.RUL.14 proposes the relief");
}

/// Done when "Disputes": a customer buyer disputes an invoice of its own
/// company in the portal (not another company's); the dispute escalates on
/// day 14.
#[renox::test]
async fn a_buyer_disputes_in_the_portal_and_the_dispute_escalates_after_14_days() {
    use wl_invoicing::{dispute, escalation};
    use wl_kernel::invoicing::{DisputeBy, DisputeRequest, InvoiceId};
    let app = manufacturing_booted().await;
    let clerk = person(&app, "Clara", RECEIVABLES).await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    let customer = company(&app, "Al Noor Trading LLC").await;
    let other = company(&app, "Other Trading LLC").await;
    let invoice = issued_invoice(
        &app,
        &clerk,
        customer,
        100_000,
        VatCategory::Standard,
        today() + Days::new(30),
    )
    .await;
    let by_other = dispute::raise(
        app.state(),
        DisputeRequest::new(
            InvoiceId(invoice.id),
            "not ours",
            DisputeBy::Party { id: other },
        ),
    )
    .await;
    assert!(by_other.is_err(), "another company cannot dispute it");
    dispute::raise(
        app.state(),
        DisputeRequest::new(
            InvoiceId(invoice.id),
            "damaged pallet",
            DisputeBy::Party { id: customer },
        )
        .amount(10_000),
    )
    .await
    .expect("the customer disputes its own invoice");
    assert_eq!(
        escalation::run(app.state(), today() + Days::new(13))
            .await
            .unwrap(),
        0
    );
    assert!(
        escalation::run(app.state(), today() + Days::new(14))
            .await
            .unwrap()
            >= 1
    );
    escalation::run(app.state(), today() + Days::new(28))
        .await
        .unwrap();
    assert!(
        !messages_to(&app, escalation::ESCALATED, &accountant)
            .await
            .is_empty(),
        "the accountant is told after 28 days"
    );
}

// ----------------------------------------------------------------- stock

/// Done when "Stock": a wastage of AED 2,100 needs approval and one of AED
/// 2,000 does not; a return to a supplier of AED 5,100 needs approval; the
/// hold reminder is 24 hours, the return chase 30 days, only a recall hold
/// needs a second person; a location blocks consumption below zero (a work
/// order issue below zero becomes a pending issue, the module's rule).
#[renox::test]
async fn the_stock_limits_are_the_07_ones() {
    let app = manufacturing_booted().await;
    let module = host::manufacturing_seed::inventory(WlInventory::new());
    let config = module.config();
    assert_eq!(config.wastage_approval_fils, 200_000);
    assert!(210_000 > config.wastage_approval_fils);
    assert!(200_000 <= config.wastage_approval_fils);
    assert_eq!(config.return_approval_fils, 500_000);
    assert!(510_000 > config.return_approval_fils);
    assert_eq!(config.hold_reminder_hours, 24);
    assert_eq!(config.return_chase_days, 30);
    assert!(config.release_needs_second_person("recall"));
    assert!(!config.release_needs_second_person("quality"));
    assert!(config.preferred_change_needs_approval);
    assert_eq!(config.score.window_months, 3);
    assert_eq!(config.score.min_receipts, 3);
    assert_eq!(config.score.threshold, 70, "69 opens a review, 70 does not");
    let mut location = wl_inventory::Location::new("RM", "Raw materials");
    location.save(app.db()).await.unwrap();
    assert_eq!(
        location.negative_stock_policy,
        wl_inventory::location::NegativeStockPolicy::Block
    );
    // Giving the production manager and the storekeeper the approvals.
    assert!(holds(
        PRODUCTION_MANAGER,
        wl_inventory::permissions::STOCK_APPROVE
    ));
    assert!(holds(OWNER, wl_inventory::permissions::RECALLS_MANAGE));
    assert!(holds(
        PRODUCTION_MANAGER,
        wl_inventory::permissions::RECALLS_MANAGE
    ));
}

// ------------------------------------------------------------ cash, budget

/// An entity's bank balance of `fils`, as of today.
async fn bank(app: &TestApp, entity: &LegalEntity, fils: i64) {
    cash_balance::enter(
        app.db(),
        None,
        &EnterBalance {
            legal_entity_id: entity.id,
            bank_account: "ENBD".to_owned(),
            amount: fils,
            as_of: today(),
            note: None,
        },
    )
    .await
    .unwrap();
}

/// An approved supplier bill of `fils` due `due`, payable by the entity.
async fn bill_due(app: &TestApp, entity: &LegalEntity, number: &str, due: NaiveDate, fils: i64) {
    let mut bill = SupplierBill {
        party_id: company(app, &format!("Vendor {number}")).await.0,
        legal_entity_id: Some(entity.id),
        supplier_invoice_number: number.to_owned(),
        number_key: number.to_lowercase(),
        bill_date: today() - Days::new(1),
        due_date: Some(due),
        currency: "AED".to_owned(),
        gross_total: fils,
        balance: fils,
        status: SupplierBillStatus::Approved,
        ..Default::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    bill.insert_in_state(&mut tx, SupplierBillStatus::Approved, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

async fn cash_scene() -> (TestApp, LegalEntity, User, User) {
    let app = manufacturing_booted().await;
    let owner = person(&app, "Omar", OWNER).await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    SupplierBill::query().delete(app.db()).await.unwrap();
    let awp = entity(&app, "AWP").await;
    (app, awp, owner, accountant)
}

async fn cash_told(app: &TestApp, user: &User) -> usize {
    messages_to(app, BELOW_THRESHOLD, user).await.len()
}

/// Done when "Cash-flow forecast": a projected balance of AED 199,999 in week
/// 5 alerts the general manager / owner and the accountant once.
#[renox::test]
async fn one_hundred_ninety_nine_thousand_in_week_five_alerts_once() {
    let (app, awp, owner, accountant) = cash_scene().await;
    // AED 300,000 in the bank; a bill of AED 100,001 falls due in week 5.
    bank(&app, &awp, 30_000_000).await;
    bill_due(&app, &awp, "SB-W5", week(5) + Days::new(2), 10_000_100).await;
    let sent = cash_alert::run(app.state(), today()).await.unwrap();
    assert_eq!(sent, 1);
    assert_eq!(cash_told(&app, &owner).await, 1);
    assert_eq!(cash_told(&app, &accountant).await, 1);
    let notice = &messages_to(&app, BELOW_THRESHOLD, &owner).await[0];
    assert_eq!(notice.data.0["week"], 5);
    assert_eq!(notice.data.0["balance"], "AED 199,999.00");
    assert_eq!(notice.data.0["threshold"], "AED 200,000.00");
    assert_eq!(cash_alert::run(app.state(), today()).await.unwrap(), 0);
    assert_eq!(cash_told(&app, &owner).await, 1);
}

/// AED 200,000 or more in every week alerts no one.
#[renox::test]
async fn two_hundred_thousand_in_every_week_alerts_no_one() {
    let (app, awp, owner, accountant) = cash_scene().await;
    bank(&app, &awp, 30_000_000).await;
    bill_due(&app, &awp, "SB-W5", week(5) + Days::new(2), 10_000_000).await;
    assert_eq!(cash_alert::run(app.state(), today()).await.unwrap(), 0);
    assert_eq!(cash_told(&app, &owner).await, 0);
    assert_eq!(cash_told(&app, &accountant).await, 0);
}

/// The forecast has 13 weekly buckets from Monday and the check runs at
/// 06:00 Asia/Dubai.
#[renox::test]
async fn the_forecast_has_thirteen_weeks_from_monday_and_runs_at_six() {
    let (app, awp, _owner, _accountant) = cash_scene().await;
    bank(&app, &awp, 30_000_000).await;
    let module = host::invoicing_for("manufacturing", &["AWP".to_owned()]);
    let config = module.config().clone();
    let inputs =
        cash_forecast_load::load(app.state(), &config, std::slice::from_ref(&awp), today())
            .await
            .unwrap();
    let forecast: Forecast = cash_forecast::assemble(&inputs);
    assert_eq!(forecast.weeks.len(), 13);
    assert_eq!(forecast.weeks[0].start, this_monday());
    assert!(
        forecast
            .weeks
            .iter()
            .all(|w| w.start.weekday() == Weekday::Mon)
    );
    use renox::Module;
    let mut registry = renox::Registry::default();
    module.register(&mut registry);
    let run = registry
        .schedule()
        .upcoming(renox::timezone::Zone::UTC)
        .into_iter()
        .find(|run| run.name == cash_alert::TASK)
        .expect("the daily cash check is scheduled");
    let at = run.at.expect("it has a next run");
    assert_eq!(run.zone.to_string(), "Asia/Dubai");
    assert_eq!(
        run.zone.local(at.timestamp()).format("%H:%M").to_string(),
        "06:00"
    );
}

/// Done when "Budget and drawings": the accountant prepares and the general
/// manager / owner approves the budget; the budget categories are the
/// accountant's; a drawing is never approved by its requester and the owner is
/// told; `FIN.RUL.24` warns the accountant at 100%.
#[renox::test]
async fn the_budget_is_approved_by_the_owner_and_the_categories_are_the_accountants() {
    let app = manufacturing_booted().await;
    let module = host::invoicing_for("manufacturing", &["AWP".to_owned()]);
    let config = module.config();
    assert!(!config.drawings_self_approve);
    assert_eq!(config.drawings_notify_role, "owner");
    assert_eq!(config.budget_variance_percent, 100);
    assert_eq!(config.budget_variance_role, "accountant");
    assert_eq!(config.cash_alert_roles, ["owner", "accountant"]);
    assert_eq!(config.cash_threshold_of("AWP"), 20_000_000);
    assert!(holds(ACCOUNTANT, wl_invoicing::permissions::BUDGETS_MANAGE));
    assert!(!holds(
        ACCOUNTANT,
        wl_invoicing::permissions::BUDGETS_APPROVE
    ));
    assert!(holds(OWNER, wl_invoicing::permissions::BUDGETS_APPROVE));
    assert!(holds(OWNER, wl_invoicing::permissions::DRAWINGS_APPROVE));
    let categories = wl_invoicing::BudgetCategory::listed(app.db(), true)
        .await
        .unwrap();
    for category in categories.iter().filter(|c| !c.is_drawings()) {
        assert_eq!(
            category.owner_role.as_deref(),
            Some("accountant"),
            "{}",
            category.name_en
        );
    }
    assert!(categories.iter().any(|c| c.is_drawings()), "owner drawings");
}

// ------------------------------------------------------- leads, suppliers

/// Done when "Lead sources": a new lead or party offers referral, website,
/// event, partner, paid ads and other; the others are off; no source needs a
/// campaign and a cost over a campaign's budget waits for a manager.
#[renox::test]
async fn the_lead_sources_are_the_07_ones() {
    let app = manufacturing_booted().await;
    for code in host::manufacturing_seed::LEAD_SOURCES {
        let source = LeadSource::by_code(app.db(), code)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{code}"));
        assert!(source.active, "{code}");
    }
    for code in ["walk_in", "aggregator", "social"] {
        let source = LeadSource::by_code(app.db(), code).await.unwrap().unwrap();
        assert!(!source.active, "07 does not use {code}");
    }
    assert!(
        holds(SALES_MANAGER, wl_sales::permissions::SALES_ORDERS_VIEW)
            || holds(SALES, wl_sales::permissions::SALES_ORDERS_VIEW)
    );
}

// ------------------------------------------------- onboarding and SOPs

/// Done when "Onboarding and SOPs": the checklists of the five job roles
/// exist and block nothing; hiring an operator in `onboarding` starts the
/// operator checklist with steps for the production manager, the line
/// supervisor and the employee; the SOP library has the owners and readers of
/// §5; a second run of the install changes nothing.
#[renox::test]
async fn hiring_an_operator_starts_the_checklist_and_the_sops_are_seeded() {
    let app = manufacturing_booted().await;
    for (name, role) in [
        ("Operator onboarding", "line_operator"),
        ("Line supervisor onboarding", "supervisor"),
        ("QC inspector onboarding", "qc_inspector"),
        ("Storekeeper onboarding", "storekeeper"),
        (
            "Maintenance technician onboarding",
            "maintenance_technician",
        ),
    ] {
        let template = OnboardingTemplate::query()
            .where_(OnboardingTemplate::NAME_EN.eq(name))
            .first(app.db())
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{name}"));
        assert_eq!(template.job_roles, role);
        assert!(!template.blocks_activation, "{name}");
    }
    let user = person(&app, "Olga", LINE_OPERATOR).await;
    let party = support::tenant(&app, "Olga Operator").await;
    let employee = wl_hr::employees::create(
        app.state(),
        wl_hr::NewEmployee::new(party, "line_operator", today()).user(user.id),
    )
    .await
    .unwrap();
    assert_eq!(employee.status, wl_hr::EmployeeStatus::Onboarding);
    let onboarding = wl_hr::Onboarding::query()
        .where_(wl_hr::Onboarding::EMPLOYEE_ID.eq(employee.id))
        .first(app.db())
        .await
        .unwrap()
        .expect("the onboarding started");
    let steps = wl_hr::OnboardingStep::query()
        .where_(wl_hr::OnboardingStep::ONBOARDING_ID.eq(onboarding.id))
        .get(app.db())
        .await
        .unwrap();
    let owners: Vec<&str> = steps.iter().map(|s| s.owner.as_str()).collect();
    for owner in ["role:production_manager", "manager", "employee"] {
        assert!(owners.contains(&owner), "{owner}");
    }
    assert_eq!(
        host::manufacturing_seed::hr(wl_hr::WlHr::new())
            .config()
            .manager_role,
        SUPERVISOR
    );

    let instructions = Sop::query()
        .where_(Sop::CODE.eq("SOP-MFG-WORK-INSTRUCTIONS"))
        .first(app.db())
        .await
        .unwrap()
        .expect("seeded");
    assert_eq!(instructions.owner_role, "production_manager");
    assert_eq!(instructions.ack_due_days, 7);
    let readers: Vec<String> = SopReader::query()
        .where_(SopReader::SOP_ID.eq(instructions.id))
        .get(app.db())
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.role_key)
        .collect();
    assert!(readers.contains(&"line_operator".to_owned()));
    let inspection = Sop::query()
        .where_(Sop::CODE.eq("SOP-MFG-INSPECTION"))
        .first(app.db())
        .await
        .unwrap()
        .expect("seeded");
    assert_eq!(inspection.owner_role, "qc_inspector");
    let sops = Sop::query().count(app.db()).await.unwrap();
    let installed = host::manufacturing_seed::install(app.state())
        .await
        .unwrap();
    assert_eq!(installed.total(), 0, "a second run changes nothing");
    assert_eq!(Sop::query().count(app.db()).await.unwrap(), sops);
}

// --------------------------------------------------------- registrations

/// Done when "Registrations": a fresh 07 deployment lists *Halal
/// certificate*, *ISO certificate*, *Other certification* and *Trademark
/// registration* on, with the QC inspector responsible and the general
/// manager / owner for the trademark; a trademark expiring in 90 days
/// notifies the general manager / owner; none has `blocks`.
#[renox::test]
async fn the_registration_kinds_have_the_qc_inspector_and_the_owner() {
    let app = manufacturing_booted().await;
    let owner = person(&app, "Omar", OWNER).await;
    for (key, role, days) in [
        ("halal_certificate", "qc_inspector", [90, 30, 7]),
        ("iso_certificate", "qc_inspector", [90, 30, 7]),
        ("other_certification", "qc_inspector", [90, 30, 7]),
        ("trademark_registration", "owner", [180, 90, 30]),
    ] {
        let kind = DocumentKind::by_key(app.db(), key)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{key}"));
        assert!(kind.enabled, "{key}");
        assert_eq!(kind.responsible_role.as_deref(), Some(role), "{key}");
        assert_eq!(kind.reminder_offsets.0, days, "{key}");
        assert!(kind.blocks(app.db()).await.unwrap().is_empty(), "{key}");
    }
    let awp = entity(&app, "AWP").await;
    let subject = Subject::new("kernel.legal_entity", awp.id);
    let kind = DocumentKind::by_key(app.db(), "trademark_registration")
        .await
        .unwrap()
        .unwrap();
    wl_documents::Document::create(
        app.db(),
        wl_documents::Document {
            subject_type: subject.subject_type.clone(),
            subject_id: subject.subject_id,
            kind_id: kind.id,
            title: "Al Wasl mark".into(),
            expiry_date: Some(today() + Days::new(90)),
            current_version: 1,
            status: DocumentStatus::Accepted,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    expiry::run(app.state(), today()).await.unwrap();
    assert_eq!(told(&app, &owner).await, 1, "the owner is told at 90 days");
}

/// The settings pages show the 07 rows, each with its value; the company
/// admin sees them.
#[renox::test]
async fn an_admin_sees_the_07_settings() {
    let app = manufacturing_booted().await;
    let admin = person(&app, "Ada", ADMIN).await;
    app.acting_as(&admin);
    app.get("/settings/compliance")
        .await
        .assert_ok()
        .assert_see("ECAS")
        .assert_see("never")
        .assert_see("MFG.RUL.08")
        .assert_see("L-19")
        .assert_see("mfg.recall_notice")
        .assert_see("Non-conformance and quarantine");
    app.get("/settings/negative-flow")
        .await
        .assert_ok()
        .assert_see("Cancellation outcomes offered")
        .assert_see("AED 2,000.00")
        .assert_see("AED 200,000.00")
        .assert_see("A-04");
}

/// A member of the staff outside the roles sees none of the settings pages.
#[renox::test]
async fn a_member_sees_no_settings_page() {
    let app = manufacturing_booted().await;
    let member = person(&app, "Mia", MEMBER).await;
    app.acting_as(&member);
    app.get("/settings/negative-flow").await.assert_forbidden();
    app.get("/settings/compliance").await.assert_forbidden();
    let _ = PartyId(0);
}
