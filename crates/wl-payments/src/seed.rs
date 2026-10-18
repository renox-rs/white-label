//! Demo receipts for `db:seed` (DEMO-DATA.md, 01 Property): a few incoming
//! payments from the first demo parties to the default legal entity,
//! recorded through the same `record` transition as the form.
//!
//! The demo invoices are drafts (a user issues them), and an invoice takes
//! an allocation only once issued, so the receipts stay unallocated: a back
//! office user allocates them from the payment page. The list is fixed;
//! each receipt has its own reference (`DEMO-RCT-N`), and one whose
//! reference is already recorded is skipped, so seeding twice changes
//! nothing. Without the demo parties (parties off, or not seeded) nothing is
//! created.

use renox::prelude::*;
use wl_kernel::LegalEntity;
use wl_kernel::parties::directory;
use wl_kernel::payments::{PaymentMethod, RecordPayment};

use crate::payment::Payment;
use crate::record::record_for;

/// One demo receipt.
#[derive(Debug, Clone, Copy)]
pub struct DemoReceipt {
    /// Its number in the list; part of its reference.
    pub number: i64,
    /// The paying party's code.
    pub party_code: &'static str,
    /// The method.
    pub method: PaymentMethod,
    /// The amount in fils (AED).
    pub amount: i64,
}

/// The demo receipts.
pub const DEMO_RECEIPTS: &[DemoReceipt] = &[
    DemoReceipt {
        number: 1,
        party_code: "P-00001",
        method: PaymentMethod::Transfer,
        amount: 500_000,
    },
    DemoReceipt {
        number: 2,
        party_code: "P-00002",
        method: PaymentMethod::Cash,
        amount: 35_000,
    },
    DemoReceipt {
        number: 3,
        party_code: "P-00003",
        method: PaymentMethod::Transfer,
        amount: 1_050_000,
    },
];

impl DemoReceipt {
    /// Its reference, unique among the demo receipts.
    pub fn reference(&self) -> String {
        format!("DEMO-RCT-{}", self.number)
    }
}

/// Records the demo receipts the database lacks; returns how many.
pub async fn run(state: &AppState) -> Result<usize> {
    // The kernel's own seeder runs after the modules', and a payment needs
    // its legal entity; installing is idempotent.
    if let Some(config) = state.provided::<std::sync::Arc<wl_kernel::ClientConfig>>() {
        wl_kernel::entities::install(&state.db, &config).await?;
    }
    let Some(entity) = LegalEntity::query()
        .where_(LegalEntity::IS_DEFAULT.eq(true))
        .first(&state.db)
        .await?
    else {
        return Ok(0);
    };
    let directory = directory(state);
    let mut created = 0;
    for demo in DEMO_RECEIPTS {
        let reference = demo.reference();
        let exists = Payment::query()
            .where_(Payment::REFERENCE.eq(reference.as_str()))
            .exists(&state.db)
            .await?;
        if exists {
            continue;
        }
        let Some(party) = directory.find_by_code(&state.db, demo.party_code).await? else {
            continue;
        };
        let request =
            RecordPayment::new(party.id, entity.id, demo.method, demo.amount).reference(&reference);
        record_for(state, None, &request)
            .await
            .map_err(|e| e.into_error())?;
        created += 1;
    }
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_demo_receipt_has_its_own_reference() {
        let mut references: Vec<String> =
            DEMO_RECEIPTS.iter().map(DemoReceipt::reference).collect();
        references.sort();
        references.dedup();
        assert_eq!(references.len(), DEMO_RECEIPTS.len());
        assert!(DEMO_RECEIPTS.iter().all(|r| r.amount > 0));
        assert_eq!(DEMO_RECEIPTS[0].reference(), "DEMO-RCT-1");
    }
}
