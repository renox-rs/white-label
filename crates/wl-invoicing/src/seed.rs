//! Demo invoices for `db:seed` (DEMO-DATA.md, 01 Property): a few drafts
//! for the first demo parties, read through the kernel's party directory,
//! with standard-rated, zero-rated and exempt lines.
//!
//! They stay drafts: `issue` is a user's transition (its owner is the user
//! who issues), so a back-office user issues them from the invoice page.
//! The list is fixed; each demo invoice has its own source
//! (`invoicing.demo:N`), and one whose source has an invoice is skipped, so
//! seeding twice changes nothing. Without the demo parties (parties off, or
//! not seeded) nothing is created.

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, VatCategory};

use crate::invoice::Invoice;
use crate::invoicer::create_draft;

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

impl DemoInvoice {
    /// Its source record.
    pub fn source(&self) -> Subject {
        Subject::new(DEMO_SOURCE, self.number)
    }
}

/// Drafts the demo invoices the database lacks; returns how many.
pub async fn run(state: &AppState) -> Result<usize> {
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
