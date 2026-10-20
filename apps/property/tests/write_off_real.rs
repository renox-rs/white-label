//! BL-PRP-18 (#268) with the real wl-invoicing: the write-off of the rent of a
//! managed building is approved by a user other than the requester
//! (`FIN.ACL.15`), whoever the requester is; on that approval the line *rent
//! written off* goes to the owner statement.

mod support;

use property::agreement::{AgreementStatus, ManagementAgreement};
use property::statement::{LineKind, StatementLine};
use property::{ChargeStatus, Lease, LeaseCharge};
use renox::auth::permissions::define_role;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::approvals::{ApprovalRequest, ApprovalStatus, approve};
use wl_kernel::invoicing::{DraftLine, InvoiceId, InvoiceStatus, invoicer};
use wl_kernel::parties::PartyKind;
use wl_kernel::{PartyId, VatCategory};

use support::{ACCOUNTANT, AGENT, Booted, MANAGER, Real, TENANT, aed, boot_with, day};

/// The owner of the managed unit.
const OWNER: PartyId = PartyId(31);

struct Scene {
    t: Booted,
    agreement: ManagementAgreement,
    lease: Lease,
    /// The accountant who asks.
    requester: User,
    /// Another holder of the approving role.
    approver: User,
    clerk: User,
}

async fn scene() -> Scene {
    let t = boot_with(Real::INVOICING).await;
    t.parties
        .add(OWNER.0, "Hessa Al Mansoori", PartyKind::Person);
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let site_manager = t.user_in("Sami", MANAGER, building.id).await;
    let lease = t.active_lease(&unit, &agent, &site_manager).await;
    let mut agreement = ManagementAgreement {
        owner_party_id: OWNER.0,
        fee_bp: 500,
        payout_frequency: "monthly".to_owned(),
        notice_days: 90,
        start_date: day(-400),
        status: AgreementStatus::Active,
        ..Default::default()
    };
    let mut tx = t.db().begin().await.unwrap();
    agreement
        .insert_in_state(&mut tx, AgreementStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    property::agreement::add_unit(t.db(), &agreement, unit.id, day(-400))
        .await
        .unwrap();
    // wl-invoicing's own approvers of a write-off are the holders of the role
    // `receivables` (or `admin`).
    define_role(
        t.db(),
        "receivables",
        &[
            wl_invoicing::permissions::VIEW,
            wl_invoicing::permissions::ISSUE,
            wl_invoicing::permissions::WRITE_OFF,
        ],
    )
    .await
    .unwrap();
    let requester = t.user_with("Amal", ACCOUNTANT).await;
    requester.assign_role(t.db(), "receivables").await.unwrap();
    let approver = t.user_with("Bilal", "receivables").await;
    let clerk = t.user_with("Clara", "receivables").await;
    Scene {
        t,
        agreement,
        lease,
        requester,
        approver,
        clerk,
    }
}

impl Scene {
    /// The lines of the agreement that wait for the next draft.
    async fn waiting(&self) -> Vec<StatementLine> {
        StatementLine::query()
            .where_(StatementLine::AGREEMENT_ID.eq(self.agreement.id))
            .where_(StatementLine::STATEMENT_ID.is_null())
            .get(self.t.db())
            .await
            .unwrap()
    }

    /// An issued rent invoice of `amount` fils, a charge of the lease.
    async fn invoice(&self, amount: i64, position: i32) -> i64 {
        let draft = InvoiceDraft::new(TENANT, self.lease.subject())
            .due_date(day(-60))
            .line(DraftLine::new("Rent", amount, VatCategory::Exempt));
        let id = invoicer(self.t.state())
            .draft(self.t.db(), draft)
            .await
            .unwrap()
            .expect("invoicing is on")
            .0;
        let issued = self
            .t
            .post(&self.clerk, &format!("/invoices/{id}/issue"), &[])
            .await;
        assert!(issued.status.is_redirection(), "{}", issued.status);
        LeaseCharge::create(
            self.t.db(),
            LeaseCharge {
                lease_id: self.lease.id,
                position,
                charge_type: "rent".to_owned(),
                description: format!("Rent {position}"),
                due_date: day(-60),
                amount,
                vat_category: VatCategory::Exempt,
                status: ChargeStatus::Invoiced,
                invoice_id: Some(id),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        id
    }
}

use wl_kernel::invoicing::InvoiceDraft;

/// AC1 and the SoD of `FIN.ACL.15`: the accountant asks to write off AED 6,000
/// of rent with the owner's consent; the request waits, the accountant's own
/// approval is refused and leaves it pending, and another user's approval
/// writes the invoice off and posts *rent written off -6,000.00* for the owner.
#[renox::test]
async fn bl_prp_18_the_requester_cannot_approve_the_write_off_of_a_managed_rent() {
    let s = scene().await;
    let invoice = s.invoice(aed(6_000), 10).await;
    let id = invoice.to_string();
    let given_on = day(-1).to_string();
    s.t.post(
        &s.requester,
        &format!("/property/leases/{}/write-off", s.lease.id),
        &[
            ("invoice", id.as_str()),
            ("reason_code", "insolvent"),
            ("consent_note", "E-mail of the owner agreeing"),
            ("consent_by", "Hessa Al Mansoori"),
            ("consent_on", given_on.as_str()),
        ],
    )
    .await
    .assert_ok();

    let subject = InvoiceId(invoice).subject();
    let pending = ApprovalRequest::pending_about(s.t.db(), &subject)
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].requester_id, s.requester.id);
    assert!(s.waiting().await.is_empty());

    // The requester's own approval is refused and the request stays pending.
    let refused = approve(s.t.state(), pending[0].id, &s.requester, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
    let still = ApprovalRequest::find(s.t.db(), pending[0].id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(still.status, ApprovalStatus::Pending);
    let summary = invoicer(s.t.state())
        .find(s.t.db(), InvoiceId(invoice))
        .await
        .unwrap()
        .unwrap();
    assert_ne!(summary.status, InvoiceStatus::WrittenOff);
    assert!(s.waiting().await.is_empty());

    // Another holder of the role approves.
    approve(s.t.state(), pending[0].id, &s.approver, "agreed")
        .await
        .unwrap()
        .expect("the final approval");
    let summary = invoicer(s.t.state())
        .find(s.t.db(), InvoiceId(invoice))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.status, InvoiceStatus::WrittenOff);
    let lines = s.waiting().await;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].kind, LineKind::WrittenOff);
    assert_eq!(lines[0].amount, -aed(6_000));
}
