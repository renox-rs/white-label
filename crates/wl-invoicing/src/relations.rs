//! The related records of an invoice (BL-AGT-03, `PLT.REL.01`): what the
//! *Related* panel of the invoice page shows, and the example every module
//! follows to declare its relations (`wl_kernel::related`). Each relation
//! reads this module's own tables, many invoices at once, in one query;
//! records of other modules are named by their `Subject` only.
//!
//! The documents of an invoice come from the generic relation the documents
//! module declares, its approval requests from the kernel's.
//!
//! BL-AGT-04 adds the credit note's invoice and customer and a party's
//! invoices; the cheques that pay an invoice are declared by the payments
//! module (it owns the cheque links), the delivery of an invoice by the
//! delivery module.

use renox::db::sql;
use wl_kernel::Subject;
use wl_kernel::invoicing::{CREDIT_NOTE_SUBJECT_TYPE, INVOICE_SUBJECT_TYPE};
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::payments::PAYMENT_SUBJECT_TYPE;
use wl_kernel::related::Relation;

use crate::permissions;

/// The subject type of an invoice line (a row of the invoice, no page of
/// its own).
pub const INVOICE_LINE_SUBJECT_TYPE: &str = "invoicing.invoice_line";

/// The permission to see payments, as the payments module names it (no
/// dependency on that crate: relations name other modules by text).
const PAYMENTS_VIEW: &str = "payments.view";

/// The permission to see parties, as the parties module names it.
const PARTIES_VIEW: &str = "parties.view";

/// Declares the relations of `invoicing.invoice`, `invoicing.credit_note`
/// and a party's invoices.
pub(crate) fn register() {
    Relation::has_many(
        INVOICE_SUBJECT_TYPE,
        "lines",
        INVOICE_LINE_SUBJECT_TYPE,
        "wl_invoicing_invoice_lines",
        "invoice_id",
    )
    .label("Lines")
    .permission(permissions::VIEW)
    .expand()
    .register();
    Relation::belongs_to(
        INVOICE_SUBJECT_TYPE,
        "customer",
        PARTY_SUBJECT_TYPE,
        "wl_invoicing_invoices",
        "party_id",
    )
    .label("Customer")
    .permission(PARTIES_VIEW)
    .link("/parties/{id}")
    .register();
    Relation::new(INVOICE_SUBJECT_TYPE, "payments", PAYMENT_SUBJECT_TYPE)
        .label("Payments")
        .permission(PAYMENTS_VIEW)
        .link("/payments/{id}")
        .load(|db, ids| {
            Box::pin(async move {
                // The payments allocated to the invoices and not taken back.
                let placeholders = vec!["?"; ids.len()].join(", ");
                let rows = sql(format!(
                    "SELECT invoice_id, source_id FROM wl_invoicing_allocations \
                     WHERE source_type = ? AND removed_at IS NULL \
                     AND invoice_id IN ({placeholders}) ORDER BY id"
                ))
                .bind(PAYMENT_SUBJECT_TYPE)
                .bind_all(ids.iter().map(|id| renox::db::DbValue::Integer(*id)))
                .fetch_all(db)
                .await?;
                let mut pairs = Vec::with_capacity(rows.len());
                for row in &rows {
                    let invoice: i64 = row.try_get("invoice_id")?;
                    let payment: i64 = row.try_get("source_id")?;
                    pairs.push((
                        Subject::new(INVOICE_SUBJECT_TYPE, invoice),
                        Subject::new(PAYMENT_SUBJECT_TYPE, payment),
                    ));
                }
                Ok(pairs)
            })
        })
        .register();
    Relation::has_many(
        INVOICE_SUBJECT_TYPE,
        "credit_notes",
        CREDIT_NOTE_SUBJECT_TYPE,
        "wl_invoicing_credit_notes",
        "invoice_id",
    )
    .label("Credit notes")
    .permission(permissions::VIEW)
    .link("/credit-notes/{id}")
    .register();
    Relation::belongs_to(
        CREDIT_NOTE_SUBJECT_TYPE,
        "invoice",
        INVOICE_SUBJECT_TYPE,
        "wl_invoicing_credit_notes",
        "invoice_id",
    )
    .label("Invoice")
    .permission(permissions::VIEW)
    .link("/invoices/{id}")
    .register();
    Relation::belongs_to(
        CREDIT_NOTE_SUBJECT_TYPE,
        "customer",
        PARTY_SUBJECT_TYPE,
        "wl_invoicing_credit_notes",
        "party_id",
    )
    .label("Customer")
    .permission(PARTIES_VIEW)
    .link("/parties/{id}")
    .register();
    // A party's invoices: the rows of the invoices table billed to it.
    Relation::has_many(
        PARTY_SUBJECT_TYPE,
        "invoices",
        INVOICE_SUBJECT_TYPE,
        "wl_invoicing_invoices",
        "party_id",
    )
    .label("Invoices")
    .permission(permissions::VIEW)
    .link("/invoices/{id}")
    .register();
}
