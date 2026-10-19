//! The duplicate hold of a supplier bill (wl-invoicing §7, `FIN.ACL.03`,
//! `PRP.ACL.22`).
//!
//! At `submit`, inside its transaction, the supplier is locked
//! ([`lock_supplier`]) so that of two bills submitted at once the second sees
//! the first. Each enabled key set ([`DuplicateKey`]) is then checked on its
//! own against the other bills of the same supplier that are not `draft` or
//! `void`; the earliest match holds the bill: it goes to `exception` with a
//! [`BillException`] row (`possible_duplicate`) that links the earlier bill.
//! Draft bills are not compared, since they have not been submitted.

use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::config::{DuplicateKey, InvoicingConfig};
use crate::supplier_bill::{SupplierBill, SupplierBillStatus};

/// The statuses of a bill that was submitted and is not void: the ones a new
/// bill is compared with.
const COMPARED: [SupplierBillStatus; 5] = [
    SupplierBillStatus::Matched,
    SupplierBillStatus::Exception,
    SupplierBillStatus::Approved,
    SupplierBillStatus::PartiallyPaid,
    SupplierBillStatus::Paid,
];

/// One reason a supplier bill is held; a bill can have several, so every
/// reason is shown.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_invoicing_supplier_bill_exceptions", index(bill_id))]
pub struct BillException {
    /// The primary key.
    pub id: i64,
    /// The held bill.
    pub bill_id: i64,
    /// The reason, e.g. `possible_duplicate`.
    pub reason_code: String,
    /// The bill the reason points to, if any.
    pub related_bill_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
}

/// The earlier bill a bill may duplicate and the key set that matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuplicateMatch {
    /// The earlier bill.
    pub bill_id: i64,
    /// The key set that matched: `supplier_invoice_number` or
    /// `contract_period_amount`.
    pub key: String,
}

/// The name of a key set, as stored in the transition's input.
fn key_name(key: DuplicateKey) -> &'static str {
    match key {
        DuplicateKey::SupplierInvoiceNumber => "supplier_invoice_number",
        DuplicateKey::ContractPeriodAmount => "contract_period_amount",
    }
}

/// Takes the lock of `party_id` (the supplier) for the transaction: a write
/// on SQLite, the row lock on PostgreSQL. Held until the commit or rollback.
pub(crate) async fn lock_supplier(tx: &mut renox::db::Transaction, party_id: i64) -> Result {
    renox::db::sql(
        "INSERT INTO wl_invoicing_supplier_locks (party_id, n) VALUES (?, 1) \
         ON CONFLICT (party_id) DO UPDATE SET n = n + 1",
    )
    .bind(party_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// The other submitted, non-void bills of `bill`'s supplier.
fn others(bill: &SupplierBill) -> renox::db::Query<SupplierBill> {
    SupplierBill::query()
        .where_(SupplierBill::PARTY_ID.eq(bill.party_id))
        .where_(SupplierBill::ID.ne(bill.id))
        .where_(SupplierBill::STATUS.is_in(COMPARED))
}

/// The earliest bill `bill` may duplicate under the enabled key sets of
/// `config`; `None` when the check is off or nothing matches. Call it in the
/// transaction that holds [`lock_supplier`].
pub(crate) async fn find(
    tx: &mut renox::db::Transaction,
    config: &InvoicingConfig,
    bill: &SupplierBill,
) -> Result<Option<DuplicateMatch>> {
    if !config.duplicate_check {
        return Ok(None);
    }
    let mut found: Option<DuplicateMatch> = None;
    for key in &config.duplicate_keys {
        let query = match key {
            DuplicateKey::SupplierInvoiceNumber => {
                if bill.number_key.is_empty() {
                    continue;
                }
                others(bill).where_(SupplierBill::NUMBER_KEY.eq(bill.number_key.clone()))
            }
            DuplicateKey::ContractPeriodAmount => {
                let (Some(contract), Some((start, end))) = (bill.contract(), bill.period()) else {
                    continue;
                };
                others(bill)
                    .where_(SupplierBill::CONTRACT_TYPE.eq(contract.subject_type))
                    .where_(SupplierBill::CONTRACT_ID.eq(contract.subject_id))
                    .where_(SupplierBill::PERIOD_START.eq(start))
                    .where_(SupplierBill::PERIOD_END.eq(end))
                    .where_(SupplierBill::CURRENCY.eq(bill.currency.clone()))
                    .where_(SupplierBill::GROSS_TOTAL.eq(bill.gross_total))
            }
        };
        let earliest = query.order_by(SupplierBill::ID).first(&mut *tx).await?;
        if let Some(earlier) = earliest
            && found.as_ref().is_none_or(|m| earlier.id < m.bill_id)
        {
            found = Some(DuplicateMatch {
                bill_id: earlier.id,
                key: key_name(*key).to_owned(),
            });
        }
    }
    Ok(found)
}

/// The reasons bill `bill_id` is held, oldest first.
pub async fn exceptions_of(db: &Db, bill_id: i64) -> Result<Vec<BillException>> {
    BillException::query()
        .where_(BillException::BILL_ID.eq(bill_id))
        .order_by(BillException::ID)
        .get(db)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_set_has_a_stable_name() {
        assert_eq!(
            key_name(DuplicateKey::SupplierInvoiceNumber),
            "supplier_invoice_number"
        );
        assert_eq!(
            key_name(DuplicateKey::ContractPeriodAmount),
            "contract_period_amount"
        );
    }
}
