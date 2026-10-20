//! Writing off the rent of a managed building, with the owner's consent, and
//! what it does to the owner statement (01 Property §3.3 `FIN.ACL.15`, §4.1
//! row "A debt is uncollectable", §7.1 *Invoice* `write_off` and `recover`;
//! BL-PRP-18).
//!
//! In a managed building the loss from an unpaid rent invoice is the owner's.
//! The write-off itself is wl-invoicing's (`FIN.ACL.15`: always approved, by a
//! property manager who did not request it, once the client configures that
//! role as the approver); this module adds what 01 needs around it:
//!
//! - **Consent.** [`request`] asks the write-off of a rent invoice of a lease.
//!   When the lease's unit is under a management agreement that reports, the
//!   owner's consent must be recorded first ([`WriteOffConsent`]: a note, the
//!   date and the owner contact who gave it), or given with the request; with
//!   none, the request is refused and no approval request exists. An invoice
//!   that is not a rent invoice of a managed unit (the management company's
//!   own fee invoice, an owner invoice for a negative balance) needs no
//!   consent.
//! - **The statement.** When the write-off is approved (`InvoiceWrittenOff`) a
//!   line *rent written off* (negative) is posted for the agreement, and a
//!   later recovery (`BadDebtRecovered`) posts *recovered after write-off*
//!   (positive). A line goes on the draft statement whose period holds the day,
//!   else it waits for the next draft: a published statement is never edited
//!   (`PRP.ACL.21`). Both lines are made once per invoice (and payment).
//!
//! Decided where the spec is silent: the consent is a recorded note; attaching
//! a letter or an e-mail as a document of the invoice is not required by the
//! platform. The unit's agreement is looked up on the day of the request.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{
    BadDebtRecovered, InvoiceId, InvoiceWrittenOff, WriteOffAnswer, invoicer,
};

use crate::agreement::ManagementAgreement;
use crate::building::{clean, refused};
use crate::charge::LeaseCharge;
use crate::deposit_forfeit::DepositSettlement;
use crate::lease::Lease;
use crate::permissions;
use crate::statement::{self, LineKind, PostLine};
use crate::texts::english;

/// The owner's consent to writing off one invoice.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "property_writeoff_consents",
    unique(invoice_id),
    index(agreement_id)
)]
pub struct WriteOffConsent {
    /// The primary key.
    pub id: i64,
    /// The invoice.
    pub invoice_id: i64,
    /// The agreement of the managed unit.
    #[model(references = "property_management_agreements")]
    pub agreement_id: i64,
    /// What was agreed (a reference to the e-mail or letter, or a note).
    pub note: String,
    /// The owner's contact who gave it.
    pub given_by: String,
    /// The day it was given.
    pub given_on: NaiveDate,
    /// The user who recorded it.
    pub recorded_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl WriteOffConsent {
    /// The consent recorded for the invoice `invoice_id`, if any.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_invoice(db: &Db, invoice_id: i64) -> Result<Option<WriteOffConsent>> {
        WriteOffConsent::query()
            .where_(WriteOffConsent::INVOICE_ID.eq(invoice_id))
            .first(db)
            .await
    }
}

/// What the accountant gives as the owner's consent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Consent {
    /// What was agreed, or where the e-mail or letter is kept.
    pub note: String,
    /// The owner's contact who gave it.
    pub given_by: String,
    /// The day it was given.
    pub given_on: NaiveDate,
}

/// Why `consent` cannot be recorded on `day`, if it cannot: a note, the
/// contact, and a date that is not in the future.
pub fn consent_problem(consent: &Consent, day: NaiveDate) -> Option<String> {
    if consent.note.trim().is_empty()
        || consent.given_by.trim().is_empty()
        || consent.given_on > day
    {
        return Some(english("property.write_off.consent_invalid", &[]));
    }
    None
}

/// The lease the invoice `invoice_id` was made for, when it is the invoice of
/// a charge of a lease or the damages invoice of a deposit applied to the
/// deductions.
///
/// # Errors
///
/// The database's errors.
pub async fn lease_of_invoice(db: &Db, invoice_id: i64) -> Result<Option<Lease>> {
    if let Some(charge) = LeaseCharge::query()
        .where_(LeaseCharge::INVOICE_ID.eq(invoice_id))
        .first(db)
        .await?
    {
        return Lease::find(db, charge.lease_id).await;
    }
    if let Some(settlement) = DepositSettlement::query()
        .where_(DepositSettlement::EXCESS_INVOICE_ID.eq(invoice_id))
        .first(db)
        .await?
    {
        return Lease::find(db, settlement.lease_id).await;
    }
    Ok(None)
}

/// The agreement that makes the invoice `invoice_id` the owner's loss: its
/// lease's unit is under a management agreement that reports on `day`.
///
/// # Errors
///
/// The database's errors.
pub async fn managing_agreement(
    db: &Db,
    invoice_id: i64,
    day: NaiveDate,
) -> Result<Option<ManagementAgreement>> {
    let Some(lease) = lease_of_invoice(db, invoice_id).await? else {
        return Ok(None);
    };
    ManagementAgreement::for_unit(db, lease.unit_id, day).await
}

/// `user` asks to write off the open balance of the invoice `invoice_id` of
/// lease `lease_id` for `reason_code` (`other` needs `reason_text`). For a
/// managed unit the owner's consent must be recorded, or given as `consent`.
/// The answer is wl-invoicing's: written off at once, or an approval request.
///
/// # Errors
///
/// 403 without `leases.view` in the lease's building (wl-invoicing checks
/// `invoices.write_off`); 404 for an invoice that is not the lease's; 422 for
/// a managed unit without the owner's consent, or consent without its note,
/// contact or a date that is not in the future, and wl-invoicing's refusals (a
/// disputed invoice, one without a balance). Nothing is requested then.
pub async fn request(
    state: &AppState,
    user: &AuthUser,
    lease_id: i64,
    invoice_id: i64,
    reason_code: &str,
    reason_text: Option<&str>,
    consent: Option<Consent>,
) -> Result<WriteOffAnswer> {
    let lease = Lease::find(&state.db, lease_id)
        .await?
        .ok_or(Error::NotFound)?;
    if !user.has_permission_in(permissions::LEASES_VIEW, &lease.scope()) {
        return Err(Error::Forbidden);
    }
    let owner_of = lease_of_invoice(&state.db, invoice_id)
        .await?
        .filter(|found| found.root_id == lease.root_id)
        .ok_or(Error::NotFound)?;
    let agreement = ManagementAgreement::for_unit(&state.db, owner_of.unit_id, today()).await?;
    if let Some(agreement) = agreement
        && WriteOffConsent::of_invoice(&state.db, invoice_id)
            .await?
            .is_none()
    {
        let Some(consent) = consent else {
            return Err(refused(english("property.write_off.consent_required", &[])));
        };
        if let Some(problem) = consent_problem(&consent, today()) {
            return Err(refused(problem));
        }
        WriteOffConsent::create(
            &state.db,
            WriteOffConsent {
                invoice_id,
                agreement_id: agreement.id,
                note: consent.note.trim().to_owned(),
                given_by: consent.given_by.trim().to_owned(),
                given_on: consent.given_on,
                recorded_by: Some(user.id),
                ..Default::default()
            },
        )
        .await?;
    }
    invoicer(state)
        .write_off(
            state,
            user,
            InvoiceId(invoice_id),
            reason_code,
            clean(reason_text.map(str::to_owned)).as_deref(),
        )
        .await
}

/// The rent was written off: for a managed unit the line *rent written off*
/// is posted for the agreement, dated today.
///
/// # Errors
///
/// The database's errors.
pub async fn on_written_off(state: &AppState, event: &InvoiceWrittenOff) -> Result {
    let Some(lease) = lease_of_invoice(&state.db, event.invoice.0).await? else {
        return Ok(());
    };
    let Some(agreement) = ManagementAgreement::for_unit(&state.db, lease.unit_id, today()).await?
    else {
        return Ok(());
    };
    let label = english("property.statements.kind.written_off", &[]);
    let line = PostLine::new(
        LineKind::WrittenOff,
        &format!("{label} · {}", lease.number()),
        -event.amount.abs(),
        today(),
    )
    .source(&event.invoice.subject().to_string())
    .by(event.approver);
    statement::post_line(&state.db, agreement.id, line).await?;
    Ok(())
}

/// Money was recovered on a written-off invoice: *recovered after write-off*
/// is posted for the agreement, dated the day of the recovery.
///
/// # Errors
///
/// The database's errors.
pub async fn on_recovered(state: &AppState, event: &BadDebtRecovered) -> Result {
    let Some(lease) = lease_of_invoice(&state.db, event.invoice.0).await? else {
        return Ok(());
    };
    let Some(agreement) =
        ManagementAgreement::for_unit(&state.db, lease.unit_id, event.recovered_on).await?
    else {
        return Ok(());
    };
    let label = english("property.statements.kind.recovered", &[]);
    let source = format!("{}|{}", event.invoice.subject(), event.source);
    let line = PostLine::new(
        LineKind::Recovered,
        &format!("{label} · {}", lease.number()),
        event.amount.abs(),
        event.recovered_on,
    )
    .source(&source);
    statement::post_line(&state.db, agreement.id, line).await?;
    Ok(())
}

/// Registers the listeners of the two invoice events.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: InvoiceWrittenOff, state: AppState| async move {
        on_written_off(&state, &event).await
    });
    app.listen(|event: BadDebtRecovered, state: AppState| async move {
        on_recovered(&state, &event).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2027, m, d).unwrap()
    }

    #[test]
    fn a_consent_needs_a_note_a_contact_and_a_date_not_in_the_future() {
        let consent = Consent {
            note: "E-mail of 3 March".into(),
            given_by: "Hessa Al Mansoori".into(),
            given_on: day(3, 3),
        };
        assert_eq!(consent_problem(&consent, day(3, 10)), None);
        assert_eq!(consent_problem(&consent, day(3, 3)), None);
        assert!(consent_problem(&consent, day(3, 2)).is_some());
        let blank = Consent {
            note: "  ".into(),
            ..consent.clone()
        };
        assert!(consent_problem(&blank, day(3, 10)).is_some());
        let nobody = Consent {
            given_by: String::new(),
            ..consent
        };
        assert!(consent_problem(&nobody, day(3, 10)).is_some());
    }
}
