//! The recipients and the recipient log of the dispute escalation
//! (wl-invoicing §5.10, #68 part 2): the shared owner resolution, the
//! finance manager holders in the invoice's entity scope, and the log's
//! one row per dispute, step and user.

mod support;

use renox::auth::permissions::{self, Scope};
use renox::prelude::*;
use wl_invoicing::dispute;
use wl_invoicing::owner::{self, OwnerRole};
use wl_invoicing::{EscalationLog, Invoice};
use wl_kernel::LegalEntity;
use wl_kernel::invoicing::{DisputeBy, DisputeRequest, InvoiceId, VatCategory};

use support::{Booted, boot};

const LINE: [(i64, VatCategory); 1] = [(100_000, VatCategory::Standard)];
const ROLE: &str = "finance_manager";

async fn invoice(t: &Booted, owner: &User) -> Invoice {
    t.issued(&LINE, Some(t.day(10)), owner).await
}

async fn manager(t: &Booted, name: &str, entity: Option<&LegalEntity>) -> User {
    let user = User::register(
        t.db(),
        name,
        &format!("{}@example.com", name.to_lowercase()),
        "password123",
    )
    .await
    .unwrap();
    permissions::define_role(t.db(), ROLE, &["invoices.view"])
        .await
        .unwrap();
    match entity {
        Some(entity) => user
            .assign_role_in(t.db(), ROLE, &Scope::of_id::<LegalEntity>(entity.id))
            .await
            .unwrap(),
        None => user.assign_role(t.db(), ROLE).await.unwrap(),
    }
    user
}

#[renox::test]
async fn the_owner_who_still_issues_is_the_only_recipient() {
    let t = boot().await;
    let a = t.user_a().await;
    let _other = t.clerk("Other").await;
    let invoice = invoice(&t, &a).await;

    let (role, users) = owner::recipients(t.db(), &invoice).await.unwrap();
    assert_eq!(role, OwnerRole::Owner);
    assert_eq!(users.iter().map(|u| u.id).collect::<Vec<_>>(), [a.id]);
}

#[renox::test]
async fn without_a_working_owner_every_issuer_is_a_recipient() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.clerk("Bilal").await;
    let viewer = t.viewer().await;
    let mut invoice = invoice(&t, &a).await;
    invoice.owner_id = Some(viewer.id);

    let (role, users) = owner::recipients(t.db(), &invoice).await.unwrap();
    assert_eq!(role, OwnerRole::Receivables);
    let ids: Vec<i64> = users.iter().map(|u| u.id).collect();
    assert!(ids.contains(&a.id) && ids.contains(&b.id));
    assert!(!ids.contains(&viewer.id));
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));

    invoice.owner_id = None;
    let (role, _) = owner::recipients(t.db(), &invoice).await.unwrap();
    assert_eq!(role, OwnerRole::Receivables);
}

#[renox::test]
async fn finance_managers_are_those_of_the_entity_and_the_global_ones() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = invoice(&t, &a).await;
    let entity = LegalEntity::find(t.db(), invoice.legal_entity_id.expect("an entity"))
        .await
        .unwrap()
        .unwrap();
    let other = t.new_entity("OTHER").await;

    let here = manager(&t, "Here", Some(&entity)).await;
    let there = manager(&t, "There", Some(&other)).await;
    let global = manager(&t, "Global", None).await;

    let ids: Vec<i64> = owner::finance_managers(t.db(), ROLE, &invoice)
        .await
        .unwrap()
        .iter()
        .map(|u| u.id)
        .collect();
    assert!(ids.contains(&here.id));
    assert!(ids.contains(&global.id));
    assert!(!ids.contains(&there.id));
}

#[renox::test]
async fn a_user_has_one_log_row_per_dispute_and_step() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = invoice(&t, &a).await;
    let request = DisputeRequest::new(
        InvoiceId(invoice.id),
        "wrong hours",
        DisputeBy::User { id: a.id },
    );
    let dispute_id = dispute::raise(t.state(), request.amount(20_000))
        .await
        .unwrap();

    let first = EscalationLog::claim(t.db(), dispute_id, invoice.id, 1, a.id, "owner")
        .await
        .unwrap();
    assert!(first.is_some());
    // The same user at the same step is skipped; another step is not.
    assert!(
        EscalationLog::claim(t.db(), dispute_id, invoice.id, 1, a.id, "owner")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        EscalationLog::claim(t.db(), dispute_id, invoice.id, 2, a.id, "owner")
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        EscalationLog::told(t.db(), dispute_id, 1).await.unwrap(),
        [a.id]
    );

    // A failed send removes the row, so a retry tells the user.
    first.unwrap().delete(t.db()).await.unwrap();
    assert!(
        EscalationLog::told(t.db(), dispute_id, 1)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        EscalationLog::claim(t.db(), dispute_id, invoice.id, 1, a.id, "owner")
            .await
            .unwrap()
            .is_some()
    );
}
