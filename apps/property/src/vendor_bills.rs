//! Approved vendor bills on the owner statement: ticket jobs and AMC shares
//! (01 Property §4.2 step 6 and row "The job is cancelled after work started",
//! §4.4 step 1, §7.2 `PRP.RUL.05`; wl-invoicing §5.4 point 4; BL-PRP-34).
//!
//! An owner statement is *collected − expenses − fee*, and the expenses are
//! the vendor bills of the owner's units. When wl-invoicing approves a
//! supplier bill (`SupplierBillApproved`), each cost object it is coded to is
//! read:
//!
//! - **A ticket** (`ticketing.ticket:N`, also a job cancelled after work
//!   started): the record it is about, through the kernel's
//!   `TaskCreator::ticket_about`. A unit gets the whole amount; a building is
//!   spread over its units by area.
//! - **A service contract** (`ticketing.service_contract:N`, an AMC): the
//!   buildings it covers ([`ContractBuilding`], recorded when the contract
//!   became active, `ServiceContractActivated`), spread by the allocation
//!   basis, then each building's share over its units by the same basis.
//!
//! The units' shares go to the management agreement that covers each unit on
//! the approval day, summed per agreement, as one expense line per bill, cost
//! object and unit or building ([`crate::statement::post_line`]): the line goes
//! on the draft statement whose period holds the approval day, else it waits
//! for the next draft, so a bill approved in or before the period is on that
//! period's statement and one approved later is on the next. A line is made
//! once per agreement and source, so generating the draft again, or a repeated
//! event, never lists a bill twice; a published statement is never edited.
//! The accountant may still add or remove lines of the draft.
//!
//! Decided where the spec is silent or the kernel offers nothing yet:
//!
//! - **Which bills:** the event is the approval; a bill that wl-invoicing
//!   reports as `draft`, `exception` (a possible duplicate) or `void` is never
//!   listed. Without wl-invoicing the event is trusted.
//! - **Amount:** the bill's total with VAT ([`SupplierBillApproved::amount`]),
//!   spread over its cost objects by their net amounts.
//! - **Allocation basis:** area (the default of wl-invoicing §5.4); a unit
//!   without an area counts by number of units when no unit of the building
//!   has one. The contract's own basis (units, asset, manual %) is not in the
//!   kernel yet (BL-INV-24), so area is used for every contract.
//! - **Rounding:** half up to a fils; the last share takes the difference so
//!   the shares add up ([`split`]).
//! - **A unit under no agreement:** its share stays with the company; no line.
//! - **A bill reduced after it was on a published statement** (a supplier
//!   debit note): the adjustment line on the next statement waits for the
//!   debit note event of the kernel (not in this issue).

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{SupplierBillApproved, SupplierBillStatus, supplier_bills};
use wl_kernel::tickets::{
    SERVICE_CONTRACT_SUBJECT_TYPE, ServiceContractActivated, TICKET_SUBJECT_TYPE, TicketId,
    task_creator,
};

use crate::agreement::ManagementAgreement;
use crate::building::{BUILDING_SUBJECT_TYPE, Building};
use crate::statement::{LineKind, PostLine, StatementLine, post_line};
use crate::texts::english;
use crate::unit::{UNIT_SUBJECT_TYPE, Unit};

/// A building a service contract covers.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "property_contract_buildings",
    unique(contract_id, building_id)
)]
pub struct ContractBuilding {
    /// The primary key.
    pub id: i64,
    /// The service contract (wl-ticketing's id).
    pub contract_id: i64,
    /// The building.
    #[model(references = "property_buildings")]
    pub building_id: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// `total` fils spread over `weights`: each share is rounded half up to a
/// fils and the last takes the difference, so the shares add up to `total`.
/// Equal weights when they are all 0; nothing for no weights.
pub fn split(total: i64, weights: &[i64]) -> Vec<i64> {
    if weights.is_empty() {
        return Vec::new();
    }
    let sum: i64 = weights.iter().map(|w| (*w).max(0)).sum();
    let weights: Vec<i64> = if sum == 0 {
        vec![1; weights.len()]
    } else {
        weights.iter().map(|w| (*w).max(0)).collect()
    };
    let sum: i128 = weights.iter().map(|w| i128::from(*w)).sum();
    let mut shares = Vec::with_capacity(weights.len());
    let mut given = 0;
    for (index, weight) in weights.iter().enumerate() {
        let share = if index + 1 == weights.len() {
            total - given
        } else {
            let numerator = i128::from(total) * i128::from(*weight) * 2 + sum;
            i64::try_from(numerator.div_euclid(2 * sum)).unwrap_or(0)
        };
        given += share;
        shares.push(share);
    }
    shares
}

/// One expense line for one agreement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Share {
    /// The agreement.
    pub agreement_id: i64,
    /// What the owner bears, in fils (positive).
    pub amount: i64,
    /// The text of the line.
    pub description: String,
    /// What it comes from, so it is made once.
    pub source: String,
}

/// What a bill is, for the text of its lines.
struct BillText {
    number: String,
    supplier: String,
}

/// The units of `building`, oldest first.
async fn units_of(db: &Db, building: i64) -> Result<Vec<Unit>> {
    Unit::query()
        .where_(Unit::BUILDING_ID.eq(building))
        .order_by(Unit::ID)
        .get(db)
        .await
}

/// The weight of `unit` in the area basis.
fn area_of(unit: &Unit) -> i64 {
    unit.area_sqft_milli.unwrap_or(0).max(0)
}

/// The weight of `building` in the area basis: the area of its units.
async fn building_area(db: &Db, building: i64) -> Result<i64> {
    Ok(units_of(db, building).await?.iter().map(area_of).sum())
}

/// `amount` fils of `building` spread over its units by area, summed per
/// agreement that covers the unit on `day`.
async fn by_owner(db: &Db, building: i64, amount: i64, day: NaiveDate) -> Result<Vec<(i64, i64)>> {
    let units = units_of(db, building).await?;
    let weights: Vec<i64> = units.iter().map(area_of).collect();
    let mut owners: Vec<(i64, i64)> = Vec::new();
    for (unit, share) in units.iter().zip(split(amount, &weights)) {
        if share == 0 {
            continue;
        }
        let Some(agreement) = ManagementAgreement::for_unit(db, unit.id, day).await? else {
            continue;
        };
        match owners.iter_mut().find(|(id, _)| *id == agreement.id) {
            Some(found) => found.1 += share,
            None => owners.push((agreement.id, share)),
        }
    }
    Ok(owners)
}

/// The expense lines of the bill `event` approved on `day`.
///
/// # Errors
///
/// The database's errors and the ticketing contract's.
pub async fn shares(
    state: &AppState,
    event: &SupplierBillApproved,
    day: NaiveDate,
) -> Result<Vec<Share>> {
    let mut found = Vec::new();
    if event.cost_objects.is_empty() || event.amount <= 0 {
        return Ok(found);
    }
    let text = bill_text(state, event).await?;
    let bill = Subject::new("invoicing.supplier_bill", event.bill.0);
    let weights: Vec<i64> = event.cost_objects.iter().map(|c| c.amount).collect();
    for (object, portion) in event.cost_objects.iter().zip(split(event.amount, &weights)) {
        let cost = &object.cost_object;
        if cost.subject_type == TICKET_SUBJECT_TYPE {
            let Some(about) = task_creator(state)
                .ticket_about(state, TicketId(cost.subject_id))
                .await?
            else {
                continue;
            };
            let subject = about.subject;
            if subject.subject_type == UNIT_SUBJECT_TYPE {
                let Some(unit) = Unit::find(&state.db, subject.subject_id).await? else {
                    continue;
                };
                let Some(agreement) =
                    ManagementAgreement::for_unit(&state.db, unit.id, day).await?
                else {
                    continue;
                };
                found.push(Share {
                    agreement_id: agreement.id,
                    amount: portion,
                    description: english(
                        "property.vendor_bills.ticket_line",
                        &[
                            ("bill", &text.number),
                            ("supplier", &text.supplier),
                            ("ticket", &about.ticket.number),
                            ("unit", &unit.number),
                        ],
                    ),
                    source: format!("{bill}|{cost}|unit:{}", unit.id),
                });
            } else if subject.subject_type == BUILDING_SUBJECT_TYPE {
                let Some(building) = Building::find(&state.db, subject.subject_id).await? else {
                    continue;
                };
                for (agreement, amount) in by_owner(&state.db, building.id, portion, day).await? {
                    found.push(Share {
                        agreement_id: agreement,
                        amount,
                        description: english(
                            "property.vendor_bills.building_line",
                            &[
                                ("bill", &text.number),
                                ("supplier", &text.supplier),
                                ("ticket", &about.ticket.number),
                                ("building", &building.name_en),
                            ],
                        ),
                        source: format!("{bill}|{cost}|building:{}", building.id),
                    });
                }
            }
        } else if cost.subject_type == SERVICE_CONTRACT_SUBJECT_TYPE {
            let covered = ContractBuilding::query()
                .where_(ContractBuilding::CONTRACT_ID.eq(cost.subject_id))
                .order_by(ContractBuilding::BUILDING_ID)
                .get(&state.db)
                .await?;
            let mut areas = Vec::with_capacity(covered.len());
            for row in &covered {
                areas.push(building_area(&state.db, row.building_id).await?);
            }
            for (row, share) in covered.iter().zip(split(portion, &areas)) {
                let Some(building) = Building::find(&state.db, row.building_id).await? else {
                    continue;
                };
                for (agreement, amount) in by_owner(&state.db, building.id, share, day).await? {
                    found.push(Share {
                        agreement_id: agreement,
                        amount,
                        description: english(
                            "property.vendor_bills.contract_line",
                            &[
                                ("bill", &text.number),
                                ("supplier", &text.supplier),
                                ("building", &building.name_en),
                            ],
                        ),
                        source: format!("{bill}|{cost}|building:{}", building.id),
                    });
                }
            }
        }
    }
    Ok(found)
}

/// The bill's number and supplier, as its lines name them.
async fn bill_text(state: &AppState, event: &SupplierBillApproved) -> Result<BillText> {
    let number = supplier_bills(state)
        .find(&state.db, event.bill)
        .await?
        .map_or_else(|| format!("#{}", event.bill.0), |bill| bill.number);
    let supplier = wl_kernel::parties::directory(state)
        .display_name(&state.db, event.supplier, "en")
        .await?
        .unwrap_or_else(|| format!("#{}", event.supplier.0));
    Ok(BillText { number, supplier })
}

/// Whether the bill of `event` may be listed: wl-invoicing does not report it
/// as a draft, held (a possible duplicate) or void.
async fn listable(state: &AppState, event: &SupplierBillApproved) -> Result<bool> {
    Ok(
        match supplier_bills(state).find(&state.db, event.bill).await? {
            Some(bill) => !matches!(
                bill.status,
                SupplierBillStatus::Draft
                    | SupplierBillStatus::Exception
                    | SupplierBillStatus::Void
            ),
            None => true,
        },
    )
}

/// Puts the bill of `event`, approved on `day`, on the owner statements of
/// its units' agreements as expense lines (`PRP.RUL.05`). A line made before
/// for the same agreement and source is returned, not made again.
///
/// # Errors
///
/// The database's errors and the contracts'.
pub async fn post_bill(
    state: &AppState,
    event: &SupplierBillApproved,
    day: NaiveDate,
) -> Result<Vec<StatementLine>> {
    if !listable(state, event).await? {
        return Ok(Vec::new());
    }
    let mut made = Vec::new();
    for share in shares(state, event, day).await? {
        if share.amount <= 0 {
            continue;
        }
        let line = PostLine::new(LineKind::Expense, &share.description, -share.amount, day)
            .source(&share.source);
        made.push(post_line(&state.db, share.agreement_id, line).await?);
    }
    Ok(made)
}

/// Records the buildings the activated contract covers: its subjects that are
/// buildings, and the buildings of its subjects that are units.
///
/// # Errors
///
/// The database's errors.
pub async fn contract_activated(state: &AppState, event: &ServiceContractActivated) -> Result {
    let mut buildings: Vec<i64> = Vec::new();
    for subject in &event.subjects {
        let building = if subject.subject_type == BUILDING_SUBJECT_TYPE {
            Some(subject.subject_id)
        } else if subject.subject_type == UNIT_SUBJECT_TYPE {
            Unit::find(&state.db, subject.subject_id)
                .await?
                .map(|unit| unit.building_id)
        } else {
            None
        };
        if let Some(id) = building
            && !buildings.contains(&id)
            && Building::find(&state.db, id).await?.is_some()
        {
            buildings.push(id);
        }
    }
    for building in buildings {
        let exists = ContractBuilding::query()
            .where_(ContractBuilding::CONTRACT_ID.eq(event.contract.0))
            .where_(ContractBuilding::BUILDING_ID.eq(building))
            .exists(&state.db)
            .await?;
        if !exists {
            ContractBuilding::create(
                &state.db,
                ContractBuilding {
                    contract_id: event.contract.0,
                    building_id: building,
                    ..Default::default()
                },
            )
            .await?;
        }
    }
    Ok(())
}

/// Registers the listeners.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: SupplierBillApproved, state: AppState| async move {
        post_bill(&state, &event, today()).await.map(drop)
    });
    app.listen(
        |event: ServiceContractActivated, state: AppState| async move {
            contract_activated(&state, &event).await
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shares_round_half_up_and_the_last_takes_the_difference() {
        // AED 8,000 by area 60/40.
        assert_eq!(split(800_000, &[60, 40]), vec![480_000, 320_000]);
        // 25% of Tower B's AED 4,800.
        assert_eq!(split(480_000, &[25, 75]), vec![120_000, 360_000]);
        // Thirds of 100 fils: 33, 33, 34.
        assert_eq!(split(100, &[1, 1, 1]), vec![33, 33, 34]);
        // Half a fils rounds up; the last share balances.
        assert_eq!(split(3, &[1, 1]), vec![2, 1]);
        // No weight: equal shares.
        assert_eq!(split(100, &[0, 0]), vec![50, 50]);
        assert!(split(100, &[]).is_empty());
        let shares = split(123_457, &[7, 11, 13]);
        assert_eq!(shares.iter().sum::<i64>(), 123_457);
    }
}
