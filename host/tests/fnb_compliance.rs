//! BL-FNB-17 (#381), 06 §3.3, §4.1, §4.4, §4.6, §5, §7.2, `X-08`, `X-09`,
//! `X-10`, `X-13`, `X-19`, `06-N10-b`, `AU-15`, `AU-20`, `A-15`: the 06
//! deployment's values for the shared stock, document, kernel, connection,
//! notice, sales and staff mechanisms are in force in a fresh 06 deployment
//! (`wl:install` and the seeds), read from the real modules (their settings
//! services, registries, seeds and jobs), not from constants. A test names the
//! Done-when point it covers; the mechanisms themselves (a stock hold, a
//! recall, an outage) are run end to end by the tests of `apps/fnb` and of
//! the modules.

mod support;

use fnb::catalogue::{OUTLET_SUBJECT_TYPE, Outlet};
use host::fnb_compliance as comp;
use host::roles::{
    ADMIN, AREA_MANAGER, HEAD_CHEF, KITCHEN_MANAGER, LINE_COOK, MEMBER, OPERATIONS_DIRECTOR,
    STOREKEEPER,
};
use renox::auth::permissions::{Scope, users_with_permission_in};
use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_documents::{Document, DocumentKind, DocumentStatus, Sop, SopReader, expiry};
use wl_hr::OnboardingTemplate;
use wl_inventory::{Item, Location, NegativeStockPolicy};
use wl_kernel::compliance::service::recipients;
use wl_kernel::compliance::{AlertKind, Raised, is_critical, raise, schedule_subject};
use wl_kernel::documents::today;
use wl_kernel::integrations::{self, ConnectionStatus, IntegrationConnection, go_down, report_failure};
use wl_kernel::late_entries::limits_for;
use wl_kernel::notify::{GiveUpReaction, notification_type};
use wl_kernel::{PartyId, Subject};
use wl_sales::LeadSource;

use support::{
    BouncingMail, company, fnb_booted, fnb_booted_with, messages_to, person, scoped_person, tenant,
};

fn holds(role: &str, permission: &str) -> bool {
    host::roles::ROLES
        .iter()
        .find(|(name, _)| *name == role)
        .is_some_and(|(_, grants)| grants.contains(&permission))
}

fn day(days: i64) -> NaiveDate {
    if days >= 0 {
        today() + Days::new(days.unsigned_abs())
    } else {
        today() - Days::new(days.unsigned_abs())
    }
}

/// An outlet `code` with a stock location of the same name, as the demo
/// catalogue makes them, after the seed scoped them.
async fn outlet(app: &TestApp, code: &str) -> (Outlet, Location) {
    let mut location = Location::new(&code.to_uppercase(), code);
    location.save(app.db()).await.unwrap();
    let mut outlet = Outlet {
        code: code.to_owned(),
        name: code.to_owned(),
        location_id: Some(location.id),
        active: true,
        ..Outlet::default()
    };
    outlet.insert(app.db()).await.unwrap();
    (outlet, location)
}

/// AC (Stock 1): Outlet 1 sells 2 kg of onions (not lot-tracked) with 1 kg on
/// hand: the balance is -1 kg; the central kitchen blocks; lot-tracked cream
/// with nothing on hand creates a pending issue instead.
#[renox::test]
async fn an_outlet_kitchen_goes_negative_on_an_untracked_item_and_a_tracked_one_waits() {
    use wl_kernel::inventory::{ConsumeRequest, stock_service};
    let app = fnb_booted().await;
    let (_marina, kitchen) = outlet(&app, "marina").await;
    let (_central, store) = outlet(&app, "central").await;
    let changed = comp::outlet_stock(app.db()).await.unwrap();
    assert_eq!(changed, 2, "both locations are scoped");
    assert_eq!(
        Location::find_or_404(app.db(), kitchen.id)
            .await
            .unwrap()
            .negative_stock_policy,
        NegativeStockPolicy::AllowConsumption
    );
    assert_eq!(
        Location::find_or_404(app.db(), store.id)
            .await
            .unwrap()
            .negative_stock_policy,
        NegativeStockPolicy::Block,
        "the central kitchen blocks"
    );
    assert_eq!(comp::outlet_stock(app.db()).await.unwrap(), 0, "a second run changes nothing");

    let mut onions = Item::new("ONION", "Onions", "kg");
    onions.save(app.db()).await.unwrap();
    let mut cream = Item::new("CREAM", "Cream", "kg").lot_tracked();
    cream.save(app.db()).await.unwrap();
    let mut receipt = wl_inventory::StockMovement::receive(
        onions.id,
        None,
        kitchen.id,
        1_000,
        400,
        wl_inventory::item::value_of(1_000, 400),
        &Subject::new("inventory.goods_receipt", 900),
        None,
    );
    receipt.save(app.db()).await.unwrap();

    let sale = Subject::new("fnb.order", 1);
    let done = stock_service(app.state())
        .consume(
            app.state(),
            ConsumeRequest::new(onions.subject(), kitchen.subject(), 2_000, sale.clone())
                .cost_object(sale.clone()),
        )
        .await
        .unwrap();
    assert_eq!((done.posted_milli, done.short_milli), (2_000, 0));
    assert_eq!(
        wl_inventory::balance::on_hand_at(app.db(), onions.id, kitchen.id, None)
            .await
            .unwrap(),
        -1_000
    );
    assert_eq!(done.pending_issue, None);

    let creamed = stock_service(app.state())
        .consume(
            app.state(),
            ConsumeRequest::new(cream.subject(), kitchen.subject(), 1_000, sale.clone())
                .cost_object(sale),
        )
        .await
        .unwrap();
    assert!(creamed.pending_issue.is_some(), "lot-tracked cream waits");
    let open = wl_inventory::pending_issue::open_of(app.db()).await.unwrap();
    assert_eq!(open.len(), 1);
}

/// AC (Stock 2, 3, 4): rejecting a receipt line for `temperature` needs the
/// reading; a recall hold needs a second holder of `lots.release`, a
/// temperature hold does not; the stock values are the 06 ones.
#[renox::test]
async fn the_stock_values_are_the_06_ones() {
    let app = fnb_booted().await;
    let config = app
        .state()
        .provided::<wl_inventory::InventoryConfig>()
        .unwrap();
    let temperature = config
        .rejection_reasons
        .iter()
        .find(|r| r.code == "temperature")
        .expect("the reason temperature");
    assert!(temperature.evidence_required, "the reading is needed");
    assert!(config.release_needs_second_person("recall"));
    assert!(!config.release_needs_second_person("temperature"));
    assert_eq!(config.wastage_approval_fils, 20_000);
    assert_eq!(config.pending_alert_hours, 24);
    assert_eq!(config.return_chase_days, 30);
    assert_eq!(config.order_limits_fils, [500_000]);
    assert_eq!(config.reminder_hours("temperature", None), 2);
    assert_eq!(config.escalation_role("temperature", None), Some(KITCHEN_MANAGER));
    assert_eq!(config.score.window_months, 1);
    assert_eq!(config.score.threshold, 70);
    assert!(config.preferred_change_needs_approval);
    assert_eq!(config.recall_notices_route.as_deref(), Some("fnb.recall_trace"));
}

/// AC (Stock 4): Outlet 1's head chef decides a hold line at Outlet 1 and not
/// at Outlet 2; Outlet 1's kitchen manager decides as deputy (the role holds
/// `lots.release`), the stock location of the outlet carries the outlet's scope.
#[renox::test]
async fn an_outlets_head_chef_decides_its_hold_lines_and_not_the_other_outlets() {
    let app = fnb_booted().await;
    let (outlet1, location1) = outlet(&app, "outlet1").await;
    let (outlet2, location2) = outlet(&app, "outlet2").await;
    comp::outlet_stock(app.db()).await.unwrap();
    let scope1 = Location::find_or_404(app.db(), location1.id)
        .await
        .unwrap()
        .scope();
    let scope2 = Location::find_or_404(app.db(), location2.id)
        .await
        .unwrap()
        .scope();
    assert_eq!(scope1, Scope::new(OUTLET_SUBJECT_TYPE, outlet1.id));
    assert_eq!(scope2, Scope::new(OUTLET_SUBJECT_TYPE, outlet2.id));

    let chef = scoped_person(&app, "Samir", HEAD_CHEF, &outlet1.scope()).await;
    let deputy = scoped_person(&app, "Mounir", KITCHEN_MANAGER, &outlet1.scope()).await;
    let at = |scope: Scope| {
        let db = app.db();
        async move {
            users_with_permission_in(db, "lots.release", &scope)
                .await
                .unwrap()
                .into_iter()
                .map(|u| u.id)
                .collect::<Vec<_>>()
        }
    };
    let at_outlet1 = at(scope1).await;
    assert!(at_outlet1.contains(&chef.id), "Outlet 1's head chef");
    assert!(at_outlet1.contains(&deputy.id), "Outlet 1's kitchen manager");
    let at_outlet2 = at(scope2).await;
    assert!(!at_outlet2.contains(&chef.id), "not at Outlet 2");
    assert!(!at_outlet2.contains(&deputy.id), "not at Outlet 2");
}

/// AC (Compliance 1): a missed or failed `FNB.RUL.04` run alerts the owner /
/// operations director, and the company admin, once.
#[renox::test]
async fn a_failed_fnb_rul_04_alerts_the_operations_director() {
    let app = fnb_booted().await;
    for rule in comp::CRITICAL_RULES {
        assert!(is_critical(rule), "{rule}");
        assert_eq!(
            wl_kernel::compliance::rule(rule)
                .and_then(|r| r.owner_role_name().map(str::to_owned))
                .as_deref(),
            Some(OPERATIONS_DIRECTOR),
            "{rule}"
        );
    }
    let admin = person(&app, "Ada", ADMIN).await;
    let director = person(&app, "Omar", OPERATIONS_DIRECTOR).await;
    let raised = Raised::new(
        "FNB.RUL.04",
        schedule_subject(),
        "FNB.RUL.04:2026-11-10T08",
        AlertKind::Missed,
    );
    let alert = raise(app.state(), raised.clone())
        .await
        .unwrap()
        .expect("an alert");
    assert_eq!(alert.owner_role, OPERATIONS_DIRECTOR);
    let told = recipients(app.db(), &alert.owner_role).await.unwrap();
    assert!(told.contains(&director.id));
    assert!(told.contains(&admin.id), "the company admin is alerted too");
    assert!(raise(app.state(), raised).await.unwrap().is_none(), "once");
}

/// AC (Compliance 2): a health card expiring on 2026-10-31 alerts the owner
/// the day after and blocks nothing; its reminders go to the kitchen manager of
/// the employee's outlet, not of another outlet.
#[renox::test]
async fn an_expired_health_card_alerts_the_owner_and_blocks_nothing() {
    let app = fnb_booted().await;
    let kind = DocumentKind::by_key(app.db(), comp::HEALTH_CARD)
        .await
        .unwrap()
        .expect("the health card kind");
    assert!(kind.compliance_critical, "the owner is told on expiry");
    assert!(kind.blocks(app.db()).await.unwrap().is_empty(), "blocks nothing");
    assert_eq!(kind.responsible_role.as_deref(), Some(KITCHEN_MANAGER));
    assert_eq!(kind.reminder_offsets.0, comp::CARD_REMINDERS);

    let director = person(&app, "Omar", OPERATIONS_DIRECTOR).await;
    let marina = Scope::new(OUTLET_SUBJECT_TYPE, 1);
    let sharjah = Scope::new(OUTLET_SUBJECT_TYPE, 2);
    let mounir = scoped_person(&app, "Mounir", KITCHEN_MANAGER, &marina).await;
    let khalid = scoped_person(&app, "Khalid", KITCHEN_MANAGER, &sharjah).await;
    let cook = person(&app, "Rizwan", LINE_COOK).await;
    let party = tenant(&app, "Rizwan Ahmed").await;
    let employee = wl_hr::employees::create(
        app.state(),
        wl_hr::NewEmployee::new(party, "line_cook", day(-300))
            .user(cook.id)
            .scope(Subject::new(OUTLET_SUBJECT_TYPE, 1)),
    )
    .await
    .unwrap();
    // The card expired yesterday: the daily run of today alerts.
    Document::create(
        app.db(),
        Document {
            subject_type: employee.subject().subject_type.clone(),
            subject_id: employee.subject().subject_id,
            kind_id: kind.id,
            title: "Health card".to_owned(),
            expiry_date: Some(day(-1)),
            current_version: 1,
            status: DocumentStatus::Accepted,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let report = expiry::run(app.state(), today()).await.unwrap();
    assert_eq!(report.failed, 0, "{report:?}");
    assert!(
        !messages_to(&app, expiry::expired::NOTIFICATION_TYPE, &director)
            .await
            .is_empty(),
        "the owner is told"
    );
    assert!(
        !messages_to(&app, expiry::expired::NOTIFICATION_TYPE, &mounir)
            .await
            .is_empty(),
        "the kitchen manager of the employee's outlet is told"
    );
    assert!(
        messages_to(&app, expiry::expired::NOTIFICATION_TYPE, &khalid)
            .await
            .is_empty(),
        "not the other outlet's"
    );
}

/// AC (Compliance 3): the trade licence kind has the operations director as its
/// responsible role; the food permit and the PIC certificate the kitchen
/// manager; none blocks anything.
#[renox::test]
async fn the_trade_licence_is_renewed_by_the_operations_director() {
    let app = fnb_booted().await;
    for (key, role) in [
        (comp::TRADE_LICENCE, OPERATIONS_DIRECTOR),
        (comp::FOOD_PERMIT, KITCHEN_MANAGER),
        (comp::HEALTH_CARD, KITCHEN_MANAGER),
        (comp::PIC_CERTIFICATE, KITCHEN_MANAGER),
    ] {
        let kind = DocumentKind::by_key(app.db(), key)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{key}"));
        assert_eq!(kind.responsible_role.as_deref(), Some(role), "{key}");
        assert!(kind.compliance_critical, "{key}");
        assert!(kind.blocks(app.db()).await.unwrap().is_empty(), "{key}");
        assert!(kind.enabled, "{key}");
    }
}

/// The sender of the email that gives up, the type's give-up reaction and the
/// task of the give-up for a notice about `subject`.
async fn gives_up(
    app: &TestApp,
    type_key: &str,
    subject: Subject,
    party: PartyId,
    data: renox::serde_json::Value,
) -> Vec<wl_ticketing::Ticket> {
    use wl_kernel::notify::{self, NotifyRequest};
    let prefix = format!("notify.give_up:{}:", subject.subject_type);
    let request = NotifyRequest::new(type_key, subject)
        .subject_label("T-1")
        .to_party(party)
        .data(data);
    notify::notifier(app.state())
        .notify(app.state(), request)
        .await
        .unwrap();
    app.run_all_jobs().await;
    wl_ticketing::Ticket::query()
        .order_by(wl_ticketing::Ticket::ID)
        .get(app.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|t| t.source_key.as_deref().is_some_and(|k| k.starts_with(&prefix)))
        .collect()
}

/// AC (Notifications): the three 06 notification types are registered with
/// their groups; an order status message that gives up creates no task; a
/// recall notice that gives up gives the recall owner a task.
#[renox::test]
async fn the_three_notification_types_take_their_groups_and_give_up_as_06_says() {
    let mail = BouncingMail::default();
    let app = fnb_booted_with(host::Tuning::default().notify({
        let mail = mail.clone();
        move |n| n.provider(mail)
    }))
    .await;
    let status = notification_type(fnb::notices::CUSTOMER_NOTICE).expect("fnb.order_status");
    assert_eq!(status.key, "fnb.order_status");
    assert_eq!(status.group, "tracking");
    assert_eq!(status.give_up, [GiveUpReaction::None]);
    let otp = notification_type(fnb::notices::LOGIN_OTP).expect("fnb.login_otp");
    assert_eq!(otp.group, "otp");
    let recall = notification_type(fnb::recall::NOTICE_TYPE).expect("fnb.recall_notice");
    assert_eq!(recall.urgency, wl_kernel::notify::Urgency::Urgent);
    assert_eq!(recall.give_up, [GiveUpReaction::StaffTask]);
    assert_eq!(recall.staff_role.as_deref(), Some(OPERATIONS_DIRECTOR));

    let director = person(&app, "Omar", OPERATIONS_DIRECTOR).await;
    let customer = company(&app, "Layla Trading LLC").await;
    let tasks = gives_up(
        &app,
        fnb::notices::CUSTOMER_NOTICE,
        Subject::new("fnb.order", 1),
        customer,
        renox::serde_json::json!({ "title": "Your order", "body": "Is on its way" }),
    )
    .await;
    assert!(!mail.attempts().is_empty(), "the notice was e-mailed");
    assert!(tasks.is_empty(), "an order status that gives up opens no task");

    let other = company(&app, "Omar Trading LLC").await;
    let tasks = gives_up(
        &app,
        fnb::recall::NOTICE_TYPE,
        Subject::new("fnb.recall_notice", 1),
        other,
        renox::serde_json::json!({ "company": "Zaatar", "orders": "SO-1" }),
    )
    .await;
    assert_eq!(tasks.len(), 1, "one task for the recall owner");
    assert_eq!(tasks[0].assignee_user_id, Some(director.id));
}

/// AC (Kernel 1): `void` is offered on a payout statement, not on an order.
#[renox::test]
async fn void_is_offered_on_a_payout_statement_and_not_on_an_order() {
    let _app = fnb_booted().await;
    assert!(wl_kernel::voids::get(fnb::payout::SUBJECT_TYPE).is_some());
    assert!(wl_kernel::voids::get(fnb::SUBJECT_TYPE).is_none());
    assert!(wl_kernel::voids::get("sales.order").is_none());
}

/// AC (Kernel 2): a late wastage entry 30 hours after its business time needs
/// the area manager; the paper order, the wastage, the readings and the proof
/// have the limits M = 30 minutes and N = 24 hours.
#[renox::test]
async fn a_late_entry_after_24_hours_needs_the_area_manager() {
    use wl_kernel::late_entries::{Assessment, Period, assess};
    let app = fnb_booted().await;
    for subject_type in comp::LATE_ENTRY_TYPES {
        let limits = limits_for(app.db(), subject_type).await.unwrap();
        assert_eq!(limits.flag_after_minutes, 30, "{subject_type}");
        assert_eq!(limits.approval_after_hours, 24, "{subject_type}");
    }
    let limits = limits_for(app.db(), "fnb.wastage").await.unwrap();
    let business = renox::db::now() - renox::chrono::TimeDelta::hours(30);
    let verdict = assess(limits, business, renox::db::now(), &Period::Open);
    assert!(
        matches!(verdict, Assessment::NeedsApproval { .. }),
        "{verdict:?}"
    );
    assert!(holds(AREA_MANAGER, "late_entries.approve"));
    assert!(!holds(KITCHEN_MANAGER, "late_entries.approve"));
}

/// AC (Kernel 3, 4, 5): Deliverect, the payment gateway and the SMS gateway are
/// connections with a fallback owned by the kitchen manager or the
/// accountant; with the gateway `down` the checkout offers cash on delivery
/// only and hides online payment.
#[renox::test]
async fn with_the_gateway_down_the_checkout_offers_cash_on_delivery_only() {
    let app = fnb_booted().await;
    for (key, owner, words) in [
        (comp::DELIVERECT, KITCHEN_MANAGER, "quick entry"),
        (integrations::PAYMENT_GATEWAY, "accountant", "Cash on delivery"),
        (integrations::SMS, KITCHEN_MANAGER, "phone"),
    ] {
        let def = integrations::get(key).unwrap_or_else(|| panic!("{key}"));
        assert_eq!(def.owner_role_name(), owner, "{key}");
        assert!(def.fallback_en().contains(words), "{key}");
        assert!(def.fallback_ar().is_some(), "{key}");
        let row = integrations::ensure(app.db(), key).await.unwrap();
        assert_eq!(row.status, ConnectionStatus::Up, "{key}");
    }
    let cashier = person(&app, "Cathy", host::roles::CASHIER).await;
    let customer = company(&app, "Layla Trading LLC").await;
    app.acting_as(&cashier);
    app.get(&format!("/checkout/payment?party={}", customer.0))
        .await
        .assert_ok()
        .assert_see("Pay online")
        .assert_see("Pay cash on delivery");

    report_failure(app.state(), integrations::PAYMENT_GATEWAY, "timeout")
        .await
        .unwrap();
    let failing = IntegrationConnection::by_key(app.db(), integrations::PAYMENT_GATEWAY)
        .await
        .unwrap()
        .unwrap();
    app.travel(std::time::Duration::from_secs(16 * 60));
    app.at_travelled_time(go_down(app.state(), &failing))
        .await
        .unwrap();
    app.get(&format!("/checkout/payment?party={}", customer.0))
        .await
        .assert_ok()
        .assert_dont_see("Pay online")
        .assert_see("Pay cash on delivery")
        .assert_see("Online payment is not available right now");
}

/// AC (Sales 1): a new lead or party offers the nine sources; no source needs a
/// campaign; a cost entry over a campaign's budget waits for a manager.
#[renox::test]
async fn the_lead_sources_and_the_campaign_approval_are_the_06_ones() {
    let app = fnb_booted().await;
    for code in [
        "walk_in", "website", "social", "paid_ads", "aggregator", "referral", "event", "partner",
        "other",
    ] {
        let source = LeadSource::by_code(app.db(), code)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{code}"));
        assert!(source.active, "{code}");
    }
    let sales = wl_sales::config::config_of(app.state());
    assert!(sales.over_budget_approval);
    assert!(sales.campaign_required_sources.is_empty());
}

/// AC (Sales 2): the supplier scorecard window is 1 month, the threshold 70,
/// a change of the preferred supplier needs a second person.
#[renox::test]
async fn the_supplier_scorecard_review_is_below_70_over_one_month() {
    let app = fnb_booted().await;
    let config = app
        .state()
        .provided::<wl_inventory::InventoryConfig>()
        .unwrap();
    assert_eq!(config.score.window_months, 1);
    assert_eq!(config.score.threshold, 70);
    assert_eq!(config.score.weights, [40, 30, 15, 15]);
    assert!(config.preferred_change_needs_approval);
    assert!(holds(
        host::roles::AREA_MANAGER,
        wl_inventory::permissions::STOCK_APPROVE
    ));
}

/// AC (Sales 3): the loyalty programme earns 1 point per AED 1, 100 points are
/// an AED 5 discount of at most 50% of the order, points expire after 12
/// months with the reminder 30 days before, an adjustment above 500 points
/// waits for a manager, campaigns go 07:00-21:00 to at most 2 messages in 7 days
/// and a broadcast above 200 recipients waits for a second person.
#[renox::test]
async fn the_loyalty_and_campaign_values_are_the_06_ones() {
    let app = fnb_booted().await;
    let programme = wl_sales::LoyaltyProgramme::first_active(app.db())
        .await
        .unwrap()
        .expect("the programme");
    assert_eq!(programme.points_for(20_000), 200, "AED 200 earns 200 points");
    assert_eq!(programme.points_for(0), 0);
    assert_eq!(programme.point_value, 5);
    assert_eq!(programme.min_redeem_points, 100);
    assert_eq!(programme.max_share_bps, 5_000);
    assert_eq!(programme.validity_months, 12);
    let sales = wl_sales::config::config_of(app.state());
    assert_eq!(sales.expiry_reminder_days, 30);
    assert_eq!(sales.adjustment_limit, 500);
    assert!(sales.confirm_redemption_by_code);
    assert_eq!(wl_notify::cap::settings(app.state()).max, 2);
    assert_eq!(wl_notify::broadcast::settings(app.state()).approval_above, 200);
    let window = wl_notify::SendWindow::query()
        .where_(wl_notify::SendWindow::CHANNEL.eq(wl_kernel::notify::NotifyChannel::Sms))
        .first(app.db())
        .await
        .unwrap()
        .expect("the send window");
    assert_eq!((window.start, window.end), (7 * 60, 21 * 60));
    for name in ["Regulars", "Lapsed customers"] {
        let found = wl_sales::Segment::query()
            .where_(wl_sales::Segment::NAME.eq(name))
            .exists(app.db())
            .await
            .unwrap();
        assert!(found, "{name}");
    }
}

/// AC (Staff 1): hiring a line cook creates the line cook checklist with
/// tasks to the kitchen manager, the head chef and the employee; the role is
/// active before it is complete; the SOP library has the owners and readers of
/// §5, and a second run of the seed changes nothing.
#[renox::test]
async fn hiring_a_line_cook_starts_the_checklist_and_the_sops_are_seeded() {
    let app = fnb_booted().await;
    for (name, role) in [
        ("Line cook onboarding", "line_cook"),
        ("Cashier and order taker onboarding", "cashier"),
        ("Rider onboarding", "driver"),
        ("Storekeeper and purchaser onboarding", "storekeeper"),
        ("Outlet and kitchen manager onboarding", "kitchen_manager"),
    ] {
        let template = OnboardingTemplate::query()
            .where_(OnboardingTemplate::NAME_EN.eq(name))
            .first(app.db())
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{name}"));
        assert_eq!(template.job_roles, role);
        assert!(!template.blocks_activation, "{name}: completion blocks nothing");
    }
    let cook = person(&app, "Rizwan", LINE_COOK).await;
    let party = tenant(&app, "Rizwan Ahmed").await;
    let employee = wl_hr::employees::create(
        app.state(),
        wl_hr::NewEmployee::new(party, "line_cook", today()).user(cook.id),
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
    for owner in ["role:kitchen_manager", "role:head_chef", "employee", "manager"] {
        assert!(owners.contains(&owner), "{owner}");
    }
    assert_eq!(
        comp::hr(wl_hr::WlHr::new()).config().manager_role,
        KITCHEN_MANAGER
    );

    let food = Sop::query()
        .where_(Sop::CODE.eq("SOP-FNB-FOOD-SAFETY"))
        .first(app.db())
        .await
        .unwrap()
        .expect("seeded");
    assert_eq!(food.owner_role, HEAD_CHEF);
    assert_eq!(food.ack_due_days, 7);
    let readers: Vec<String> = SopReader::query()
        .where_(SopReader::SOP_ID.eq(food.id))
        .get(app.db())
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.role_key)
        .collect();
    assert!(readers.contains(&LINE_COOK.to_owned()));
    let cash = Sop::query()
        .where_(Sop::CODE.eq("SOP-FNB-CASH"))
        .first(app.db())
        .await
        .unwrap()
        .expect("seeded");
    assert_eq!(cash.owner_role, AREA_MANAGER);

    let sops = Sop::query().count(app.db()).await.unwrap();
    let installed = comp::install(app.state()).await.unwrap();
    assert_eq!(installed.total(), 0, "a second run changes nothing");
    assert_eq!(Sop::query().count(app.db()).await.unwrap(), sops);
}

/// AC (Registrations): a fresh 06 deployment lists *Halal certificate*,
/// *Trademark registration* and *Other certification* with the storekeeper for
/// Halal and the operations director for the others, no blocks; a trademark
/// expiring in 90 days notifies the operations director.
#[renox::test]
async fn the_registration_kinds_have_their_roles_and_a_trademark_notifies_the_owner() {
    let app = fnb_booted().await;
    let director = person(&app, "Omar", OPERATIONS_DIRECTOR).await;
    for (key, role) in [
        ("halal_certificate", STOREKEEPER),
        ("trademark_registration", OPERATIONS_DIRECTOR),
        ("other_certification", OPERATIONS_DIRECTOR),
    ] {
        let kind = DocumentKind::by_key(app.db(), key)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{key}"));
        assert_eq!(kind.responsible_role.as_deref(), Some(role), "{key}");
        assert!(kind.enabled, "{key}");
        assert!(kind.blocks(app.db()).await.unwrap().is_empty(), "{key}");
    }
    let trademark = DocumentKind::by_key(app.db(), "trademark_registration")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(trademark.reminder_offsets.0, [180, 90, 30]);
    let zcd = support::entity(&app, "ZCD").await;
    Document::create(
        app.db(),
        Document {
            subject_type: "kernel.legal_entity".to_owned(),
            subject_id: zcd.id,
            kind_id: trademark.id,
            title: "Zaatar mark".to_owned(),
            expiry_date: Some(day(90)),
            current_version: 1,
            status: DocumentStatus::Accepted,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    expiry::run(app.state(), today()).await.unwrap();
    let messages = wl_notify::Message::query().get(app.db()).await.unwrap();
    assert!(
        messages.iter().any(|m| m.user_id == Some(director.id)),
        "the operations director is told"
    );
}

/// The compliance page: the operations director and the company admin read the
/// 06 rows; a member does not.
#[renox::test]
async fn the_operations_director_sees_the_06_compliance_settings_and_a_member_does_not() {
    let app = fnb_booted().await;
    let director = person(&app, "Omar", OPERATIONS_DIRECTOR).await;
    app.acting_as(&director);
    app.get("/settings/compliance")
        .await
        .assert_ok()
        .assert_see("Negative stock policy")
        .assert_see("Compliance-critical rules")
        .assert_see("FNB.RUL.04")
        .assert_see("Health card and PIC certificate expiry")
        .assert_see("AU-15")
        .assert_see("A-15");
    let admin = person(&app, "Ada", ADMIN).await;
    app.acting_as(&admin);
    app.get("/settings/compliance").await.assert_ok();
    let member = person(&app, "Mia", MEMBER).await;
    app.acting_as(&member);
    app.get("/settings/compliance").await.assert_forbidden();
}
