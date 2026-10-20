//! A unit leaves a management agreement while the agreement goes on:
//! `transfer_unit` (01 Property §4.4 row "The management agreement ends, or a
//! unit is sold mid-lease", §7.1 *Management agreement*; BL-PRP-23).
//!
//! The agreement is `active` or `ending` before and after, so this is not a
//! change of the agreement's state and the Renox machine, which has no
//! transition that stays in its state, does not carry it. A [`UnitTransfer`]
//! is the request with its approval instead: a property manager asks
//! ([`request`]) with the unit, the transfer date and the new owner or "no
//! longer managed"; a property manager other than the requester approves it
//! ([`approve`]) or rejects it ([`reject`]). Until it is approved nothing
//! changes.
//!
//! **A sale that the company still manages.** The new owner needs an `active`
//! agreement that lists the unit from the transfer date: the property manager
//! drafts it, adds the unit (the row waits, `awaiting_transfer`, and counts
//! for nothing yet, [`crate::agreement::add_unit`]) and activates it with the
//! signed agreement attached ([`crate::agreement_lifecycle::activate`]).
//! Nothing is activated automatically. Without it the request and the
//! approval are refused with "new owner's agreement not active". At the
//! approval, in one transaction: the unit goes out of the old agreement the day
//! before the transfer date and comes in to the new one on it; the scheduled
//! charges of the unit's leases from the transfer date are cancelled and
//! recreated with the new owner's legal entity as their issuer (issued
//! invoices stay with the old owner); the new owner's receipt for the deposit
//! that stays `held` is recorded (it is required when a deposit is held). After
//! the commit the cheques are relinked to the recreated charges and the tenants
//! are told the new payee. The old owner's statement includes the unit up to
//! the transfer date and the new owner's from it, because the statements read
//! the days the unit is under each agreement.
//!
//! **No longer managed.** The same handover as `end` for that unit only: the
//! pack of the transfer lists the unit's deposits and cheques not deposited,
//! each needs the new holder's receipt ([`record_receipt`]) before the
//! approval; at the approval the leases are marked "handed over", their
//! charges after the transfer date are cancelled, and after the commit the
//! cheques are returned as *handed over*, the deposits transferred to the owner
//! of the old agreement (or the holder named at its notice) and the tenants
//! told. A deposit in `settlement_pending` refuses the approval.
//!
//! When the last unit leaves, the agreement stays as it is with no unit and
//! the property managers get a task to give notice ([`empty_task_key`]).
//!
//! Decided where the spec is silent:
//!
//! - The owner of the units after "no longer managed" is not recorded; the
//!   deposits go to the holder of the old agreement ([`crate::agreement_lifecycle::holder_of`]).
//! - A request can be rejected by any property manager, the requester included
//!   (withdrawn).

use renox::chrono::{Duration, NaiveDate};
use renox::db::Transaction;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::cheques::{ChequeStatus, cheques};
use wl_kernel::deposits::deposits;
use wl_kernel::documents::today;
use wl_kernel::role_tasks::RoleTask;
use wl_kernel::{PartyId, Subject};

use crate::agreement::{AgreementStatus, AgreementUnit, ManagementAgreement};
use crate::agreement_lifecycle::{
    HandoverItem, IN_FORCE, build_items, hand_over_items, hand_over_leases, holder_of,
    open_unit_ids, tell_tenants,
};
use crate::building::refused;
use crate::charge::{ChargeStatus, LeaseCharge};
use crate::lease::Lease;
use crate::permissions::{self, ACCOUNTANT_ROLE, PROPERTY_MANAGER_ROLE};
use crate::texts::english;
use crate::unit::Unit;

/// What the unit becomes.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TransferKind {
    /// Sold to a new owner whose agreement the company still holds.
    #[default]
    Sale,
    /// No longer managed by the company.
    Unmanaged,
}

/// Where a transfer stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TransferStatus {
    /// Asked for; waits for a property manager other than the requester.
    #[default]
    Requested,
    /// Approved and carried out.
    Approved,
    /// Rejected or withdrawn: nothing changed.
    Rejected,
}

/// The request to move a unit out of an agreement, and its decision.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "property_unit_transfers", index(agreement_id), index(unit_id))]
pub struct UnitTransfer {
    /// The primary key.
    pub id: i64,
    /// The agreement the unit leaves.
    #[model(references = "property_management_agreements")]
    pub agreement_id: i64,
    /// The unit.
    #[model(references = "property_units")]
    pub unit_id: i64,
    /// The day the sale completes or the management stops.
    pub transfer_date: NaiveDate,
    /// A sale that the company still manages, or no longer managed.
    pub kind: TransferKind,
    /// The new owner, for a sale.
    pub new_owner_party_id: Option<i64>,
    /// The new owner's agreement that lists the unit from the transfer date.
    pub new_agreement_id: Option<i64>,
    /// Where it stands.
    pub status: TransferStatus,
    /// The user who asked.
    pub requested_by: Option<i64>,
    /// The day it was asked.
    pub requested_on: NaiveDate,
    /// The user who approved or rejected it.
    pub decided_by: Option<i64>,
    /// The day it was decided.
    pub decided_on: Option<NaiveDate>,
    /// A note of the requester.
    pub note: Option<String>,
    /// The new owner's receipt for the deposit that stays held (a sale).
    pub deposit_receipt: Option<String>,
    /// When the row was created.
    pub created_at: Option<renox::db::DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<renox::db::DateTime>,
}

impl UnitTransfer {
    /// The transfers of `agreement_id`, newest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_agreement(db: &Db, agreement_id: i64) -> Result<Vec<UnitTransfer>> {
        UnitTransfer::query()
            .where_(UnitTransfer::AGREEMENT_ID.eq(agreement_id))
            .order_by_desc(UnitTransfer::ID)
            .get(db)
            .await
    }

    /// Whether it still waits for a decision.
    pub fn is_open(&self) -> bool {
        self.status == TransferStatus::Requested
    }
}

/// The key the task "give notice, the agreement has no unit" is found by.
pub fn empty_task_key(agreement: &ManagementAgreement) -> String {
    format!("property.agreement_empty:{}", agreement.id)
}

/// What a user enters to move a unit out of an agreement.
#[derive(Debug, Clone)]
pub struct TransferRequest {
    /// The unit.
    pub unit_id: i64,
    /// The transfer date.
    pub date: NaiveDate,
    /// The new owner for a sale the company still manages; `None` for "no
    /// longer managed".
    pub new_owner: Option<PartyId>,
    /// A note.
    pub note: Option<String>,
}

fn require_manager(user: &AuthUser) -> Result {
    if user.has_permission(permissions::AGREEMENTS_MANAGE) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// The agreement of `owner` that lists `unit_id` from `date` and waits for the
/// transfer, newest first, if there is one.
async fn awaiting_agreement(
    db: &Db,
    owner: i64,
    unit_id: i64,
    date: NaiveDate,
) -> Result<Option<ManagementAgreement>> {
    let rows = AgreementUnit::query()
        .where_(AgreementUnit::UNIT_ID.eq(unit_id))
        .where_(AgreementUnit::AWAITING_TRANSFER.eq(true))
        .order_by_desc(AgreementUnit::ID)
        .get(db)
        .await?;
    for row in rows {
        if row.date_in != date {
            continue;
        }
        if let Some(found) = ManagementAgreement::find(db, row.agreement_id).await?
            && found.owner_party_id == owner
        {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

/// The `active` agreement of the new owner for a sale, or the refusal
/// "new owner's agreement not active".
async fn active_target(
    db: &Db,
    owner: i64,
    unit_id: i64,
    date: NaiveDate,
) -> Result<ManagementAgreement> {
    match awaiting_agreement(db, owner, unit_id, date).await? {
        Some(found) if found.status == AgreementStatus::Active => Ok(found),
        _ => Err(refused(english("property.transfers.new_agreement_inactive", &[]))),
    }
}

/// `user` asks to move a unit out of the agreement `agreement_id`.
///
/// # Errors
///
/// 403 without `agreements.manage`; 404 for an unknown agreement; 422 unless
/// the agreement is `active` or `ending`, the unit is under it, the date is
/// after the day the unit came in, no other request for the unit waits, and,
/// for a sale, the new owner's agreement is `active` and lists the unit from
/// the transfer date ("new owner's agreement not active").
pub async fn request(
    state: &AppState,
    user: &AuthUser,
    agreement_id: i64,
    asked: TransferRequest,
) -> Result<UnitTransfer> {
    require_manager(user)?;
    let agreement = ManagementAgreement::find(&state.db, agreement_id)
        .await?
        .ok_or(Error::NotFound)?;
    if !AgreementStatus::REPORTING.contains(&agreement.status) {
        return Err(refused(english("property.transfers.state", &[])));
    }
    let rows = agreement.units(&state.db).await?;
    let Some(row) = rows
        .iter()
        .find(|row| row.unit_id == asked.unit_id && row.date_out.is_none() && !row.awaiting_transfer)
    else {
        return Err(refused(english("property.transfers.not_in_agreement", &[])));
    };
    if asked.date <= row.date_in {
        return Err(refused(english("property.transfers.date_early", &[])));
    }
    let waiting = UnitTransfer::query()
        .where_(UnitTransfer::UNIT_ID.eq(asked.unit_id))
        .where_(UnitTransfer::STATUS.eq(TransferStatus::Requested))
        .exists(&state.db)
        .await?;
    if waiting {
        return Err(refused(english("property.transfers.open_request", &[])));
    }
    let (kind, new_agreement) = match asked.new_owner {
        Some(owner) => {
            if owner.0 == agreement.owner_party_id {
                return Err(refused(english("property.transfers.same_owner", &[])));
            }
            let target = active_target(&state.db, owner.0, asked.unit_id, asked.date).await?;
            (TransferKind::Sale, Some(target.id))
        }
        None => (TransferKind::Unmanaged, None),
    };
    let note = asked.note.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty());
    let transfer = UnitTransfer::create(
        &state.db,
        UnitTransfer {
            agreement_id,
            unit_id: asked.unit_id,
            transfer_date: asked.date,
            kind,
            new_owner_party_id: asked.new_owner.map(|owner| owner.0),
            new_agreement_id: new_agreement,
            status: TransferStatus::Requested,
            requested_by: Some(user.id),
            requested_on: today(),
            note,
            ..Default::default()
        },
    )
    .await?;
    if kind == TransferKind::Unmanaged {
        build_items(state, &agreement, &[asked.unit_id], Some(transfer.id)).await?;
    }
    Ok(transfer)
}

/// `user` records the reference of the new holder's receipt for the line
/// `item_id` of the pack of the transfer `id` (no longer managed).
///
/// # Errors
///
/// 403 without `agreements.manage`; 404 for a line that is not on the pack;
/// 422 unless the transfer waits for its approval, or for a blank reference.
pub async fn record_receipt(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    item_id: i64,
    reference: &str,
) -> Result<HandoverItem> {
    require_manager(user)?;
    let transfer = UnitTransfer::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    if !transfer.is_open() {
        return Err(refused(english("property.transfers.not_requested", &[])));
    }
    let reference = reference.trim();
    if reference.is_empty() {
        return Err(refused(english("property.handover.receipt_blank", &[])));
    }
    let item = HandoverItem::find(&state.db, item_id)
        .await?
        .filter(|item| item.transfer_id == Some(id))
        .ok_or(Error::NotFound)?;
    HandoverItem::query()
        .where_(HandoverItem::ID.eq(item.id))
        .update(
            &state.db,
            &[
                ("receipt", &Some(reference.to_owned())),
                ("receipt_by", &Some(user.id)),
                ("receipt_on", &Some(today())),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    HandoverItem::find(&state.db, item.id)
        .await?
        .ok_or(Error::NotFound)
}

/// `user` rejects, or withdraws, the request `id`. Nothing changes.
///
/// # Errors
///
/// 403 without `agreements.manage`; 404; 422 unless it waits for a decision.
pub async fn reject(state: &AppState, user: &AuthUser, id: i64) -> Result<UnitTransfer> {
    require_manager(user)?;
    let transfer = UnitTransfer::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    let claimed = UnitTransfer::query()
        .where_(UnitTransfer::ID.eq(id))
        .where_(UnitTransfer::STATUS.eq(TransferStatus::Requested))
        .update(
            &state.db,
            &[
                ("status", &TransferStatus::Rejected),
                ("decided_by", &Some(user.id)),
                ("decided_on", &Some(today())),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    if claimed == 0 {
        return Err(refused(english("property.transfers.not_requested", &[])));
    }
    UnitTransfer::find(&state.db, transfer.id)
        .await?
        .ok_or(Error::NotFound)
}

/// What [`approve`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransferReport {
    /// The charges cancelled and recreated for the new owner.
    pub recreated: usize,
    /// The cheques relinked to the recreated charges.
    pub relinked: usize,
    /// The cheques returned as handed over.
    pub returned: usize,
    /// The deposits transferred.
    pub transferred: usize,
    /// The tenants told the new payee.
    pub told: usize,
    /// What could not be done and was left to the accountant.
    pub problems: Vec<String>,
}

/// The charges of the leases in force on the unit from `date`, cancelled and
/// recreated with `entity` as issuer. Returns the old and the new charge of
/// each. The issued invoices are not touched.
async fn recreate_charges(
    tx: &mut Transaction,
    unit_id: i64,
    date: NaiveDate,
    entity: Option<i64>,
) -> Result<Vec<(LeaseCharge, LeaseCharge)>> {
    let leases = Lease::query()
        .where_(Lease::UNIT_ID.eq(unit_id))
        .where_(Lease::STATUS.is_in(IN_FORCE))
        .order_by(Lease::ID)
        .get(&mut *tx)
        .await?;
    let mut pairs = Vec::new();
    for lease in leases {
        let old = LeaseCharge::query()
            .where_(LeaseCharge::LEASE_ID.eq(lease.id))
            .where_(LeaseCharge::STATUS.is_in([ChargeStatus::Planned, ChargeStatus::Scheduled]))
            .where_(LeaseCharge::DUE_DATE.gte(date))
            .order_by(LeaseCharge::POSITION)
            .order_by(LeaseCharge::ID)
            .get(&mut *tx)
            .await?;
        for charge in old {
            LeaseCharge::query()
                .where_(LeaseCharge::ID.eq(charge.id))
                .update(
                    &mut *tx,
                    &[
                        ("status", &ChargeStatus::Cancelled),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await?;
            let mut fresh = LeaseCharge {
                id: 0,
                status: charge.status,
                invoice_id: None,
                legal_entity_id: entity,
                created_at: None,
                updated_at: None,
                ..charge.clone()
            };
            fresh.insert(&mut *tx).await?;
            pairs.push((charge, fresh));
        }
    }
    Ok(pairs)
}

/// The leases in force on the unit.
async fn leases_in_force(db: &Db, unit_id: i64) -> Result<Vec<Lease>> {
    Lease::query()
        .where_(Lease::UNIT_ID.eq(unit_id))
        .where_(Lease::STATUS.is_in(IN_FORCE))
        .order_by(Lease::ID)
        .get(db)
        .await
}

/// `user` approves the request `id` and the transfer is carried out (see the
/// module documentation). `deposit_receipt` is the new owner's receipt for
/// the deposit that stays held; it is required for a sale when a deposit is
/// held.
///
/// # Errors
///
/// 403 without `agreements.manage`; 404; 422 unless the request waits for a
/// decision and `user` is not the requester, the agreement is still `active`
/// or `ending`, for a sale the new owner's agreement is `active`
/// ("new owner's agreement not active") and the receipt is given when a deposit
/// is held, and for "no longer managed" every line of the pack has its receipt
/// and no deposit waits for its settlement.
pub async fn approve(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    deposit_receipt: Option<&str>,
) -> Result<TransferReport> {
    require_manager(user)?;
    let transfer = UnitTransfer::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    if !transfer.is_open() {
        return Err(refused(english("property.transfers.not_requested", &[])));
    }
    if transfer.requested_by == Some(user.id) {
        return Err(refused(english("property.transfers.same_user", &[])));
    }
    let agreement = ManagementAgreement::find(&state.db, transfer.agreement_id)
        .await?
        .ok_or(Error::NotFound)?;
    if !AgreementStatus::REPORTING.contains(&agreement.status) {
        return Err(refused(english("property.transfers.state", &[])));
    }
    let date = transfer.transfer_date;
    let leases = leases_in_force(&state.db, transfer.unit_id).await?;
    let receipt = deposit_receipt
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_owned);

    let mut target: Option<ManagementAgreement> = None;
    let mut items: Vec<HandoverItem> = Vec::new();
    match transfer.kind {
        TransferKind::Sale => {
            let owner = transfer.new_owner_party_id.unwrap_or_default();
            target = Some(active_target(&state.db, owner, transfer.unit_id, date).await?);
            if receipt.is_none() {
                for lease in &leases {
                    if deposits(state)
                        .held_for(&state.db, &lease.agreement())
                        .await?
                        .is_some()
                    {
                        return Err(refused(english("property.transfers.receipt_required", &[])));
                    }
                }
            }
        }
        TransferKind::Unmanaged => {
            build_items(state, &agreement, &[transfer.unit_id], Some(transfer.id)).await?;
            items = HandoverItem::of_transfer(&state.db, transfer.id).await?;
            let missing = items.iter().filter(|item| !item.has_receipt()).count();
            if missing > 0 {
                return Err(refused(english(
                    "property.agreements.receipts_missing",
                    &[("count", &missing)],
                )));
            }
            for lease in &leases {
                if deposits(state)
                    .in_settlement(&state.db, &lease.agreement())
                    .await?
                {
                    return Err(refused(english("property.agreements.settlement_pending", &[])));
                }
            }
        }
    }

    // The transfer, in one transaction.
    let mut pairs: Vec<(LeaseCharge, LeaseCharge)> = Vec::new();
    let mut tx = state.db.begin().await?;
    let claimed = UnitTransfer::query()
        .where_(UnitTransfer::ID.eq(id))
        .where_(UnitTransfer::STATUS.eq(TransferStatus::Requested))
        .update(
            &mut tx,
            &[
                ("status", &TransferStatus::Approved),
                ("decided_by", &Some(user.id)),
                ("decided_on", &Some(today())),
                ("deposit_receipt", &receipt),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    if claimed == 0 {
        return Err(refused(english("property.transfers.not_requested", &[])));
    }
    AgreementUnit::query()
        .where_(AgreementUnit::AGREEMENT_ID.eq(agreement.id))
        .where_(AgreementUnit::UNIT_ID.eq(transfer.unit_id))
        .where_(AgreementUnit::DATE_OUT.is_null())
        .update(
            &mut tx,
            &[
                ("date_out", &Some(date - Duration::days(1))),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    match &target {
        Some(new) => {
            AgreementUnit::query()
                .where_(AgreementUnit::AGREEMENT_ID.eq(new.id))
                .where_(AgreementUnit::UNIT_ID.eq(transfer.unit_id))
                .where_(AgreementUnit::AWAITING_TRANSFER.eq(true))
                .update(
                    &mut tx,
                    &[
                        ("awaiting_transfer", &false),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await?;
            pairs = recreate_charges(&mut tx, transfer.unit_id, date, new.legal_entity_id).await?;
        }
        None => {
            hand_over_leases(&mut tx, &[transfer.unit_id], date).await?;
        }
    }
    tx.commit().await?;

    // What follows the commit.
    let mut report = TransferReport {
        recreated: pairs.len(),
        ..TransferReport::default()
    };
    match &target {
        Some(new) => {
            for lease in &leases {
                let held = cheques(state)
                    .of_agreement(&state.db, &lease.agreement())
                    .await?;
                for cheque in held {
                    if !matches!(
                        cheque.status,
                        ChequeStatus::Received | ChequeStatus::DueForDeposit
                    ) {
                        continue;
                    }
                    for (old, fresh) in &pairs {
                        let from: Subject = old.subject();
                        if !cheque.linked.contains(&from) {
                            continue;
                        }
                        match cheques(state)
                            .relink(state, cheque.id, from, fresh.subject())
                            .await
                        {
                            Ok(()) => report.relinked += 1,
                            Err(error) => report.problems.push(format!("cheque {}: {error}", cheque.number)),
                        }
                    }
                }
            }
            let told = leases_in_force(&state.db, transfer.unit_id).await?;
            report.told = tell_tenants(state, &told, PartyId(new.owner_party_id)).await?;
        }
        None => {
            let (returned, moved) = hand_over_items(
                state,
                user,
                &items,
                holder_of(&agreement),
                &mut report.problems,
            )
            .await;
            report.returned = returned.len();
            report.transferred = moved.len();
            let told = leases_in_force(&state.db, transfer.unit_id).await?;
            report.told = tell_tenants(state, &told, holder_of(&agreement)).await?;
        }
    }
    if !report.problems.is_empty() {
        let unit_label = Unit::find(&state.db, transfer.unit_id)
            .await?
            .map(|unit| unit.number)
            .unwrap_or_default();
        let title = english(
            "property.transfers.task_problems",
            &[("unit", &unit_label), ("agreement", &agreement.number())],
        );
        let task = RoleTask::new(
            agreement.subject(),
            ACCOUNTANT_ROLE,
            &title,
            &format!("property.transfer_problems:{}", transfer.id),
        )
        .description(&report.problems.join("\n"));
        crate::tasks::open(state, &renox::auth::permissions::Scope::global(), task).await?;
    }
    // The last unit left: the agreement stays, and a task suggests the notice.
    if open_unit_ids(&state.db, &agreement).await?.is_empty() {
        let title = english("property.transfers.task_empty", &[("agreement", &agreement.number())]);
        let task = RoleTask::new(
            agreement.subject(),
            PROPERTY_MANAGER_ROLE,
            &title,
            &empty_task_key(&agreement),
        );
        crate::tasks::open(state, &renox::auth::permissions::Scope::global(), task).await?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transfer_is_requested_first_and_open_until_decided() {
        let transfer = UnitTransfer::default();
        assert_eq!(transfer.status, TransferStatus::Requested);
        assert!(transfer.is_open());
        let decided = UnitTransfer {
            status: TransferStatus::Approved,
            ..Default::default()
        };
        assert!(!decided.is_open());
    }

    #[test]
    fn kinds_and_statuses_are_stored_in_snake_case() {
        assert_eq!(TransferKind::Unmanaged.as_str(), "unmanaged");
        assert_eq!(TransferKind::default(), TransferKind::Sale);
        assert_eq!(TransferStatus::Rejected.as_str(), "rejected");
    }

    #[test]
    fn the_empty_agreement_task_is_found_by_its_agreement() {
        let agreement = ManagementAgreement {
            id: 12,
            ..Default::default()
        };
        assert_eq!(empty_task_key(&agreement), "property.agreement_empty:12");
    }
}
