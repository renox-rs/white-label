//! The demo client with `parties` and `notify` enabled: a hard bounce marks
//! the real wl-parties contact point invalid (#58 AC8).

use std::sync::Arc;

use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::notify::{
    self, Category, GiveUpReaction, NotificationTypeDef, NotifyChannel, NotifyRequest,
};
use wl_notify::channels::{HardError, ProviderFuture, Providers};
use wl_notify::{ChannelProvider, MessageStatus, Outgoing, SendError};
use wl_parties::{ContactPoint, ContactStatus, ContactType, Party};

use host::roles::{self, ADMIN};

/// An e-mail provider whose every send is a hard bounce.
struct Bounces;

impl ChannelProvider for Bounces {
    fn name(&self) -> &'static str {
        "bouncing-email"
    }

    fn channel(&self) -> NotifyChannel {
        NotifyChannel::Email
    }

    fn available(&self, _state: &AppState) -> bool {
        true
    }

    fn send<'a>(
        &'a self,
        _state: &'a AppState,
        _outgoing: &'a Outgoing,
    ) -> ProviderFuture<'a, std::result::Result<String, SendError>> {
        Box::pin(async {
            Err(SendError::hard(
                HardError::HardBounce,
                "550",
                "mailbox does not exist",
            ))
        })
    }
}

#[renox::test]
async fn a_hard_bounce_marks_a_seeded_partys_contact_point_invalid() {
    let mut providers = Providers::default();
    providers.replace(Arc::new(Bounces));
    let app = TestApp::new(host::app().provide(providers)).await;
    app.kernel().seed().await.unwrap();
    let al_noor = Party::query()
        .where_(Party::LEGAL_NAME_EN.eq("Al Noor Trading LLC"))
        .first(app.db())
        .await
        .unwrap()
        .expect("the seeded party");
    let email = ContactPoint::query()
        .where_(ContactPoint::PARTY_ID.eq(al_noor.id))
        .get(app.db())
        .await
        .unwrap()
        .into_iter()
        .find(|p| p.contact_type == ContactType::Email)
        .expect("an e-mail contact point");
    assert_eq!(email.status, ContactStatus::Valid);

    notify::register_type(
        NotificationTypeDef::new("host_test.invoice", "invoicing")
            .category(Category::Reminder)
            .give_up(&[
                GiveUpReaction::MarkContactInvalid,
                GiveUpReaction::StaffTask,
            ])
            .template(
                NotifyChannel::Email,
                "en",
                Some("Invoice {{ number }}"),
                "Invoice {{ number }} is issued.",
            ),
    );
    app.kernel()
        .call("notify:install", Vec::<String>::new())
        .await
        .unwrap();
    let request = NotifyRequest::new(
        "host_test.invoice",
        wl_kernel::Subject::new("invoicing.invoice", 1),
    )
    .to_party(al_noor.party_id())
    .data(renox::serde_json::json!({ "number": "INV-0001" }));
    let state = app.state();
    let ids = notify::notifier(state)
        .notify(state, request)
        .await
        .unwrap();
    assert_eq!(ids.len(), 1);
    app.run_all_jobs().await;

    let message = wl_notify::Message::find(app.db(), ids[0].0)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(message.status, MessageStatus::Abandoned);
    assert_eq!(message.error_kind.as_deref(), Some("hard_bounce"));

    let marked = ContactPoint::find(app.db(), email.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(marked.status, ContactStatus::Invalid);
    assert_eq!(marked.invalid_reason.as_deref(), Some("hard_bounce"));
    assert!(marked.is_primary);

    let mut admin = User::register(app.db(), "Rana", "notify.admin@test.example", "secret123")
        .await
        .unwrap();
    admin.email_verified_at = Some(renox::db::now());
    admin.save(app.db()).await.unwrap();
    roles::define(app.db()).await.unwrap();
    admin.assign_role(app.db(), ADMIN).await.unwrap();
    app.acting_as(&admin);
    app.get(&format!("/parties/{}", al_noor.id))
        .await
        .assert_ok()
        .assert_see("Contact needs updating")
        .assert_see(&email.value);
}
