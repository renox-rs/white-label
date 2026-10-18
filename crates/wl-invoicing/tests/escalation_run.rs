//! The daily run of `FIN.RUL.15` (#68 part 3): the current step, one notice
//! per user, no repeat. The full acceptance checks are in
//! `dispute_escalation.rs`.

mod support;

use renox::auth::permissions::{self, Scope};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::{EscalationLog, dispute, escalation};
use wl_kernel::LegalEntity;
use wl_kernel::invoicing::{DisputeBy, DisputeRequest, InvoiceId, VatCategory};

use support::boot;

const LINE: [(i64, VatCategory); 1] = [(100_000, VatCategory::Standard)];

fn day(month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, month, day).unwrap()
}

#[renox::test]
async fn an_open_dispute_is_told_at_each_whole_period_and_never_twice() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = t.issued(&LINE, Some(t.day(10)), &a).await;
    let entity = LegalEntity::find(t.db(), invoice.legal_entity_id.expect("an entity"))
        .await
        .unwrap()
        .unwrap();
    let f = User::register(t.db(), "Farah", "farah@example.com", "password123")
        .await
        .unwrap();
    permissions::define_role(t.db(), "finance_manager", &["invoices.view"])
        .await
        .unwrap();
    f.assign_role_in(
        t.db(),
        "finance_manager",
        &Scope::of_id::<LegalEntity>(entity.id),
    )
    .await
    .unwrap();

    let request = DisputeRequest::new(
        InvoiceId(invoice.id),
        "wrong hours",
        DisputeBy::User { id: a.id },
    );
    let id = dispute::raise(t.state(), request.amount(20_000))
        .await
        .unwrap();
    let mut row = dispute::Dispute::find(t.db(), id).await.unwrap().unwrap();
    row.raised_at = day(11, 1).and_hms_opt(9, 0, 0).unwrap().and_utc();
    row.save(t.db()).await.unwrap();

    assert_eq!(escalation::run(t.state(), day(11, 14)).await.unwrap(), 0);
    assert_eq!(escalation::run(t.state(), day(11, 15)).await.unwrap(), 1);
    assert_eq!(escalation::run(t.state(), day(11, 15)).await.unwrap(), 0);
    assert_eq!(escalation::run(t.state(), day(11, 16)).await.unwrap(), 0);
    assert_eq!(EscalationLog::told(t.db(), id, 1).await.unwrap(), [a.id]);

    assert_eq!(escalation::run(t.state(), day(11, 29)).await.unwrap(), 2);
    let mut told = EscalationLog::told(t.db(), id, 2).await.unwrap();
    told.sort_unstable();
    assert_eq!(told, [a.id.min(f.id), a.id.max(f.id)]);
    assert_eq!(
        t.notices.of_type(escalation::ESCALATED).len(),
        3,
        "one notice per user per step"
    );
}
