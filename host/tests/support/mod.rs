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
use wl_parties::{CompanyDetails, ContactPoint, ContactType, NewParty, PersonDetails, create};

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

/// The primary e-mail of the party `name`.
fn email_of(name: &str) -> ContactPoint {
    ContactPoint {
        contact_type: ContactType::Email,
        value: format!("{}@parties.example", name.to_lowercase().replace(' ', ".")),
        is_primary: true,
        ..Default::default()
    }
}

/// A person party (a tenant) with a billing e-mail.
pub async fn tenant(app: &TestApp, name: &str) -> PartyId {
    create(
        app.state(),
        NewParty {
            kind: PartyKind::Person,
            legal_name_en: name.into(),
            person: Some(PersonDetails::default()),
            contacts: vec![email_of(name)],
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
            contacts: vec![email_of(name)],
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

/// A building `code` with a unit `number` of `usage`.
pub async fn unit_of(
    app: &TestApp,
    code: &str,
    number: &str,
    usage: property::UnitUse,
) -> property::Unit {
    let building = property::create_building(
        app.db(),
        property::NewBuilding {
            code: code.to_owned(),
            name_en: format!("Tower {code}"),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    property::create_unit(
        app.db(),
        property::NewUnit {
            building_id: building.id,
            number: number.to_owned(),
            usage,
            asking_rent: Some(10_000_000),
            ..Default::default()
        },
    )
    .await
    .unwrap()
}

/// A draft lease of `unit` for `tenant`, one year from `start_in` days.
pub async fn draft_lease(
    app: &TestApp,
    unit: &property::Unit,
    tenant: PartyId,
    start_in: i64,
) -> property::Lease {
    let new = property::NewLease::new(
        unit.id,
        tenant,
        day(start_in),
        day(start_in + 364),
        12_000_000,
        4,
    );
    property::leases::create_draft(app.state(), None, new)
        .await
        .unwrap()
}

/// An issued invoice for `lease`, due `due`, charged as the rent instalment
/// `position` of the lease (what `PRP.RUL.10` reads).
pub async fn invoiced_charge(
    app: &TestApp,
    by: &User,
    lease: &property::Lease,
    fils: i64,
    category: VatCategory,
    due: NaiveDate,
    position: i32,
) -> Invoice {
    let draft = draft_invoice(
        app,
        PartyId(lease.tenant_party_id),
        lease.subject(),
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
    property::LeaseCharge::create(
        app.db(),
        property::LeaseCharge {
            lease_id: lease.id,
            position,
            charge_type: "rent".to_owned(),
            description: format!("Rent {position}"),
            due_date: due,
            amount: fils,
            vat_category: category,
            status: property::ChargeStatus::Invoiced,
            invoice_id: Some(draft.id),
            ..Default::default()
        },
    )
    .await
    .unwrap();
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
