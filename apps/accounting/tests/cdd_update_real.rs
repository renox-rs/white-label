//! Acceptance checks of #336 (BL-ACC-12): an expired KYC document of a client
//! being served opens a CDD update task for the compliance officer, due 14
//! days after the expiry, with the engagement's manager told; the engagement
//! stays `active`; the renewal completes the task; a second `DocumentExpired`
//! for the same document opens nothing. With the real wl-documents (its
//! nightly expiry run emits the event) and wl-parties.

mod support;

use accounting::kyc_updates::{CDD_UPDATE_NOTICE, source_key};
use accounting::seed::{PASSPORT, TRADE_LICENCE};
use accounting::{CddFile, Engagement, EngagementStatus};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_documents::Documents;
use wl_kernel::documents::DocumentExpired;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::tickets::Assignee;
use wl_kernel::{PartyId, Subject};

use support::{Booted, Client, boot, d, dubai};

/// The prefix of the CDD update tasks' keys.
const KEY: &str = "accounting.cdd_update:";

/// `engagement` is run by the manager `manager`.
async fn managed_by(t: &Booted, engagement: &Engagement, manager: &User) {
    Engagement::query()
        .where_(Engagement::ID.eq(engagement.id))
        .update(t.db(), &[("manager_user_id", &Some(manager.id))])
        .await
        .unwrap();
}

/// `DocumentExpired` for the current document of `kind` about `holder`, as
/// wl-documents emits it.
async fn expired_again(t: &Booted, holder: PartyId, kind: &str, expiry: NaiveDate) {
    let subject: Subject = holder.subject();
    let document = Documents::current(t.db(), &subject, kind)
        .await
        .unwrap()
        .expect("the document exists");
    t.state()
        .emit(DocumentExpired {
            document: document.document_id(),
            subject,
            kind: kind.to_owned(),
            expires_on: expiry,
            blocks: Vec::new(),
            blocked: Vec::new(),
        })
        .await
        .unwrap();
}

/// DW1-DW3: the trade licence of an `active` client expires on 2026-11-10;
/// the run of 2026-11-11 gives the MLRO a CDD update task due 2026-11-24
/// about the CDD file, the manager is told, the engagement stays `active`;
/// a second event opens no second task; the accepted renewal completes it.
#[renox::test]
async fn bl_acc_12_an_expired_trade_licence_opens_one_cdd_update_task_until_renewed() {
    let t = boot().await;
    let mlro = t.mlro().await;
    let manager = t.manager().await;
    let reviewer = t.reviewer().await;
    let client: Client = t.client().await;
    let file = t.cdd_approved(&client, &mlro).await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    managed_by(&t, &engagement, &manager).await;
    t.kyc(&reviewer, client.company, TRADE_LICENCE, d(11, 10))
        .await;

    let report = t
        .run(wl_documents::expiry::run::run(t.state(), d(11, 11)))
        .await
        .unwrap();
    assert!(report.expired_events >= 1, "{report:?}");

    let tasks = t.tickets.with_key(KEY);
    assert_eq!(tasks.len(), 1, "{tasks:?}");
    let task = &tasks[0];
    assert!(task.open);
    assert_eq!(
        task.key.as_deref(),
        Some(source_key(client.company, client.company, TRADE_LICENCE).as_str())
    );
    assert_eq!(task.assignee, Some(Assignee::user(mlro.id)));
    assert_eq!(task.due_at, Some(dubai(24, 9, 0)), "due 2026-11-24");
    let current: CddFile = t.cdd(&file).await;
    assert_eq!(task.subject, current.subject());
    assert_eq!(t.status(&engagement).await, EngagementStatus::Active);
    assert_eq!(t.cdd(&file).await.status, file.status, "the CDD file stays");
    assert!(
        t.notices
            .all()
            .iter()
            .any(|r| r.type_key == CDD_UPDATE_NOTICE
                && r.recipients
                    .contains(&NotifyRecipient::User { id: manager.id })),
        "the manager watches it"
    );

    expired_again(&t, client.company, TRADE_LICENCE, d(11, 10)).await;
    assert_eq!(t.tickets.with_key(KEY).len(), 1, "no second task");

    t.kyc(
        &reviewer,
        client.company,
        TRADE_LICENCE,
        NaiveDate::from_ymd_opt(2027, 11, 10).unwrap(),
    )
    .await;
    let tasks = t.tickets.with_key(KEY);
    assert_eq!(tasks.len(), 1);
    assert!(!tasks[0].open, "the renewal completes the task");
    assert_eq!(t.status(&engagement).await, EngagementStatus::Active);
}

/// A UBO's expired passport gives a task to each client being served (a
/// `suspended` one too), none to an `onboarding` client, and never moves an
/// engagement.
#[renox::test]
async fn bl_acc_12_a_ubo_passport_gives_a_task_per_served_client_only() {
    let t = boot().await;
    let mlro = t.mlro().await;
    let reviewer = t.reviewer().await;
    let client = t.client().await;
    t.cdd_approved(&client, &mlro).await;
    let suspended = t
        .engagement_in(&client, EngagementStatus::Suspended, Some("non_payment"))
        .await;
    let onboarding_client = t.client_named(" B").await;
    let onboarding = t.engagement(&onboarding_client).await;
    t.kyc(&reviewer, client.ubo, PASSPORT, d(11, 10)).await;
    t.kyc(
        &reviewer,
        onboarding_client.company,
        TRADE_LICENCE,
        d(11, 10),
    )
    .await;

    t.run(wl_documents::expiry::run::run(t.state(), d(11, 11)))
        .await
        .unwrap();

    let tasks = t.tickets.with_key(KEY);
    assert_eq!(tasks.len(), 1, "{tasks:?}");
    assert_eq!(
        tasks[0].key.as_deref(),
        Some(source_key(client.company, client.ubo, PASSPORT).as_str())
    );
    assert_eq!(t.status(&suspended).await, EngagementStatus::Suspended);
    assert_eq!(t.status(&onboarding).await, EngagementStatus::Onboarding);
}
