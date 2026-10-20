//! Changed terms against the tenant (`PRP.ACL.17`, 01 Property §3.3, §4.3,
//! §7.1 `submit`; BL-PRP-04).
//!
//! A renewal or an amendment version that changes terms against the tenant
//! is submitted only when
//!
//! - **a renewal**: the notice of changed terms was delivered, with proof,
//!   at least the notice period before the current end date (the deadline day
//!   counts; the proof is read from the version being replaced, see
//!   [`crate::renewal::delivered_on`]); otherwise only with the tenant's
//!   signed acceptance attached to the draft ([`TENANT_ACCEPTANCE_KIND`]);
//! - **an amendment** (mid-term, by agreement): always with that acceptance;
//!   a notice alone is not enough.
//!
//! Against the tenant ([`Changes::automatic`]): a higher annual rent, a
//! higher deposit, or fewer cheques than the current version, found by
//! comparing the versions. Any other changed term ([`Changes::other`]):
//! different clauses, a different payment plan, an added charge or another
//! contract notice period, counts unless the submitter answers *no* with a
//! reason ([`answer`]); not answering counts as *yes*. A rent decrease alone
//! is not against the tenant, and a version on the current terms is not
//! checked at all.
//!
//! The decision needs the notifier and the document guard, which a Renox
//! guard cannot call, so [`refusal`] runs before the `submit` transition and
//! the transition raises its message after its own checks
//! ([`crate::lease_machine::SubmitInput`]). The reason of a *no* is the
//! reason of the `submit` run, so it is kept in the history.
//!
//! Decided where the spec is silent:
//!
//! - Cheques are counted as the rent instalments of the payment plan. For a
//!   renewal the current version's instalments due inside its term are
//!   compared; for an amendment those due on or after the effective date
//!   (the amendment's plan starts there). The day of the month of the first
//!   instalment is compared for a renewal only.
//! - The acceptance is a document of the kind [`TENANT_ACCEPTANCE_KIND`]
//!   attached to the draft version; the document guard says whether it is
//!   valid. Without `wl-documents` every document is valid (the module's
//!   rule), so the acceptance counts as attached.

use std::collections::BTreeSet;

use renox::chrono::Datelike;
use renox::prelude::*;
use wl_kernel::documents::{guard, today};

use crate::building::{clean, refused};
use crate::charge::{ChargeStatus, LeaseCharge, RENT};
use crate::lease::{Lease, LeaseStatus, VersionKind};
use crate::lease_checks::previous_accepts;
use crate::permissions;
use crate::renewal::{delivered_on, notice_deadline};
use crate::texts::english;

/// The key of the document kind *tenant acceptance of changed terms*
/// (§7.2 *Document kinds*, seeded with the other kinds of the vertical).
pub const TENANT_ACCEPTANCE_KIND: &str = "tenant_acceptance";

/// What differs between a version and the one it replaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Changes {
    /// The annual rent is higher.
    pub rent_up: bool,
    /// The deposit is higher.
    pub deposit_up: bool,
    /// There are fewer cheques (rent instalments).
    pub fewer_cheques: bool,
    /// Another term differs: clauses, payment plan, added charges, notice
    /// period.
    pub other: bool,
}

impl Changes {
    /// Whether a change is against the tenant whatever the submitter says.
    pub fn automatic(&self) -> bool {
        self.rent_up || self.deposit_up || self.fewer_cheques
    }

    /// Whether `lease` changes terms against the tenant: found
    /// automatically, or another changed term the submitter did not answer
    /// *no* to.
    pub fn against_tenant(&self, lease: &Lease) -> bool {
        self.automatic() || (self.other && !lease.terms_answered_no)
    }
}

/// The live rent instalments of `charges`.
fn rent_of(charges: &[LeaseCharge]) -> Vec<&LeaseCharge> {
    charges
        .iter()
        .filter(|c| c.charge_type == RENT && c.status != ChargeStatus::Cancelled)
        .collect()
}

/// The charge types other than rent among the live `charges`.
fn extras_of(charges: &[LeaseCharge]) -> BTreeSet<&str> {
    charges
        .iter()
        .filter(|c| c.charge_type != RENT && c.status != ChargeStatus::Cancelled)
        .map(|c| c.charge_type.as_str())
        .collect()
}

/// The clauses as compared: trimmed, blank as none.
fn clauses(lease: &Lease) -> &str {
    lease.terms.as_deref().map_or("", str::trim)
}

/// Compares `lease` (a draft renewal or amendment) with `previous`, the
/// version it replaces. `before` and `after` are their charges.
pub fn compare(
    lease: &Lease,
    previous: &Lease,
    before: &[LeaseCharge],
    after: &[LeaseCharge],
) -> Changes {
    let amendment = lease.version_kind == VersionKind::Amendment;
    // The instalments of the current version that the new plan replaces.
    let replaced: Vec<&LeaseCharge> = rent_of(before)
        .into_iter()
        .filter(|c| match (amendment, lease.effective_date) {
            (true, Some(from)) => c.due_date >= from,
            _ => c.due_date <= previous.end_date,
        })
        .collect();
    let new_rent = rent_of(after);
    let same_day = amendment
        || match (replaced.first(), new_rent.first()) {
            (Some(old), Some(new)) => old.due_date.day() == new.due_date.day(),
            _ => true,
        };
    let extras_before = extras_of(before);
    let added = extras_of(after).difference(&extras_before).next().is_some();
    Changes {
        rent_up: lease.annual_rent > previous.annual_rent,
        deposit_up: lease.deposit_amount > previous.deposit_amount,
        fewer_cheques: new_rent.len() < replaced.len(),
        other: clauses(lease) != clauses(previous)
            || new_rent.len() != replaced.len()
            || !same_day
            || added
            || lease.contract_notice_days != previous.contract_notice_days,
    }
}

/// Whether the tenant's signed acceptance of changed terms is attached to the
/// draft `lease` and valid today.
///
/// # Errors
///
/// The document guard's errors.
pub async fn acceptance_attached(state: &AppState, lease: &Lease) -> Result<bool> {
    Ok(guard(state)
        .is_valid(&state.db, &lease.subject(), TENANT_ACCEPTANCE_KIND, today())
        .await?
        .is_valid())
}

/// What `PRP.ACL.17` found about the draft `lease`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assessment {
    /// The comparison with the version it replaces.
    pub changes: Changes,
    /// Whether the changes are against the tenant.
    pub against_tenant: bool,
    /// The tenant's signed acceptance is attached.
    pub accepted: bool,
}

/// Assesses a renewal or amendment draft against the version it replaces.
/// `None` for a new lease, a version whose previous version cannot be found
/// or is not in a state its `renew` or `amend` accepts (the transition's own
/// checks refuse those first).
///
/// # Errors
///
/// The database's and the document guard's errors.
pub async fn assess(state: &AppState, lease: &Lease) -> Result<Option<Assessment>> {
    if !lease.is_follow_up() {
        return Ok(None);
    }
    let Some(previous) = lease.previous(&state.db).await? else {
        return Ok(None);
    };
    if !previous_accepts(lease, &previous) {
        return Ok(None);
    }
    let before = previous.charges(&state.db).await?;
    let after = lease.charges(&state.db).await?;
    let changes = compare(lease, &previous, &before, &after);
    let against_tenant = changes.against_tenant(lease);
    let accepted = if against_tenant {
        acceptance_attached(state, lease).await?
    } else {
        false
    };
    Ok(Some(Assessment {
        changes,
        against_tenant,
        accepted,
    }))
}

/// Why the draft `lease` cannot be submitted under `PRP.ACL.17`, naming the
/// deadline, the proof status and the two ways forward; `None` when it can.
///
/// # Errors
///
/// The database's, the notifier's and the document guard's errors.
pub async fn refusal(state: &AppState, lease: &Lease) -> Result<Option<String>> {
    let Some(found) = assess(state, lease).await? else {
        return Ok(None);
    };
    if !found.against_tenant || found.accepted {
        return Ok(None);
    }
    if lease.version_kind == VersionKind::Amendment {
        return Ok(Some(english(
            "property.terms.amendment_needs_acceptance",
            &[],
        )));
    }
    let Some(previous) = lease.previous(&state.db).await? else {
        return Ok(None);
    };
    let deadline = notice_deadline(&previous);
    Ok(match delivered_on(state, &previous).await? {
        Some(day) if day <= deadline => None,
        Some(day) => Some(english(
            "property.terms.notice_late",
            &[("deadline", &deadline), ("delivered", &day)],
        )),
        None => Some(english(
            "property.terms.notice_missing",
            &[("deadline", &deadline)],
        )),
    })
}

/// The reason `submit` runs with: the reason of a *no*, kept in the history.
pub fn history_reason(lease: &Lease) -> Option<String> {
    if lease.terms_answered_no {
        clean(lease.terms_answer_reason.clone())
    } else {
        None
    }
}

/// `user` answers "changes terms against the tenant?" on the draft renewal or
/// amendment `id`: `against` is yes; *no* needs a reason, which is kept in
/// the history at `submit`.
///
/// # Errors
///
/// 403 without `leases.create` in the lease's building; 422 unless the lease
/// is a draft renewal or amendment, or for a *no* without a reason.
pub async fn answer(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    against: bool,
    reason: Option<String>,
) -> Result<Lease> {
    let lease = Lease::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    if !user.has_permission_in(permissions::LEASES_CREATE, &lease.scope()) {
        return Err(Error::Forbidden);
    }
    if lease.status != LeaseStatus::Draft || !lease.is_follow_up() {
        return Err(refused(english("property.terms.answer_draft_only", &[])));
    }
    let reason = clean(reason);
    if !against && reason.is_none() {
        return Err(refused(english("property.terms.reason_required", &[])));
    }
    Lease::query()
        .where_(Lease::ID.eq(id))
        .update(
            &state.db,
            &[
                ("terms_answered_no", &!against),
                ("terms_answer_reason", &if against { None } else { reason }),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Lease::find(&state.db, id).await?.ok_or(Error::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::chrono::NaiveDate;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn charge(due: NaiveDate, kind: &str) -> LeaseCharge {
        LeaseCharge {
            charge_type: kind.to_owned(),
            due_date: due,
            amount: 100,
            ..Default::default()
        }
    }

    /// Four quarterly instalments from `year`-01-01.
    fn plan(year: i32) -> Vec<LeaseCharge> {
        [1, 4, 7, 10]
            .into_iter()
            .map(|m| charge(day(year, m, 1), RENT))
            .collect()
    }

    fn versions() -> (Lease, Lease) {
        let previous = Lease {
            start_date: day(2027, 1, 1),
            end_date: day(2027, 12, 31),
            annual_rent: 10_000_000,
            deposit_amount: 500_000,
            ..Default::default()
        };
        let renewal = Lease {
            version_kind: VersionKind::Renewal,
            start_date: day(2028, 1, 1),
            end_date: day(2028, 12, 31),
            annual_rent: 10_000_000,
            deposit_amount: 500_000,
            ..Default::default()
        };
        (previous, renewal)
    }

    #[test]
    fn a_renewal_on_the_current_terms_changes_nothing() {
        let (previous, renewal) = versions();
        let changes = compare(&renewal, &previous, &plan(2027), &plan(2028));
        assert_eq!(changes, Changes::default());
        assert!(!changes.against_tenant(&renewal));
    }

    #[test]
    fn a_higher_rent_or_deposit_or_fewer_cheques_is_against_the_tenant() {
        let (previous, renewal) = versions();
        let dearer = Lease {
            annual_rent: 10_500_000,
            ..renewal.clone()
        };
        assert!(compare(&dearer, &previous, &plan(2027), &plan(2028)).rent_up);
        let deposit = Lease {
            deposit_amount: 600_000,
            ..renewal.clone()
        };
        assert!(compare(&deposit, &previous, &plan(2027), &plan(2028)).deposit_up);
        let mut two = plan(2028);
        two.truncate(2);
        let fewer = compare(&renewal, &previous, &plan(2027), &two);
        assert!(fewer.fewer_cheques && fewer.automatic());
    }

    #[test]
    fn a_rent_decrease_alone_is_not_against_the_tenant() {
        let (previous, renewal) = versions();
        let cheaper = Lease {
            annual_rent: 9_000_000,
            deposit_amount: 400_000,
            ..renewal
        };
        let changes = compare(&cheaper, &previous, &plan(2027), &plan(2028));
        assert!(!changes.against_tenant(&cheaper));
    }

    #[test]
    fn another_changed_term_is_against_the_tenant_unless_answered_no() {
        let (previous, renewal) = versions();
        let clause = Lease {
            terms: Some("No pets".into()),
            ..renewal
        };
        let changes = compare(&clause, &previous, &plan(2027), &plan(2028));
        assert!(changes.other && !changes.automatic());
        assert!(
            changes.against_tenant(&clause),
            "not answered counts as yes"
        );
        let no = Lease {
            terms_answered_no: true,
            ..clause
        };
        assert!(!changes.against_tenant(&no));
        // A no does not excuse a higher rent.
        let and_rent = Changes {
            rent_up: true,
            ..changes
        };
        assert!(and_rent.against_tenant(&no));
    }

    #[test]
    fn blank_clauses_equal_none_and_trailing_spaces_do_not_count() {
        let (mut previous, mut renewal) = versions();
        previous.terms = None;
        renewal.terms = Some("  ".into());
        assert!(!compare(&renewal, &previous, &plan(2027), &plan(2028)).other);
        previous.terms = Some("No pets".into());
        renewal.terms = Some("No pets ".into());
        assert!(!compare(&renewal, &previous, &plan(2027), &plan(2028)).other);
    }

    #[test]
    fn an_added_charge_or_a_moved_payment_day_is_another_changed_term() {
        let (previous, renewal) = versions();
        let mut with_parking = plan(2028);
        with_parking.push(charge(day(2028, 1, 1), "parking"));
        assert!(compare(&renewal, &previous, &plan(2027), &with_parking).other);
        let moved: Vec<_> = plan(2028)
            .into_iter()
            .map(|mut c| {
                c.due_date = c.due_date.with_day(15).unwrap();
                c
            })
            .collect();
        assert!(compare(&renewal, &previous, &plan(2027), &moved).other);
    }

    #[test]
    fn the_holding_over_extension_of_the_current_version_is_not_compared() {
        let (previous, renewal) = versions();
        let mut extended = plan(2027);
        extended.extend(plan(2028));
        let changes = compare(&renewal, &previous, &extended, &plan(2028));
        assert_eq!(changes, Changes::default());
    }

    #[test]
    fn an_amendment_compares_the_instalments_from_its_effective_date() {
        let (previous, _) = versions();
        let amendment = Lease {
            version_kind: VersionKind::Amendment,
            effective_date: Some(day(2027, 7, 1)),
            start_date: day(2027, 7, 1),
            end_date: day(2027, 12, 31),
            annual_rent: 10_000_000,
            deposit_amount: 500_000,
            ..Default::default()
        };
        // Two instalments are left on the current plan (1 July, 1 October).
        let rest = vec![
            charge(day(2027, 7, 1), RENT),
            charge(day(2027, 10, 1), RENT),
        ];
        assert_eq!(
            compare(&amendment, &previous, &plan(2027), &rest),
            Changes::default()
        );
        let one = vec![charge(day(2027, 7, 1), RENT)];
        assert!(compare(&amendment, &previous, &plan(2027), &one).fewer_cheques);
    }

    #[test]
    fn a_no_is_the_reason_of_the_submit_run() {
        let mut lease = Lease {
            terms_answer_reason: Some(" typo fix ".into()),
            ..Default::default()
        };
        assert_eq!(history_reason(&lease), None, "not answered");
        lease.terms_answered_no = true;
        assert_eq!(history_reason(&lease).as_deref(), Some("typo fix"));
    }
}
