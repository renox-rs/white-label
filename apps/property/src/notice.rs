//! Recording a notice to end a lease (`record_notice`, `PRP.RUL.04`,
//! 01 Property §4.3 rows "Early termination by the tenant" and "holding
//! over", §7.1 *Lease*, §7.2 *Statutory notices*; BL-PRP-09).
//!
//! A notice can be recorded from `active`, `renewal_due` and `expired` (from
//! `expired` it is how the landlord objects to holding over, reason
//! *landlord's ground*). It records the date, who gave it, why, and:
//!
//! - **A tenant's notice** needs the planned move-out date (not the actual
//!   one, which `terminate` records). With the reason *early termination* the
//!   user may enter the penalty the contract sets (no formula): one charge of
//!   the *early termination fee* type, due on the planned move-out date and
//!   moved with it while it is not invoiced ([`change_planned_move_out`]).
//!   The VAT treatment comes from the charge type the client configured
//!   ([`crate::charge_type`]); without it the penalty is refused with
//!   "configure the charge type first" (`⚠ open: accountant (A-06)`).
//! - **A landlord's notice** is a statutory notice: it is sent to the tenant
//!   first ([`crate::notices::LANDLORD_NOTICE`]) and the transition is allowed
//!   once it was sent. The lease shows "notice not delivered" until the
//!   provider's receipt or the proof staff recorded is known
//!   ([`delivered_on`]); the planned move-out date is computed from that date
//!   only (`notice_days`, the contract's period or 90 days), so nothing that
//!   depends on the notice starts before the proof. The landlord's ground is
//!   recorded as text; the platform does not judge it (`⚠ open: lawyer
//!   L-22`).
//!
//! The transition (in one transaction) records the notice, moves the unit to
//! `notice_given` and creates the penalty charge; after the commit the move-out
//! inspection ticket is created ([`after_record`]). The ticket is of the
//! category [`MOVE_OUT_CATEGORY`], which the vertical's starter data seeds;
//! where ticketing does not know it yet it is a staff task.
//!
//! Decided where the spec is silent:
//!
//! - A penalty needs a tenant's notice with the reason *early termination*.
//! - A landlord's planned move-out date is not entered: it is the proof date
//!   plus the notice period of the lease.
//! - The ticket is due at 09:00 Asia/Dubai on the planned move-out date; a
//!   landlord's ticket has no due time until the proof arrives.

use renox::chrono::{Duration, NaiveDate};
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::{dubai_date, today};
use wl_kernel::notify::{NoticeStatus, StatutoryNoticeProofRecorded, notifier};
use wl_kernel::role_tasks::RoleTask;

use crate::building::{clean, refused};
use crate::charge::{ChargeStatus, LeaseCharge};
use crate::charge_type::{ChargeType, EARLY_TERMINATION_FEE};
use crate::db::set_lease;
use crate::lease::{LEASE_SUBJECT_TYPE, Lease, LeaseStatus};
use crate::lease_machine::RECORD_NOTICE;
use crate::notices::{self, LANDLORD_NOTICE};
use crate::permissions::{self, PROPERTY_MANAGER_ROLE};
use crate::texts::english;
use crate::unit::Unit;
use crate::unit_machine::MARK_NOTICE;

/// The ticket category of the move-out inspection.
pub const MOVE_OUT_CATEGORY: &str = "move_out_inspection";

/// The key the move-out inspection ticket of `lease` is found by while open.
pub fn move_out_key(lease: &Lease) -> String {
    format!("property.move_out:{}", lease.id)
}

/// Who gave the notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeParty {
    /// The tenant ends the lease.
    Tenant,
    /// The landlord ends the lease.
    Landlord,
}

impl NoticeParty {
    /// The stored text.
    pub const fn as_str(self) -> &'static str {
        match self {
            NoticeParty::Tenant => "tenant",
            NoticeParty::Landlord => "landlord",
        }
    }

    /// The party stored as `text`.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "tenant" => Some(NoticeParty::Tenant),
            "landlord" => Some(NoticeParty::Landlord),
            _ => None,
        }
    }
}

/// Why the lease is ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeReason {
    /// The lease ends at its term.
    EndOfTerm,
    /// The tenant leaves before the end of the term.
    EarlyTermination,
    /// A ground of the landlord (recorded as text).
    LandlordsGround,
    /// The tenant did not pay.
    NonPayment,
}

impl NoticeReason {
    /// The stored text.
    pub const fn as_str(self) -> &'static str {
        match self {
            NoticeReason::EndOfTerm => "end_of_term",
            NoticeReason::EarlyTermination => "early_termination",
            NoticeReason::LandlordsGround => "landlords_ground",
            NoticeReason::NonPayment => "non_payment",
        }
    }

    /// The reason stored as `text`.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "end_of_term" => Some(NoticeReason::EndOfTerm),
            "early_termination" => Some(NoticeReason::EarlyTermination),
            "landlords_ground" => Some(NoticeReason::LandlordsGround),
            "non_payment" => Some(NoticeReason::NonPayment),
            _ => None,
        }
    }
}

/// What a user enters to record a notice.
#[derive(Debug, Clone)]
pub struct NewNotice {
    /// The date of the notice.
    pub date: NaiveDate,
    /// Who gave it.
    pub party: NoticeParty,
    /// Why.
    pub reason: NoticeReason,
    /// The landlord's ground, as text.
    pub ground: Option<String>,
    /// The date the tenant will leave; required for a tenant's notice. A
    /// landlord's is computed from the proof of delivery.
    pub planned_move_out: Option<NaiveDate>,
    /// The early termination penalty from the contract, in fils; 0 for none.
    pub penalty: i64,
}

impl NewNotice {
    /// A tenant's notice dated `date` for `reason`; the tenant leaves on
    /// `planned_move_out`.
    pub fn tenant(date: NaiveDate, reason: NoticeReason, planned_move_out: NaiveDate) -> Self {
        NewNotice {
            date,
            party: NoticeParty::Tenant,
            reason,
            ground: None,
            planned_move_out: Some(planned_move_out),
            penalty: 0,
        }
    }

    /// A landlord's notice dated `date` for `reason`.
    pub fn landlord(date: NaiveDate, reason: NoticeReason) -> Self {
        NewNotice {
            date,
            party: NoticeParty::Landlord,
            reason,
            ground: None,
            planned_move_out: None,
            penalty: 0,
        }
    }

    /// With the landlord's ground as text.
    pub fn ground(mut self, ground: &str) -> Self {
        self.ground = Some(ground.to_owned());
        self
    }

    /// With the early termination penalty of the contract, in fils.
    pub fn penalty(mut self, fils: i64) -> Self {
        self.penalty = fils;
        self
    }
}

/// What a `record_notice` run carries to its guard and effect.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoticeInput {
    /// The date of the notice.
    #[serde(default)]
    pub date: Option<NaiveDate>,
    /// `tenant` or `landlord`.
    #[serde(default)]
    pub party: String,
    /// `end_of_term`, `early_termination`, `landlords_ground` or
    /// `non_payment`.
    #[serde(default)]
    pub reason: String,
    /// The landlord's ground, as text.
    #[serde(default)]
    pub ground: Option<String>,
    /// The planned move-out date of a tenant's notice.
    #[serde(default)]
    pub planned_move_out: Option<NaiveDate>,
    /// The penalty, in fils.
    #[serde(default)]
    pub penalty: i64,
    /// The day a landlord's notice was sent.
    #[serde(default)]
    pub sent_on: Option<NaiveDate>,
}

/// The input of a `record_notice` run.
pub fn notice_input(ctx: &TransitionCtx) -> NoticeInput {
    ctx.input_as::<NoticeInput>().unwrap_or_default()
}

/// Why `input` cannot be recorded, if it cannot: the notice needs its date,
/// party and reason; a tenant's notice the planned move-out date, not before
/// the notice; a penalty only on a tenant's notice for early termination.
pub fn notice_problem(input: &NoticeInput) -> Option<String> {
    let Some(party) = NoticeParty::parse(&input.party) else {
        return Some(english("property.notice.party_required", &[]));
    };
    let Some(reason) = NoticeReason::parse(&input.reason) else {
        return Some(english("property.notice.reason_required", &[]));
    };
    let Some(date) = input.date else {
        return Some(english("property.notice.date_required", &[]));
    };
    if party == NoticeParty::Tenant {
        match input.planned_move_out {
            None => return Some(english("property.notice.planned_required", &[])),
            Some(planned) if planned < date => {
                return Some(english("property.notice.planned_before", &[]));
            }
            Some(_) => {}
        }
    }
    if input.penalty < 0 {
        return Some(english("property.notice.penalty_negative", &[]));
    }
    if input.penalty > 0
        && !(party == NoticeParty::Tenant && reason == NoticeReason::EarlyTermination)
    {
        return Some(english("property.notice.penalty_not_allowed", &[]));
    }
    None
}

/// The effect of `record_notice` inside its transaction: the notice is
/// stored, the unit moves to `notice_given` and the penalty becomes a charge.
pub(crate) async fn record_effects(
    tx: &mut Transaction,
    lease: &Lease,
    input: &NoticeInput,
) -> Result {
    let (Some(party), Some(reason)) = (
        NoticeParty::parse(&input.party),
        NoticeReason::parse(&input.reason),
    ) else {
        return Err(refused(english("property.notice.party_required", &[])));
    };
    let planned = if party == NoticeParty::Tenant {
        input.planned_move_out
    } else {
        None
    };
    set_lease(
        &mut *tx,
        lease.id,
        &[
            ("notice_date", &input.date),
            ("notice_party", &Some(party.as_str().to_owned())),
            ("notice_reason", &Some(reason.as_str().to_owned())),
            ("notice_ground", &clean(input.ground.clone())),
            ("notice_sent_on", &input.sent_on),
            ("notice_delivered_on", &None::<NaiveDate>),
            ("planned_move_out", &planned),
            ("inspection_done_on", &None::<NaiveDate>),
            ("keys_returned_on", &None::<NaiveDate>),
            ("inspection_defects", &false),
        ],
    )
    .await?;

    let unit = Unit::find(&mut *tx, lease.unit_id)
        .await?
        .ok_or(Error::NotFound)?;
    let outcome = unit
        .transition(MARK_NOTICE)
        .by_system()
        .run_in(&mut *tx)
        .await
        .map_err(TransitionError::into_error)?;
    drop(outcome);

    if input.penalty > 0 {
        let Some(kind) = ChargeType::active_by_code(&mut *tx, EARLY_TERMINATION_FEE).await? else {
            return Err(refused(english("property.notice.charge_type_missing", &[])));
        };
        let Some(due) = planned else {
            return Err(refused(english("property.notice.planned_required", &[])));
        };
        let position = LeaseCharge::query()
            .where_(LeaseCharge::LEASE_ID.eq(lease.id))
            .get(&mut *tx)
            .await?
            .iter()
            .map(|c| c.position)
            .max()
            .unwrap_or(0)
            + 1;
        let description = english(
            "property.notice.penalty_line",
            &[("lease", &lease.number_with_version())],
        );
        LeaseCharge::create(
            &mut *tx,
            LeaseCharge {
                lease_id: lease.id,
                position,
                charge_type: EARLY_TERMINATION_FEE.to_owned(),
                description,
                due_date: due,
                amount: input.penalty,
                vat_category: kind.vat_category,
                status: ChargeStatus::Scheduled,
                ..Default::default()
            },
        )
        .await?;
    }
    Ok(())
}

/// The lease `id`, or a 404.
async fn load(state: &AppState, id: i64) -> std::result::Result<Lease, TransitionError> {
    Lease::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))
}

/// `user` records a notice on lease `id`. A landlord's notice is sent to the
/// tenant first and the transition follows.
///
/// # Errors
///
/// `Forbidden` without `leases.manage` in the lease's building, `NotFrom`
/// unless the lease is `active`, `renewal_due` or `expired`, `Guard` (422)
/// for a notice without its party, reason, date or (a tenant's) planned
/// move-out date, or a penalty the charge catalogue cannot bill.
pub async fn record(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    new: NewNotice,
) -> std::result::Result<Lease, TransitionError> {
    let lease = load(state, id).await?;
    if !user.has_permission_in(permissions::LEASES_MANAGE, &lease.scope()) {
        return Err(TransitionError::Forbidden);
    }
    if !matches!(
        lease.status,
        LeaseStatus::Active | LeaseStatus::RenewalDue | LeaseStatus::Expired
    ) {
        return Err(TransitionError::NotFrom(lease.status.as_str().to_owned()));
    }
    let mut input = NoticeInput {
        date: Some(new.date),
        party: new.party.as_str().to_owned(),
        reason: new.reason.as_str().to_owned(),
        ground: clean(new.ground),
        planned_move_out: new.planned_move_out,
        penalty: new.penalty,
        sent_on: None,
    };
    if let Some(problem) = notice_problem(&input) {
        return Err(TransitionError::Guard(problem));
    }
    if input.penalty > 0
        && ChargeType::active_by_code(&state.db, EARLY_TERMINATION_FEE)
            .await?
            .is_none()
    {
        return Err(TransitionError::Guard(english(
            "property.notice.charge_type_missing",
            &[],
        )));
    }
    if new.party == NoticeParty::Landlord {
        input.planned_move_out = None;
        let ground = input.ground.clone().unwrap_or_default();
        notices::landlord_notice(state, &lease, new.date, &ground)
            .await
            .map_err(TransitionError::Failed)?;
        input.sent_on = Some(today());
    }
    let _outcome = lease
        .transition(RECORD_NOTICE)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    load(state, id).await
}

/// The day the landlord's notice of `lease` was delivered, when it was: stored
/// once known, else asked from the notifier (the provider's receipt, or the
/// proof staff recorded when the message gave up). The planned move-out date
/// is computed from it, once: the proof date plus the notice period of the
/// lease. `None` for a tenant's notice and while it is not delivered.
///
/// # Errors
///
/// The notifier's and the database's errors.
pub async fn delivered_on(state: &AppState, lease: &Lease) -> Result<Option<NaiveDate>> {
    if lease.notice_party.as_deref() != Some(NoticeParty::Landlord.as_str()) {
        return Ok(None);
    }
    if let Some(day) = lease.notice_delivered_on {
        return Ok(Some(day));
    }
    let status = notifier(state)
        .notice_status(state, &lease.subject(), LANDLORD_NOTICE)
        .await?;
    let day = match status {
        NoticeStatus::Delivered { at } => dubai_date(at),
        NoticeStatus::ProofRecorded { delivered_on } => delivered_on,
        NoticeStatus::NotDelivered => return Ok(None),
    };
    let planned = day + Duration::days(lease.notice_days());
    Lease::query()
        .where_(Lease::ID.eq(lease.id))
        .where_(Lease::NOTICE_DELIVERED_ON.is_null())
        .update(
            &state.db,
            &[
                ("notice_delivered_on", &Some(day)),
                ("planned_move_out", &Some(planned)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(Some(day))
}

/// The proof of delivery of a landlord's notice was recorded: the planned
/// move-out date is computed from its date.
///
/// # Errors
///
/// The notifier's and the database's errors.
pub(crate) async fn on_proof(state: &AppState, event: &StatutoryNoticeProofRecorded) -> Result {
    if event.type_key != LANDLORD_NOTICE || event.subject.subject_type != LEASE_SUBJECT_TYPE {
        return Ok(());
    }
    let Some(lease) = Lease::find(&state.db, event.subject.subject_id).await? else {
        return Ok(());
    };
    delivered_on(state, &lease).await?;
    Ok(())
}

/// `user` changes the planned move-out date of the tenant's notice on lease
/// `id` to `date`. The penalty that is not invoiced yet is due on the new
/// date; an invoiced one stays.
///
/// # Errors
///
/// 403 without `leases.manage` in the lease's building; 422 unless the lease
/// has a tenant's notice, or for a date before the notice.
pub async fn change_planned_move_out(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    date: NaiveDate,
) -> Result<Lease> {
    let lease = Lease::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    if !user.has_permission_in(permissions::LEASES_MANAGE, &lease.scope()) {
        return Err(Error::Forbidden);
    }
    if lease.status != LeaseStatus::NoticeGiven
        || lease.notice_party.as_deref() != Some(NoticeParty::Tenant.as_str())
    {
        return Err(refused(english(
            "property.notice.planned_edit_refused",
            &[],
        )));
    }
    if lease.notice_date.is_some_and(|notice| date < notice) {
        return Err(refused(english("property.notice.planned_before", &[])));
    }
    let mut tx = state.db.begin().await?;
    set_lease(&mut tx, id, &[("planned_move_out", &Some(date))]).await?;
    LeaseCharge::query()
        .where_(LeaseCharge::LEASE_ID.eq(id))
        .where_(LeaseCharge::CHARGE_TYPE.eq(EARLY_TERMINATION_FEE))
        .where_(LeaseCharge::STATUS.is_in([ChargeStatus::Planned, ChargeStatus::Scheduled]))
        .update(
            &mut tx,
            &[("due_date", &date), ("updated_at", &renox::db::now())],
        )
        .await?;
    tx.commit().await?;
    Lease::find(&state.db, id).await?.ok_or(Error::NotFound)
}

/// After the commit of `record_notice` on lease `id`: the move-out inspection
/// ticket is created for the property manager, due on the planned move-out
/// date when it is known.
///
/// # Errors
///
/// The database's and ticketing's errors.
pub(crate) async fn after_record(state: &AppState, id: i64) -> Result {
    let lease = Lease::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    let title = english(
        "property.notice.ticket_title",
        &[("lease", &lease.number_with_version())],
    );
    let description = match lease.planned_move_out {
        Some(day) => english("property.notice.ticket_description", &[("day", &day)]),
        None => english("property.notice.ticket_description_open", &[]),
    };
    let mut task = RoleTask::new(
        lease.subject(),
        PROPERTY_MANAGER_ROLE,
        &title,
        &move_out_key(&lease),
    )
    .description(&description);
    if let Some(due) = lease.planned_move_out.and_then(crate::tasks::dubai_morning) {
        task = task.due_at(due);
    }
    crate::tasks::open_in(state, &lease.scope(), task, MOVE_OUT_CATEGORY).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2027, m, d).unwrap()
    }

    fn tenant_input() -> NoticeInput {
        NoticeInput {
            date: Some(day(6, 1)),
            party: "tenant".into(),
            reason: "early_termination".into(),
            ground: None,
            planned_move_out: Some(day(8, 31)),
            penalty: 2_000_000,
            sent_on: None,
        }
    }

    #[test]
    fn parties_and_reasons_are_stored_in_snake_case() {
        assert_eq!(NoticeParty::Landlord.as_str(), "landlord");
        assert_eq!(NoticeParty::parse(" tenant "), Some(NoticeParty::Tenant));
        assert_eq!(NoticeParty::parse("agent"), None);
        for reason in [
            NoticeReason::EndOfTerm,
            NoticeReason::EarlyTermination,
            NoticeReason::LandlordsGround,
            NoticeReason::NonPayment,
        ] {
            assert_eq!(NoticeReason::parse(reason.as_str()), Some(reason));
        }
        assert_eq!(NoticeReason::parse(""), None);
    }

    #[test]
    fn a_complete_tenant_notice_is_accepted() {
        assert_eq!(notice_problem(&tenant_input()), None);
    }

    #[test]
    fn a_notice_without_party_reason_or_date_is_refused() {
        let mut input = tenant_input();
        input.party = String::new();
        assert!(notice_problem(&input).is_some());
        let mut input = tenant_input();
        input.reason = "because".into();
        assert!(notice_problem(&input).is_some());
        let mut input = tenant_input();
        input.date = None;
        assert!(notice_problem(&input).is_some());
        assert!(notice_problem(&NoticeInput::default()).is_some());
    }

    #[test]
    fn a_tenant_notice_needs_a_planned_move_out_date_not_before_the_notice() {
        let mut input = tenant_input();
        input.planned_move_out = None;
        assert!(notice_problem(&input).unwrap().contains("planned move-out"));
        input.planned_move_out = Some(day(5, 31));
        assert!(
            notice_problem(&input)
                .unwrap()
                .contains("before the notice")
        );
        input.planned_move_out = Some(day(6, 1));
        assert_eq!(notice_problem(&input), None);
    }

    #[test]
    fn a_landlord_notice_needs_no_planned_date() {
        let input = NoticeInput {
            date: Some(day(2, 1)),
            party: "landlord".into(),
            reason: "landlords_ground".into(),
            ..Default::default()
        };
        assert_eq!(notice_problem(&input), None);
    }

    #[test]
    fn a_penalty_belongs_to_a_tenants_early_termination() {
        let mut input = tenant_input();
        input.reason = "end_of_term".into();
        assert!(
            notice_problem(&input)
                .unwrap()
                .contains("early termination")
        );
        let mut input = tenant_input();
        input.party = "landlord".into();
        input.planned_move_out = None;
        assert!(notice_problem(&input).is_some());
        let mut input = tenant_input();
        input.penalty = -1;
        assert!(notice_problem(&input).unwrap().contains("negative"));
    }

    #[test]
    fn the_inspection_ticket_is_found_by_the_lease() {
        let lease = Lease {
            id: 14,
            ..Default::default()
        };
        assert_eq!(move_out_key(&lease), "property.move_out:14");
    }
}
