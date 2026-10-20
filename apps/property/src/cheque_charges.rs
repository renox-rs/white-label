//! What wl-payments asks of the lease's charges about a cheque (wl-payments
//! §4 *Cheque*; 01 Property §4.1, §7.2 `bounce`, `write_off`): the bounce fee
//! charge of a bounced rent cheque, and the invoice of a charge a cheque pays
//! that is not invoiced yet.
//!
//! The lease keeps its own charge schedule ([`crate::charge`]), so 01 is the
//! module that owns these charges in this build:
//!
//! - [`LeaseBounceFees`] (kernel `BounceFees`): a bounce of a cheque received
//!   with a lease (its agreement) adds a *scheduled* charge of the fee type
//!   and amount the client set, with its VAT category, due on the bounce date,
//!   to the version of the lease in force. The daily run invoices it like any
//!   charge. A cheque with no lease, a lease no longer in force or a fee in
//!   another currency than AED makes no charge: wl-payments gives the
//!   accountant a task to raise it.
//! - [`LeaseChargeInvoicer`] (kernel `ChargeInvoicer`): a cheque write-off
//!   asks for the invoice of each linked lease charge. A *scheduled* charge is
//!   invoiced now as the daily run would and the invoice issued; an invoiced
//!   one gives its invoice (issued now if it is still a draft). A planned or
//!   cancelled charge cannot be invoiced, so the write-off is refused.

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::cheques::{BounceFeeCharge, BounceFees, ChargeInvoicer, ChequeFuture};
use wl_kernel::entities::LegalEntity;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, invoicer};

use crate::building::BUILDING_SUBJECT_TYPE;
use crate::charge::{CHARGE_SUBJECT_TYPE, ChargeStatus, LeaseCharge};
use crate::invoicing::invoice_charge;
use crate::lease::{LEASE_SUBJECT_TYPE, Lease, LeaseStatus};
use crate::texts::english;

/// The bounce fee charges of the leases.
#[derive(Debug, Clone, Copy, Default)]
pub struct LeaseBounceFees;

/// The invoices of the lease charges a cheque pays.
#[derive(Debug, Clone, Copy, Default)]
pub struct LeaseChargeInvoicer;

/// Registers both with the kernel.
pub(crate) fn register(app: &mut Registry) {
    wl_kernel::cheques::provide_bounce_fees(app, LeaseBounceFees);
    wl_kernel::cheques::provide_charge_invoicer(app, LeaseChargeInvoicer);
}

/// The version in force of the lease whose agreement is `agreement`.
async fn in_force(db: &Db, agreement: &Subject) -> Result<Option<Lease>> {
    if agreement.subject_type != LEASE_SUBJECT_TYPE {
        return Ok(None);
    }
    let versions = Lease::versions_of(db, agreement.subject_id).await?;
    Ok(versions
        .into_iter()
        .rev()
        .find(|l| LeaseStatus::RUNNING.contains(&l.status)))
}

impl BounceFees for LeaseBounceFees {
    fn add<'a>(
        &'a self,
        state: &'a AppState,
        charge: BounceFeeCharge,
    ) -> ChequeFuture<'a, Option<Subject>> {
        Box::pin(async move {
            if charge.amount <= 0 || !charge.currency.eq_ignore_ascii_case("AED") {
                return Ok(None);
            }
            let Some(agreement) = charge.agreement.as_ref() else {
                return Ok(None);
            };
            let Some(lease) = in_force(&state.db, agreement).await? else {
                return Ok(None);
            };
            let issuer =
                LegalEntity::for_scope(&state.db, Some((BUILDING_SUBJECT_TYPE, lease.building_id)))
                    .await?
                    .id;
            let position = lease
                .charges(&state.db)
                .await?
                .iter()
                .map(|c| c.position)
                .max()
                .unwrap_or(0)
                + 1;
            let description = english(
                "property.cheques.bounce_fee_line",
                &[("lease", &lease.number_with_version())],
            );
            let made = LeaseCharge::create(
                &state.db,
                LeaseCharge {
                    lease_id: lease.id,
                    position,
                    charge_type: charge.charge_type.clone(),
                    description,
                    due_date: charge.due,
                    amount: charge.amount,
                    vat_category: charge.vat_category,
                    status: ChargeStatus::Scheduled,
                    legal_entity_id: (charge.legal_entity_id != issuer)
                        .then_some(charge.legal_entity_id),
                    ..Default::default()
                },
            )
            .await?;
            Ok(Some(made.subject()))
        })
    }
}

/// Issues the draft invoice `id` as the system; an invoice already issued is
/// left as it is.
async fn issued(state: &AppState, id: InvoiceId) -> Result<InvoiceId> {
    let found = invoicer(state).find(&state.db, id).await?;
    if found.is_some_and(|invoice| invoice.status == InvoiceStatus::Draft) {
        invoicer(state).issue(state, id, None).await?;
    }
    Ok(id)
}

impl ChargeInvoicer for LeaseChargeInvoicer {
    fn owns(&self, link: &Subject) -> bool {
        link.subject_type == CHARGE_SUBJECT_TYPE
            || link.subject_type == wl_kernel::cheques::CHARGE_SUBJECT_TYPE
    }

    fn invoice<'a>(
        &'a self,
        state: &'a AppState,
        charge: &'a Subject,
    ) -> ChequeFuture<'a, Option<InvoiceId>> {
        Box::pin(async move {
            if charge.subject_type != CHARGE_SUBJECT_TYPE {
                return Ok(None);
            }
            let Some(found) = LeaseCharge::find(&state.db, charge.subject_id).await? else {
                return Ok(None);
            };
            match (found.status, found.invoice_id) {
                (ChargeStatus::Invoiced, Some(id)) => Ok(Some(issued(state, InvoiceId(id)).await?)),
                (ChargeStatus::Scheduled, None) => {
                    let Some(lease) = Lease::find(&state.db, found.lease_id).await? else {
                        return Ok(None);
                    };
                    match invoice_charge(state, &lease, &found).await? {
                        Some(id) => Ok(Some(issued(state, id).await?)),
                        // Another run invoiced it first: take that invoice.
                        None => {
                            let again = LeaseCharge::find(&state.db, found.id).await?;
                            match again.and_then(|c| c.invoice_id) {
                                Some(id) => Ok(Some(issued(state, InvoiceId(id)).await?)),
                                None => Ok(None),
                            }
                        }
                    }
                }
                _ => Ok(None),
            }
        })
    }
}
