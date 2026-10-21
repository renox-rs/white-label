//! Charge capture (05 §4.2 step 5, `CLN.ENC.01`; base for BL-CLN-05 and
//! BL-CLN-06): the practitioner records the services delivered, the desk the
//! consumables issued, and the visit is priced and split when it ends
//! ([`crate::billing`]).
//!
//! - [`record_service`] and [`record_consumable`] add a line to an open
//!   visit. A service of a *self-pay* visit must be on the signed consent
//!   ([`crate::consent::ensure_covered`]); an updated consent
//!   ([`crate::consent::sign_updated`]) is the way to add one. On an insured
//!   visit, the units of a service that needs a prior authorisation
//!   ([`crate::auth_check::plan`]) that no authorisation covers go self-pay
//!   and must be on the consent too.
//! - [`mark_none`] says nothing was done (`SCH.ACL.06`); it is refused once a
//!   line exists. [`accept_work`] records that the patient accepts the work
//!   done after the clinic stopped the visit. [`mark_not_suitable`] takes
//!   the procedure out and records the consultation given instead
//!   (BL-CLN-06). [`refund_deposit_in_full`] is the front desk's choice for
//!   the deposit of such a visit.
//! - [`quote`] prices the lines and divides them between the patient and the
//!   payer by the payer contract ([`crate::contracts`], [`crate::split`]);
//!   the page shows it and the bill stores it.
//! - [`ChargeWork`] is the app's [`WorkDoneStatus`]: wl-scheduling cancels a
//!   checked-in booking only once it says *nothing* or *captured*.
//!
//! Decisions:
//!
//! - A line is priced when it is recorded, on the visit date: the payer's
//!   tariff for an insured service (else the self-pay price list), the
//!   self-pay price list otherwise. The bill uses that price, not the
//!   estimate of the consent.
//! - A service line *is* the practitioner's record that it was delivered; a
//!   booked service that is not recorded is not billed.
//! - `Captured.amount` for wl-scheduling is the **patient share** of the
//!   work done: the deposit is applied to what the patient owes, never to
//!   the payer's part.
//! - Nothing here moves stock: the issue of a consumable from a lot is
//!   BL-CLN-13's (`STK.MOV.01`); the line is the charge.

use std::collections::BTreeMap;

use renox::db::{Json, now};
use renox::prelude::*;
use wl_kernel::documents::today;
use wl_kernel::sales::self_pay_prices;
use wl_kernel::scheduling::{BookingId, SchedulingFuture, WorkDone, WorkDoneStatus};
use wl_kernel::Subject;

use crate::appointment::Appointment;
use crate::auth_check::{self, Need};
use crate::charge::{ChargeKind, ChargeLine};
use crate::consent::{self, SelfPayConsent};
use crate::contract::{PayerContract, PayerTariff};
use crate::contracts;
use crate::coverage::{CoverageOutcome, PendingCheckIn, VisitService};
use crate::db::refused;
use crate::policy::InsurancePolicy;
use crate::split::{self, Part};
use crate::texts::english;
use crate::visit::Visit;

/// A line to record.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct NewCharge {
    /// The service or consumable code.
    pub code: String,
    /// The units; at least one.
    pub units: i64,
    /// Standard rated (5%); otherwise zero-rated.
    pub vat_standard: bool,
}

impl NewCharge {
    /// `units` of `code`, zero-rated.
    pub fn new(code: &str, units: i64) -> Self {
        NewCharge {
            code: code.trim().to_owned(),
            units,
            vat_standard: false,
        }
    }

    /// Standard rated (5%).
    pub fn standard_rated(mut self) -> Self {
        self.vat_standard = true;
        self
    }
}

/// The visit `id` when it can still change.
async fn open_visit(state: &AppState, id: i64) -> Result<Visit> {
    let visit = Visit::find_or_404(&state.db, id).await?;
    if visit.billed_at.is_some() || !visit.status.is_open() {
        return Err(refused(english("clinics.charges.problem.closed", &[])));
    }
    Ok(visit)
}

/// The policy a visit's insured part is read from: the policy of the split
/// for *insured* and *re-split*; none for *self-pay* and for a visit without
/// a policy.
pub async fn insured_policy(state: &AppState, visit: &Visit) -> Result<Option<InsurancePolicy>> {
    if visit.coverage_outcome == Some(CoverageOutcome::SelfPay) {
        return Ok(None);
    }
    match visit.coverage_policy_id.or(visit.policy_id) {
        Some(id) => InsurancePolicy::find(&state.db, id).await,
        None => Ok(None),
    }
}

/// The prior-authorisation needs of the visit's service lines, with `extra`
/// units of a service added: the plan of `auth_check` for exactly what was
/// delivered.
async fn needs_for(
    state: &AppState,
    visit: &Visit,
    lines: &[ChargeLine],
    extra: Option<(&str, i64)>,
) -> Result<Vec<Need>> {
    let Some(appointment) = Appointment::of_booking(&state.db, visit.booking_id).await? else {
        return Ok(Vec::new());
    };
    let mut services: Vec<VisitService> = Vec::new();
    let mut wanted: Vec<(&str, i64)> = lines
        .iter()
        .filter(|l| l.kind == ChargeKind::Service)
        .map(|l| (l.code.as_str(), l.units))
        .collect();
    wanted.extend(extra);
    for (code, units) in wanted {
        match services.iter().position(|s| s.code == code) {
            Some(index) => services[index].units += units,
            None => services.push(VisitService::new(code, units)),
        }
    }
    if services.is_empty() {
        // `plan` reads no services as the booked one: here none were
        // delivered, so none needs an authorisation.
        return Ok(Vec::new());
    }
    let pending = PendingCheckIn {
        booking_id: visit.booking_id,
        coverage_policy_id: visit.coverage_policy_id.or(visit.policy_id),
        outcome: visit.coverage_outcome,
        services: Json(services),
        ..PendingCheckIn::default()
    };
    auth_check::plan(state, &appointment, Some(&pending)).await
}

/// Records a line of `kind`.
async fn record(
    state: &AppState,
    by: Option<i64>,
    visit_id: i64,
    kind: ChargeKind,
    new: NewCharge,
) -> Result<ChargeLine> {
    let visit = open_visit(state, visit_id).await?;
    if new.code.is_empty() || new.units < 1 {
        return Err(refused(english("clinics.charges.problem.line", &[])));
    }
    if kind == ChargeKind::Service {
        // `CLN.ACL.15`: a service not on the signed self-pay consent waits
        // for an updated one.
        consent::ensure_covered(state, visit.id, &new.code).await?;
    }
    let listed = self_pay_prices(state)
        .price_on(&state.db, &new.code, visit.visit_date)
        .await?;
    let no_price = || {
        refused(english(
            "clinics.consent.problem.no_price",
            &[("service", &new.code)],
        ))
    };
    let policy = insured_policy(state, &visit).await?;
    let unit_price = match &policy {
        Some(policy) => {
            let contract = contracts::for_policy(&state.db, policy, visit.visit_date)
                .await?
                .ok_or_else(|| refused(english("clinics.charges.problem.no_contract", &[])))?;
            let tariff = PayerTariff::of(&state.db, contract.id, &new.code).await?;
            tariff
                .and_then(|t| t.price)
                .or(listed)
                .ok_or_else(no_price)?
        }
        None => listed.ok_or_else(no_price)?,
    };
    if kind == ChargeKind::Service && policy.is_some() {
        // Units no authorisation covers go self-pay and need the consent.
        let existing = ChargeLine::active_of(&state.db, visit.id).await?;
        let needs = needs_for(state, &visit, &existing, Some((new.code.as_str(), new.units))).await?;
        for need in needs.iter().filter(|n| n.code == new.code && n.self_pay > 0) {
            let consent = match visit.consent_id {
                Some(id) => SelfPayConsent::find(&state.db, id).await?,
                None => None,
            };
            let signed = consent
                .is_some_and(|c| !c.is_superseded() && c.units_of(&need.code) >= need.self_pay);
            if !signed {
                return Err(refused(english(
                    "clinics.consent.problem.not_on_consent",
                    &[("service", &new.code)],
                )));
            }
        }
    }
    let mut line = ChargeLine {
        visit_id: visit.id,
        kind,
        code: new.code,
        units: new.units,
        unit_price,
        list_price: unit_price,
        self_pay_price: listed.unwrap_or(unit_price),
        vat_standard: new.vat_standard,
        recorded_by: by,
        ..ChargeLine::default()
    };
    line.save(&state.db).await?;
    // Something was done: *none* no longer holds.
    Visit::query()
        .where_(Visit::ID.eq(visit.id))
        .update(
            &state.db,
            &[("work_none", &false), ("updated_at", &now())],
        )
        .await?;
    Ok(line)
}

/// The practitioner records a service as delivered.
///
/// # Errors
///
/// A 422 for a visit that is over or billed, a blank code or fewer than one
/// unit, a service with no price, an insured visit without a payer contract,
/// and a service (or units) that is not on the self-pay consent.
pub async fn record_service(
    state: &AppState,
    by: Option<i64>,
    visit: i64,
    new: NewCharge,
) -> Result<ChargeLine> {
    record(state, by, visit, ChargeKind::Service, new).await
}

/// The desk records a consumable issued to the visit, at its price.
///
/// # Errors
///
/// As [`record_service`], except the consent: a consumable is not a service.
pub async fn record_consumable(
    state: &AppState,
    by: Option<i64>,
    visit: i64,
    new: NewCharge,
) -> Result<ChargeLine> {
    record(state, by, visit, ChargeKind::Consumable, new).await
}

/// Says nothing was delivered and no consumable issued (`SCH.ACL.06`).
///
/// # Errors
///
/// A 422 when a line exists: a service marked delivered or a consumable
/// issued, or for a visit that is over.
pub async fn mark_none(state: &AppState, visit: i64) -> Result<Visit> {
    let visit = open_visit(state, visit).await?;
    if !ChargeLine::active_of(&state.db, visit.id).await?.is_empty() {
        return Err(refused(english("clinics.charges.problem.none_with_work", &[])));
    }
    Visit::query()
        .where_(Visit::ID.eq(visit.id))
        .update(&state.db, &[("work_none", &true), ("updated_at", &now())])
        .await?;
    Visit::find_or_404(&state.db, visit.id).await
}

/// Records that the patient accepts to pay for the work done, after the
/// clinic stopped the visit (05 §4.2): the name of `by` and the time are kept.
///
/// # Errors
///
/// A 422 for a visit that is over.
pub async fn accept_work(state: &AppState, by: Option<i64>, visit: i64) -> Result<Visit> {
    let visit = open_visit(state, visit).await?;
    if visit.work_accepted_at.is_none() {
        Visit::query()
            .where_(Visit::ID.eq(visit.id))
            .update(
                &state.db,
                &[
                    ("work_accepted_by", &by),
                    ("work_accepted_at", &Some(now())),
                    ("updated_at", &now()),
                ],
            )
            .await?;
    }
    Visit::find_or_404(&state.db, visit.id).await
}

/// The practitioner decides the planned procedure is not suitable
/// (BL-CLN-06): the booked service is taken out of charge capture, the
/// `consultation` given is recorded in its place and the visit keeps that
/// the procedure was not performed (never why). The appointment is then
/// completed as usual.
///
/// # Errors
///
/// A 422 without a consultation service, or when the consultation is the
/// booked service itself; the errors of [`record_service`].
pub async fn mark_not_suitable(
    state: &AppState,
    by: Option<i64>,
    visit: i64,
    consultation: NewCharge,
) -> Result<ChargeLine> {
    let visit = open_visit(state, visit).await?;
    if consultation.code.is_empty() || consultation.code == visit.service_code {
        return Err(refused(english(
            "clinics.charges.problem.consultation_needed",
            &[],
        )));
    }
    let line = record(state, by, visit.id, ChargeKind::Service, consultation).await?;
    ChargeLine::query()
        .where_(ChargeLine::VISIT_ID.eq(visit.id))
        .where_(ChargeLine::CODE.eq(visit.service_code.as_str()))
        .where_(ChargeLine::REMOVED_ON.is_null())
        .update(
            &state.db,
            &[
                ("removed_on", &Some(today())),
                ("removed_reason", &Some("not_suitable".to_owned())),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Visit::query()
        .where_(Visit::ID.eq(visit.id))
        .update(
            &state.db,
            &[("not_suitable", &true), ("updated_at", &now())],
        )
        .await?;
    Ok(line)
}

/// The front desk refunds the deposit in full instead of applying it to the
/// patient share (the payer covers the consultation, or the clinic waives
/// it). Never forfeited.
///
/// # Errors
///
/// A 422 for a visit that is over.
pub async fn refund_deposit_in_full(state: &AppState, visit: i64) -> Result<Visit> {
    let visit = open_visit(state, visit).await?;
    Visit::query()
        .where_(Visit::ID.eq(visit.id))
        .update(
            &state.db,
            &[("deposit_refund_full", &true), ("updated_at", &now())],
        )
        .await?;
    Visit::find_or_404(&state.db, visit.id).await
}

/// The lines priced and divided between the patient and the payer.
#[derive(Debug, Clone)]
pub struct Quote {
    /// The active lines, in order, with the units split into covered and
    /// self-pay and the parts of the patient and the payer filled in.
    pub lines: Vec<ChargeLine>,
    /// What the patient pays, in fils.
    pub patient_total: i64,
    /// What the payer is claimed for, in fils.
    pub payer_total: i64,
    /// The prior-authorisation needs of the service lines, with the
    /// authorisations the covered units come from.
    pub needs: Vec<Need>,
    /// The payer contract the insured part follows.
    pub contract: Option<PayerContract>,
}

/// Prices and divides the active lines of `visit` (`CLN.ENC.02`).
///
/// - A visit with no insured policy (self-pay, or not insured): every unit
///   is the patient's at the self-pay price.
/// - An insured visit: the covered units follow the payer contract of the
///   policy of the split; the units of a service that needs a prior
///   authorisation and that no authorisation covers go self-pay.
///
/// # Errors
///
/// A 422 for an insured visit without a payer contract; the database's
/// errors.
pub async fn quote(state: &AppState, visit: &Visit) -> Result<Quote> {
    let mut lines = ChargeLine::active_of(&state.db, visit.id).await?;
    let policy = insured_policy(state, visit).await?;
    let Some(policy) = policy else {
        let mut patient_total = 0;
        for line in &mut lines {
            line.self_pay_units = line.units;
            line.authorised_units = 0;
            line.patient_share = line.self_pay_total();
            line.payer_share = 0;
            patient_total += line.patient_share;
        }
        return Ok(Quote {
            lines,
            patient_total,
            payer_total: 0,
            needs: Vec::new(),
            contract: None,
        });
    };
    let contract = contracts::for_policy(&state.db, &policy, visit.visit_date)
        .await?
        .ok_or_else(|| refused(english("clinics.charges.problem.no_contract", &[])))?;
    let needs = needs_for(state, visit, &lines, None).await?;
    let mut authorised: BTreeMap<String, i64> = needs
        .iter()
        .map(|need| (need.code.clone(), need.covered()))
        .collect();
    let mut parts = Vec::with_capacity(lines.len());
    for line in &mut lines {
        let needs_authorisation = line.kind == ChargeKind::Service
            && needs.iter().any(|need| need.code == line.code);
        let covered = if needs_authorisation {
            let left = authorised.entry(line.code.clone()).or_insert(0);
            let taken = line.units.min(*left);
            *left -= taken;
            taken
        } else {
            line.units
        };
        line.authorised_units = if needs_authorisation { covered } else { 0 };
        line.self_pay_units = line.units - covered;
        let excluded = PayerTariff::of(&state.db, contract.id, &line.code)
            .await?
            .is_some_and(|tariff| !tariff.covered);
        parts.push(Part {
            covered: line.covered_total(),
            self_pay: line.self_pay_total(),
            excluded,
        });
    }
    let shares = split::split(&parts, contract.terms());
    let (mut patient_total, mut payer_total) = (0, 0);
    for (line, share) in lines.iter_mut().zip(shares) {
        line.patient_share = share.patient;
        line.payer_share = share.payer;
        patient_total += share.patient;
        payer_total += share.payer;
    }
    Ok(Quote {
        lines,
        patient_total,
        payer_total,
        needs,
        contract: Some(contract),
    })
}

/// What the visit of `booking` has done, as wl-scheduling asks (`SCH.ACL.06`):
///
/// - no visit: nothing;
/// - no line and not marked *none*: pending, so the booking cannot be
///   cancelled yet;
/// - no line and marked *none*: nothing;
/// - lines: captured, worth the patient share (the total of the lines when
///   the share cannot be worked out, for example without a payer contract).
///
/// # Errors
///
/// The database's errors.
pub async fn work_done(state: &AppState, booking: BookingId) -> Result<WorkDone> {
    let Some(visit) = Visit::of_booking(&state.db, booking.0).await? else {
        return Ok(WorkDone::Nothing);
    };
    let lines = ChargeLine::active_of(&state.db, visit.id).await?;
    if lines.is_empty() {
        return Ok(if visit.work_none {
            WorkDone::Nothing
        } else {
            WorkDone::Pending
        });
    }
    let amount = match quote(state, &visit).await {
        Ok(quoted) => quoted.patient_total,
        Err(_) => lines.iter().map(|l| l.units * l.unit_price).sum(),
    };
    Ok(WorkDone::Captured { amount })
}

/// The app's [`WorkDoneStatus`]: the answer comes from the charge lines of
/// the visit the check-in opened.
#[derive(Debug, Clone, Copy, Default)]
pub struct ChargeWork;

impl WorkDoneStatus for ChargeWork {
    fn status<'a>(
        &'a self,
        state: &'a AppState,
        booking: BookingId,
        _record: Option<Subject>,
    ) -> SchedulingFuture<'a, WorkDone> {
        Box::pin(async move { work_done(state, booking).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_charge_is_zero_rated_unless_it_says_otherwise() {
        let charge = NewCharge::new(" laser ", 2);
        assert_eq!(charge.code, "laser");
        assert!(!charge.vat_standard);
        assert!(charge.standard_rated().vat_standard);
    }
}
