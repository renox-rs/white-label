//! Renewing a lease without billing a period twice (`renew`, 01 Property §7.1,
//! §4.3 row "holding over"; BL-PRP-07).
//!
//! A renewal version is activated while the version it renews is
//! `renewal_due` or `expired`. The activation runs `renew` on the old version
//! in its own transaction ([`crate::lease_checks::apply_activation`]); the
//! effect ([`renew_effects`]) does three things:
//!
//! - **Links the versions both ways:** the new version already points back
//!   with `previous_id`; the old one gets `next_id`.
//! - **Cancels the old version's scheduled charges that the new one covers:**
//!   those due on or after the new start date. A charge already invoiced is
//!   never touched, so the invoices of a holding-over period stay and are not
//!   credited or voided.
//! - **Does not bill a period twice:** a charge of the new version whose
//!   period overlaps a period the old version already invoiced is cancelled
//!   (the new schedule "skips" it). A period runs from a charge's due date to
//!   the day before the next charge of the same type, the last one to the end
//!   date of its version ([`periods`]).
//!
//! A different rent for a period already invoiced is not adjusted by the
//! platform: [`differences`] lists the difference per period and the page of
//! the new version shows it, so that the property manager can add a charge or
//! a credit by hand. The cheques linked to charges that were cancelled stay
//! `received`; after the commit the accountant gets one task to relink them to
//! the new version's charges ([`after_renew`]).
//!
//! Decided where the spec is silent:
//!
//! - A charge whose period straddles the start date counts as covered when
//!   its due date is on or after the start date (no proration).
//! - Charge types are compared as they are: a rent charge only overlaps a
//!   rent charge.

use std::collections::BTreeMap;

use renox::chrono::{Duration, NaiveDate};
use renox::db::Transaction;
use renox::prelude::*;
use wl_kernel::cheques::{ChequeStatus, ChequeSummary, cheques};
use wl_kernel::role_tasks::RoleTask;

use crate::charge::{CHARGE_SUBJECT_TYPE, ChargeStatus, LeaseCharge};
use crate::db::{refused, set_lease};
use crate::lease::{Lease, LeaseStatus, VersionKind};
use crate::permissions::ACCOUNTANT_ROLE;
use crate::texts::english;

/// The period each of `charges` stands for, by charge id: from its due date
/// to the day before the next charge of the same type, the last one to
/// `last_end` (the end date of its version) or its own due date, whichever
/// is later.
pub fn periods<'a>(
    charges: impl IntoIterator<Item = &'a LeaseCharge>,
    last_end: NaiveDate,
) -> BTreeMap<i64, (NaiveDate, NaiveDate)> {
    let mut by_type: BTreeMap<&str, Vec<&LeaseCharge>> = BTreeMap::new();
    for charge in charges {
        by_type
            .entry(charge.charge_type.as_str())
            .or_default()
            .push(charge);
    }
    let mut found = BTreeMap::new();
    for list in by_type.values_mut() {
        list.sort_by_key(|c| (c.due_date, c.id));
        for (index, charge) in list.iter().enumerate() {
            let end = list
                .get(index + 1)
                .filter(|next| next.due_date > charge.due_date)
                .map_or_else(
                    || last_end.max(charge.due_date),
                    |next| next.due_date - Duration::days(1),
                );
            found.insert(charge.id, (charge.due_date, end));
        }
    }
    found
}

/// Whether the periods `a` and `b` (first and last day) share a day.
fn overlap(a: (NaiveDate, NaiveDate), b: (NaiveDate, NaiveDate)) -> bool {
    a.0 <= b.1 && b.0 <= a.1
}

/// The charges of the old version that count as periods of its plan once the
/// renewal starts on `start`: the live ones and those `renew` cancelled (due
/// on or after `start`), but not those cancelled earlier.
fn plan_of(charges: &[LeaseCharge], start: NaiveDate) -> Vec<&LeaseCharge> {
    charges
        .iter()
        .filter(|c| c.status != ChargeStatus::Cancelled || c.due_date >= start)
        .collect()
}

/// The charges of `new` that fall in a period `old_charges` already
/// invoiced, each with the invoiced charge it overlaps. With `cancelled_only`
/// only a charge of `new` that is cancelled counts (the ones `renew` skipped);
/// otherwise every charge given does.
fn skipped<'a>(
    old: &Lease,
    old_charges: &'a [LeaseCharge],
    new: &Lease,
    new_charges: &'a [LeaseCharge],
    cancelled_only: bool,
) -> Vec<(&'a LeaseCharge, &'a LeaseCharge)> {
    let old_periods = periods(plan_of(old_charges, new.start_date), old.end_date);
    let new_periods = periods(new_charges.iter(), new.end_date);
    let mut found = Vec::new();
    for charge in new_charges {
        if cancelled_only && charge.status != ChargeStatus::Cancelled {
            continue;
        }
        let Some(mine) = new_periods.get(&charge.id) else {
            continue;
        };
        let invoiced = old_charges.iter().find(|o| {
            o.status == ChargeStatus::Invoiced
                && o.charge_type == charge.charge_type
                && old_periods
                    .get(&o.id)
                    .is_some_and(|theirs| overlap(*theirs, *mine))
        });
        if let Some(invoiced) = invoiced {
            found.push((charge, invoiced));
        }
    }
    found
}

/// The effect of `renew` inside its transaction. `next` is the renewal
/// version that was activated; without it the `active` renewal version of
/// `old` is looked for.
///
/// # Errors
///
/// 422 when no renewal version of `old` is `active`; the database's errors.
pub(crate) async fn renew_effects(tx: &mut Transaction, old: &Lease, next: Option<i64>) -> Result {
    let found = match next {
        Some(id) => Lease::find(&mut *tx, id).await?,
        None => {
            Lease::query()
                .where_(Lease::PREVIOUS_ID.eq(old.id))
                .where_(Lease::VERSION_KIND.eq(VersionKind::Renewal))
                .where_(Lease::STATUS.eq(LeaseStatus::Active))
                .first(&mut *tx)
                .await?
        }
    };
    let new = found
        .filter(|n| {
            n.previous_id == Some(old.id)
                && n.version_kind == VersionKind::Renewal
                && n.status == LeaseStatus::Active
        })
        .ok_or_else(|| refused(english("property.renew.no_renewal", &[])))?;

    set_lease(&mut *tx, old.id, &[("next_id", &Some(new.id))]).await?;

    // Periods are read before anything is cancelled.
    let old_charges = LeaseCharge::query()
        .where_(LeaseCharge::LEASE_ID.eq(old.id))
        .order_by(LeaseCharge::DUE_DATE)
        .order_by(LeaseCharge::ID)
        .get(&mut *tx)
        .await?;
    let new_charges = LeaseCharge::query()
        .where_(LeaseCharge::LEASE_ID.eq(new.id))
        .where_(LeaseCharge::STATUS.ne(ChargeStatus::Cancelled))
        .order_by(LeaseCharge::DUE_DATE)
        .order_by(LeaseCharge::ID)
        .get(&mut *tx)
        .await?;
    let ids: Vec<i64> = skipped(old, &old_charges, &new, &new_charges, false)
        .iter()
        .map(|(charge, _)| charge.id)
        .collect();

    // The old version's scheduled charges from the start date on.
    LeaseCharge::query()
        .where_(LeaseCharge::LEASE_ID.eq(old.id))
        .where_(LeaseCharge::STATUS.is_in([ChargeStatus::Planned, ChargeStatus::Scheduled]))
        .where_(LeaseCharge::DUE_DATE.gte(new.start_date))
        .update(
            &mut *tx,
            &[
                ("status", &ChargeStatus::Cancelled),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    // The new version's charges for periods that are already invoiced.
    if !ids.is_empty() {
        LeaseCharge::query()
            .where_(LeaseCharge::ID.is_in(ids))
            .update(
                &mut *tx,
                &[
                    ("status", &ChargeStatus::Cancelled),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
    }
    Ok(())
}

/// A period the old version had invoiced before the renewal started, and
/// what the renewal charges for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Difference {
    /// The first day of the period (the due date of the charge).
    pub period: NaiveDate,
    /// The amount the old version invoiced, before VAT, in fils.
    pub invoiced: i64,
    /// The amount the renewal's schedule has for the same period, in fils.
    pub renewal: i64,
}

impl Difference {
    /// What the renewal charges more (positive) or less (negative) than was
    /// invoiced, in fils.
    pub fn difference(&self) -> i64 {
        self.renewal - self.invoiced
    }
}

/// The difference per period already invoiced for the renewal version
/// `lease`, oldest period first; empty for any other version. The platform
/// adjusts nothing: the property manager adds a charge or a credit by hand.
///
/// # Errors
///
/// The database's errors.
pub async fn differences(db: &Db, lease: &Lease) -> Result<Vec<Difference>> {
    if lease.version_kind != VersionKind::Renewal {
        return Ok(Vec::new());
    }
    let Some(old) = lease.previous(db).await? else {
        return Ok(Vec::new());
    };
    let old_charges = old.charges(db).await?;
    // The charges `renew` skipped are the cancelled ones that fall in a
    // period already invoiced; a period is shown once.
    let new_charges = lease.charges(db).await?;
    let mut found = skipped(&old, &old_charges, lease, &new_charges, true);
    found.sort_by_key(|(renewal, _)| renewal.id);
    let mut rows: BTreeMap<NaiveDate, Difference> = BTreeMap::new();
    for (renewal, invoiced) in found {
        rows.insert(
            renewal.due_date,
            Difference {
                period: renewal.due_date,
                invoiced: invoiced.amount,
                renewal: renewal.amount,
            },
        );
    }
    Ok(rows.into_values().collect())
}

/// Whether any of `rows` is a real difference.
pub fn any_difference(rows: &[Difference]) -> bool {
    rows.iter().any(|row| row.difference() != 0)
}

/// The cheques of the lease that are linked to a charge that `renew`
/// cancelled and are still `received` or `due_for_deposit`: they stay as they
/// are and wait to be relinked.
fn to_relink(all: Vec<ChequeSummary>, cancelled: &[i64]) -> Vec<ChequeSummary> {
    all.into_iter()
        .filter(|cheque| {
            matches!(
                cheque.status,
                ChequeStatus::Received | ChequeStatus::DueForDeposit
            ) && cheque.linked.iter().any(|link| {
                link.subject_type == CHARGE_SUBJECT_TYPE && cancelled.contains(&link.subject_id)
            })
        })
        .collect()
}

/// After the commit of the activation of the renewal version `new`: the
/// accountant gets one task to relink the cheques whose charges were
/// cancelled to the charges of the new version. Nothing when there is none.
///
/// # Errors
///
/// The database's, the cheque register's and ticketing's errors.
pub(crate) async fn after_renew(state: &AppState, new: &Lease) -> Result {
    if new.version_kind != VersionKind::Renewal {
        return Ok(());
    }
    let Some(old) = new.previous(&state.db).await? else {
        return Ok(());
    };
    let mut cancelled: Vec<i64> = old
        .charges(&state.db)
        .await?
        .iter()
        .filter(|c| c.status == ChargeStatus::Cancelled && c.due_date >= new.start_date)
        .map(|c| c.id)
        .collect();
    cancelled.extend(
        new.charges(&state.db)
            .await?
            .iter()
            .filter(|c| c.status == ChargeStatus::Cancelled)
            .map(|c| c.id),
    );
    if cancelled.is_empty() {
        return Ok(());
    }
    let all = cheques(state)
        .of_agreement(&state.db, &new.agreement())
        .await?;
    let waiting = to_relink(all, &cancelled);
    if waiting.is_empty() {
        return Ok(());
    }
    let list = waiting
        .iter()
        .map(|c| {
            format!(
                "{} ({}, {}, {})",
                c.number,
                c.bank,
                c.date,
                crate::money_text(c.amount)
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    let title = english(
        "property.tasks.relink_cheques",
        &[("lease", &new.number_with_version())],
    );
    let task =
        RoleTask::new(new.subject(), ACCOUNTANT_ROLE, &title, &relink_key(new)).description(&list);
    crate::tasks::open(state, &new.scope(), task).await?;
    Ok(())
}

/// The key the relink task of the renewal version `lease` is found by.
pub fn relink_key(lease: &Lease) -> String {
    format!("property.relink_cheques:{}", lease.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::VatCategory;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn monthly(first: NaiveDate, months: u32, first_id: i64) -> Vec<LeaseCharge> {
        (0..months)
            .map(|i| LeaseCharge {
                id: first_id + i64::from(i),
                charge_type: "rent".into(),
                due_date: first
                    .checked_add_months(renox::chrono::Months::new(i))
                    .unwrap(),
                amount: 1_000_000,
                vat_category: VatCategory::Exempt,
                status: ChargeStatus::Scheduled,
                ..Default::default()
            })
            .collect()
    }

    #[test]
    fn a_period_runs_to_the_day_before_the_next_charge() {
        let charges = monthly(day(2028, 1, 1), 3, 1);
        let found = periods(charges.iter(), day(2028, 3, 31));
        assert_eq!(found[&1], (day(2028, 1, 1), day(2028, 1, 31)));
        assert_eq!(found[&2], (day(2028, 2, 1), day(2028, 2, 29)));
        assert_eq!(found[&3], (day(2028, 3, 1), day(2028, 3, 31)));
    }

    #[test]
    fn charges_of_other_types_do_not_end_a_period() {
        let mut charges = monthly(day(2027, 1, 1), 2, 1);
        charges.push(LeaseCharge {
            id: 9,
            charge_type: "parking".into(),
            due_date: day(2027, 1, 15),
            ..Default::default()
        });
        let found = periods(charges.iter(), day(2027, 2, 28));
        assert_eq!(found[&1], (day(2027, 1, 1), day(2027, 1, 31)));
        assert_eq!(found[&9].0, day(2027, 1, 15));
    }

    #[test]
    fn a_back_dated_renewal_skips_the_periods_already_invoiced() {
        let end = day(2027, 12, 31);
        let old = Lease {
            id: 1,
            end_date: end,
            ..Default::default()
        };
        let new = Lease {
            id: 2,
            start_date: day(2028, 1, 1),
            end_date: day(2028, 12, 31),
            ..Default::default()
        };
        // Holding-over charges of 2028: January and February invoiced.
        let mut old_charges = monthly(day(2028, 1, 1), 12, 100);
        old_charges[0].status = ChargeStatus::Invoiced;
        old_charges[1].status = ChargeStatus::Invoiced;
        for charge in &mut old_charges[2..] {
            // Cancelled by `renew` just now.
            charge.status = ChargeStatus::Cancelled;
        }
        let new_charges = monthly(day(2028, 1, 1), 12, 200);
        let found = skipped(&old, &old_charges, &new, &new_charges, false);
        let ids: Vec<i64> = found.iter().map(|(charge, _)| charge.id).collect();
        assert_eq!(ids, [200, 201]);
        assert_eq!(found[0].1.id, 100);
    }

    #[test]
    fn a_renewal_that_starts_the_day_after_the_end_skips_nothing() {
        let old = Lease {
            id: 1,
            end_date: day(2027, 12, 31),
            ..Default::default()
        };
        let new = Lease {
            id: 2,
            start_date: day(2028, 1, 1),
            end_date: day(2028, 12, 31),
            ..Default::default()
        };
        let mut old_charges = monthly(day(2027, 1, 1), 12, 100);
        for charge in &mut old_charges {
            charge.status = ChargeStatus::Invoiced;
        }
        let new_charges = monthly(day(2028, 1, 1), 12, 200);
        assert!(skipped(&old, &old_charges, &new, &new_charges, false).is_empty());
    }

    #[test]
    fn the_difference_is_what_the_renewal_charges_over_what_was_invoiced() {
        let row = Difference {
            period: day(2028, 1, 1),
            invoiced: 1_000_000,
            renewal: 1_100_000,
        };
        assert_eq!(row.difference(), 100_000);
        assert!(any_difference(&[row]));
        assert!(!any_difference(&[]));
    }
}
