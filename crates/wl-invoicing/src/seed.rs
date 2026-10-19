//! Demo invoices for `db:seed` (DEMO-DATA.md, 01 Property): a few drafts
//! for the first demo parties, read through the kernel's party directory,
//! with standard-rated, zero-rated and exempt lines.
//!
//! They stay drafts: `issue` is a user's transition (its owner is the user
//! who issues), so a back-office user issues them from the invoice page.
//! The list is fixed; each demo invoice has its own source
//! (`invoicing.demo:N`), and one whose source has an invoice is skipped, so
//! seeding twice changes nothing. It also opens the running period's VAT
//! summary of each VAT-registered entity. Without the demo parties (parties off, or
//! not seeded) nothing is created.

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, VatCategory};

use crate::bill_line::BillLineDraft;
use crate::bill_submit::submit;
use crate::credit_notes::By;
use crate::invoice::Invoice;
use crate::invoicer::create_draft;
use crate::supplier_bill::{self, BillDraft, SupplierBill, number_key};

/// The source type of the demo invoices.
pub const DEMO_SOURCE: &str = "invoicing.demo";

/// One demo invoice.
#[derive(Debug, Clone, Copy)]
pub struct DemoInvoice {
    /// Its number in the list: the id of its source.
    pub number: i64,
    /// The billed party's code.
    pub party_code: &'static str,
    /// The lines: description, unit price in fils, VAT category.
    pub lines: &'static [(&'static str, i64, VatCategory)],
}

/// The demo invoices.
pub const DEMO_INVOICES: &[DemoInvoice] = &[
    DemoInvoice {
        number: 1,
        party_code: "P-00001",
        lines: &[
            ("Service charge, Q4 2026", 100_000, VatCategory::Standard),
            (
                "Residential rent, October 2026",
                750_000,
                VatCategory::Exempt,
            ),
        ],
    },
    DemoInvoice {
        number: 2,
        party_code: "P-00002",
        lines: &[("Maintenance call-out", 35_000, VatCategory::Standard)],
    },
    DemoInvoice {
        number: 3,
        party_code: "P-00003",
        lines: &[
            ("Office fit-out works", 1_000_000, VatCategory::Standard),
            ("Export handling", 50_000, VatCategory::ZeroRated),
        ],
    },
];

/// The code of the party the demo supplier bills come from.
pub const DEMO_SUPPLIER_CODE: &str = "P-00003";

/// The supplier invoice numbers of the demo supplier bills: the second is
/// the first again (in another case, with a space), so submitting both holds
/// the second as a possible duplicate.
pub const DEMO_BILL_NUMBERS: [&str; 2] = ["SB-2026-0101", "sb-2026-0101 "];

impl DemoInvoice {
    /// Its source record.
    pub fn source(&self) -> Subject {
        Subject::new(DEMO_SOURCE, self.number)
    }
}

/// Drafts the demo invoices the database lacks; returns how many.
pub async fn run(state: &AppState) -> Result<usize> {
    // The kernel's own seeder runs after the modules', and an invoice needs
    // its issuing entity; installing is idempotent.
    if let Some(config) = state.provided::<std::sync::Arc<wl_kernel::ClientConfig>>() {
        wl_kernel::entities::install(&state.db, &config).await?;
    }
    let directory = wl_kernel::parties::directory(state);
    let mut created = 0;
    for demo in DEMO_INVOICES {
        let source = demo.source();
        let exists = Invoice::query()
            .where_(Invoice::SOURCE_TYPE.eq(source.subject_type.as_str()))
            .where_(Invoice::SOURCE_ID.eq(source.subject_id))
            .exists(&state.db)
            .await?;
        if exists {
            continue;
        }
        let Some(party) = directory.find_by_code(&state.db, demo.party_code).await? else {
            continue;
        };
        let draft = demo.lines.iter().fold(
            InvoiceDraft::new(party.id, source),
            |draft, (description, price, category)| {
                draft.line(DraftLine::new(description, *price, *category))
            },
        );
        create_draft(&state.db, draft).await?;
        created += 1;
    }
    // The running period's open VAT summary of each VAT-registered demo
    // entity; a period that has one (in any state) gets no second.
    crate::vat_summary::run(state, wl_kernel::documents::today()).await?;
    bills(state).await?;
    Ok(created)
}

/// Enters and submits the two demo supplier bills the database lacks, as
/// the system: the first is matched, the second has the first's number and
/// is held as a possible duplicate (`FIN.ACL.03`). A bill whose supplier
/// invoice number the supplier already has is skipped, so seeding twice
/// changes nothing. Returns how many were created; none without the demo
/// supplier party.
pub async fn bills(state: &AppState) -> Result<usize> {
    let directory = wl_kernel::parties::directory(state);
    let Some(supplier) = directory
        .find_by_code(&state.db, DEMO_SUPPLIER_CODE)
        .await?
    else {
        return Ok(0);
    };
    let today = wl_kernel::documents::today();
    let mut created = 0;
    for number in DEMO_BILL_NUMBERS {
        let exists = SupplierBill::query()
            .where_(SupplierBill::PARTY_ID.eq(supplier.id.0))
            .where_(SupplierBill::NUMBER_KEY.eq(number_key(number)))
            .exists(&state.db)
            .await?;
        if exists {
            continue;
        }
        let draft = BillDraft::new(supplier.id, number, today)
            .due_date(today + renox::chrono::Duration::days(30))
            .line(BillLineDraft::new(
                "Office fit-out works, stage 2",
                850_000,
                VatCategory::Standard,
            ))
            .line(BillLineDraft::new(
                "Site cleaning",
                25_000,
                VatCategory::Standard,
            ));
        let bill = supplier_bill::create(&state.db, &draft, None).await?;
        submit(state, By::System(None), bill.id)
            .await
            .map_err(renox::state_machine::TransitionError::into_error)?;
        created += 1;
    }
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_demo_invoice_has_its_own_source_and_lines() {
        let mut sources: Vec<String> = DEMO_INVOICES
            .iter()
            .map(|d| d.source().to_string())
            .collect();
        sources.dedup();
        assert_eq!(sources.len(), DEMO_INVOICES.len());
        assert!(DEMO_INVOICES.iter().all(|d| !d.lines.is_empty()));
        assert_eq!(DEMO_INVOICES[0].source().to_string(), "invoicing.demo:1");
    }
}
