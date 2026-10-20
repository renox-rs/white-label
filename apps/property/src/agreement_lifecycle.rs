//! The lifecycle of a management agreement: `activate`, `give_notice` and
//! `end` with the handover pack (01 Property §4.4 row "The management
//! agreement ends, or a unit is sold mid-lease", §6, §7.1 *Management
//! agreement*; BL-PRP-22).
//!
//! | Transition | From | To | By |
//! |---|---|---|---|
//! | `activate` | draft | active | `agreements.manage` |
//! | `give_notice` | active | ending | `agreements.manage` |
//! | `end` | ending | ended | `agreements.manage` |
//!
//! **`activate`** needs the owner (a party), at least one unit, a fee and the
//! payout frequency, and the signed agreement attached: the transition is run
//! through the document guard `agreement.activate` ([`crate::documents`]), so
//! the `blocks` of the document kinds decide which document is needed. The
//! units' owner and issuing entity are read from the agreement that covers
//! them ([`ManagementAgreement::for_unit`]), from the day they come in, so
//! nothing else is written.
//!
//! **`give_notice`** records the notice date, the party who gave it (the owner
//! or the company), the handover date and the holder the handover goes to (the
//! owner when not given). After the commit the pack is started
//! ([`start_pack`]): a line per deposit held and per cheque not deposited of
//! the leases of its units, each waiting for the new holder's receipt
//! ([`record_receipt`]); the property managers and accountants are told.
//!
//! **The pack** ([`pack`]) also lists, live, the open invoices of the leases
//! and the Ejari numbers. No Ejari action is automated (`⚠ open: authority
//! AU-24`): the numbers are only recorded.
//!
//! **`end`** ([`end`]) is refused until every line has its receipt, no
//! deposit is `settlement_pending` (it cannot be transferred, BL-PAY-16) and
//! a final owner statement up to the handover date is `approved`, `published`
//! or `settled` ([`prepare_final_statement`] makes the draft by hand from
//! the agreement; publishing and the payout follow after `end`, like any
//! statement). In its transaction ([`end_effects`]): the units go out the day
//! before the handover date, the leases of the units are marked "handed over
//! on <date>" (they stay as they are otherwise, read-only history, and the
//! reminders on their invoices stop, [`crate::legal_case::LeaseHolds`]), and
//! their scheduled charges after the handover date are cancelled. Issued
//! invoices stay with the owner and are not credited. After the commit
//! ([`settle_end`]): the cheques not deposited are returned as *handed over*,
//! the deposits are transferred to the new holder against the receipts of the
//! pack, the tenants are told the new payee, and a step that fails becomes one
//! task for the accountant.
//!
//! Decided where the spec is silent:
//!
//! - Every cheque not deposited is returned, whatever its date: the pack lists
//!   them all and each has its receipt.
//! - A final statement that already covers the handover date (the regular
//!   monthly one) counts; [`prepare_final_statement`] makes the draft only for
//!   the days no statement covers.
//! - The permission of the transitions is the global `agreements.manage`;
//!   agreements are not tied to one building.

use renox::chrono::{Duration, NaiveDate};
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::cheques::{ChequeId, ChequeStatus, ReturnCheques, cheques};
use wl_kernel::deposits::{TransferDeposit, deposits};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceId, invoicer};
use wl_kernel::parties::directory;
use wl_kernel::role_tasks::RoleTask;
use wl_kernel::{PartyId, Subject};

use crate::agreement::{AgreementStatus, AgreementUnit, ManagementAgreement};
use crate::building::refused;
use crate::charge::{ChargeStatus, LeaseCharge};
use crate::documents;
use crate::lease::{LEASE_SUBJECT_TYPE, Lease, LeaseStatus};
use crate::notices;
use crate::permissions::{self, ACCOUNTANT_ROLE};
use crate::statement::{self, OwnerStatement, StatementStatus};
use crate::texts::english;

/// The `activate` transition's name.
pub const ACTIVATE: &str = "activate";
/// The `give_notice` transition's name.
pub const GIVE_NOTICE: &str = "give_notice";
/// The `end` transition's name.
pub const END: &str = "end";

/// The document kind of the signed management agreement that the `blocks`
/// of `agreement.activate` name (§7.2 *Document kinds*, seeded with the
/// other kinds of the vertical).
pub const SIGNED_AGREEMENT_KIND: &str = "management_agreement";

/// The reason a cheque is returned with when the agreement ends or a unit
/// leaves it.
pub const HANDED_OVER: &str = "handed over";

/// The states in which a lease is in force and is handed over with its unit.
pub const IN_FORCE: [LeaseStatus; 4] = [
    LeaseStatus::Active,
    LeaseStatus::RenewalDue,
    LeaseStatus::Expired,
    LeaseStatus::NoticeGiven,
];

/// Who gave the notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoticeParty {
    /// The landlord ends the contract.
    Owner,
    /// The management company ends it.
    Company,
}

impl NoticeParty {
    /// The text stored: `owner` or `company`.
    pub fn as_str(self) -> &'static str {
        match self {
            NoticeParty::Owner => "owner",
            NoticeParty::Company => "company",
        }
    }

    /// The party named by `text`, if it is one.
    pub fn parse(text: &str) -> Option<NoticeParty> {
        match text.trim() {
            "owner" => Some(NoticeParty::Owner),
            "company" => Some(NoticeParty::Company),
            _ => None,
        }
    }
}

/// What a line of the handover pack is.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ItemKind {
    /// A deposit held for a lease.
    #[default]
    Deposit,
    /// A cheque received and not deposited.
    Cheque,
}

/// One line of the handover pack: a deposit held or a cheque not deposited,
/// with the reference of the new holder's receipt once it is recorded.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "property_handover_items", index(agreement_id), index(unit_id))]
pub struct HandoverItem {
    /// The primary key.
    pub id: i64,
    /// The agreement.
    #[model(references = "property_management_agreements")]
    pub agreement_id: i64,
    /// The unit.
    pub unit_id: i64,
    /// The lease (the id of its version 1) the deposit and the cheque are held
    /// for.
    pub lease_root_id: i64,
    /// Deposit or cheque.
    pub kind: ItemKind,
    /// The deposit's or the cheque's id.
    pub item_ref: i64,
    /// What the line is, as people read it.
    pub description: String,
    /// The amount, in fils.
    pub amount: i64,
    /// The reference of the new holder's receipt.
    pub receipt: Option<String>,
    /// The user who recorded it.
    pub receipt_by: Option<i64>,
    /// The day it was recorded.
    pub receipt_on: Option<NaiveDate>,
    /// Handed over: the cheque returned, the deposit transferred.
    pub done: bool,
    /// The transfer of one unit this line belongs to (BL-PRP-23).
    pub transfer_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<renox::db::DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<renox::db::DateTime>,
}

impl HandoverItem {
    /// The lines of the pack of `agreement_id` that belong to no unit
    /// transfer, in the order they were made.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_agreement(db: &Db, agreement_id: i64) -> Result<Vec<HandoverItem>> {
        HandoverItem::query()
            .where_(HandoverItem::AGREEMENT_ID.eq(agreement_id))
            .where_(HandoverItem::TRANSFER_ID.is_null())
            .order_by(HandoverItem::ID)
            .get(db)
            .await
    }

    /// The lines of the transfer `transfer_id`.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_transfer(db: &Db, transfer_id: i64) -> Result<Vec<HandoverItem>> {
        HandoverItem::query()
            .where_(HandoverItem::TRANSFER_ID.eq(transfer_id))
            .order_by(HandoverItem::ID)
            .get(db)
            .await
    }

    /// Whether the new holder's receipt is recorded.
    pub fn has_receipt(&self) -> bool {
        self.receipt
            .as_deref()
            .is_some_and(|r| !r.trim().is_empty())
    }

    /// The lease as a kernel subject: what the deposit and the cheques are held for.
    pub fn lease_subject(&self) -> Subject {
        Subject::new(LEASE_SUBJECT_TYPE, self.lease_root_id)
    }
}

/// What a lifecycle run carries to its guards and effects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgreementInput {
    /// The notice date (`give_notice`).
    #[serde(default)]
    pub notice_on: Option<NaiveDate>,
    /// `owner` or `company` (`give_notice`).
    #[serde(default)]
    pub notice_party: Option<String>,
    /// The handover date (`give_notice`).
    #[serde(default)]
    pub handover_date: Option<NaiveDate>,
    /// The party the handover goes to, when it is not the owner.
    #[serde(default)]
    pub new_holder: Option<i64>,
    /// How many units the agreement lists (`activate`).
    #[serde(default)]
    pub units: u32,
    /// Whether the owner is a party that exists (`activate`).
    #[serde(default)]
    pub owner_ok: bool,
    /// How many lines of the pack lack their receipt (`end`).
    #[serde(default)]
    pub missing_receipts: u32,
    /// How many deposits are `settlement_pending` (`end`).
    #[serde(default)]
    pub settlement_pending: u32,
    /// Whether a final statement up to the handover date is approved (`end`).
    #[serde(default)]
    pub final_statement: bool,
    /// The day the run is for; today when empty.
    #[serde(default)]
    pub day: Option<NaiveDate>,
}

/// The input of a run.
fn input_of(ctx: &TransitionCtx) -> AgreementInput {
    ctx.input_as::<AgreementInput>().unwrap_or_default()
}

/// Why `agreement` cannot be activated with `input`, if it cannot: the owner,
/// the units, the fee and the payout frequency are set.
pub fn activate_problem(agreement: &ManagementAgreement, input: &AgreementInput) -> Option<String> {
    if !input.owner_ok {
        return Some(english("property.agreements.no_owner", &[]));
    }
    if input.units == 0 {
        return Some(english("property.agreements.no_units", &[]));
    }
    if (agreement.fee_bp <= 0 && agreement.fee_fixed <= 0)
        || agreement.payout_frequency.trim().is_empty()
    {
        return Some(english("property.agreements.fee_missing", &[]));
    }
    None
}

/// Why notice cannot be given with `input`, if it cannot: the notice date,
/// the party and the handover date are recorded, and the handover is not
/// before the notice.
pub fn notice_problem(input: &AgreementInput) -> Option<String> {
    let party = input.notice_party.as_deref().and_then(NoticeParty::parse);
    let (Some(notice), Some(handover), Some(_)) = (input.notice_on, input.handover_date, party)
    else {
        return Some(english("property.agreements.notice_missing", &[]));
    };
    if handover < notice {
        return Some(english("property.agreements.handover_early", &[]));
    }
    None
}

/// Why the agreement cannot end with `input`, if it cannot.
pub fn end_problem(agreement: &ManagementAgreement, input: &AgreementInput) -> Option<String> {
    if agreement.handover_date.is_none() {
        return Some(english("property.agreements.notice_missing", &[]));
    }
    if input.missing_receipts > 0 {
        return Some(english(
            "property.agreements.receipts_missing",
            &[("count", &input.missing_receipts)],
        ));
    }
    if input.settlement_pending > 0 {
        return Some(english("property.agreements.settlement_pending", &[]));
    }
    if !input.final_statement {
        return Some(english("property.agreements.final_statement", &[]));
    }
    None
}

/// The `activate` transition.
pub(crate) fn activate_transition() -> Transition<ManagementAgreement> {
    Transition::new(ACTIVATE)
        .label("Activate")
        .tone("primary")
        .permission(permissions::AGREEMENTS_MANAGE)
        .from([AgreementStatus::Draft])
        .to(AgreementStatus::Active)
        .guard("ready", |agreement: &ManagementAgreement, ctx| {
            let problem = activate_problem(agreement, &input_of(ctx));
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
}

/// The `give_notice` transition.
pub(crate) fn give_notice_transition() -> Transition<ManagementAgreement> {
    Transition::new(GIVE_NOTICE)
        .label("Give notice")
        .tone("warning")
        .permission(permissions::AGREEMENTS_MANAGE)
        .from([AgreementStatus::Active])
        .to(AgreementStatus::Ending)
        .guard(
            "notice_recorded",
            |_agreement: &ManagementAgreement, ctx| {
                let problem = notice_problem(&input_of(ctx));
                Box::pin(async move { problem.map_or(Ok(()), Err) })
            },
        )
        .effect(|agreement: &ManagementAgreement, ctx, tx| {
            let id = agreement.id;
            let input = input_of(ctx);
            let by = ctx.actor().map(|actor| actor.id);
            Box::pin(async move {
                ManagementAgreement::query()
                    .where_(ManagementAgreement::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("notice_on", &input.notice_on),
                            ("notice_party", &input.notice_party),
                            ("notice_by", &by),
                            ("handover_date", &input.handover_date),
                            ("handover_to_party_id", &input.new_holder),
                            ("updated_at", &renox::db::now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// The `end` transition.
pub(crate) fn end_transition() -> Transition<ManagementAgreement> {
    Transition::new(END)
        .label("End")
        .tone("danger")
        .permission(permissions::AGREEMENTS_MANAGE)
        .from([AgreementStatus::Ending])
        .to(AgreementStatus::Ended)
        .guard(
            "handover_complete",
            |agreement: &ManagementAgreement, ctx| {
                let problem = end_problem(agreement, &input_of(ctx));
                Box::pin(async move { problem.map_or(Ok(()), Err) })
            },
        )
        .effect(|agreement: &ManagementAgreement, ctx, tx| {
            let agreement = agreement.clone();
            let day = input_of(ctx).day.unwrap_or_else(today);
            Box::pin(async move { end_effects(&mut *tx, &agreement, day).await })
        })
}

/// The effect of `end` inside its transaction: the day it ended, the units
/// out, the leases marked and their later charges cancelled.
pub(crate) async fn end_effects(
    tx: &mut Transaction,
    agreement: &ManagementAgreement,
    day: NaiveDate,
) -> Result {
    let Some(handover) = agreement.handover_date else {
        return Err(refused(english("property.agreements.notice_missing", &[])));
    };
    ManagementAgreement::query()
        .where_(ManagementAgreement::ID.eq(agreement.id))
        .update(
            &mut *tx,
            &[("ended_on", &Some(day)), ("updated_at", &renox::db::now())],
        )
        .await?;
    let rows = AgreementUnit::query()
        .where_(AgreementUnit::AGREEMENT_ID.eq(agreement.id))
        .where_(AgreementUnit::DATE_OUT.is_null())
        .get(&mut *tx)
        .await?;
    let unit_ids: Vec<i64> = rows.iter().map(|row| row.unit_id).collect();
    AgreementUnit::query()
        .where_(AgreementUnit::AGREEMENT_ID.eq(agreement.id))
        .where_(AgreementUnit::DATE_OUT.is_null())
        .update(
            &mut *tx,
            &[
                ("date_out", &Some(handover - Duration::days(1))),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    hand_over_leases(&mut *tx, &unit_ids, handover).await
}

/// Marks the leases in force on `unit_ids` "handed over on `handover`" and
/// cancels their scheduled charges after that day. The issued invoices are
/// not touched.
pub(crate) async fn hand_over_leases(
    tx: &mut Transaction,
    unit_ids: &[i64],
    handover: NaiveDate,
) -> Result {
    if unit_ids.is_empty() {
        return Ok(());
    }
    let leases = Lease::query()
        .where_(Lease::UNIT_ID.is_in(unit_ids.to_vec()))
        .where_(Lease::STATUS.is_in(IN_FORCE))
        .get(&mut *tx)
        .await?;
    if leases.is_empty() {
        return Ok(());
    }
    let roots: Vec<i64> = leases.iter().map(|lease| lease.root_id).collect();
    let versions: Vec<i64> = leases.iter().map(|lease| lease.id).collect();
    Lease::query()
        .where_(Lease::ROOT_ID.is_in(roots))
        .update(
            &mut *tx,
            &[
                ("handed_over_on", &Some(handover)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    LeaseCharge::query()
        .where_(LeaseCharge::LEASE_ID.is_in(versions))
        .where_(LeaseCharge::STATUS.is_in([ChargeStatus::Planned, ChargeStatus::Scheduled]))
        .where_(LeaseCharge::DUE_DATE.gt(handover))
        .update(
            &mut *tx,
            &[
                ("status", &ChargeStatus::Cancelled),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// The agreement `id`, or a 404.
async fn load(
    state: &AppState,
    id: i64,
) -> std::result::Result<ManagementAgreement, TransitionError> {
    ManagementAgreement::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))
}

/// `user` activates the draft agreement `id`.
///
/// # Errors
///
/// `Forbidden` without `agreements.manage`, `NotFrom` unless `draft`, `Guard`
/// (422) without an owner that exists, a unit, a fee and the payout
/// frequency, or without the signed agreement attached.
pub async fn activate(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<ManagementAgreement, TransitionError> {
    let agreement = load(state, id).await?;
    let units = agreement.units(&state.db).await?;
    let owner_ok = directory(state)
        .find(&state.db, agreement.owner())
        .await?
        .is_some();
    let input = AgreementInput {
        units: u32::try_from(units.len()).unwrap_or(u32::MAX),
        owner_ok,
        ..AgreementInput::default()
    };
    let _outcome = documents::guarded(
        state,
        documents::AGREEMENT_ACTIVATE,
        id,
        Some(user.id),
        || async {
            agreement
                .transition(ACTIVATE)
                .by(user)
                .input(&input)
                .run(state)
                .await
        },
    )
    .await?;
    load(state, id).await
}

/// What a user enters to give notice.
#[derive(Debug, Clone)]
pub struct Notice {
    /// The day the notice was given.
    pub on: NaiveDate,
    /// Who gave it.
    pub party: NoticeParty,
    /// The day the units are no longer managed.
    pub handover: NaiveDate,
    /// The party the handover goes to; the owner when empty.
    pub new_holder: Option<PartyId>,
}

/// `user` records the notice on the active agreement `id`: it becomes
/// `ending`, the pack is started and the property managers and accountants
/// are told.
///
/// # Errors
///
/// `Forbidden` without `agreements.manage`, `NotFrom` unless `active`,
/// `Guard` (422) for a handover before the notice or a new holder that is not
/// a party.
pub async fn give_notice(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    notice: Notice,
) -> std::result::Result<ManagementAgreement, TransitionError> {
    let agreement = load(state, id).await?;
    if let Some(holder) = notice.new_holder
        && directory(state).find(&state.db, holder).await?.is_none()
    {
        return Err(TransitionError::Guard(english(
            "property.agreements.holder_unknown",
            &[],
        )));
    }
    let input = AgreementInput {
        notice_on: Some(notice.on),
        notice_party: Some(notice.party.as_str().to_owned()),
        handover_date: Some(notice.handover),
        new_holder: notice.new_holder.map(|holder| holder.0),
        ..AgreementInput::default()
    };
    let _outcome = agreement
        .transition(GIVE_NOTICE)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    let agreement = load(state, id).await?;
    start_pack(state, &agreement)
        .await
        .map_err(TransitionError::Failed)?;
    notices::agreement_notice(state, &agreement)
        .await
        .map_err(TransitionError::Failed)?;
    Ok(agreement)
}

/// The leases in force on `unit_ids`, oldest first.
async fn leases_on(db: &Db, unit_ids: &[i64]) -> Result<Vec<Lease>> {
    if unit_ids.is_empty() {
        return Ok(Vec::new());
    }
    Lease::query()
        .where_(Lease::UNIT_ID.is_in(unit_ids.to_vec()))
        .where_(Lease::STATUS.is_in(IN_FORCE))
        .order_by(Lease::ID)
        .get(db)
        .await
}

/// The units still under `agreement`: they have no day out.
pub(crate) async fn open_unit_ids(db: &Db, agreement: &ManagementAgreement) -> Result<Vec<i64>> {
    Ok(agreement
        .units(db)
        .await?
        .iter()
        .filter(|row| row.date_out.is_none())
        .map(|row| row.unit_id)
        .collect())
}

/// Makes the lines of the pack for the deposits held and the cheques not
/// deposited of the leases in force on `unit_ids` (all the open units of the
/// agreement when `transfer_id` is empty). A line made before is not made
/// twice. Returns how many lines were added.
///
/// # Errors
///
/// The database's and the contracts' errors.
pub(crate) async fn build_items(
    state: &AppState,
    agreement: &ManagementAgreement,
    unit_ids: &[i64],
    transfer_id: Option<i64>,
) -> Result<usize> {
    let known = HandoverItem::query()
        .where_(HandoverItem::AGREEMENT_ID.eq(agreement.id))
        .get(&state.db)
        .await?;
    let has = |kind: ItemKind, item_ref: i64| {
        known.iter().any(|item| {
            item.kind == kind && item.item_ref == item_ref && item.transfer_id == transfer_id
        })
    };
    let mut added = 0;
    for lease in leases_on(&state.db, unit_ids).await? {
        let source = lease.agreement();
        if let Some(held) = deposits(state).held_for(&state.db, &source).await?
            && !has(ItemKind::Deposit, held.deposit.0)
        {
            let description = english(
                "property.handover.deposit_line",
                &[("lease", &lease.number())],
            );
            HandoverItem::create(
                &state.db,
                HandoverItem {
                    agreement_id: agreement.id,
                    unit_id: lease.unit_id,
                    lease_root_id: lease.root_id,
                    kind: ItemKind::Deposit,
                    item_ref: held.deposit.0,
                    description,
                    amount: held.held,
                    transfer_id,
                    ..Default::default()
                },
            )
            .await?;
            added += 1;
        }
        for cheque in cheques(state).of_agreement(&state.db, &source).await? {
            if !matches!(
                cheque.status,
                ChequeStatus::Received | ChequeStatus::DueForDeposit
            ) || has(ItemKind::Cheque, cheque.id.0)
            {
                continue;
            }
            let description = english(
                "property.handover.cheque_line",
                &[
                    ("lease", &lease.number()),
                    ("number", &cheque.number),
                    ("date", &cheque.date),
                ],
            );
            HandoverItem::create(
                &state.db,
                HandoverItem {
                    agreement_id: agreement.id,
                    unit_id: lease.unit_id,
                    lease_root_id: lease.root_id,
                    kind: ItemKind::Cheque,
                    item_ref: cheque.id.0,
                    description,
                    amount: cheque.amount,
                    transfer_id,
                    ..Default::default()
                },
            )
            .await?;
            added += 1;
        }
    }
    Ok(added)
}

/// Starts the handover pack of `agreement`: a line per deposit and per
/// cheque of its open units. Safe to repeat.
///
/// # Errors
///
/// The database's and the contracts' errors.
pub async fn start_pack(state: &AppState, agreement: &ManagementAgreement) -> Result<usize> {
    let units = open_unit_ids(&state.db, agreement).await?;
    build_items(state, agreement, &units, None).await
}

/// `user` records the reference of the new holder's receipt for the line
/// `item_id` of the pack of the ending agreement `id`.
///
/// # Errors
///
/// 403 without `agreements.manage`; 404 for a line that is not on the pack;
/// 422 unless the agreement is `ending`, or for a blank reference.
pub async fn record_receipt(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    item_id: i64,
    reference: &str,
) -> Result<HandoverItem> {
    if !user.has_permission(permissions::AGREEMENTS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let agreement = ManagementAgreement::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    if agreement.status != AgreementStatus::Ending {
        return Err(refused(english("property.handover.not_ending", &[])));
    }
    let reference = reference.trim();
    if reference.is_empty() {
        return Err(refused(english("property.handover.receipt_blank", &[])));
    }
    let item = HandoverItem::find(&state.db, item_id)
        .await?
        .filter(|item| item.agreement_id == id && item.transfer_id.is_none())
        .ok_or(Error::NotFound)?;
    if item.done {
        return Err(refused(english("property.handover.done_already", &[])));
    }
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

/// Whether a statement up to the handover date is approved (or further): it
/// ends on or after the handover date and is `approved`, `published` or
/// `settled`.
///
/// # Errors
///
/// The database's errors.
pub async fn final_statement_ready(db: &Db, agreement: &ManagementAgreement) -> Result<bool> {
    let Some(handover) = agreement.handover_date else {
        return Ok(false);
    };
    OwnerStatement::query()
        .where_(OwnerStatement::AGREEMENT_ID.eq(agreement.id))
        .where_(OwnerStatement::PERIOD_END.gte(handover))
        .where_(OwnerStatement::STATUS.is_in([
            StatementStatus::Approved,
            StatementStatus::Published,
            StatementStatus::Settled,
        ]))
        .exists(db)
        .await
}

/// `user` prepares the final owner statement of the ending agreement `id` by
/// hand: the draft from the day after the latest statement (the agreement's
/// first day when it has none) up to the handover date. The user becomes its
/// preparer, so another user approves it (`PRP.ACL.08`). `None` when a
/// statement for that period exists already.
///
/// # Errors
///
/// 403 without `owner_statements.prepare`; 422 unless the agreement is
/// `ending`, or when a statement covers the handover date already.
pub async fn prepare_final_statement(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> Result<Option<OwnerStatement>> {
    if !user.has_permission(permissions::OWNER_STATEMENTS_PREPARE) {
        return Err(Error::Forbidden);
    }
    let agreement = ManagementAgreement::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    let (AgreementStatus::Ending, Some(handover)) = (agreement.status, agreement.handover_date)
    else {
        return Err(refused(english("property.handover.not_ending", &[])));
    };
    let latest = OwnerStatement::query()
        .where_(OwnerStatement::AGREEMENT_ID.eq(id))
        .where_(OwnerStatement::STATUS.ne(StatementStatus::Superseded))
        .order_by_desc(OwnerStatement::PERIOD_END)
        .first(&state.db)
        .await?;
    let start = latest.map_or(agreement.start_date, |last| {
        last.period_end + Duration::days(1)
    });
    if start > handover {
        return Err(refused(english("property.handover.statement_covers", &[])));
    }
    let Some(draft) = statement::generate(state, id, start, handover).await? else {
        return Ok(None);
    };
    OwnerStatement::query()
        .where_(OwnerStatement::ID.eq(draft.id))
        .update(
            &state.db,
            &[
                ("prepared_by", &Some(user.id)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(OwnerStatement::find(&state.db, draft.id).await?)
}

/// An open invoice of a lease in the pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackInvoice {
    /// The lease number.
    pub lease: String,
    /// The invoice.
    pub invoice: i64,
    /// Its due date.
    pub due: NaiveDate,
    /// What is still owed, in fils.
    pub balance: i64,
}

/// The Ejari registration of a lease in the pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackEjari {
    /// The lease number.
    pub lease: String,
    /// The registration number.
    pub number: String,
}

/// The handover pack of an agreement (§4.4): the deposits held and the
/// cheques not deposited with their receipts, the open invoices and arrears
/// per lease, and the Ejari numbers.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Pack {
    /// The deposits held.
    pub deposits: Vec<HandoverItem>,
    /// The cheques not deposited.
    pub cheques: Vec<HandoverItem>,
    /// The invoices with a balance.
    pub invoices: Vec<PackInvoice>,
    /// The Ejari numbers.
    pub ejari: Vec<PackEjari>,
}

impl Pack {
    /// How many lines still wait for their receipt.
    pub fn missing_receipts(&self) -> usize {
        self.deposits
            .iter()
            .chain(self.cheques.iter())
            .filter(|item| !item.has_receipt())
            .count()
    }

    /// What the deposits add up to, in fils.
    pub fn deposit_total(&self) -> i64 {
        self.deposits.iter().map(|item| item.amount).sum()
    }
}

/// The handover pack of `agreement`.
///
/// # Errors
///
/// The database's and the invoicer's errors.
pub async fn pack(state: &AppState, agreement: &ManagementAgreement) -> Result<Pack> {
    let items = HandoverItem::of_agreement(&state.db, agreement.id).await?;
    let mut pack = Pack::default();
    for item in items {
        match item.kind {
            ItemKind::Deposit => pack.deposits.push(item),
            ItemKind::Cheque => pack.cheques.push(item),
        }
    }
    let units: Vec<i64> = agreement
        .units(&state.db)
        .await?
        .iter()
        .map(|row| row.unit_id)
        .collect();
    for lease in leases_on(&state.db, &units).await? {
        if let Some(number) = lease.ejari_number.clone() {
            pack.ejari.push(PackEjari {
                lease: lease.number(),
                number,
            });
        }
        for charge in lease.charges(&state.db).await? {
            let (ChargeStatus::Invoiced, Some(invoice)) = (charge.status, charge.invoice_id) else {
                continue;
            };
            let Some(found) = invoicer(state).find(&state.db, InvoiceId(invoice)).await? else {
                continue;
            };
            if found.balance > 0 {
                pack.invoices.push(PackInvoice {
                    lease: lease.number(),
                    invoice,
                    due: charge.due_date,
                    balance: found.balance,
                });
            }
        }
    }
    Ok(pack)
}

/// What [`end`] did after the commit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EndReport {
    /// The cheques returned as handed over.
    pub returned: Vec<ChequeId>,
    /// The deposits transferred (their ids).
    pub transferred: Vec<i64>,
    /// The tenants told the new payee.
    pub told: usize,
    /// What could not be done and was left to the accountant.
    pub problems: Vec<String>,
}

/// `user` ends the agreement `id`: the pack is complete, no deposit waits for
/// its settlement and the final statement is approved. See the module
/// documentation for what follows.
///
/// # Errors
///
/// `Forbidden` without `agreements.manage`, `NotFrom` unless `ending`,
/// `Guard` (422) naming what is missing.
pub async fn end(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<EndReport, TransitionError> {
    let agreement = load(state, id).await?;
    if agreement.status == AgreementStatus::Ending {
        start_pack(state, &agreement)
            .await
            .map_err(TransitionError::Failed)?;
    }
    let items = HandoverItem::of_agreement(&state.db, id).await?;
    let missing = items.iter().filter(|item| !item.has_receipt()).count();
    let mut settling = 0;
    for item in items.iter().filter(|item| item.kind == ItemKind::Deposit) {
        if deposits(state)
            .in_settlement(&state.db, &item.lease_subject())
            .await?
        {
            settling += 1;
        }
    }
    // A lease whose deposit left the book for its settlement has no line:
    // look at the leases of the units as well.
    let units = open_unit_ids(&state.db, &agreement).await?;
    for lease in leases_on(&state.db, &units).await? {
        let listed = items
            .iter()
            .any(|item| item.kind == ItemKind::Deposit && item.lease_root_id == lease.root_id);
        if !listed
            && deposits(state)
                .in_settlement(&state.db, &lease.agreement())
                .await?
        {
            settling += 1;
        }
    }
    let input = AgreementInput {
        missing_receipts: u32::try_from(missing).unwrap_or(u32::MAX),
        settlement_pending: settling,
        final_statement: final_statement_ready(&state.db, &agreement)
            .await
            .map_err(TransitionError::Failed)?,
        day: Some(today()),
        ..AgreementInput::default()
    };
    let _outcome = agreement
        .transition(END)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    let ended = load(state, id).await?;
    settle_end(state, user, &ended, &units)
        .await
        .map_err(TransitionError::Failed)
}

/// The holder the handover of `agreement` goes to: the party named at the
/// notice, else the owner.
pub fn holder_of(agreement: &ManagementAgreement) -> PartyId {
    PartyId(
        agreement
            .handover_to_party_id
            .unwrap_or(agreement.owner_party_id),
    )
}

/// Hands over what the pack lists after `end` committed: the cheques are
/// returned, the deposits transferred, the tenants told. A step that fails
/// does not stop the others; the accountant gets one task that lists them.
async fn settle_end(
    state: &AppState,
    user: &AuthUser,
    agreement: &ManagementAgreement,
    units: &[i64],
) -> Result<EndReport> {
    let mut report = EndReport::default();
    let items: Vec<HandoverItem> = HandoverItem::of_agreement(&state.db, agreement.id)
        .await?
        .into_iter()
        .filter(|item| !item.done)
        .collect();
    let (returned, transferred) = hand_over_items(
        state,
        user,
        &items,
        holder_of(agreement),
        &mut report.problems,
    )
    .await;
    report.returned = returned;
    report.transferred = transferred;
    let leases = leases_on(&state.db, units).await?;
    report.told = tell_tenants(state, &leases, holder_of(agreement)).await?;
    if !report.problems.is_empty() {
        let title = english(
            "property.handover.task_problems",
            &[("agreement", &agreement.number())],
        );
        let task = RoleTask::new(
            agreement.subject(),
            ACCOUNTANT_ROLE,
            &title,
            &format!("property.handover_problems:{}", agreement.id),
        )
        .description(&report.problems.join("\n"));
        crate::tasks::open(state, &renox::auth::permissions::Scope::global(), task).await?;
    }
    Ok(report)
}

/// Returns the cheques of `items` as handed over and transfers their
/// deposits to `holder`, and marks the lines done. Returns the cheques
/// returned and the deposits transferred; a failure is added to `problems`.
pub(crate) async fn hand_over_items(
    state: &AppState,
    user: &AuthUser,
    items: &[HandoverItem],
    holder: PartyId,
    problems: &mut Vec<String>,
) -> (Vec<ChequeId>, Vec<i64>) {
    let mut returned: Vec<ChequeId> = Vec::new();
    let mut transferred: Vec<i64> = Vec::new();
    let cheque_ids: Vec<ChequeId> = items
        .iter()
        .filter(|item| item.kind == ItemKind::Cheque)
        .map(|item| ChequeId(item.item_ref))
        .collect();
    if !cheque_ids.is_empty() {
        let reason = english("property.handover.return_reason", &[]);
        match cheques(state)
            .return_undeposited(
                state,
                ReturnCheques::new(cheque_ids, &reason),
                Some(user.id),
            )
            .await
        {
            Ok(Some(back)) => returned = back.returned,
            Ok(None) => {}
            Err(error) => problems.push(format!("cheques: {error}")),
        }
    }
    let mut done: Vec<i64> = items
        .iter()
        .filter(|item| item.kind == ItemKind::Cheque && returned.contains(&ChequeId(item.item_ref)))
        .map(|item| item.id)
        .collect();
    for item in items.iter().filter(|item| item.kind == ItemKind::Deposit) {
        let receipt = item.receipt.clone().unwrap_or_default();
        let request = TransferDeposit::new(item.lease_subject(), holder, &receipt);
        match deposits(state).transfer(state, user, request).await {
            Ok(Some(moved)) => {
                transferred.push(moved.deposit.0);
                done.push(item.id);
            }
            Ok(None) => {}
            Err(error) => problems.push(format!("{}: {error}", item.description)),
        }
    }
    if !done.is_empty()
        && let Err(error) = HandoverItem::query()
            .where_(HandoverItem::ID.is_in(done))
            .update(
                &state.db,
                &[("done", &true), ("updated_at", &renox::db::now())],
            )
            .await
    {
        problems.push(format!("pack: {error}"));
    }
    (returned, transferred)
}

/// Tells the tenant of each of `leases` who the new payee is. Returns how
/// many were told.
pub(crate) async fn tell_tenants(
    state: &AppState,
    leases: &[Lease],
    holder: PartyId,
) -> Result<usize> {
    let payee = directory(state)
        .display_name(&state.db, holder, "en")
        .await?
        .unwrap_or_else(|| format!("#{}", holder.0));
    let mut told = 0;
    for lease in leases {
        notices::payee_changed(state, lease, &payee).await?;
        told += 1;
    }
    Ok(told)
}

/// Registers nothing at run time; the machine is declared in
/// [`crate::agreement`], and the document guard in [`crate::documents`].
#[cfg(test)]
mod tests {
    use super::*;

    fn agreement() -> ManagementAgreement {
        ManagementAgreement {
            fee_bp: 500,
            payout_frequency: "monthly".to_owned(),
            handover_date: NaiveDate::from_ymd_opt(2027, 6, 30),
            ..Default::default()
        }
    }

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2027, m, d).unwrap()
    }

    #[test]
    fn an_agreement_needs_an_owner_units_and_a_fee_to_be_activated() {
        let ready = AgreementInput {
            units: 3,
            owner_ok: true,
            ..Default::default()
        };
        assert!(activate_problem(&agreement(), &ready).is_none());
        let no_owner = AgreementInput {
            owner_ok: false,
            ..ready.clone()
        };
        assert!(activate_problem(&agreement(), &no_owner).is_some());
        let no_units = AgreementInput {
            units: 0,
            ..ready.clone()
        };
        assert!(activate_problem(&agreement(), &no_units).is_some());
        let free = ManagementAgreement {
            fee_bp: 0,
            fee_fixed: 0,
            ..agreement()
        };
        assert!(activate_problem(&free, &ready).is_some());
        let fixed = ManagementAgreement {
            fee_bp: 0,
            fee_fixed: 50_000,
            ..agreement()
        };
        assert!(activate_problem(&fixed, &ready).is_none());
    }

    #[test]
    fn a_notice_needs_its_date_party_and_handover() {
        let full = AgreementInput {
            notice_on: Some(day(3, 1)),
            notice_party: Some("owner".to_owned()),
            handover_date: Some(day(6, 30)),
            ..Default::default()
        };
        assert!(notice_problem(&full).is_none());
        for broken in [
            AgreementInput {
                notice_on: None,
                ..full.clone()
            },
            AgreementInput {
                notice_party: Some("tenant".to_owned()),
                ..full.clone()
            },
            AgreementInput {
                handover_date: None,
                ..full.clone()
            },
            AgreementInput {
                handover_date: Some(day(2, 1)),
                ..full.clone()
            },
        ] {
            assert!(notice_problem(&broken).is_some());
        }
    }

    #[test]
    fn the_end_waits_for_receipts_settlements_and_the_final_statement() {
        let ok = AgreementInput {
            final_statement: true,
            ..Default::default()
        };
        assert!(end_problem(&agreement(), &ok).is_none());
        let receipts = AgreementInput {
            missing_receipts: 1,
            ..ok.clone()
        };
        assert!(end_problem(&agreement(), &receipts).is_some());
        let settling = AgreementInput {
            settlement_pending: 1,
            ..ok.clone()
        };
        assert!(end_problem(&agreement(), &settling).is_some());
        let draft = AgreementInput {
            final_statement: false,
            ..ok.clone()
        };
        assert!(end_problem(&agreement(), &draft).is_some());
        let undated = ManagementAgreement {
            handover_date: None,
            ..agreement()
        };
        assert!(end_problem(&undated, &ok).is_some());
    }

    #[test]
    fn the_holder_is_the_owner_unless_a_new_manager_is_named() {
        let own = ManagementAgreement {
            owner_party_id: 31,
            ..Default::default()
        };
        assert_eq!(holder_of(&own), PartyId(31));
        let named = ManagementAgreement {
            owner_party_id: 31,
            handover_to_party_id: Some(40),
            ..Default::default()
        };
        assert_eq!(holder_of(&named), PartyId(40));
    }

    #[test]
    fn the_party_of_a_notice_has_a_name() {
        assert_eq!(NoticeParty::parse("owner"), Some(NoticeParty::Owner));
        assert_eq!(NoticeParty::parse(" company "), Some(NoticeParty::Company));
        assert_eq!(NoticeParty::parse("tenant"), None);
        assert_eq!(NoticeParty::Owner.as_str(), "owner");
    }

    #[test]
    fn a_line_without_a_receipt_is_missing_one() {
        let mut item = HandoverItem::default();
        assert!(!item.has_receipt());
        item.receipt = Some("  ".to_owned());
        assert!(!item.has_receipt());
        item.receipt = Some("RCPT-1".to_owned());
        assert!(item.has_receipt());
    }
}
