//! Shared by the host tests that run the Property client's behaviour with the
//! real modules (BL-PRP-26, BL-PRP-27): the booted app, the people, the
//! parties and the invoices.

#![allow(dead_code)]

use renox::chrono::{Days, NaiveDate, NaiveDateTime};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_invoicing::Invoice;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, VatCategory, invoicer};
use wl_kernel::{BusinessCalendar, LegalEntity, PartyId, PartyKind, Subject};
use wl_notify::{Clock, Message};
use wl_parties::{CompanyDetails, NewParty, PersonDetails, create};

/// The notify module's clock fixed at noon in Dubai: inside every send window
/// and outside the quiet hours (21:00-08:00, `NTF.RUL.02`), so a send does not
/// depend on the hour the test runs at.
pub fn noon_clock() -> Clock {
    let local = NaiveDateTime::parse_from_str("2026-10-19 12:00", "%Y-%m-%d %H:%M")
        .expect("a date and time");
    Clock::fixed(BusinessCalendar::default().from_local(local))
}

/// The Property host, with `wl:install` and the seeds run as the demo does.
pub async fn booted() -> TestApp {
    let app = TestApp::new(host::app().provide(noon_clock())).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

/// A verified user `name` holding `role`.
pub async fn person(app: &TestApp, name: &str, role: &str) -> User {
    let mut user = User::register(
        app.db(),
        name,
        &format!("{}@example.com", name.to_lowercase()),
        "secret123",
    )
    .await
    .unwrap();
    user.email_verified_at = Some(renox::db::now());
    user.save(app.db()).await.unwrap();
    host::roles::define(app.db()).await.unwrap();
    user.assign_role(app.db(), role).await.unwrap();
    user
}

/// A person party (a tenant) with a billing e-mail.
pub async fn tenant(app: &TestApp, name: &str) -> PartyId {
    create(
        app.state(),
        NewParty {
            kind: PartyKind::Person,
            legal_name_en: name.into(),
            person: Some(PersonDetails::default()),
            source: "back_office".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .party_id()
}

/// A company party (a vendor, a company tenant or an owner).
pub async fn company(app: &TestApp, name: &str) -> PartyId {
    create(
        app.state(),
        NewParty {
            kind: PartyKind::Company,
            legal_name_en: name.into(),
            company: Some(CompanyDetails::default()),
            source: "back_office".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .party_id()
}

/// The legal entity with `code`.
pub async fn entity(app: &TestApp, code: &str) -> LegalEntity {
    LegalEntity::query()
        .where_(LegalEntity::CODE.eq(code))
        .first(app.db())
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("the entity {code}"))
}

/// `days` from today (negative: before).
pub fn day(days: i64) -> NaiveDate {
    if days >= 0 {
        today() + Days::new(days as u64)
    } else {
        today() - Days::new(days.unsigned_abs())
    }
}

/// A draft invoice to `party` for the lease subject `lease`, one line of
/// `fils`, due `due`; the invoice, not yet issued.
pub async fn draft_invoice(
    app: &TestApp,
    party: PartyId,
    subject: Subject,
    fils: i64,
    category: VatCategory,
    due: NaiveDate,
) -> Invoice {
    let draft = InvoiceDraft::new(party, subject)
        .line(DraftLine::new("Rent", fils, category))
        .due_date(due);
    let id = invoicer(app.state())
        .draft(app.db(), draft)
        .await
        .unwrap()
        .expect("invoicing is on");
    Invoice::find_or_404(app.db(), id.0).await.unwrap()
}

/// An issued invoice (issued by `by` through the page) of `fils` to `party`
/// for the lease subject, due `due`.
pub async fn issued_invoice(
    app: &TestApp,
    by: &User,
    party: PartyId,
    fils: i64,
    category: VatCategory,
    due: NaiveDate,
) -> Invoice {
    let draft = draft_invoice(
        app,
        party,
        Subject::new("property.lease", 9),
        fils,
        category,
        due,
    )
    .await;
    app.acting_as(by);
    app.htmx()
        .post(&format!("/invoices/{}/issue", draft.id), &[])
        .await
        .assert_redirect(&format!("/invoices/{}", draft.id));
    Invoice::find_or_404(app.db(), draft.id).await.unwrap()
}

/// The invoice, as stored now.
pub async fn reload(app: &TestApp, invoice: &Invoice) -> Invoice {
    Invoice::find_or_404(app.db(), invoice.id).await.unwrap()
}

/// The messages of type `type_key` that went to `user`, oldest first.
pub async fn messages_to(app: &TestApp, type_key: &str, user: &User) -> Vec<Message> {
    Message::query()
        .where_(Message::TYPE_KEY.eq(type_key))
        .where_(Message::USER_ID.eq(user.id))
        .order_by(Message::ID)
        .get(app.db())
        .await
        .unwrap()
}

/// How many messages of type `type_key` exist at all.
pub async fn count_messages(app: &TestApp, type_key: &str) -> usize {
    Message::query()
        .where_(Message::TYPE_KEY.eq(type_key))
        .count(app.db())
        .await
        .unwrap() as usize
}
