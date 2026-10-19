//! The lease services: drafting a lease, a renewal or an amendment, editing a
//! draft, submitting it for signature, activating it, and recording its
//! Ejari registration (`PRP.LSE.01`, `PRP.LSE.02`, 01 Property §4.1, §7.1).
//!
//! State changes go through the Lease machine
//! ([`crate::lease_machine`]); this module adds what the machine cannot see:
//! the kernel's contracts (the tenant, the documents, the deposit) and the
//! rows of a new version. Money is `i64` fils, dates are calendar dates in
//! Asia/Dubai.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::PartyId;
use wl_kernel::deposits::deposits;
use wl_kernel::parties::directory;
use wl_kernel::tickets::task_creator;

use crate::building::{clean, refused};
use crate::charge::{ChargeStatus, LeaseCharge, RENT, rent_description, rent_plan};
use crate::documents::{self, ACTIVATE as GUARD_ACTIVATE, SUBMIT as GUARD_SUBMIT};
use crate::lease::{Lease, LeaseStatus, VersionKind};
use crate::lease_machine::{ACTIVATE, SUBMIT};
use crate::permissions;
use crate::texts::english;
use crate::unit::Unit;

/// The most rent instalments a year the plan has (§2: 1 to 12 cheques).
pub const MAX_INSTALMENTS: u32 = 12;

/// What a user enters for a new lease.
#[derive(Debug, Clone)]
pub struct NewLease {
    /// The unit.
    pub unit_id: i64,
    /// The billed tenant.
    pub tenant: PartyId,
    /// The first day of the term.
    pub start_date: NaiveDate,
    /// The last day of the term.
    pub end_date: NaiveDate,
    /// The annual rent, in fils.
    pub annual_rent: i64,
    /// The security deposit, in fils.
    pub deposit_amount: i64,
    /// How many rent instalments: 1 to 12.
    pub instalments: u32,
    /// The due date of the first instalment; the start date when `None`.
    pub first_due: Option<NaiveDate>,
    /// The contract's own notice period in days, when not the legal 90.
    pub contract_notice_days: Option<i32>,
    /// The clauses.
    pub terms: Option<String>,
}

impl NewLease {
    /// A lease of `unit_id` for `tenant` from `start_date` to `end_date` at
    /// `annual_rent` fils, paid in `instalments`, with no deposit.
    pub fn new(
        unit_id: i64,
        tenant: PartyId,
        start_date: NaiveDate,
        end_date: NaiveDate,
        annual_rent: i64,
        instalments: u32,
    ) -> Self {
        NewLease {
            unit_id,
            tenant,
            start_date,
            end_date,
            annual_rent,
            deposit_amount: 0,
            instalments,
            first_due: None,
            contract_notice_days: None,
            terms: None,
        }
    }

    /// With this security deposit, in fils.
    pub fn deposit(mut self, fils: i64) -> Self {
        self.deposit_amount = fils;
        self
    }

    /// With the first instalment due on `date`.
    pub fn first_due(mut self, date: NaiveDate) -> Self {
        self.first_due = Some(date);
        self
    }
}

/// What a user enters for a renewal or an amendment of a lease.
#[derive(Debug, Clone)]
pub struct NewVersion {
    /// [`VersionKind::Renewal`] or [`VersionKind::Amendment`].
    pub kind: VersionKind,
    /// The first day of the term of the version.
    pub start_date: NaiveDate,
    /// The last day of the term.
    pub end_date: NaiveDate,
    /// The annual rent, in fils.
    pub annual_rent: i64,
    /// The security deposit, in fils.
    pub deposit_amount: i64,
    /// How many rent instalments: 1 to 12.
    pub instalments: u32,
    /// The due date of the first instalment: the start date of a renewal, the
    /// effective date of an amendment, when `None`.
    pub first_due: Option<NaiveDate>,
    /// The day an amendment applies from; required for an amendment.
    pub effective_date: Option<NaiveDate>,
    /// The clauses; the previous version's when `None`.
    pub terms: Option<String>,
}

impl NewVersion {
    /// A renewal for the term `start_date` to `end_date` at `annual_rent`.
    pub fn renewal(
        start_date: NaiveDate,
        end_date: NaiveDate,
        annual_rent: i64,
        deposit_amount: i64,
        instalments: u32,
    ) -> Self {
        NewVersion {
            kind: VersionKind::Renewal,
            start_date,
            end_date,
            annual_rent,
            deposit_amount,
            instalments,
            first_due: None,
            effective_date: None,
            terms: None,
        }
    }

    /// An amendment that applies from `effective_date` and runs to
    /// `end_date`.
    pub fn amendment(
        effective_date: NaiveDate,
        end_date: NaiveDate,
        annual_rent: i64,
        deposit_amount: i64,
        instalments: u32,
    ) -> Self {
        NewVersion {
            kind: VersionKind::Amendment,
            start_date: effective_date,
            end_date,
            annual_rent,
            deposit_amount,
            instalments,
            first_due: None,
            effective_date: Some(effective_date),
            terms: None,
        }
    }
}

/// The terms of a lease that are checked the same way for a new lease, a
/// version and an edit.
fn check_terms(
    start: NaiveDate,
    end: NaiveDate,
    rent: i64,
    deposit: i64,
    instalments: u32,
) -> Result {
    if end <= start {
        return Err(refused(english("property.leases.dates", &[])));
    }
    if rent <= 0 {
        return Err(refused(english("property.leases.rent_required", &[])));
    }
    if deposit < 0 {
        return Err(refused(english("property.leases.deposit_negative", &[])));
    }
    if instalments == 0 || instalments > MAX_INSTALMENTS {
        return Err(refused(english(
            "property.leases.instalments_range",
            &[("max", &MAX_INSTALMENTS)],
        )));
    }
    Ok(())
}

/// Inserts the rent plan of `lease` in `tx`.
async fn insert_plan(
    tx: &mut renox::db::Transaction,
    lease: &Lease,
    first_due: NaiveDate,
    instalments: u32,
) -> Result {
    let plan = rent_plan(first_due, lease.annual_rent, instalments);
    let of = plan.len();
    for (index, (due, amount)) in plan.into_iter().enumerate() {
        let number = i32::try_from(index + 1).unwrap_or(i32::MAX);
        LeaseCharge::create(
            &mut *tx,
            LeaseCharge {
                lease_id: lease.id,
                position: number,
                charge_type: RENT.to_owned(),
                description: rent_description(number, of),
                due_date: due,
                amount,
                vat_category: lease.vat_category,
                status: ChargeStatus::Planned,
                ..Default::default()
            },
        )
        .await?;
    }
    Ok(())
}

/// Drafts a lease: version 1 of a new lease in `draft`, with its rent plan.
/// `by` is the user who drafts it, `None` for the system.
///
/// # Errors
///
/// 422 for an unknown unit or tenant, dates that do not run forward, a rent
/// of 0 or less, a negative deposit or a number of instalments outside 1 to
/// 12. Nothing is written then.
pub async fn create_draft(state: &AppState, by: Option<i64>, new: NewLease) -> Result<Lease> {
    let unit = Unit::find(&state.db, new.unit_id)
        .await?
        .ok_or_else(|| refused(english("property.leases.no_unit", &[])))?;
    check_terms(
        new.start_date,
        new.end_date,
        new.annual_rent,
        new.deposit_amount,
        new.instalments,
    )?;
    if directory(state).find(&state.db, new.tenant).await?.is_none() {
        return Err(refused(english("property.leases.no_tenant", &[])));
    }
    let mut tx = state.db.begin().await?;
    let mut lease = Lease::create(
        &mut tx,
        Lease {
            version: 1,
            version_kind: VersionKind::New,
            building_id: unit.building_id,
            unit_id: unit.id,
            tenant_party_id: new.tenant.0,
            start_date: new.start_date,
            end_date: new.end_date,
            annual_rent: new.annual_rent,
            deposit_amount: new.deposit_amount,
            vat_category: unit.usage.rent_vat(),
            contract_notice_days: new.contract_notice_days,
            terms: clean(new.terms),
            status: LeaseStatus::Draft,
            created_by: by,
            ..Default::default()
        },
    )
    .await?;
    // Version 1 is the root its later versions point to.
    Lease::query()
        .where_(Lease::ID.eq(lease.id))
        .update(&mut tx, &[("root_id", &lease.id)])
        .await?;
    lease.root_id = lease.id;
    insert_plan(
        &mut tx,
        &lease,
        new.first_due.unwrap_or(new.start_date),
        new.instalments,
    )
    .await?;
    tx.commit().await?;
    Ok(lease)
}

/// Drafts a renewal or an amendment of `previous_id`: the next version of the
/// same lease for the same tenant and unit, in `draft`, with its own rent
/// plan. A version that changes the tenant is a new lease, so the tenant is
/// not an input.
///
/// # Errors
///
/// 422 for a version that is neither a renewal nor an amendment, an
/// amendment without its effective date, terms as [`create_draft`] checks
/// them, or when another renewal or amendment of the lease is still being
/// prepared or signed (§7.1: only one at a time). Nothing is written then.
pub async fn create_version(
    state: &AppState,
    by: Option<i64>,
    previous_id: i64,
    new: NewVersion,
) -> Result<Lease> {
    let previous = Lease::find(&state.db, previous_id)
        .await?
        .ok_or(Error::NotFound)?;
    if new.kind == VersionKind::New {
        return Err(refused(english("property.leases.version_kind_bad", &[])));
    }
    if new.kind == VersionKind::Amendment && new.effective_date.is_none() {
        return Err(refused(english("property.leases.effective_required", &[])));
    }
    check_terms(
        new.start_date,
        new.end_date,
        new.annual_rent,
        new.deposit_amount,
        new.instalments,
    )?;
    if !Lease::follow_ups_in_progress(&state.db, previous.root_id)
        .await?
        .is_empty()
    {
        return Err(refused(english("property.leases.one_in_progress", &[])));
    }
    let latest = Lease::versions_of(&state.db, previous.root_id)
        .await?
        .iter()
        .map(|l| l.version)
        .max()
        .unwrap_or(previous.version);
    let first_due = new
        .first_due
        .or(new.effective_date)
        .unwrap_or(new.start_date);
    let mut tx = state.db.begin().await?;
    let lease = Lease::create(
        &mut tx,
        Lease {
            root_id: previous.root_id,
            version: latest + 1,
            version_kind: new.kind,
            previous_id: Some(previous.id),
            effective_date: new.effective_date,
            building_id: previous.building_id,
            unit_id: previous.unit_id,
            tenant_party_id: previous.tenant_party_id,
            start_date: new.start_date,
            end_date: new.end_date,
            annual_rent: new.annual_rent,
            deposit_amount: new.deposit_amount,
            vat_category: previous.vat_category,
            contract_notice_days: previous.contract_notice_days,
            terms: clean(new.terms).or_else(|| previous.terms.clone()),
            status: LeaseStatus::Draft,
            created_by: by,
            ..Default::default()
        },
    )
    .await?;
    insert_plan(&mut tx, &lease, first_due, new.instalments).await?;
    tx.commit().await?;
    Ok(lease)
}

/// The changes to a draft (`PRP.ACL.04`): only what is `Some` changes, and a
/// new rent, term or number of instalments replaces the rent plan.
#[derive(Debug, Clone, Default)]
pub struct DraftEdit {
    /// The first day of the term.
    pub start_date: Option<NaiveDate>,
    /// The last day of the term.
    pub end_date: Option<NaiveDate>,
    /// The annual rent, in fils.
    pub annual_rent: Option<i64>,
    /// The security deposit, in fils.
    pub deposit_amount: Option<i64>,
    /// The number of rent instalments.
    pub instalments: Option<u32>,
    /// The due date of the first instalment.
    pub first_due: Option<NaiveDate>,
    /// The clauses.
    pub terms: Option<String>,
}

/// Edits a lease that is still a `draft`; a lease changes only through a new
/// version once it is submitted (`PRP.ACL.04`).
///
/// # Errors
///
/// 422 when the lease is not a draft, or for terms as [`create_draft`]
/// checks them.
pub async fn revise_draft(state: &AppState, id: i64, edit: DraftEdit) -> Result<Lease> {
    let lease = Lease::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    if lease.status != LeaseStatus::Draft {
        return Err(refused(english("property.leases.edit_draft_only", &[])));
    }
    let start = edit.start_date.unwrap_or(lease.start_date);
    let end = edit.end_date.unwrap_or(lease.end_date);
    let rent = edit.annual_rent.unwrap_or(lease.annual_rent);
    let deposit = edit.deposit_amount.unwrap_or(lease.deposit_amount);
    let live = lease
        .charges(&state.db)
        .await?
        .iter()
        .filter(|c| c.charge_type == RENT && c.status != ChargeStatus::Cancelled)
        .count();
    let instalments = edit
        .instalments
        .unwrap_or_else(|| u32::try_from(live).unwrap_or(1).max(1));
    check_terms(start, end, rent, deposit, instalments)?;
    let replan = edit.annual_rent.is_some()
        || edit.instalments.is_some()
        || edit.first_due.is_some()
        || edit.start_date.is_some();
    let mut tx = state.db.begin().await?;
    Lease::query()
        .where_(Lease::ID.eq(id))
        .update(
            &mut tx,
            &[
                ("start_date", &start),
                ("end_date", &end),
                ("annual_rent", &rent),
                ("deposit_amount", &deposit),
                (
                    "terms",
                    &clean(edit.terms.clone()).or_else(|| lease.terms.clone()),
                ),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    if replan {
        LeaseCharge::query()
            .where_(LeaseCharge::LEASE_ID.eq(id))
            .where_(LeaseCharge::STATUS.eq(ChargeStatus::Planned))
            .update(
                &mut tx,
                &[
                    ("status", &ChargeStatus::Cancelled),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
        let edited = Lease {
            start_date: start,
            end_date: end,
            annual_rent: rent,
            ..lease.clone()
        };
        let first = edit
            .first_due
            .or(lease.effective_date)
            .unwrap_or(start);
        insert_plan(&mut tx, &edited, first, instalments).await?;
    }
    tx.commit().await?;
    Lease::find(&state.db, id).await?.ok_or(Error::NotFound)
}

/// The lease `id`, or a 404.
async fn load(state: &AppState, id: i64) -> std::result::Result<Lease, TransitionError> {
    Lease::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))
}

/// `user` submits the draft `id` for signature. The tenant's documents are
/// checked first by the document guard (`lease.submit`); the unit, the
/// previous version, the amendment's date and the payment plan by the
/// transition ([`crate::lease_checks`]).
///
/// # Errors
///
/// `Forbidden` without `leases.create` in the lease's building, `NotFrom`
/// unless `draft`, 422 with the reason when a document or a check refuses.
pub async fn submit(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Lease, TransitionError> {
    let lease = load(state, id).await?;
    if !user.has_permission_in(permissions::LEASES_CREATE, &lease.scope()) {
        return Err(TransitionError::Forbidden);
    }
    let outcome = documents::guarded(state, GUARD_SUBMIT, id, Some(user.id), || async {
        lease.transition(SUBMIT).by(user).run(state).await
    })
    .await?;
    wl_kernel::approvals::announce(state, &outcome)
        .await
        .map_err(TransitionError::Failed)?;
    load(state, id).await
}

/// `user` activates the signed lease `id`. The signed contract is checked by
/// the document guard (`lease.activate`); a new lease also needs its deposit
/// received or waived. A renewal or amendment carries the held deposit over.
///
/// # Errors
///
/// `Forbidden` without `leases.activate` in the lease's building, `NotFrom`
/// unless `pending_signature`, 422 with the reason when a document, the
/// deposit or a check refuses.
pub async fn activate(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Lease, TransitionError> {
    let lease = load(state, id).await?;
    if !user.has_permission_in(permissions::LEASES_ACTIVATE, &lease.scope()) {
        return Err(TransitionError::Forbidden);
    }
    let outcome = documents::guarded(state, GUARD_ACTIVATE, id, Some(user.id), || async {
        if lease.status == LeaseStatus::PendingSignature {
            deposit_check(state, &lease).await?;
        }
        lease.transition(ACTIVATE).by(user).run(state).await
    })
    .await?;
    wl_kernel::approvals::announce(state, &outcome)
        .await
        .map_err(TransitionError::Failed)?;
    load(state, id).await
}

/// Whether the deposit of a new lease is received (or waived): the held
/// amount reaches the lease's. A renewal or amendment carries the held
/// deposit over, so it is not asked for again.
async fn deposit_check(state: &AppState, lease: &Lease) -> std::result::Result<(), TransitionError> {
    if lease.is_follow_up() || lease.deposit_amount <= 0 || lease.deposit_waived {
        return Ok(());
    }
    let held = deposits(state)
        .held_for(&state.db, &lease.agreement())
        .await?
        .map_or(0, |deposit| deposit.held);
    if held < lease.deposit_amount {
        return Err(TransitionError::Guard(english(
            "property.leases.deposit_missing",
            &[
                ("needed", &crate::money_text(lease.deposit_amount)),
                ("held", &crate::money_text(held)),
            ],
        )));
    }
    Ok(())
}

/// A property manager waives the deposit of a lease that is not active yet,
/// so `activate` does not wait for it (§7.1 `activate`: "or waiver
/// approved"; the approval limits of `FIN.ACL.07` are BL-PRP-26).
///
/// # Errors
///
/// 403 without `leases.activate` in the lease's building; 422 unless the lease
/// is a draft or waiting for signature.
pub async fn waive_deposit(state: &AppState, user: &AuthUser, id: i64) -> Result<Lease> {
    let lease = Lease::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    if !user.has_permission_in(permissions::LEASES_ACTIVATE, &lease.scope()) {
        return Err(Error::Forbidden);
    }
    if !LeaseStatus::IN_PROGRESS.contains(&lease.status) {
        return Err(refused(english("property.leases.waive_status", &[])));
    }
    Lease::query()
        .where_(Lease::ID.eq(id))
        .update(
            &state.db,
            &[("deposit_waived", &true), ("updated_at", &renox::db::now())],
        )
        .await?;
    Lease::find(&state.db, id).await?.ok_or(Error::NotFound)
}

/// The Ejari or Tawtheeq registration of a lease (`PRP.LSE.02`).
#[derive(Debug, Clone)]
pub struct EjariInput {
    /// The registration number, typed in (manual in the MVP, §8).
    pub number: String,
    /// The day it was registered.
    pub registered_on: NaiveDate,
    /// The day the registration expires; the lease's end date when `None`.
    pub expires_on: Option<NaiveDate>,
}

/// The key the Ejari task of a lease version is found by while it is open.
pub fn ejari_task_key(lease: &Lease) -> String {
    format!("property.ejari:{}", lease.id)
}

/// Records the Ejari or Tawtheeq registration of lease `id` and closes the
/// Ejari task. The certificate is a document of the kind *Ejari / Tawtheeq
/// certificate* attached to the lease; the document guard
/// (`lease.record_ejari`) refuses or flags a missing one by the kind's mode.
///
/// # Errors
///
/// 403 without `leases.activate` in the lease's building; 422 for a blank
/// number, a lease that is not in force, or a certificate the guard refuses.
pub async fn record_ejari(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    input: EjariInput,
) -> Result<Lease> {
    let lease = Lease::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    if !user.has_permission_in(permissions::LEASES_ACTIVATE, &lease.scope()) {
        return Err(Error::Forbidden);
    }
    let number = input.number.trim().to_owned();
    if number.is_empty() {
        return Err(refused(english("property.ejari.number_required", &[])));
    }
    if !LeaseStatus::RUNNING.contains(&lease.status) {
        return Err(refused(english("property.ejari.not_running", &[])));
    }
    let expires = input.expires_on.unwrap_or(lease.end_date);
    documents::guarded(state, documents::RECORD_EJARI, id, Some(user.id), || async {
        Lease::query()
            .where_(Lease::ID.eq(id))
            .update(
                &state.db,
                &[
                    ("ejari_number", &Some(number.clone())),
                    ("ejari_registered_on", &Some(input.registered_on)),
                    ("ejari_expires_on", &Some(expires)),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await
            .map_err(TransitionError::Failed)?;
        Ok(())
    })
    .await
    .map_err(TransitionError::into_error)?;
    let creator = task_creator(state);
    if let Some(task) = creator.find_open(state, &ejari_task_key(&lease)).await? {
        creator
            .complete(state, task.id, &english("property.ejari.recorded", &[]))
            .await?;
    }
    Lease::find(&state.db, id).await?.ok_or(Error::NotFound)
}
