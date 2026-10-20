//! BL-PRP-23 (#273): `transfer_unit`. A unit leaves a management agreement
//! because it is sold (the company still manages it for the buyer) or is no
//! longer managed. A property manager asks, another approves; nothing changes
//! before.

mod support;

use property::agreement::{
    self, AgreementStatus, AgreementUnit, ManagementAgreement, NewAgreement,
};
use property::agreement_lifecycle::HandoverItem;
use property::notices::PAYEE_CHANGED;
use property::unit_transfer::{TransferStatus, UnitTransfer, empty_task_key};
use property::{ChargeStatus, Lease, LeaseCharge, Unit};
use renox::chrono::Duration;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::PartyId;
use wl_kernel::cheques::{ChequeId, ChequeStatus};
use wl_kernel::parties::PartyKind;

use support::{AGENT, Booted, MANAGER, TENANT, aed, boot, day};

const OWNER: PartyId = PartyId(31);
const BUYER: PartyId = PartyId(41);

struct Scene {
    t: Booted,
    old: ManagementAgreement,
    unit: Unit,
    lease: Lease,
    cheque: ChequeId,
    first: User,
    second: User,
}

async fn scene() -> Scene {
    let t = boot().await;
    t.parties
        .add(OWNER.0, "Hessa Al Mansoori", PartyKind::Person);
    t.parties.add(BUYER.0, "Omar Khalid", PartyKind::Person);
    let building = t.building().await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let site = t.user_in("Sami", MANAGER, building.id).await;
    let unit = t.unit(&building, "803").await;
    let lease = t.active_lease(&unit, &agent, &site).await;
    t.deposits.hold(&lease.agreement(), TENANT, aed(5_000));
    let charges = LeaseCharge::of_lease(t.db(), lease.id).await.unwrap();
    let last = charges.last().expect("a charge").clone();
    assert!(last.due_date > day(-60));
    let cheque = t.cheques.receive(
        TENANT,
        &lease.agreement(),
        "803-4",
        last.due_date,
        aed(25_000),
        &[last.subject()],
    );
    let mut old = ManagementAgreement {
        owner_party_id: OWNER.0,
        fee_bp: 500,
        payout_frequency: "monthly".to_owned(),
        notice_days: 90,
        start_date: day(-400),
        status: AgreementStatus::Active,
        ..Default::default()
    };
    let mut tx = t.db().begin().await.unwrap();
    old.insert_in_state(&mut tx, AgreementStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    agreement::add_unit(t.db(), &old, unit.id, day(-400))
        .await
        .unwrap();
    let first = t.user_with("Mona", MANAGER).await;
    let second = t.user_with("Mira", MANAGER).await;
    Scene {
        t,
        old,
        unit,
        lease,
        cheque,
        first,
        second,
    }
}

impl Scene {
    fn date(&self) -> String {
        day(-60).to_string()
    }

    async fn request(&self, new_owner: &str) -> renox::testing::TestResponse {
        let date = self.date();
        self.t
            .post(
                &self.first,
                &format!("/property/agreements/{}/transfers", self.old.id),
                &[
                    ("unit", self.unit.id.to_string().as_str()),
                    ("date", date.as_str()),
                    ("new_owner", new_owner),
                    ("note", ""),
                ],
            )
            .await
    }

    async fn transfer(&self) -> UnitTransfer {
        UnitTransfer::of_agreement(self.t.db(), self.old.id)
            .await
            .unwrap()
            .remove(0)
    }

    async fn approve(&self, user: &User, receipt: &str) -> renox::testing::TestResponse {
        let id = self.transfer().await.id;
        self.t
            .post(
                user,
                &format!("/property/transfers/{id}/approve"),
                &[("receipt", receipt)],
            )
            .await
    }

    async fn buyer_agreement(&self, activate: bool) -> ManagementAgreement {
        let mut draft = agreement::create_draft(
            self.t.state(),
            None,
            NewAgreement::new(BUYER, day(-60), 400),
        )
        .await
        .unwrap();
        ManagementAgreement::query()
            .where_(ManagementAgreement::ID.eq(draft.id))
            .update(self.t.db(), &[("legal_entity_id", &Some(2_i64))])
            .await
            .unwrap();
        agreement::add_unit(self.t.db(), &draft, self.unit.id, day(-60))
            .await
            .unwrap();
        if activate {
            self.t
                .post(
                    &self.first,
                    &format!("/property/agreements/{}/activate", draft.id),
                    &[],
                )
                .await
                .assert_ok();
        }
        draft = ManagementAgreement::find_or_404(self.t.db(), draft.id)
            .await
            .unwrap();
        draft
    }
}

/// AC1, AC2, AC5: a sale still managed is refused until the buyer's agreement
/// is active; once approved by another manager the charges from the date are
/// recreated with the buyer's entity, April's invoice stays, the unit moves.
#[renox::test]
async fn bl_prp_23_a_managed_sale_moves_the_unit_and_recreates_the_charges() {
    let s = scene().await;
    let buyer = s.buyer_agreement(false).await;
    assert_eq!(buyer.status, AgreementStatus::Draft);
    s.request(&BUYER.0.to_string()).await.assert_status(422);
    assert!(
        UnitTransfer::of_agreement(s.t.db(), s.old.id)
            .await
            .unwrap()
            .is_empty()
    );

    s.t.post(
        &s.first,
        &format!("/property/agreements/{}/activate", buyer.id),
        &[],
    )
    .await
    .assert_ok();
    s.request(&BUYER.0.to_string()).await.assert_ok();
    let asked = s.transfer().await;
    assert_eq!(asked.status, TransferStatus::Requested);

    // Without approval nothing changes.
    let rows = s.old.units(s.t.db()).await.unwrap();
    assert!(rows[0].date_out.is_none());
    s.approve(&s.first, "NEW-OWNER-RCPT")
        .await
        .assert_status(422);
    s.approve(&s.second, "").await.assert_status(422);
    assert_eq!(s.transfer().await.status, TransferStatus::Requested);

    s.approve(&s.second, "NEW-OWNER-RCPT").await.assert_ok();
    assert_eq!(s.transfer().await.status, TransferStatus::Approved);
    assert_eq!(
        s.transfer().await.deposit_receipt.as_deref(),
        Some("NEW-OWNER-RCPT")
    );

    let date = day(-60);
    let charges = LeaseCharge::of_lease(s.t.db(), s.lease.id).await.unwrap();
    let cancelled: Vec<_> = charges
        .iter()
        .filter(|c| c.status == ChargeStatus::Cancelled)
        .collect();
    let fresh: Vec<_> = charges
        .iter()
        .filter(|c| c.legal_entity_id == Some(2))
        .collect();
    assert_eq!(cancelled.len(), 1);
    assert_eq!(fresh.len(), 1);
    assert_eq!(fresh[0].status, ChargeStatus::Scheduled);
    assert_eq!(fresh[0].due_date, cancelled[0].due_date);
    assert!(
        charges
            .iter()
            .filter(|c| c.due_date < date)
            .all(|c| c.legal_entity_id.is_none())
    );

    // The cheque follows the recreated charge; the deposit stays held.
    assert!(
        s.t.cheques
            .get(s.cheque)
            .linked
            .contains(&fresh[0].subject())
    );
    assert!(s.t.deposits.is_held(&s.lease.agreement()));

    // The old owner's statements stop the day before; the buyer's start on it.
    let old_rows = s.old.units(s.t.db()).await.unwrap();
    assert_eq!(old_rows[0].date_out, Some(date - Duration::days(1)));
    let new_rows: Vec<AgreementUnit> = buyer.units(s.t.db()).await.unwrap();
    assert!(!new_rows[0].awaiting_transfer);
    let before = ManagementAgreement::for_unit(s.t.db(), s.unit.id, date - Duration::days(1))
        .await
        .unwrap();
    let after = ManagementAgreement::for_unit(s.t.db(), s.unit.id, date)
        .await
        .unwrap();
    assert_eq!(before.map(|a| a.id), Some(s.old.id));
    assert_eq!(after.map(|a| a.id), Some(buyer.id));

    assert_eq!(s.t.notices.of_type(PAYEE_CHANGED).len(), 1);
    // The last unit left: the property managers are told to give notice.
    assert_eq!(s.t.tasks.with_key(&empty_task_key(&s.old)).len(), 1);
    assert_eq!(
        ManagementAgreement::find_or_404(s.t.db(), s.old.id)
            .await
            .unwrap()
            .status,
        AgreementStatus::Active
    );
}

/// AC3: a "no longer managed" transfer returns the undeposited cheques and
/// transfers the deposit against a receipt.
#[renox::test]
async fn bl_prp_23_no_longer_managed_hands_the_unit_over() {
    let s = scene().await;
    s.request("").await.assert_ok();
    let id = s.transfer().await.id;
    let items = HandoverItem::of_transfer(s.t.db(), id).await.unwrap();
    assert_eq!(items.len(), 2);

    // A line without a receipt refuses the approval.
    s.approve(&s.second, "").await.assert_status(422);
    for item in &items {
        s.t.post(
            &s.first,
            &format!("/property/agreements/{}/transfer-receipts", s.old.id),
            &[
                ("item", item.id.to_string().as_str()),
                ("reference", "RCPT-X"),
            ],
        )
        .await
        .assert_ok();
    }
    s.approve(&s.second, "").await.assert_ok();

    assert_eq!(s.t.cheques.get(s.cheque).status, ChequeStatus::Returned);
    assert_eq!(s.t.cheques.returns()[0].reason, "handed over");
    assert_eq!(s.t.deposits.transferred().len(), 1);
    assert_eq!(s.t.deposits.transferred()[0].0.receipt, "RCPT-X");
    let lease = Lease::find_or_404(s.t.db(), s.lease.id).await.unwrap();
    assert_eq!(lease.handed_over_on, Some(day(-60)));
    assert_eq!(s.t.notices.of_type(PAYEE_CHANGED).len(), 1);
}

/// A rejected request changes nothing; the requester cannot approve it.
#[renox::test]
async fn bl_prp_23_a_rejected_request_changes_nothing() {
    let s = scene().await;
    s.request("").await.assert_ok();
    let id = s.transfer().await.id;
    s.t.post(&s.second, &format!("/property/transfers/{id}/reject"), &[])
        .await
        .assert_ok();
    assert_eq!(s.transfer().await.status, TransferStatus::Rejected);
    s.approve(&s.second, "").await.assert_status(422);
    let rows = s.old.units(s.t.db()).await.unwrap();
    assert!(rows[0].date_out.is_none());
    assert!(s.t.cheques.returns().is_empty());
}
