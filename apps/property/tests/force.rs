//! BL-PRP-12 (#262): `force_terminate` for an abandoned unit or an executed
//! eviction. The property manager requests it with the evidence; another user
//! with `leases.force_terminate` approves; the lease ends as `terminated`, the
//! unit is vacated, the deposit goes to settlement, the later cheques are kept
//! as evidence or returned, an eviction flag is closed with its outcome and
//! `LeaseForceTerminated` is emitted.

mod support;

use property::force::{
    EXECUTION_RECORD_KIND, INSPECTION_KIND, JUDGMENT_KIND, REPOSSESSION_BASIS_KIND,
};
use property::legal_case::{CaseOutcome, CaseStatus, CaseType, LegalCase};
use property::{ChargeStatus, Lease, LeaseCharge, LeaseStatus, NewLease, UnitStatus};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::approvals::{ApprovalRequest, approve};
use wl_kernel::cheques::{ChequeId, ChequeStatus};
use wl_kernel::invoicing::InvoiceStatus;

use support::{AGENT, Booted, MANAGER, TENANT, aed, boot, d};

struct Scene {
    t: Booted,
    unit: property::Unit,
    agent: User,
    manager: User,
    admin: User,
}

async fn scene() -> Scene {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    let admin = t.user_with("Bader", wl_kernel::COMPANY_ADMIN_ROLE).await;
    Scene {
        t,
        unit,
        agent,
        manager,
        admin,
    }
}

impl Scene {
    /// An `active` lease 2027-01-01 to 2027-12-31 at AED 9,000 a month, a
    /// deposit of AED 8,000 held, and the invoices of January to March issued
    /// and unpaid.
    async fn leased(&self) -> Lease {
        let new = NewLease::new(self.unit.id, TENANT, d(1, 1), d(12, 31), aed(108_000), 12)
            .deposit(aed(8_000));
        let lease = property::leases::create_draft(self.t.state(), None, new)
            .await
            .unwrap();
        self.t.deposits.hold(&lease.agreement(), TENANT, aed(8_000));
        self.t.submit(&self.agent, &lease).await.assert_ok();
        self.t.activate(&self.manager, &lease).await.assert_ok();
        property::invoicing::run(self.t.state(), d(3, 31))
            .await
            .unwrap();
        for invoice in self.t.invoices.all() {
            self.t.invoices.issue(invoice.summary.id.0);
        }
        self.t.reload(&lease).await
    }

    /// The evidence of an abandoned unit is attached to the lease.
    fn abandonment_evidence(&self, lease: &Lease) {
        self.t.docs.accept(&lease.subject(), INSPECTION_KIND);
        self.t
            .docs
            .accept(&lease.subject(), REPOSSESSION_BASIS_KIND);
    }

    fn path(&self, lease: &Lease) -> String {
        format!("/property/leases/{}/force-terminate", lease.id)
    }

    /// The manager requests the forced termination for `reason`.
    async fn ask(
        &self,
        lease: &Lease,
        reason: &str,
        extra: &[(&str, &str)],
    ) -> renox::testing::TestResponse {
        let mut form = vec![("reason", reason), ("move_out", "2027-03-31")];
        if reason == "abandoned" {
            form.push(("basis", "Municipality repossession order 41/2027"));
        }
        form.extend_from_slice(extra);
        self.t.post(&self.manager, &self.path(lease), &form).await
    }

    async fn request(&self, lease: &Lease) -> ApprovalRequest {
        ApprovalRequest::pending_about(self.t.db(), &lease.subject())
            .await
            .unwrap()
            .remove(0)
    }

    async fn charges(&self, lease: &Lease) -> Vec<LeaseCharge> {
        let mut all = lease.charges(self.t.db()).await.unwrap();
        all.sort_by_key(|c| (c.due_date, c.id));
        all
    }

    /// A cheque for each charge due on or after `from`.
    async fn cheques_from(&self, lease: &Lease, from: NaiveDate) -> Vec<(ChequeId, NaiveDate)> {
        let mut made = Vec::new();
        for charge in self
            .charges(lease)
            .await
            .iter()
            .filter(|c| c.due_date >= from)
        {
            let id = self.t.cheques.receive(
                TENANT,
                &lease.agreement(),
                &format!("C-{}", charge.due_date),
                charge.due_date,
                charge.amount,
                &[charge.subject()],
            );
            made.push((id, charge.due_date));
        }
        made
    }

    /// Sets a legal case flag of `case_type` on the lease.
    async fn flag(&self, lease: &Lease, case_type: &str) {
        self.t
            .post(
                &self.manager,
                &format!("/property/leases/{}/legal-cases", lease.id),
                &[
                    ("case_type", case_type),
                    ("reference", "RDC-2027-77"),
                    ("ticket", "new"),
                ],
            )
            .await
            .assert_ok();
    }
}

/// AC1, AC2: property manager A requests `abandoned` with the inspection and
/// the repossession basis: the approval request goes to the admin and the
/// lease stays `active`. A cannot approve. When the admin approves, the lease
/// is `terminated`, the unit vacated, the deposit in settlement, the invoices
/// to the move-out date stay open and `LeaseForceTerminated` is emitted.
#[renox::test]
async fn bl_prp_12_an_abandoned_unit_is_terminated_after_the_admin_approves() {
    let s = scene().await;
    let lease = s.leased().await;
    s.abandonment_evidence(&lease);

    s.ask(&lease, "abandoned", &[]).await.assert_ok();

    let waiting = s.t.reload(&lease).await;
    assert_eq!(waiting.status, LeaseStatus::Active);
    assert_eq!(s.t.unit_now(&s.unit).await.status, UnitStatus::Occupied);
    let request = s.request(&lease).await;
    assert!(s.t.amended.all().is_empty());
    assert!(s.t.forced.all().is_empty());

    // The requester cannot approve; neither can the leasing agent.
    let refused = approve(s.t.state(), request.id, &s.manager, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
    let refused = approve(s.t.state(), request.id, &s.agent, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
    assert_eq!(s.t.status(&lease).await, LeaseStatus::Active);

    let decided = approve(s.t.state(), request.id, &s.admin, "abandoned")
        .await
        .unwrap();
    assert!(decided.is_some());

    let after = s.t.reload(&lease).await;
    assert_eq!(after.status, LeaseStatus::Terminated);
    assert_eq!(after.actual_move_out, Some(d(3, 31)));
    assert_eq!(after.force_reason.as_deref(), Some("abandoned"));
    assert_eq!(
        after.force_basis.as_deref(),
        Some("Municipality repossession order 41/2027")
    );
    assert_eq!(s.t.unit_now(&s.unit).await.status, UnitStatus::Available);
    assert_eq!(s.t.deposits.started(), vec![lease.agreement()]);

    // The invoices to the move-out date stay open, the later charges are cancelled.
    assert!(s.t.invoices.credits().is_empty());
    for invoice in s.t.invoices.all() {
        assert_eq!(invoice.summary.status, InvoiceStatus::Issued);
    }
    for charge in s.charges(&after).await {
        let expected = if charge.due_date > d(3, 31) {
            ChargeStatus::Cancelled
        } else {
            ChargeStatus::Invoiced
        };
        assert_eq!(charge.status, expected, "{}", charge.due_date);
    }

    let events = s.t.forced.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].reason, "abandoned");
    assert_eq!(events[0].move_out, d(3, 31));
    assert_eq!(events[0].requested_by, Some(s.manager.id));
    // The portal turns read-only on `LeaseTerminated`.
    assert_eq!(s.t.terminated.all().len(), 1);
}

/// An abandoned unit needs the inspection, the repossession basis attachment
/// and the legal basis as text; the missing one is named and no request is
/// created.
#[renox::test]
async fn bl_prp_12_the_evidence_of_an_abandoned_unit_is_required() {
    let s = scene().await;
    let lease = s.leased().await;

    let refused = s.ask(&lease, "abandoned", &[]).await;
    refused.assert_status(422);
    assert!(
        refused
            .text()
            .to_lowercase()
            .contains("abandonment inspection missing"),
        "{}",
        refused.text()
    );

    s.t.docs.accept(&lease.subject(), INSPECTION_KIND);
    let refused = s.ask(&lease, "abandoned", &[]).await;
    refused.assert_status(422);
    assert!(
        refused
            .text()
            .to_lowercase()
            .contains("repossession basis missing"),
        "{}",
        refused.text()
    );

    s.t.docs.accept(&lease.subject(), REPOSSESSION_BASIS_KIND);
    // No legal basis as text.
    let refused =
        s.t.post(
            &s.manager,
            &s.path(&lease),
            &[("reason", "abandoned"), ("move_out", "2027-03-31")],
        )
        .await;
    refused.assert_status(422);
    assert!(refused.text().contains("legal basis"), "{}", refused.text());

    assert!(
        ApprovalRequest::pending_about(s.t.db(), &lease.subject())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(s.t.status(&lease).await, LeaseStatus::Active);
}

/// AC3: `eviction_executed` needs two attachments. With only the judgment it is
/// refused ("execution record missing"), with only the execution record
/// ("judgment missing"); with both, the request is created.
#[renox::test]
async fn bl_prp_12_an_eviction_needs_the_judgment_and_the_execution_record() {
    let s = scene().await;
    let lease = s.leased().await;

    s.t.docs.accept(&lease.subject(), JUDGMENT_KIND);
    let refused = s.ask(&lease, "eviction_executed", &[]).await;
    refused.assert_status(422);
    assert!(
        refused
            .text()
            .to_lowercase()
            .contains("execution record missing"),
        "{}",
        refused.text()
    );

    let other = scene().await;
    let second = other.leased().await;
    other
        .t
        .docs
        .accept(&second.subject(), EXECUTION_RECORD_KIND);
    let refused = other.ask(&second, "eviction_executed", &[]).await;
    refused.assert_status(422);
    assert!(
        refused.text().to_lowercase().contains("judgment missing"),
        "{}",
        refused.text()
    );

    s.t.docs.accept(&lease.subject(), EXECUTION_RECORD_KIND);
    s.ask(&lease, "eviction_executed", &[]).await.assert_ok();
    assert_eq!(
        ApprovalRequest::pending_about(s.t.db(), &lease.subject())
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(s.t.status(&lease).await, LeaseStatus::Active);
}

/// AC4: an open legal case flag of type eviction is closed with the outcome
/// "eviction executed"; a flag of another type stays open.
#[renox::test]
async fn bl_prp_12_an_eviction_flag_is_closed_with_its_outcome() {
    let s = scene().await;
    let lease = s.leased().await;
    s.flag(&lease, "eviction").await;
    s.flag(&lease, "arrears").await;
    s.t.docs.accept(&lease.subject(), JUDGMENT_KIND);
    s.t.docs.accept(&lease.subject(), EXECUTION_RECORD_KIND);
    s.ask(&lease, "eviction_executed", &[]).await.assert_ok();
    let request = s.request(&lease).await;

    approve(s.t.state(), request.id, &s.admin, "")
        .await
        .unwrap();

    assert_eq!(s.t.status(&lease).await, LeaseStatus::Terminated);
    let cases = LegalCase::of_root(s.t.db(), lease.root_id).await.unwrap();
    let eviction = cases
        .iter()
        .find(|c| c.case_type == CaseType::Eviction)
        .unwrap();
    assert_eq!(eviction.status, CaseStatus::Cleared);
    assert_eq!(eviction.outcome, Some(CaseOutcome::EvictionExecuted));
    let arrears = cases
        .iter()
        .find(|c| c.case_type == CaseType::Arrears)
        .unwrap();
    assert_eq!(arrears.status, CaseStatus::Open);
    assert_eq!(s.t.forced.all()[0].closed_cases, 1);
}

/// AC5: the cheques the requester marks "keep as evidence" stay `received` and
/// are held as evidence (never deposited); the ones to return go back against a
/// receipt. Cheques dated up to the move-out are not touched.
#[renox::test]
async fn bl_prp_12_later_cheques_are_kept_as_evidence_or_returned_as_chosen() {
    let s = scene().await;
    let lease = s.leased().await;
    s.abandonment_evidence(&lease);
    let cheques = s.cheques_from(&lease, d(3, 1)).await;
    let march = cheques[0].0;
    let april = cheques[1].0;
    let may = cheques[2].0;
    let june = cheques[3].0;
    let keep = format!("{}, {}", april.0, may.0);
    let back = june.0.to_string();

    s.ask(
        &lease,
        "abandoned",
        &[("keep", &keep), ("hand_back", &back)],
    )
    .await
    .assert_ok();
    let request = s.request(&lease).await;
    approve(s.t.state(), request.id, &s.admin, "")
        .await
        .unwrap();

    assert_eq!(s.t.cheques.get(march).status, ChequeStatus::Received);
    assert_eq!(s.t.cheques.get(april).status, ChequeStatus::Received);
    assert_eq!(s.t.cheques.get(may).status, ChequeStatus::Received);
    assert_eq!(s.t.cheques.get(june).status, ChequeStatus::Returned);
    assert_eq!(s.t.cheques.evidence(), vec![april, may]);
    // The others, up to December, were not chosen and go back (no flag is open).
    for (id, date) in &cheques[4..] {
        assert_eq!(
            s.t.cheques.get(*id).status,
            ChequeStatus::Returned,
            "{date}"
        );
    }
    assert_eq!(s.t.forced.all()[0].evidence_cheques, 2);
}

/// While a legal case flag is open, the later cheques not chosen are kept as
/// evidence (the default); a cheque named to go back goes back.
#[renox::test]
async fn bl_prp_12_with_a_legal_case_open_later_cheques_are_kept_by_default() {
    let s = scene().await;
    let lease = s.leased().await;
    s.abandonment_evidence(&lease);
    s.flag(&lease, "arrears").await;
    let cheques = s.cheques_from(&lease, d(4, 1)).await;
    let back = cheques[0].0.0.to_string();

    s.ask(&lease, "abandoned", &[("hand_back", &back)])
        .await
        .assert_ok();
    let request = s.request(&lease).await;
    approve(s.t.state(), request.id, &s.admin, "")
        .await
        .unwrap();

    assert_eq!(s.t.cheques.get(cheques[0].0).status, ChequeStatus::Returned);
    for (id, _) in &cheques[1..] {
        assert_eq!(s.t.cheques.get(*id).status, ChequeStatus::Received);
    }
    assert_eq!(s.t.cheques.evidence().len(), cheques.len() - 1);
}

/// A cheque that was not received with the lease cannot be named; a forced
/// termination from `draft` or `cancelled` is not possible; the request needs
/// `leases.manage` in the building.
#[renox::test]
async fn bl_prp_12_the_request_is_checked() {
    let s = scene().await;
    let lease = s.leased().await;
    s.abandonment_evidence(&lease);

    let refused = s.ask(&lease, "abandoned", &[("keep", "999")]).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("not received with this lease"),
        "{}",
        refused.text()
    );

    let refused =
        s.t.post(
            &s.agent,
            &s.path(&lease),
            &[
                ("reason", "abandoned"),
                ("move_out", "2027-03-31"),
                ("basis", "x"),
            ],
        )
        .await;
    refused.assert_status(403);

    let draft = s.t.lease(&s.unit, 400).await;
    s.abandonment_evidence(&draft);
    let refused = s.ask(&draft, "abandoned", &[]).await;
    assert!(refused.status.as_u16() >= 400, "{}", refused.text());
    assert_eq!(s.t.status(&draft).await, LeaseStatus::Draft);
}
