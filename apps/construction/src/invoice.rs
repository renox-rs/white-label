//! Invoicing a certified IPA with its deductions (`invoice`, 02 Construction
//! §4.4 steps 5 and 6, §7.1 *IPA*, §9 "Certificate deductions"; BL-CON-06).
//!
//! The accountant (`invoices.issue`) runs [`invoice`] on a `certified` IPA.
//! Everything goes through the kernel's contracts:
//!
//! 1. **The tax invoice** covers the certified work of the period (the
//!    cumulative certified work less the previous certified) plus 5% VAT. It
//!    is **never reduced by the deductions** and holds no retention (VAT on
//!    the retention is due on its own invoice, BL-CON-07).
//! 2. **A credit note** against that invoice for the accepted
//!    `liquidated_damages` and `other` lines (price reductions), VAT as on the
//!    invoice line. The contract takes the net per invoice line, so the lines'
//!    references are in the credit note's reason. Back- and contra-charges
//!    never get a credit note.
//! 3. **The dispute flag** on the invoice for the contested lines plus their
//!    VAT (reason *contested certificate deduction*): the unpaid part is
//!    disputed and reminders pause, a module behaviour.
//! 4. **A supplier bill from the client** with one line per accepted back- or
//!    contra-charge, VAT 5%, no purchase order to match; the accountant gets a
//!    task to approve it and then to record the set-off (wl-payments' method
//!    `set_off`, which moves no money and waits for an exception while the
//!    client is blocked, BL-CON-03). Until then the invoice balance includes
//!    the back-charge amount.
//! 5. **The transition** `certified -> invoiced`, whose effect withholds the
//!    retention of the period and adds it to the held parts
//!    ([`crate::retention::withhold_in`]); no tax document is issued for it.
//!
//! The documents live in other modules, so they cannot share the IPA's
//! transaction. Each is made once and its id is kept on the IPA the moment it
//! exists ([`Ipa::invoice_id`] and the others), so a run that stopped
//! half-way is run again and resumes: the tax invoice that was drafted but
//! not issued is issued, and nothing is made twice. The transition runs last,
//! and its guard wants the invoice.
//!
//! Decisions where the spec is silent:
//!
//! - The VAT of the credit note and the bill follows the invoice line (5%,
//!   `⚠ open: accountant (A-07)` for liquidated damages).
//! - Advance recovery is not booked: advances are not built yet.
//! - The cap notice to the accountant (`CON.ACL` retention cap) comes with
//!   the retention lifecycle.

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{
    BillRequest, CreditLine, DisputeBy, DisputeRequest, DraftLine, InvoiceDraft, InvoiceId,
    InvoiceStatus, VatCategory, invoicer, supplier_bills,
};

use crate::deduction::{self, DeductionKind, DeductionStatus, IpaDeduction};
use crate::ipa::{INVOICE, Ipa, IpaStatus};
use crate::permissions;
use crate::project::Project;
use crate::retention;
use crate::tasks::{self, Task};
use crate::texts::english;

/// What invoicing an IPA makes, worked out from its numbers and its lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The certified work of the period the tax invoice covers, excluding VAT.
    pub work: i64,
    /// The accepted liquidated damages and `other` lines, excluding VAT: the
    /// net of the credit note.
    pub credit_net: i64,
    /// The references of the lines of the credit note, in order.
    pub credit_references: Vec<String>,
    /// The contested lines plus their VAT: the disputed amount.
    pub contested_gross: i64,
    /// One line of the supplier bill per accepted back- or contra-charge.
    pub bill_lines: Vec<DraftLine>,
}

/// The certified work of the period `ipa` covers: the cumulative certified
/// amount less the previous certified.
pub fn period_work(ipa: &Ipa) -> i64 {
    ipa.certified_amount.unwrap_or_default() - ipa.previous_certified
}

/// What invoicing `ipa` with `lines` makes (see the module).
pub fn plan(ipa: &Ipa, lines: &[IpaDeduction]) -> Plan {
    let mut plan = Plan {
        work: period_work(ipa),
        credit_net: 0,
        credit_references: Vec::new(),
        contested_gross: 0,
        bill_lines: Vec::new(),
    };
    for line in lines {
        let priced = DraftLine::new(&line.reference, line.amount, VatCategory::Standard);
        match (line.status, line.kind) {
            (DeductionStatus::Contested, _) => plan.contested_gross += priced.gross(),
            (
                DeductionStatus::Accepted,
                DeductionKind::LiquidatedDamages | DeductionKind::Other,
            ) => {
                plan.credit_net += line.amount;
                plan.credit_references.push(line.reference.clone());
            }
            (
                DeductionStatus::Accepted,
                DeductionKind::BackCharge | DeductionKind::ContraCharge,
            ) => {
                let kind = line.kind.as_str().replace('_', " ");
                let description = format!("{} {kind}", line.reference);
                plan.bill_lines.push(DraftLine::new(
                    &description,
                    line.amount,
                    VatCategory::Standard,
                ));
            }
            (DeductionStatus::Undecided, _) => {}
        }
    }
    plan
}

/// Why the IPA cannot move to `invoiced` yet, if it cannot: the tax invoice
/// is not made.
pub fn invoice_problem(ipa: &Ipa) -> Option<String> {
    ipa.invoice_id
        .is_none()
        .then(|| english("construction.invoice.not_made", &[]))
}

/// The effect of `invoice` inside its transaction: the retention of the
/// period is added to the held parts.
pub(crate) async fn invoice_effects(tx: &mut renox::db::Transaction, ipa: &Ipa) -> Result {
    let project = Project::find_or_404(&mut *tx, ipa.project_id).await?;
    retention::withhold_in(&mut *tx, &project, period_work(ipa)).await?;
    Ok(())
}

/// A 422 with `message`.
fn guard(message: String) -> TransitionError {
    TransitionError::Guard(message)
}

/// Keeps `value` in the column `column` of IPA `id`.
async fn keep(db: &Db, id: i64, column: &str, value: i64) -> Result {
    let value = Some(value);
    Ipa::query()
        .where_(Ipa::ID.eq(id))
        .update(db, &[(column, &value)])
        .await?;
    Ok(())
}

/// `user` invoices the certified IPA `id` (see the module).
///
/// # Errors
///
/// `Forbidden` without `invoices.issue` in the project, `Guard` (422) when
/// the IPA is not `certified`, certifies no work for the period, a line is
/// undecided, or invoicing is off, and the refusals of the contracts (the
/// invoice cannot be issued...). Whatever was made stays on the IPA, and a
/// new run resumes from it.
pub async fn invoice(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Ipa, TransitionError> {
    let ipa = Ipa::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    let project = Project::find_or_404(&state.db, ipa.project_id).await?;
    if !user.has_permission_in(permissions::INVOICES_ISSUE, &project.scope()) {
        return Err(TransitionError::Forbidden);
    }
    if ipa.status != IpaStatus::Certified {
        return Err(guard(english(
            "construction.invoice.not_certified",
            &[("number", &ipa.number)],
        )));
    }
    let lines = deduction::lines_of(&state.db, ipa.id).await?;
    if lines
        .iter()
        .any(|line| line.status == DeductionStatus::Undecided)
    {
        return Err(guard(english("construction.invoice.undecided", &[])));
    }
    let plan = plan(&ipa, &lines);
    if plan.work <= 0 {
        return Err(guard(english("construction.invoice.no_work", &[])));
    }
    let invoices = invoicer(state);

    // 1. The tax invoice for the certified work, issued.
    let invoice_id = match ipa.invoice_id {
        Some(made) => InvoiceId(made),
        None => {
            let description = english(
                "construction.invoice.line",
                &[("number", &ipa.number), ("project", &project.name)],
            );
            let line = DraftLine::new(&description, plan.work, VatCategory::Standard)
                .origin(ipa.subject());
            let mut draft =
                InvoiceDraft::new(project.client(), ipa.subject()).scope(project.subject());
            if let Some(on) = ipa.certified_on {
                draft = draft.date_of_supply(on);
            }
            let made = invoices
                .draft(&state.db, draft.line(line))
                .await?
                .ok_or_else(|| guard(english("construction.invoice.invoicing_off", &[])))?;
            keep(&state.db, ipa.id, "invoice_id", made.0).await?;
            made
        }
    };
    let summary = invoices
        .find(&state.db, invoice_id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    if summary.status == InvoiceStatus::Draft {
        invoices.issue(state, invoice_id, Some(user)).await?;
    }

    // 2. The credit note for the accepted liquidated damages and price
    //    reductions.
    if plan.credit_net > 0 && ipa.credit_note_id.is_none() {
        let reason = english(
            "construction.invoice.credit_reason",
            &[("references", &plan.credit_references.join(", "))],
        );
        let note = invoices
            .credit_partly(
                state,
                invoice_id,
                vec![CreditLine::new(1, plan.credit_net)],
                reason,
                Some(user.id),
            )
            .await?;
        keep(&state.db, ipa.id, "credit_note_id", note.0).await?;
    }

    // 3. The unpaid part for the contested deductions is disputed.
    if plan.contested_gross > 0 && ipa.dispute_id.is_none() {
        let request = DisputeRequest::new(
            invoice_id,
            english("construction.invoice.dispute_reason", &[]),
            DisputeBy::System,
        )
        .amount(plan.contested_gross)
        .source(ipa.subject());
        let dispute = invoices.dispute(state, request).await?;
        keep(&state.db, ipa.id, "dispute_id", dispute).await?;
    }

    // 4. The back- and contra-charges are a supplier bill from the client,
    //    which the accountant approves and sets off.
    if !plan.bill_lines.is_empty() && ipa.supplier_bill_id.is_none() {
        let number = english(
            "construction.invoice.bill_number",
            &[("project", &project.code), ("number", &ipa.number)],
        );
        let mut request = BillRequest::new(project.client(), number, today(), ipa.subject());
        for line in &plan.bill_lines {
            request = request.line(line.clone());
        }
        let bill = supplier_bills(state)
            .create_submitted(state, request)
            .await?
            .ok_or_else(|| guard(english("construction.invoice.invoicing_off", &[])))?;
        keep(&state.db, ipa.id, "supplier_bill_id", bill.0).await?;
    }
    let fresh = Ipa::find_or_404(&state.db, ipa.id).await?;
    if let Some(bill) = fresh.supplier_bill_id {
        tasks::open(
            state,
            &project,
            Task {
                subject: fresh.subject(),
                role: permissions::ACCOUNTANT_ROLE,
                assignee: None,
                source_key: format!("construction.ipa_set_off:{}", fresh.id),
                title: english(
                    "construction.invoice.set_off_title",
                    &[("number", &fresh.number), ("project", &project.name)],
                ),
                description: english(
                    "construction.invoice.set_off_body",
                    &[("bill", &bill), ("invoice", &invoice_id)],
                ),
                due: None,
            },
        )
        .await?;
    }

    // 5. The IPA is invoiced; the retention of the period is held.
    // `run` commits and emits the transition's event.
    let _outcome = fresh.transition(INVOICE).by(user).run(state).await?;
    Ok(Ipa::find_or_404(&state.db, ipa.id).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(
        kind: DeductionKind,
        status: DeductionStatus,
        reference: &str,
        amount: i64,
    ) -> IpaDeduction {
        IpaDeduction {
            kind,
            status,
            reference: reference.into(),
            amount,
            ..IpaDeduction::default()
        }
    }

    fn certified() -> Ipa {
        Ipa {
            certified_amount: Some(18_000_000),
            ..Ipa::new(1)
        }
    }

    #[test]
    fn the_invoice_covers_the_work_of_the_period_whatever_is_deducted() {
        let mut ipa = certified();
        let lines = [line(
            DeductionKind::BackCharge,
            DeductionStatus::Accepted,
            "BC-12",
            500_000,
        )];
        assert_eq!(plan(&ipa, &lines).work, 18_000_000);
        ipa.previous_certified = 4_000_000;
        assert_eq!(plan(&ipa, &lines).work, 14_000_000);
    }

    #[test]
    fn each_kind_of_line_goes_where_the_spec_sends_it() {
        use DeductionKind::{BackCharge, ContraCharge, LiquidatedDamages, Other};
        use DeductionStatus::{Accepted, Contested, Undecided};
        let lines = [
            line(BackCharge, Accepted, "BC-12", 500_000),
            line(LiquidatedDamages, Accepted, "LD-1", 3_000_000),
            line(ContraCharge, Contested, "CC-3", 1_200_000),
            line(Other, Accepted, "OT-9", 100_000),
            line(ContraCharge, Accepted, "CC-4", 200_000),
            line(BackCharge, Undecided, "BC-13", 999),
        ];
        let plan = plan(&certified(), &lines);
        // Credit note: LD and other; never the back- and contra-charges.
        assert_eq!(plan.credit_net, 3_100_000);
        assert_eq!(plan.credit_references, ["LD-1", "OT-9"]);
        // Dispute: AED 12,000 + 5% VAT.
        assert_eq!(plan.contested_gross, 1_260_000);
        // Supplier bill: the accepted back- and contra-charges.
        assert_eq!(plan.bill_lines.len(), 2);
        assert_eq!(plan.bill_lines[0].net(), 500_000);
        assert_eq!(plan.bill_lines[0].vat(), 25_000);
        assert!(plan.bill_lines[0].description.contains("BC-12"));
    }

    #[test]
    fn the_transition_waits_for_the_invoice() {
        let mut ipa = certified();
        assert!(invoice_problem(&ipa).is_some());
        ipa.invoice_id = Some(3);
        assert_eq!(invoice_problem(&ipa), None);
    }
}
