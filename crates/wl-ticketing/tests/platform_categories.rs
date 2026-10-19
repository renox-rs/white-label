//! BL-TKT-11 (#156): the categories other modules open tickets in for their
//! negative flows: seeded with the module that opens them, renamed but not
//! deleted, and the labour case confidential (`WRK.ACL.03`).

mod support;

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::tickets::{Assignee, OpenTicket, task_creator};
use wl_ticketing::config::FieldKind;
use wl_ticketing::seed::{
    BILLING_DISPUTE_CATEGORY, DELIVERY_CLAIM_CATEGORY, LABOUR_CASE_CATEGORY,
};
use wl_ticketing::visibility::VIEWED;
use wl_ticketing::{Category, Ticket, WlTicketing, permissions as perm};

use support::{Booted, boot, boot_client};

/// A client with every module that opens a platform category on.
const ALL_ON: &str = r#"
[client]
name = "Gulf Staffing LLC"
app = "manpower"

[modules]
enabled = ["parties", "ticketing", "invoicing", "hr", "delivery"]

[[legal_entities]]
code = "GST"
name_en = "Gulf Staffing LLC"
default = true
"#;

/// Invoicing and hr on, delivery off.
const NO_DELIVERY: &str = r#"
[client]
name = "Gulf Staffing LLC"
app = "manpower"

[modules]
enabled = ["parties", "ticketing", "invoicing", "hr"]

[[legal_entities]]
code = "GST"
name_en = "Gulf Staffing LLC"
default = true
"#;

async fn category(t: &Booted, key: &str) -> Option<Category> {
    Category::by_key(t.db(), key).await.unwrap()
}

/// AC1: with invoicing, hr and delivery on, the three categories exist with
/// the properties of the spec: fixed keys, Normal priority and the standard
/// SLA, the labour case confidential for the role `hr` with its custom
/// fields and no sign-off.
#[renox::test]
async fn tkt11_the_three_platform_categories_exist_with_their_properties() {
    let t = boot_client(ALL_ON, WlTicketing::new()).await;

    let dispute = category(&t, BILLING_DISPUTE_CATEGORY).await.expect("billing dispute");
    assert!(dispute.platform && dispute.active);
    assert!(!dispute.confidential && !dispute.requires_signoff);
    assert_eq!(dispute.default_priority, wl_ticketing::Priority::Normal);
    assert!(dispute.sla_policy_id.is_some());
    assert_eq!(dispute.name_en, "Billing dispute");

    let labour = category(&t, LABOUR_CASE_CATEGORY).await.expect("labour case");
    assert!(labour.platform && labour.confidential);
    assert!(!labour.requires_signoff);
    assert_eq!(labour.default_assignee_role.as_deref(), Some("hr"));
    assert_eq!(labour.default_priority, wl_ticketing::Priority::Normal);
    let fields: Vec<_> = labour
        .custom_fields
        .0
        .iter()
        .map(|f| (f.key.as_str(), f.kind))
        .collect();
    assert_eq!(
        fields,
        [
            ("forum", FieldKind::Choice),
            ("reference", FieldKind::Text),
            ("hearing_date", FieldKind::Date),
            ("reply_date", FieldKind::Date),
        ]
    );
    assert_eq!(labour.custom_fields.0[0].choices, ["mohre", "court"]);

    let claim = category(&t, DELIVERY_CLAIM_CATEGORY).await.expect("delivery claim");
    assert!(claim.platform && !claim.confidential && !claim.requires_signoff);
    assert_eq!(claim.default_priority, wl_ticketing::Priority::Normal);
    assert!(claim.name_ar.len() > 2);
}

/// AC2: with delivery off, there is no delivery claim; with no module on,
/// none of the three; seeding again adds nothing.
#[renox::test]
async fn tkt11_a_category_exists_only_when_its_module_is_on() {
    let t = boot_client(NO_DELIVERY, WlTicketing::new()).await;
    assert!(category(&t, BILLING_DISPUTE_CATEGORY).await.is_some());
    assert!(category(&t, LABOUR_CASE_CATEGORY).await.is_some());
    assert!(category(&t, DELIVERY_CLAIM_CATEGORY).await.is_none());

    let plain = boot().await;
    for key in [BILLING_DISPUTE_CATEGORY, LABOUR_CASE_CATEGORY, DELIVERY_CLAIM_CATEGORY] {
        assert!(category(&plain, key).await.is_none(), "{key}");
    }

    let before = Category::query().count(t.db()).await.unwrap();
    t.app
        .kernel()
        .call("ticketing:install", Vec::<String>::new())
        .await
        .unwrap();
    assert_eq!(Category::query().count(t.db()).await.unwrap(), before);
}

/// AC3: deleting or turning off *billing dispute* is refused; renaming it to
/// "Invoice query" works, the key stays, and a ticket is still opened in it.
#[renox::test]
async fn tkt11_a_platform_category_is_renamed_not_deleted() {
    let t = boot_client(ALL_ON, WlTicketing::new()).await;
    let manager = t.user("Mina", &[perm::VIEW, perm::MANAGE]).await;
    t.app.acting_as(&manager);

    t.app
        .htmx()
        .delete("/ticket-categories/billing_dispute")
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(
            "/ticket-categories/billing_dispute/active",
            &[("active", "0")],
        )
        .await
        .assert_status(422);
    let dispute = category(&t, BILLING_DISPUTE_CATEGORY).await.unwrap();
    assert!(dispute.active);

    t.app
        .htmx()
        .post(
            "/ticket-categories/billing_dispute",
            &[("name_en", "Invoice query"), ("name_ar", "استفسار عن فاتورة")],
        )
        .await
        .assert_ok();
    let renamed = category(&t, BILLING_DISPUTE_CATEGORY).await.unwrap();
    assert_eq!(renamed.name_en, "Invoice query");
    assert_eq!(renamed.key, "billing_dispute");
    t.app
        .get("/ticket-categories")
        .await
        .assert_ok()
        .assert_see("Invoice query");

    let opened = task_creator(t.state())
        .open(
            t.state(),
            OpenTicket::new(
                Subject::new("invoicing.invoice", 7),
                "billing_dispute",
                "Dispute on INV-ACME-2026-0310",
            ),
        )
        .await
        .unwrap()
        .expect("ticketing is on");
    assert!(opened.number.starts_with("TKT-"));

    // Another user cannot change categories at all.
    let viewer = t.user("Vera", &[perm::VIEW]).await;
    t.app.acting_as(&viewer);
    t.app
        .htmx()
        .delete("/ticket-categories/billing_dispute")
        .await
        .assert_status(403);
}

/// A category of the client's own is deleted when no ticket uses it, and
/// kept when one does.
#[renox::test]
async fn tkt11_an_ordinary_category_is_deleted_only_without_tickets() {
    let t = boot().await;
    let manager = t.user("Mina", &[perm::VIEW, perm::MANAGE]).await;
    Category::create(
        t.db(),
        Category {
            key: "noise".into(),
            name_en: "Noise".into(),
            active: true,
            ..Category::default()
        },
    )
    .await
    .unwrap();
    t.open("general", None).await;
    t.app.acting_as(&manager);
    t.app
        .htmx()
        .delete("/ticket-categories/general")
        .await
        .assert_status(422);
    assert!(category(&t, "general").await.is_some());
    // The reply goes back to the page; what counts is that the row is gone.
    let _ = t.app.htmx().delete("/ticket-categories/noise").await;
    assert!(category(&t, "noise").await.is_none());
}

/// AC4: a labour case ticket is not visible to a site supervisor: not in
/// the list, not on its page, no action on it; a holder of
/// `tickets.confidential.view` and the requester see it, and each view is
/// audited.
#[renox::test]
async fn tkt11_a_labour_case_is_not_visible_to_a_site_supervisor() {
    let t = boot_client(ALL_ON, WlTicketing::new()).await;
    let supervisor = t.user("Sami", &[perm::VIEW, perm::WORK, perm::ASSIGN]).await;
    let hr = t
        .user("Hana", &[perm::VIEW, perm::WORK, perm::CONFIDENTIAL_VIEW])
        .await;
    let worker = t.user("Walid", &[perm::VIEW]).await;
    let ticket = t.open(LABOUR_CASE_CATEGORY, Some(&worker)).await;
    assert!(ticket.confidential);
    let ordinary = t.open("general", None).await;
    assert!(!ordinary.confidential);
    let page = format!("/tickets/{}", ticket.id);

    t.app.acting_as(&supervisor);
    t.app.get(&page).await.assert_not_found();
    t.app
        .htmx()
        .post(&format!("{page}/actions/start"), &[])
        .await
        .assert_not_found();
    t.app
        .htmx()
        .post(&format!("{page}/cancel"), &[("reason", "no")])
        .await
        .assert_not_found();
    t.app
        .get("/tickets")
        .await
        .assert_ok()
        .assert_dont_see(&ticket.number);
    t.app
        .get(&format!("/tickets/{}", ordinary.id))
        .await
        .assert_ok();
    let before = renox::audit::for_subject(t.db(), Ticket::TABLE, ticket.id, 10)
        .await
        .unwrap();
    assert!(before.iter().all(|e| e.action != VIEWED));

    t.app.acting_as(&hr);
    t.app.get(&page).await.assert_ok();
    t.app.acting_as(&worker);
    t.app.get(&page).await.assert_ok();

    let entries = renox::audit::for_subject(t.db(), Ticket::TABLE, ticket.id, 10)
        .await
        .unwrap();
    let viewers: Vec<Option<i64>> = entries
        .iter()
        .filter(|e| e.action == VIEWED)
        .map(|e| e.user_id)
        .collect();
    assert_eq!(viewers.len(), 2, "{entries:?}");
    assert!(viewers.contains(&Some(hr.id)) && viewers.contains(&Some(worker.id)));
}

/// The category's assignee is the one the opening module names: a dispute
/// opened for the invoice owner is assigned to them, one opened without an
/// owner stays open for triage.
#[renox::test]
async fn tkt11_a_billing_dispute_takes_the_assignee_its_module_resolves() {
    let t = boot_client(ALL_ON, WlTicketing::new()).await;
    let nadia = t.coordinator().await;
    let creator = task_creator(t.state());
    let invoice = Subject::new("invoicing.invoice", 310);

    let owned = creator
        .open(
            t.state(),
            OpenTicket::new(invoice.clone(), "billing_dispute", "Dispute on INV-ACME-2026-0310")
                .assign_to(Assignee::user(nadia.id)),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(t.ticket(owned.id.0).await.assignee(), Some(Assignee::user(nadia.id)));

    let unowned = creator
        .open(
            t.state(),
            OpenTicket::new(invoice, "billing_dispute", "Dispute on INV-ACME-2026-0311"),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(t.ticket(unowned.id.0).await.assignee(), None);
}
