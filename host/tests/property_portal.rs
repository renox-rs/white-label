//! BL-PRP-30 (#475) in the real Property host: the tenant portal shows the
//! party's lease, its cheques and its documents (`POR.SLF.01`, `PRP.ACL.01`),
//! and nothing of another tenant's, even by URL; the finance contact has no
//! *Lease* or *Cheques* entry; a blocked party sees the same pages with no
//! block text; after the lease is terminated the read-only grant still shows
//! and downloads them.

mod support;

use host::roles::MEMBER;
use renox::prelude::*;
use renox::testing::TestApp;
use support::{active_lease, block, company, day, draft_lease, person, unit_of};
use wl_documents::{Document, DocumentKind, DocumentStatus, DocumentVersion};
use wl_kernel::PartyId;
use wl_portal::{AccessGrant, GrantStatus, NewGrant, PortalAccess};

/// A portal user `name` acting for `party` with the tenant `role`.
async fn portal_user(app: &TestApp, name: &str, party: PartyId, role: &str) -> (User, AccessGrant) {
    let user = person(app, name, MEMBER).await;
    let grant = PortalAccess::register(
        app.state(),
        user.id,
        NewGrant::new(party, party, "tenant", &[role]),
    )
    .await
    .unwrap();
    (user, grant)
}

/// A document of `kind` on the lease version `lease`, accepted, with a stored
/// file holding `content`.
async fn lease_document(
    app: &TestApp,
    lease: &property::Lease,
    kind: &str,
    content: &'static [u8],
) -> Document {
    let kind = DocumentKind::by_key(app.db(), kind)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("the kind {kind} is seeded"));
    let document = Document::create(
        app.db(),
        Document {
            subject_type: "property.lease".into(),
            subject_id: lease.id,
            kind_id: kind.id,
            title: kind.name_en.clone(),
            current_version: 1,
            status: DocumentStatus::Accepted,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let key = format!("tests/lease-{}-{}.pdf", lease.id, document.id);
    app.state()
        .storage
        .put(&key, renox::axum::body::Bytes::from_static(content))
        .await
        .unwrap();
    DocumentVersion::create(
        app.db(),
        DocumentVersion {
            document_id: document.id,
            version_no: 1,
            storage_key: key,
            file_name: format!("{}.pdf", kind.key),
            mime_type: "application/pdf".into(),
            size_bytes: i64::try_from(content.len()).unwrap(),
            checksum: wl_documents::documents::sha256_hex(content),
            uploaded_at: renox::db::now(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    document
}

struct Scene {
    app: TestApp,
    party: PartyId,
    lease: property::Lease,
    other_party: PartyId,
    other_lease: property::Lease,
    sara: User,
    sara_grant: AccessGrant,
    contract: Document,
    ejari: Document,
    other_document: Document,
}

/// Al Noor Trading LLC leases unit 1204 for AED 120,000 in 4 cheques; another
/// tenant leases 1205 on the same floor. Sara is Al Noor's primary contact.
async fn scene() -> Scene {
    let app = support::booted().await;
    let party = company(&app, "Al Noor Trading LLC").await;
    let unit = unit_of(&app, "TNB", "1204", property::UnitUse::Residential).await;
    let lease = active_lease(&app, &unit, party, true).await;
    let charges: Vec<property::LeaseCharge> = lease
        .charges(app.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|c| c.charge_type == "rent")
        .collect();
    assert_eq!(charges.len(), 4);
    let entries = charges
        .iter()
        .enumerate()
        .map(|(i, c)| {
            property::cheques::ChequeEntry::new(
                &format!("40000{}", i + 1),
                c.due_date,
                c.amount,
                Some(c.id),
            )
        })
        .collect();
    property::cheques::register_cheques(app.state(), None, &lease, "Emirates NBD", entries)
        .await
        .unwrap();
    let ejari = lease_document(&app, &lease, "ejari_certificate", b"%PDF-ejari").await;
    let contract = Document::query()
        .where_(Document::SUBJECT_TYPE.eq("property.lease"))
        .where_(Document::SUBJECT_ID.eq(lease.id))
        .order_by(Document::ID)
        .first(app.db())
        .await
        .unwrap()
        .expect("the signed contract of the activation");

    let other_party = company(&app, "Gulf Pearl Trading").await;
    let other_unit = unit_of(&app, "TNC", "1205", property::UnitUse::Residential).await;
    let other_lease = draft_lease(&app, &other_unit, other_party, -30).await;
    renox::db::sql("UPDATE property_leases SET status = 'active' WHERE id = ?")
        .bind(other_lease.id)
        .execute(app.db())
        .await
        .unwrap();
    let other_document =
        lease_document(&app, &other_lease, "ejari_certificate", b"%PDF-other").await;
    let (sara, sara_grant) = portal_user(&app, "Sara", party, "primary_contact").await;
    Scene {
        app,
        party,
        lease,
        other_party,
        other_lease,
        sara,
        sara_grant,
        contract,
        ejari,
        other_document,
    }
}

impl Scene {
    fn section(&self, key: &str) -> String {
        format!("/portal/{}/{key}", self.party.0)
    }

    /// Sara sees L-0042 with its term and rent, the 4 cheques and the
    /// documents, and nothing of the other tenant.
    async fn sara_sees_her_lease(&self) {
        self.app.acting_as(&self.sara);
        let lease = self.app.get(&self.section("lease")).await;
        lease.assert_ok();
        let text = lease.text();
        assert!(text.contains(&self.lease.number_with_version()), "{text}");
        assert!(text.contains("AED 120,000.00"), "{text}");
        assert!(text.contains(&self.lease.start_date.to_string()));
        assert!(text.contains(&self.lease.end_date.to_string()));
        assert!(text.contains("4 cheques"));
        assert!(text.contains("Active"));
        assert!(!text.contains(&self.other_lease.number()), "{text}");

        let cheques = self.app.get(&self.section("cheques")).await;
        cheques.assert_ok();
        let text = cheques.text();
        for n in 1..=4 {
            assert!(text.contains(&format!("40000{n}")), "{text}");
        }
        assert!(text.contains("Received"));
        assert!(text.contains("Emirates NBD"));

        let documents = self.app.get(&self.section("documents")).await;
        documents.assert_ok();
        let text = documents.text();
        assert!(text.contains("Signed tenancy contract"), "{text}");
        assert!(text.contains("Ejari"), "{text}");
        let file = self
            .app
            .get(&format!(
                "/portal/{}/lease-documents/{}",
                self.party.0, self.ejari.id
            ))
            .await;
        file.assert_ok();
        assert_eq!(file.text(), "%PDF-ejari");
    }
}

/// Done when 1: Sara (primary contact) sees L-0042 with its term and rent,
/// the 4 cheques with their statuses and the signed contract and Ejari
/// certificate.
#[renox::test]
async fn prp30_the_primary_contact_sees_the_lease_its_cheques_and_documents() {
    let s = scene().await;
    s.app.acting_as(&s.sara);
    let home = s.app.get(&format!("/portal/{}", s.party.0)).await;
    home.assert_ok();
    for key in ["lease", "cheques", "documents"] {
        assert!(
            home.text().contains(&s.section(key)),
            "{key}: {}",
            home.text()
        );
    }
    s.sara_sees_her_lease().await;
    assert!(s.contract.id > 0);
}

/// Done when 2: she does not see lease L-0043 of another tenant on the same
/// floor, even by URL.
#[renox::test]
async fn prp30_another_tenants_lease_is_not_shown_even_by_url() {
    let s = scene().await;
    s.app.acting_as(&s.sara);
    let other = s.other_party.0;
    for key in ["lease", "cheques", "documents"] {
        let response = s.app.get(&format!("/portal/{other}/{key}")).await;
        assert_eq!(response.status, StatusCode::FORBIDDEN, "{key}");
    }
    let foreign = s
        .app
        .get(&format!(
            "/portal/{}/lease-documents/{}",
            s.party.0, s.other_document.id
        ))
        .await;
    assert_eq!(foreign.status, StatusCode::NOT_FOUND);
    let foreign_party = s
        .app
        .get(&format!(
            "/portal/{other}/lease-documents/{}",
            s.other_document.id
        ))
        .await;
    assert_eq!(foreign_party.status, StatusCode::FORBIDDEN);
    let documents = s.app.get(&s.section("documents")).await;
    assert!(!documents.text().contains(&s.other_lease.number()));
}

/// Done when 3: Omar (finance contact) has no Lease or Cheques entry, and the
/// pages answer 403.
#[renox::test]
async fn prp30_the_finance_contact_has_no_lease_or_cheques() {
    let s = scene().await;
    let (omar, _) = portal_user(&s.app, "Omar", s.party, "finance_contact").await;
    s.app.acting_as(&omar);
    let home = s.app.get(&format!("/portal/{}", s.party.0)).await;
    home.assert_ok();
    for key in ["lease", "cheques", "documents"] {
        assert!(!home.text().contains(&s.section(key)), "{key}");
        let page = s.app.get(&s.section(key)).await;
        assert_eq!(page.status, StatusCode::FORBIDDEN, "{key}");
    }
}

/// Done when 4: with the party blocked (`non_payment`), the same pages show
/// with no block text.
#[renox::test]
async fn prp30_a_blocked_tenant_sees_the_same_pages_without_block_text() {
    let s = scene().await;
    block(&s.app, s.party).await;
    s.sara_sees_her_lease().await;
    for key in ["lease", "cheques", "documents"] {
        let text = s.app.get(&s.section(key)).await.text().to_lowercase();
        assert!(!text.contains("blocked"), "{key}");
        assert!(!text.contains("non_payment"), "{key}");
        assert!(!text.contains("non-payment"), "{key}");
    }
}

/// Done when 5: after the lease is terminated her grant is read-only and she
/// still sees and downloads them.
#[renox::test]
async fn prp30_after_termination_the_read_only_grant_still_shows_and_downloads() {
    let s = scene().await;
    renox::db::sql("UPDATE property_leases SET status = 'terminated' WHERE id = ?")
        .bind(s.lease.id)
        .execute(s.app.db())
        .await
        .unwrap();
    s.app
        .state()
        .emit(property::LeaseTerminated {
            lease: s.lease.id,
            root: s.lease.root_id,
            unit: s.lease.unit_id,
            tenant: s.party,
            move_out: day(0),
            rent_due_until: None,
            credited: 0,
        })
        .await
        .unwrap();
    let grant = AccessGrant::find_or_404(s.app.db(), s.sara_grant.id)
        .await
        .unwrap();
    assert_eq!(grant.status, GrantStatus::ReadOnly);

    s.app.acting_as(&s.sara);
    let lease = s.app.get(&s.section("lease")).await;
    lease.assert_ok();
    assert!(lease.text().contains("Terminated"));
    assert!(lease.text().contains(&s.lease.number_with_version()));
    let file = s
        .app
        .get(&format!(
            "/portal/{}/lease-documents/{}",
            s.party.0, s.ejari.id
        ))
        .await;
    file.assert_ok();
    assert_eq!(file.text(), "%PDF-ejari");
}
