//! Billing a visit (05 §4.2 steps 6 and 7, `CLN.ENC.02`; base for BL-CLN-05
//! and BL-CLN-06): when the visit is completed, or cancelled after the
//! check-in, its charge lines are priced, divided between the patient and the
//! payer, and billed through the kernel contracts.
//!
//! What a bill does, in this order, each step once (the visit remembers what
//! is done, so a bill that stopped half way is finished by running it
//! again):
//!
//! 1. A visit stopped by the **clinic** without the patient's acceptance
//!    ([`Basis::Cancelled`] with `business`) charges nothing: the lines stay,
//!    at price 0 with the reason *clinic stop* (stock and cost reports stay
//!    right), and neither an invoice nor a claim is made.
//! 2. A line of a self-pay service is paid by the patient's prepaid package
//!    when it has credit (`PackageLedger`); only a service recorded as
//!    delivered consumes a session.
//! 3. The lines are priced and divided ([`crate::charges::quote`]); each line
//!    keeps its parts.
//! 4. The patient's share is drafted as an invoice from the visit through the
//!    kernel's `Invoicer` and issued at once (the host lets the system issue
//!    the `clinics.visit` source), so that the deposit can be allocated.
//! 5. The payer's share becomes a draft payer bill (`PayerBills::create`)
//!    under the payer agreement of the contract, and the clinic's claim wraps
//!    it ([`crate::claims::create_for_visit`], `CLN.CLM.01`): the claim to
//!    prepare, validate and submit.
//! 6. The units of the prior authorisations the covered services used are
//!    consumed ([`crate::authorisations::consume`]).
//! 7. The deposit: on a cancellation wl-scheduling decided (`SCH.ACL.05`) how
//!    much is applied to the work and refunded the rest; here that part is
//!    allocated to the invoice, and what the bill cannot use (a package paid
//!    the service) is refunded. On a completion the deposit is applied to the
//!    patient share first and the remainder is refunded, or all of it is
//!    refunded when the desk chose so. It is never forfeited.

use renox::db::now;
use renox::prelude::*;
use wl_kernel::invoicing::{
    DraftLine, InvoiceDraft, InvoiceId, InvoiceStatus, VatCategory, invoicer,
};
use wl_kernel::payer_bills::{NewPayerBill, NewPayerBillLine, PayerBillId, payer_bills};
use wl_kernel::payments::{
    AllocationTarget, PaymentStatus, RefundRequest, RefundSource, payment_recorder,
};
use wl_kernel::sales::{ConsumePackage, PackageConsumption, package_ledger};
use wl_kernel::{LegalEntity, PartyId};

use crate::authorisation::PriorAuthorisation;
use crate::authorisations;
use crate::charge::{ChargeKind, ChargeLine};
use crate::charges;
use crate::claim::Claim;
use crate::claims;
use crate::db::refused;
use crate::texts::english;
use crate::visit::Visit;

/// Why the visit is billed now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    /// The appointment was completed.
    Completed,
    /// The booking was cancelled after the check-in.
    Cancelled {
        /// The clinic's side stopped the visit (a business-initiated reason).
        business: bool,
        /// The part of the deposit wl-scheduling applied to the work done,
        /// in fils (a party-initiated cancellation).
        deposit_applied: i64,
    },
}

/// What a bill came to.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Billed {
    /// What the patient pays, in fils.
    pub patient_total: i64,
    /// What the payer is claimed for, in fils.
    pub payer_total: i64,
    /// The patient's invoice.
    pub invoice: Option<i64>,
    /// The draft payer bill.
    pub payer_bill: Option<i64>,
    /// The deposit applied to the invoice, in fils.
    pub deposit_applied: i64,
    /// The deposit asked back, in fils.
    pub deposit_refunded: i64,
}

impl Basis {
    /// The basis a visit that is over was billed, or is to be billed, on.
    /// `None` while the visit is open.
    pub fn of_visit(visit: &Visit) -> Option<Basis> {
        match visit.status {
            crate::visit::VisitStatus::Completed => Some(Basis::Completed),
            crate::visit::VisitStatus::Cancelled => Some(Basis::Cancelled {
                business: visit.clinic_stop,
                deposit_applied: visit.scheduling_applied,
            }),
            _ => None,
        }
    }
}

impl Billed {
    /// What the visit recorded of its bill.
    pub fn of(visit: &Visit) -> Billed {
        Billed {
            patient_total: visit.patient_total,
            payer_total: visit.payer_total,
            invoice: visit.invoice_id,
            payer_bill: visit.payer_bill_id,
            deposit_applied: visit.deposit_applied,
            deposit_refunded: visit.deposit_refunded,
        }
    }
}

fn vat(line: &ChargeLine) -> VatCategory {
    if line.vat_standard {
        VatCategory::Standard
    } else {
        VatCategory::ZeroRated
    }
}

/// `kind code x units`, for an invoice or a claim.
fn describe(line: &ChargeLine) -> String {
    format!("{} x {}", line.code, line.units)
}

/// Bills the visit `id`. A visit billed already, or being billed by another
/// run, returns what it recorded.
///
/// # Errors
///
/// A 422 when an insured visit has no payer contract or agreement, when
/// invoicing is off while something is to be billed, and the errors of the
/// contracts it calls (payments, prior authorisations). The visit is then
/// not marked billed and the bill can be run again.
pub async fn bill(state: &AppState, id: i64, basis: Basis) -> Result<Billed> {
    let visit = Visit::find_or_404(&state.db, id).await?;
    if visit.billed_at.is_some() {
        return Ok(Billed::of(&visit));
    }
    let (clinic_stop, scheduling_applied) = match basis {
        Basis::Completed => (false, 0_i64),
        Basis::Cancelled {
            business,
            deposit_applied,
        } => (business, deposit_applied),
    };
    let claimed = Visit::query()
        .where_(Visit::ID.eq(id))
        .where_(Visit::BILLED_AT.is_null())
        .update(
            &state.db,
            &[
                ("billed_at", &Some(now())),
                ("clinic_stop", &clinic_stop),
                ("scheduling_applied", &scheduling_applied),
            ],
        )
        .await?;
    if claimed == 0 {
        return Ok(Billed::of(&Visit::find_or_404(&state.db, id).await?));
    }
    match run(state, id, basis).await {
        Ok(billed) => Ok(billed),
        Err(error) => {
            let none: Option<renox::db::DateTime> = None;
            Visit::query()
                .where_(Visit::ID.eq(id))
                .update(&state.db, &[("billed_at", &none)])
                .await?;
            Err(error)
        }
    }
}

async fn run(state: &AppState, id: i64, basis: Basis) -> Result<Billed> {
    let visit = Visit::find_or_404(&state.db, id).await?;
    let lines = ChargeLine::active_of(&state.db, id).await?;
    let clinic_stop = matches!(basis, Basis::Cancelled { business: true, .. })
        && visit.work_accepted_at.is_none();
    if clinic_stop {
        // Issued stays issued; nothing is charged, and no claim is made.
        ChargeLine::query()
            .where_(ChargeLine::VISIT_ID.eq(id))
            .where_(ChargeLine::REMOVED_ON.is_null())
            .update(
                &state.db,
                &[
                    ("unit_price", &0_i64),
                    ("self_pay_price", &0_i64),
                    ("patient_share", &0_i64),
                    ("payer_share", &0_i64),
                    ("reason", &Some("clinic_stop".to_owned())),
                    ("updated_at", &now()),
                ],
            )
            .await?;
        return Ok(Billed::of(&Visit::find_or_404(&state.db, id).await?));
    }
    if lines.is_empty() {
        // Nothing was charged: a deposit is still settled on a completion
        // (it is refunded), and wl-scheduling settled it on a cancellation.
        if basis == Basis::Completed {
            settle_deposit(state, id, basis, 0).await?;
        }
        return Ok(Billed::of(&Visit::find_or_404(&state.db, id).await?));
    }
    pay_with_packages(state, &visit, &lines).await?;
    let visit = Visit::find_or_404(&state.db, id).await?;
    let quote = charges::quote(state, &visit).await?;
    for line in &quote.lines {
        ChargeLine::query()
            .where_(ChargeLine::ID.eq(line.id))
            .update(
                &state.db,
                &[
                    ("self_pay_units", &line.self_pay_units),
                    ("authorised_units", &line.authorised_units),
                    ("patient_share", &line.patient_share),
                    ("payer_share", &line.payer_share),
                    ("updated_at", &now()),
                ],
            )
            .await?;
    }
    Visit::query()
        .where_(Visit::ID.eq(id))
        .update(
            &state.db,
            &[
                ("patient_total", &quote.patient_total),
                ("payer_total", &quote.payer_total),
                ("updated_at", &now()),
            ],
        )
        .await?;
    let mut visit = Visit::find_or_404(&state.db, id).await?;
    let patient = visit
        .patient_party_id
        .map(PartyId)
        .ok_or_else(|| refused(english("clinics.billing.problem.no_patient", &[])))?;
    // The patient's share, as an invoice from the visit.
    let mut gross = 0;
    if quote.patient_total > 0 {
        let mut draft =
            InvoiceDraft::new(patient, visit.subject()).date_of_supply(visit.visit_date);
        for line in quote.lines.iter().filter(|l| l.patient_share > 0) {
            let (quantity, price, text) = if line.patient_share % line.units == 0 {
                (
                    line.units * 1000,
                    line.patient_share / line.units,
                    describe(line),
                )
            } else {
                (
                    1000,
                    line.patient_share,
                    english("clinics.billing.share_line", &[("line", &describe(line))]),
                )
            };
            let draft_line = DraftLine::new(&text, price, vat(line))
                .quantity_milli(quantity)
                .origin(line.subject());
            gross += draft_line.gross();
            draft = draft.line(draft_line);
        }
        if visit.invoice_id.is_none() {
            let made = invoicer(state)
                .draft(&state.db, draft)
                .await?
                .ok_or_else(|| refused(english("clinics.billing.problem.invoicing_off", &[])))?;
            Visit::query()
                .where_(Visit::ID.eq(id))
                .update(&state.db, &[("invoice_id", &Some(made.0))])
                .await?;
            visit.invoice_id = Some(made.0);
        }
        if let Some(invoice) = visit.invoice_id {
            let found = invoicer(state).find(&state.db, InvoiceId(invoice)).await?;
            if found.is_some_and(|summary| summary.status == InvoiceStatus::Draft) {
                invoicer(state)
                    .issue(state, InvoiceId(invoice), None)
                    .await?;
            }
        }
    }
    // The payer's share, as a draft payer bill: the claim to prepare.
    if quote.payer_total > 0 && visit.payer_bill_id.is_none() {
        let agreement = quote
            .contract
            .as_ref()
            .and_then(|contract| contract.agreement_id)
            .ok_or_else(|| refused(english("clinics.billing.problem.no_agreement", &[])))?;
        let mut request = NewPayerBill::new(agreement, patient, visit.subject(), visit.visit_date)
            .void_before_first_submission();
        if let Some(reference) = first_reference(state, &quote).await? {
            request = request.payer_reference(&reference);
        }
        for line in quote.lines.iter().filter(|l| l.payer_share > 0) {
            request = request.line(
                NewPayerBillLine::new(&describe(line), line.payer_share).vat_category(vat(line)),
            );
        }
        let made: PayerBillId = payer_bills(state).create(state, request, None).await?;
        Visit::query()
            .where_(Visit::ID.eq(id))
            .update(&state.db, &[("payer_bill_id", &Some(made.0))])
            .await?;
        visit.payer_bill_id = Some(made.0);
    }
    // The claim that wraps the payer bill; a bill that stopped before it was
    // made finishes it.
    if let Some(bill) = visit.payer_bill_id
        && Claim::of_payer_bill(&state.db, bill).await?.is_none()
    {
        let reference = first_reference(state, &quote).await?;
        claims::create_for_visit(
            state,
            &visit,
            PayerBillId(bill),
            quote.contract.as_ref(),
            reference,
            None,
        )
        .await?;
    }
    // The units of the authorisations the covered services used.
    if visit.authorisations_consumed_at.is_none() {
        let at = now();
        for need in &quote.needs {
            for (authorisation, units) in &need.authorised {
                authorisations::consume(state, *authorisation, *units, at).await?;
            }
        }
        Visit::query()
            .where_(Visit::ID.eq(id))
            .update(&state.db, &[("authorisations_consumed_at", &Some(at))])
            .await?;
    }
    settle_deposit(state, id, basis, gross).await?;
    Ok(Billed::of(&Visit::find_or_404(&state.db, id).await?))
}

/// The payer's reference of the first authorisation the bill consumes.
async fn first_reference(state: &AppState, quote: &charges::Quote) -> Result<Option<String>> {
    for need in &quote.needs {
        if let Some((authorisation, _)) = need.authorised.first() {
            let found = PriorAuthorisation::find(&state.db, *authorisation).await?;
            if let Some(reference) = found.and_then(|a| a.reference) {
                return Ok(Some(reference));
            }
        }
    }
    Ok(None)
}

/// A self-pay service line is paid by the patient's active package when it
/// has the credits: the session is consumed because the service was recorded
/// as delivered, and the line is not charged. Insured visits never use one.
async fn pay_with_packages(state: &AppState, visit: &Visit, lines: &[ChargeLine]) -> Result {
    if charges::insured_policy(state, visit).await?.is_some() {
        return Ok(());
    }
    let Some(patient) = visit.patient_party_id else {
        return Ok(());
    };
    for line in lines
        .iter()
        .filter(|l| l.kind == ChargeKind::Service && l.package_id.is_none())
    {
        let request = ConsumePackage::new(PartyId(patient), &line.code, line.units, line.subject());
        let package = match package_ledger(state).consume(state, request).await? {
            PackageConsumption::Consumed { package, .. }
            | PackageConsumption::Already { package } => package,
            PackageConsumption::Unpaid { .. } | PackageConsumption::NoCredit => continue,
        };
        ChargeLine::query()
            .where_(ChargeLine::ID.eq(line.id))
            .update(
                &state.db,
                &[
                    ("unit_price", &0_i64),
                    ("self_pay_price", &0_i64),
                    ("package_id", &Some(package.0)),
                    ("reason", &Some("package".to_owned())),
                    ("updated_at", &now()),
                ],
            )
            .await?;
    }
    Ok(())
}

/// The deposit of the booking: applied to the invoice and refunded as the
/// basis says. Once.
async fn settle_deposit(state: &AppState, id: i64, basis: Basis, gross: i64) -> Result {
    let visit = Visit::find_or_404(&state.db, id).await?;
    if visit.deposit_settled_at.is_some() {
        return Ok(());
    }
    let recorder = payment_recorder(state);
    let subject = visit.booking().subject();
    let payments: Vec<_> = recorder
        .payments_for(&state.db, &subject)
        .await?
        .into_iter()
        .filter(|p| p.status == PaymentStatus::Received && p.unallocated > 0)
        .collect();
    let prepaid: i64 = payments.iter().map(|p| p.unallocated).sum();
    let (applied, refund) = match basis {
        // wl-scheduling decided how much of the deposit goes to the work and
        // asked its own refund of the rest. What the bill cannot use (a
        // package paid the service) is paid back here, never kept.
        Basis::Cancelled {
            deposit_applied, ..
        } => {
            let decided = deposit_applied.max(0);
            let applied = decided.min(gross).max(0);
            (applied, (decided - applied).min(prepaid))
        }
        Basis::Completed if visit.deposit_refund_full => (0, prepaid),
        Basis::Completed => {
            let applied = prepaid.min(gross).max(0);
            (applied, prepaid - applied)
        }
    };
    if let (true, Some(invoice)) = (applied > 0, visit.invoice_id) {
        let mut left = applied;
        for payment in &payments {
            if left == 0 {
                break;
            }
            let part = left.min(payment.unallocated);
            recorder
                .allocate(
                    state,
                    payment.id,
                    AllocationTarget::Invoice(InvoiceId(invoice)),
                    part,
                )
                .await?;
            left -= part;
        }
    }
    if refund > 0
        && let Some(first) = payments.first()
    {
        let entity = LegalEntity::default_entity(&state.db).await?;
        let reason = english(
            "clinics.billing.deposit_refund",
            &[("number", &visit.number())],
        );
        recorder
            .request_refund(
                state,
                RefundRequest {
                    source: RefundSource::Booking(subject),
                    party: first.party,
                    legal_entity_id: entity.id,
                    amount: refund,
                    currency: first.currency.clone(),
                    method: None,
                    gateway_txn: None,
                    reason,
                },
                None,
            )
            .await?;
    }
    Visit::query()
        .where_(Visit::ID.eq(id))
        .update(
            &state.db,
            &[
                ("deposit_applied", &applied),
                ("deposit_refunded", &refund),
                ("deposit_settled_at", &Some(now())),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_described_by_its_code_and_units() {
        let line = ChargeLine {
            code: "laser".into(),
            units: 2,
            ..ChargeLine::default()
        };
        assert_eq!(describe(&line), "laser x 2");
        assert_eq!(vat(&line), VatCategory::ZeroRated);
        let standard = ChargeLine {
            vat_standard: true,
            ..line
        };
        assert_eq!(vat(&standard), VatCategory::Standard);
    }

    #[test]
    fn a_bill_reads_back_from_the_visit() {
        let visit = Visit {
            patient_total: 100,
            payer_total: 400,
            invoice_id: Some(3),
            ..Visit::default()
        };
        let billed = Billed::of(&visit);
        assert_eq!(billed.patient_total, 100);
        assert_eq!(billed.invoice, Some(3));
        assert_eq!(billed.payer_bill, None);
    }
}
