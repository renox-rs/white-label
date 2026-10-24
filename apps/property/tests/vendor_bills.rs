//! BL-PRP-34 (#479): approved vendor bills on the owner statement. The bill
//! of a ticket job on an owner's unit, and each building's share of an AMC
//! bill split by area, become expense lines of the draft statement of the
//! period that holds the approval day, once per agreement.

mod support;

use property::agreement::{AgreementStatus, ManagementAgreement};
use property::statement::{self, LineKind, OwnerStatement, StatementLine};
use property::vendor_bills::{self, ContractBuilding};
use property::{Building, NewUnit, Unit, UnitUse, create_unit};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::invoicing::{
    BillRequest, CostObjectAmount, DraftLine, SupplierBillApproved, SupplierBillId,
    SupplierBillStatus, supplier_bills,
};
use wl_kernel::parties::PartyKind;
use wl_kernel::tickets::{
    OpenTicket, SERVICE_CONTRACT_SUBJECT_TYPE, ServiceContractActivated, ServiceContractId,
    TICKET_SUBJECT_TYPE, task_creator,
};
use wl_kernel::{PartyId, Subject, VatCategory};

use support::{Booted, Real, aed, boot, boot_with};

/// The owner of the units.
const OWNER: PartyId = PartyId(31);
/// The vendor.
const COOLAIR: PartyId = PartyId(40);

fn ymd(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

struct Scene {
    t: Booted,
    agreement: ManagementAgreement,
    tower_b: Building,
    tower_c: Building,
    unit: Unit,
}

async fn unit_with_area(t: &Booted, building: &Building, number: &str, sqft: i64) -> Unit {
    create_unit(
        t.db(),
        NewUnit {
            building_id: building.id,
            number: number.to_owned(),
            usage: UnitUse::Residential,
            area_sqft_milli: Some(sqft * 1_000),
            asking_rent: Some(aed(100_000)),
            ..Default::default()
        },
    )
    .await
    .unwrap()
}

/// Tower B (unit 1204 of 1,500 sq ft is the owner's, 1205 of 4,500 sq ft is
/// not under management: the owner holds 25% of Tower B's area) and Tower C
/// (4,000 sq ft, not the owner's); an active agreement since 2026-01-01.
async fn scene_with(t: Booted) -> Scene {
    t.parties
        .add(OWNER.0, "Hessa Al Mansoori", PartyKind::Person);
    t.parties.add(COOLAIR.0, "CoolAir", PartyKind::Company);
    let tower_b = t.building_coded("TWB").await;
    let tower_c = t.building_coded("TWC").await;
    let unit = unit_with_area(&t, &tower_b, "1204", 1_500).await;
    unit_with_area(&t, &tower_b, "1205", 4_500).await;
    unit_with_area(&t, &tower_c, "301", 4_000).await;
    let mut agreement = ManagementAgreement {
        owner_party_id: OWNER.0,
        fee_bp: 500,
        payout_frequency: "monthly".to_owned(),
        notice_days: 90,
        start_date: ymd(2026, 1, 1),
        status: AgreementStatus::Active,
        ..Default::default()
    };
    let mut tx = t.db().begin().await.unwrap();
    agreement
        .insert_in_state(&mut tx, AgreementStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    property::agreement::add_unit(t.db(), &agreement, unit.id, ymd(2026, 1, 1))
        .await
        .unwrap();
    Scene {
        t,
        agreement,
        tower_b,
        tower_c,
        unit,
    }
}

impl Scene {
    /// A maintenance ticket on unit 1204: its id.
    async fn ticket(&self) -> i64 {
        task_creator(self.t.state())
            .open(
                self.t.state(),
                OpenTicket::new(
                    Subject::new("property.unit", self.unit.id),
                    "maintenance",
                    "Chiller not cooling",
                ),
            )
            .await
            .unwrap()
            .expect("a ticket")
            .id
            .0
    }

    /// The approval of bill `bill` of AED `dirhams` coded to `object`.
    fn approved(&self, bill: i64, dirhams: i64, object: Subject) -> SupplierBillApproved {
        SupplierBillApproved {
            bill: SupplierBillId(bill),
            supplier: COOLAIR,
            amount: aed(dirhams),
            cost_objects: vec![CostObjectAmount {
                cost_object: object,
                amount: aed(dirhams),
            }],
            landed_costs: Vec::new(),
        }
    }

    async fn post(&self, event: &SupplierBillApproved, on: NaiveDate) -> Vec<StatementLine> {
        vendor_bills::post_bill(self.t.state(), event, on)
            .await
            .unwrap()
    }

    /// The draft of the month starting `start`, generated as `PRP.RUL.05`.
    async fn draft(&self, start: NaiveDate, end: NaiveDate) -> OwnerStatement {
        statement::generate(self.t.state(), self.agreement.id, start, end)
            .await
            .unwrap()
            .expect("a draft")
    }

    async fn expenses_of(&self, statement: &OwnerStatement) -> Vec<StatementLine> {
        statement
            .lines(self.t.db())
            .await
            .unwrap()
            .into_iter()
            .filter(|l| l.kind == LineKind::Expense)
            .collect()
    }
}

/// Done when 1, 4 and 5: the bill of AED 350 of a job on unit 1204, approved
/// on 2026-10-20, is an expense of AED 350 on the draft for October generated
/// in November; generating again or a repeated approval does not list it
/// twice; a bill approved on 2026-11-03 is not on October's statement but on
/// November's.
#[renox::test]
async fn bl_prp_34_a_ticket_bill_is_an_expense_of_the_period_it_was_approved_in() {
    let s = scene_with(boot().await).await;
    let ticket = s.ticket().await;
    let october = s.approved(101, 350, Subject::new(TICKET_SUBJECT_TYPE, ticket));
    let made = s.post(&october, ymd(2026, 10, 20)).await;
    assert_eq!(made.len(), 1);
    assert!(made[0].statement_id.is_none(), "it waits for the draft");
    let november = s.approved(102, 120, Subject::new(TICKET_SUBJECT_TYPE, ticket));
    s.post(&november, ymd(2026, 11, 3)).await;

    let draft = s.draft(ymd(2026, 10, 1), ymd(2026, 10, 31)).await;
    let lines = s.expenses_of(&draft).await;
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].amount, -aed(350));
    assert!(lines[0].description.contains("CoolAir"), "{}", lines[0].description);
    assert!(
        lines[0].description.contains(&format!("TKT-{ticket} (unit 1204)")),
        "{}",
        lines[0].description
    );
    assert_eq!(draft.expenses, aed(350));

    // Again: the same approval and the same draft list it once.
    s.post(&october, ymd(2026, 10, 20)).await;
    assert!(
        statement::generate(
            s.t.state(),
            s.agreement.id,
            ymd(2026, 10, 1),
            ymd(2026, 10, 31)
        )
        .await
        .unwrap()
        .is_none()
    );
    let october_bill = StatementLine::query()
        .where_(StatementLine::AGREEMENT_ID.eq(s.agreement.id))
        .where_(StatementLine::KIND.eq(LineKind::Expense))
        .get(s.t.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|l| {
            l.source
                .as_deref()
                .is_some_and(|src| src.starts_with("invoicing.supplier_bill:101|"))
        })
        .count();
    assert_eq!(october_bill, 1);
    assert_eq!(s.expenses_of(&draft).await.len(), 1);

    // The bill approved on 2026-11-03 is on November's statement.
    let next = s.draft(ymd(2026, 11, 1), ymd(2026, 11, 30)).await;
    let lines = s.expenses_of(&next).await;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].amount, -aed(120));
}

/// Done when 2: an AMC bill of AED 8,000 for Tower B and Tower C, split by
/// area 60/40, with the owner holding 25% of Tower B's area, gives the owner a
/// line of AED 1,200.
#[renox::test]
async fn bl_prp_34_an_amc_bill_gives_the_owner_its_share_by_area() {
    let s = scene_with(boot().await).await;
    vendor_bills::contract_activated(
        s.t.state(),
        &ServiceContractActivated {
            contract: ServiceContractId(7),
            vendor: COOLAIR,
            subjects: vec![
                Subject::new("property.building", s.tower_b.id),
                Subject::new("property.building", s.tower_c.id),
            ],
            start: ymd(2026, 1, 1),
            end: ymd(2026, 12, 31),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        ContractBuilding::query()
            .where_(ContractBuilding::CONTRACT_ID.eq(7))
            .count(s.t.db())
            .await
            .unwrap(),
        2
    );

    let amc = s.approved(201, 8_000, Subject::new(SERVICE_CONTRACT_SUBJECT_TYPE, 7));
    let made = s.post(&amc, ymd(2026, 10, 20)).await;

    assert_eq!(made.len(), 1, "only the owner's share of Tower B");
    assert_eq!(made[0].amount, -aed(1_200));
    assert!(
        made[0].description.contains("Tower TWB, owner's share"),
        "{}",
        made[0].description
    );
    let draft = s.draft(ymd(2026, 10, 1), ymd(2026, 10, 31)).await;
    assert_eq!(draft.expenses, aed(1_200));
}

/// Done when 3: a bill held as a possible duplicate (wl-invoicing put it in
/// `exception`) is not listed; the first bill with that number is.
#[renox::test]
async fn bl_prp_34_a_bill_held_as_a_possible_duplicate_is_not_listed() {
    let s = scene_with(boot_with(Real::INVOICING).await).await;
    let ticket = s.ticket().await;
    let request = || {
        BillRequest::new(
            COOLAIR,
            "B-101",
            ymd(2026, 10, 18),
            Subject::new("property.unit", s.unit.id),
        )
        .line(DraftLine::new("Chiller repair", aed(350), VatCategory::Exempt))
    };
    let bills = supplier_bills(s.t.state());
    let first = bills
        .create_submitted(s.t.state(), request())
        .await
        .unwrap()
        .expect("invoicing is on");
    let second = bills
        .create_submitted(s.t.state(), request())
        .await
        .unwrap()
        .expect("invoicing is on");
    let held = bills.find(s.t.db(), second).await.unwrap().unwrap();
    assert_eq!(held.status, SupplierBillStatus::Exception);

    let duplicate = s.approved(second.0, 350, Subject::new(TICKET_SUBJECT_TYPE, ticket));
    assert!(s.post(&duplicate, ymd(2026, 10, 20)).await.is_empty());
    let original = s.approved(first.0, 350, Subject::new(TICKET_SUBJECT_TYPE, ticket));
    let made = s.post(&original, ymd(2026, 10, 20)).await;
    assert_eq!(made.len(), 1);
    assert!(made[0].description.contains("B-101"), "{}", made[0].description);

    let draft = s.draft(ymd(2026, 10, 1), ymd(2026, 10, 31)).await;
    assert_eq!(s.expenses_of(&draft).await.len(), 1);
    assert_eq!(draft.expenses, aed(350));
}
