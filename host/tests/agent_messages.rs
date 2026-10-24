//! A message from a template through an assistant (BL-AGT-10, #550): the
//! preview shows the message rendered in the party's language, a confirm
//! queues it once and it is sent (Renox's test mailer stands in for the
//! provider), it is kept on the record as "sent by Budi via Claude", and a
//! party who opted out of WhatsApp is refused with the reason.

mod support;

use renox::prelude::*;
use renox::serde_json::Value;
use renox::testing::{TestApp, TestResponse};
use wl_invoicing::Invoice;
use wl_kernel::agent::AgentsOn;
use wl_kernel::invoicing::VatCategory;
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel};
use wl_kernel::{PartyId, PartyKind, Subject};
use wl_notify::{Message, MessageStatus, Suppression, SuppressionReason};
use wl_parties::party::Language;
use wl_parties::{ContactPoint, ContactType, NewParty, PersonDetails, create};

use support::{day, draft_invoice, noon_clock, person, tenant_with_mobile};

/// The reminder template the tests send: e-mail in English and Arabic,
/// WhatsApp in English.
const REMINDER: &str = "invoice.agent_reminder";

fn register_reminder() {
    notify::register_type(
        NotificationTypeDef::new(REMINDER, "invoicing")
            .category(Category::Reminder)
            .channels(&[NotifyChannel::Email, NotifyChannel::Whatsapp])
            .template(
                NotifyChannel::Email,
                "en",
                Some("Invoice {{ number }} is due"),
                "Dear {{ party }}, invoice {{ number }} is due.",
            )
            .template(
                NotifyChannel::Email,
                "ar",
                Some("الفاتورة {{ number }} مستحقة"),
                "عزيزي {{ party }}، الفاتورة {{ number }} مستحقة.",
            )
            .template(
                NotifyChannel::Whatsapp,
                "en",
                None,
                "Invoice {{ number }} is due.",
            ),
    );
}

async fn booted_with_agents() -> TestApp {
    register_reminder();
    let policy = wl_kernel::agent::access::policy(AgentsOn::all());
    let app = TestApp::new(host::app().provide(noon_clock()).provide(policy)).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

async fn tool(app: &TestApp, name: &str, args: Value) -> TestResponse {
    app.request()
        .header("X-Agent-Client", "Claude")
        .json()
        .post_json(&format!("/agent/tools/{name}"), &args)
        .await
}

/// A tenant who reads Arabic, with an e-mail.
async fn arabic_tenant(app: &TestApp) -> PartyId {
    create(
        app.state(),
        NewParty {
            kind: PartyKind::Person,
            legal_name_en: "Al Ali".into(),
            default_language: Language::Ar,
            person: Some(PersonDetails::default()),
            contacts: vec![ContactPoint {
                contact_type: ContactType::Email,
                value: "alali@parties.example".into(),
                is_primary: true,
                verified_at: Some(renox::db::now()),
                ..Default::default()
            }],
            source: "back_office".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .party_id()
}

async fn invoice_to(app: &TestApp, party: PartyId) -> Invoice {
    draft_invoice(
        app,
        party,
        Subject::new("property.lease", 9),
        100_000,
        VatCategory::Standard,
        day(30),
    )
    .await
}

fn line<'a>(preview: &'a Value, label: &str) -> &'a str {
    preview["change"]["lines"]
        .as_array()
        .expect("lines")
        .iter()
        .find(|l| l["label"] == label)
        .unwrap_or_else(|| panic!("no line {label}"))["value"]
        .as_str()
        .expect("text")
}

async fn messages(app: &TestApp, party: PartyId) -> Vec<Message> {
    Message::query()
        .where_(Message::TYPE_KEY.eq(REMINDER))
        .where_(Message::PARTY_ID.eq(party.0))
        .get(app.db())
        .await
        .unwrap()
}

#[renox::test]
async fn the_preview_shows_the_message_in_the_partys_language_and_confirm_sends_it_once() {
    let app = booted_with_agents().await;
    let budi = person(&app, "Budi", host::roles::RECEIVABLES).await;
    let party = arabic_tenant(&app).await;
    let invoice = invoice_to(&app, party).await;
    let record = format!("invoicing.invoice:{}", invoice.id);
    app.acting_as(&budi);

    let preview = tool(
        &app,
        "preview_action",
        json!({
            "action": "notify.message.send_message",
            "arguments": { "record": record, "template": REMINDER, "channel": "email" }
        }),
    )
    .await;
    preview.assert_ok();
    let preview: Value = preview.json();
    assert_eq!(line(&preview, "Language"), "ar");
    let body = line(&preview, "Message");
    assert!(body.starts_with("عزيزي Al Ali"), "{body}");
    assert!(body.contains("مستحقة"));
    // Nothing is queued by a preview.
    assert!(messages(&app, party).await.is_empty());

    let token = preview["token"].as_str().unwrap().to_owned();
    let sent = tool(&app, "confirm_action", json!({ "token": token })).await;
    sent.assert_ok();
    let sent: Value = sent.json();
    assert_eq!(sent["outcome"], "sent");
    let again: Value = tool(&app, "confirm_action", json!({ "token": token }))
        .await
        .json();
    assert_eq!(again, sent);
    app.run_jobs().await;

    let queued = messages(&app, party).await;
    assert_eq!(queued.len(), 1);
    let message = &queued[0];
    assert_eq!(message.status, MessageStatus::Sent);
    assert_eq!(message.channel, NotifyChannel::Email);
    assert_eq!(message.address.as_deref(), Some("alali@parties.example"));
    assert_eq!(message.rendered_body, body);
    assert_eq!(message.subject_type, "invoicing.invoice");
    assert_eq!(message.subject_id, invoice.id);
    assert_eq!(
        wl_notify::agent_message::sent_by(message).as_deref(),
        Some("sent by Budi via Claude")
    );
}

#[renox::test]
async fn a_party_who_opted_out_of_whatsapp_is_refused_with_the_reason() {
    let app = booted_with_agents().await;
    let budi = person(&app, "Budi", host::roles::RECEIVABLES).await;
    let party = tenant_with_mobile(&app, "Rashid", "+971501112233").await;
    Suppression::create(
        app.db(),
        Suppression {
            address: "+971501112233".into(),
            channel: NotifyChannel::Whatsapp,
            reason: SuppressionReason::OptOut,
            source: Some("whatsapp_stop".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let invoice = invoice_to(&app, party).await;
    app.acting_as(&budi);

    let refused = tool(
        &app,
        "preview_action",
        json!({
            "action": "notify.message.send_message",
            "arguments": {
                "record": format!("invoicing.invoice:{}", invoice.id),
                "template": REMINDER,
                "channel": "whatsapp"
            }
        }),
    )
    .await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("opted out of WhatsApp"),
        "{}",
        refused.text()
    );
    assert!(messages(&app, party).await.is_empty());
}

#[renox::test]
async fn free_text_and_parties_not_on_the_record_are_refused() {
    let app = booted_with_agents().await;
    let budi = person(&app, "Budi", host::roles::RECEIVABLES).await;
    let party = arabic_tenant(&app).await;
    let stranger = tenant_with_mobile(&app, "Stranger", "+971509998877").await;
    let invoice = invoice_to(&app, party).await;
    let record = format!("invoicing.invoice:{}", invoice.id);
    app.acting_as(&budi);

    tool(
        &app,
        "preview_action",
        json!({
            "action": "notify.message.send_message",
            "arguments": { "record": record, "template": REMINDER, "channel": "email", "body": "Pay now" }
        }),
    )
    .await
    .assert_status(422);
    tool(
        &app,
        "preview_action",
        json!({
            "action": "notify.message.send_message",
            "arguments": { "record": record, "template": REMINDER, "channel": "email", "party": stranger.0 }
        }),
    )
    .await
    .assert_status(422);
}
