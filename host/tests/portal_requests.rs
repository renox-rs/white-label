//! BL-TKT-14 (#474) in the real Property host: a tenant raises a maintenance
//! request from the portal with photos and follows it under *Requests*;
//! she never sees another tenant's ticket or an internal comment; a blocked
//! party still raises requests; a `read_only` grant keeps the list but not
//! *New request*; a resolved chargeable job shows its amount with Confirm
//! and Reopen.

mod support;

use host::roles::MEMBER;
use renox::prelude::*;
use renox::testing::TestApp;
use support::{active_lease, block, company, draft_lease, person, unit_of};
use wl_kernel::invoicing::VatCategory;
use wl_kernel::tickets::{OpenTicket, TaskComment, TicketChannel};
use wl_kernel::{PartyId, Subject};
use wl_portal::{AccessGrant, NewGrant, PortalAccess};
use wl_ticketing::{Category, ChargeLine, Priority, SlaPolicy, Ticket, TicketStatus, Tickets};

/// The category tenants pick for a broken AC.
const AC: &str = "ac_not_cooling";

struct Scene {
    app: TestApp,
    party: PartyId,
    unit: Subject,
    other_unit: Subject,
    sara: User,
    grant: AccessGrant,
}

/// A tiny PNG (the signature is enough for the type check).
fn png() -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(&[0_u8; 64]);
    bytes
}

/// Whether `response` redirects to a request's page.
fn redirects_to_a_request(response: &renox::testing::TestResponse) -> bool {
    response.status.is_redirection()
        && response
            .header("location")
            .is_some_and(|to| to.contains("/requests/"))
}

/// Al Noor leases unit 1204 and Sara is its primary contact; Gulf Pearl
/// leases 1205. The category "AC not cooling" is open to the portal.
async fn scene() -> Scene {
    let app = support::booted().await;
    let party = company(&app, "Al Noor Trading LLC").await;
    let unit = unit_of(&app, "TNB", "1204", property::UnitUse::Residential).await;
    let _lease = active_lease(&app, &unit, party, true).await;
    let other_party = company(&app, "Gulf Pearl Trading").await;
    let other = unit_of(&app, "TNC", "1205", property::UnitUse::Residential).await;
    let _other_lease = draft_lease(&app, &other, other_party, -30).await;

    let policy = SlaPolicy::by_key(app.db(), wl_ticketing::seed::STANDARD_POLICY)
        .await
        .unwrap();
    Category::create(
        app.db(),
        Category {
            key: AC.into(),
            name_en: "AC not cooling".into(),
            default_priority: Priority::Normal,
            sla_policy_id: policy.map(|p| p.id),
            active: true,
            portal: true,
            chargeable: true,
            ..Category::default()
        },
    )
    .await
    .unwrap();

    let sara = person(&app, "Sara", MEMBER).await;
    let grant = PortalAccess::register(
        app.state(),
        sara.id,
        NewGrant::new(party, party, "tenant", &["primary_contact"]),
    )
    .await
    .unwrap();
    Scene {
        app,
        party,
        unit: unit.subject(),
        other_unit: other.subject(),
        sara,
        grant,
    }
}

impl Scene {
    /// Sara raises "AC not cooling" on unit 1204 with two photos; the
    /// response.
    async fn raise(&self) -> renox::testing::TestResponse {
        self.app.acting_as(&self.sara);
        let unit = self.unit.subject_id.to_string();
        let png = png();
        self.app
            .post_multipart(
                &format!("/portal/{}/requests", self.party.0),
                &[
                    ("unit", unit.as_str()),
                    ("category", AC),
                    ("description", "The AC in the bedroom blows warm air."),
                ],
                &[
                    ("photos", "ac-1.png", png.as_slice()),
                    ("photos", "ac-2.png", png.as_slice()),
                ],
            )
            .await
    }

    async fn last_ticket(&self) -> Ticket {
        Ticket::query()
            .order_by_desc(Ticket::ID)
            .first(self.app.db())
            .await
            .unwrap()
            .expect("a ticket")
    }

    fn requests(&self) -> String {
        format!("/portal/{}/requests", self.party.0)
    }
}

/// Done when 1: the ticket has channel *portal*, requester Sara and the
/// category's SLA, carries the two photos and shows *Open* in her Requests.
#[renox::test]
async fn sara_raises_a_request_with_two_photos_and_sees_it_open() {
    let s = scene().await;
    let response = s.raise().await;
    assert!(redirects_to_a_request(&response), "{}", response.text());
    let ticket = s.last_ticket().await;
    assert_eq!(ticket.channel, TicketChannel::Portal);
    assert_eq!(ticket.requester_user_id, Some(s.sara.id));
    assert_eq!(ticket.subject(), s.unit);
    let category = Category::by_key(s.app.db(), AC).await.unwrap().unwrap();
    assert_eq!(ticket.sla_policy_id, category.sla_policy_id);
    let photos = wl_documents::Documents::of_subject(
        s.app.db(),
        &Subject::new("ticketing.ticket", ticket.id),
    )
    .await
    .unwrap();
    assert_eq!(photos.len(), 2, "both photos are stored on the ticket");

    let list = s.app.get(&s.requests()).await;
    list.assert_ok().assert_see(&ticket.number).assert_see("Open");
}

/// Done when 2: she does not see the ticket of unit 1205, nor an internal
/// comment on her own ticket.
#[renox::test]
async fn sara_sees_neither_another_units_ticket_nor_an_internal_comment() {
    let s = scene().await;
    s.raise().await;
    let mine = s.last_ticket().await;
    let theirs = Tickets::open(
        s.app.state(),
        OpenTicket::new(s.other_unit.clone(), AC, "Leak in 1205"),
    )
    .await
    .unwrap();
    Tickets::comment(
        s.app.state(),
        mine.ticket_id(),
        TaskComment::internal("Tenant is difficult, call before 9"),
    )
    .await
    .unwrap();
    Tickets::comment(
        s.app.state(),
        mine.ticket_id(),
        TaskComment::visible("A technician comes tomorrow at 10:00."),
    )
    .await
    .unwrap();

    s.app.acting_as(&s.sara);
    s.app
        .get(&s.requests())
        .await
        .assert_ok()
        .assert_see(&mine.number)
        .assert_dont_see(&theirs.number);
    s.app
        .get(&format!("{}/{}", s.requests(), theirs.id.0))
        .await
        .assert_not_found();
    s.app
        .get(&format!("{}/{}", s.requests(), mine.id))
        .await
        .assert_ok()
        .assert_see("A technician comes tomorrow at 10:00.")
        .assert_dont_see("Tenant is difficult");
}

/// Done when 3: with her party blocked (`non_payment`) she can still raise
/// a request.
#[renox::test]
async fn a_blocked_party_still_raises_requests() {
    let s = scene().await;
    block(&s.app, s.party).await;
    let response = s.raise().await;
    assert!(redirects_to_a_request(&response), "{}", response.text());
    let ticket = s.last_ticket().await;
    assert_eq!(ticket.requester_user_id, Some(s.sara.id));
}

/// Done when 4: with her grant `read_only` the list shows and *New request*
/// is refused.
#[renox::test]
async fn a_read_only_grant_keeps_the_list_and_refuses_new_requests() {
    let s = scene().await;
    s.raise().await;
    let ticket = s.last_ticket().await;
    PortalAccess::end(s.app.state(), None, s.grant.id, Some("lease ended"))
        .await
        .unwrap();
    s.app.acting_as(&s.sara);
    s.app
        .get(&s.requests())
        .await
        .assert_ok()
        .assert_see(&ticket.number);
    assert_eq!(
        s.app
            .get(&format!("{}/new", s.requests()))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    let before = Ticket::query().count(s.app.db()).await.unwrap();
    assert_eq!(s.raise().await.status, StatusCode::FORBIDDEN);
    assert_eq!(Ticket::query().count(s.app.db()).await.unwrap(), before);
}

/// Done when 5: a resolved chargeable ticket shows AED 262.50 (incl. VAT)
/// with Confirm and Reopen.
#[renox::test]
async fn a_resolved_chargeable_ticket_shows_its_amount_with_confirm_and_reopen() {
    let s = scene().await;
    s.raise().await;
    let ticket = s.last_ticket().await;
    ChargeLine::create(
        s.app.db(),
        ChargeLine {
            ticket_id: ticket.id,
            position: 1,
            description: "Gas refill".into(),
            quantity_milli: 1000,
            unit_price: 25_000,
            vat_category: VatCategory::Standard,
            ..ChargeLine::default()
        },
    )
    .await
    .unwrap();
    // Resolved by the technician (the work flow has its own tests).
    renox::db::sql("UPDATE wl_ticketing_tickets SET status = ?, resolved_at = ? WHERE id = ?")
        .bind(TicketStatus::Resolved.as_str())
        .bind(renox::db::now())
        .bind(ticket.id)
        .execute(s.app.db())
        .await
        .unwrap();

    s.app.acting_as(&s.sara);
    let page = s.app.get(&format!("{}/{}", s.requests(), ticket.id)).await;
    page.assert_ok()
        .assert_see("AED 262.50")
        .assert_see("incl. VAT")
        .assert_see("Confirm")
        .assert_see("Reopen");

    s.app
        .htmx()
        .post(&format!("{}/{}/confirm", s.requests(), ticket.id), &[])
        .await
        .assert_ok();
    let closed = Ticket::find_or_404(s.app.db(), ticket.id).await.unwrap();
    assert_eq!(closed.status, TicketStatus::Closed);
}
