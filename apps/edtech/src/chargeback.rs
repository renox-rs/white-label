//! A reopened invoice of an enrolment (10 §4.3 *A card payment is charged
//! back*, §7.1 *Enrolment*, BL-EDU-06): the invoice, the balance it came back
//! with and the day it was paid again. The services are in
//! [`crate::chargebacks`].

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// An invoice of an enrolment reopened by a reversed payment.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "edtech_chargebacks", unique(invoice_id), index(enrolment_id))]
pub struct Chargeback {
    /// The primary key.
    pub id: i64,
    /// The enrolment the invoice bills.
    pub enrolment_id: i64,
    /// The invoice (`invoicing.invoice`).
    pub invoice_id: i64,
    /// The balance it came back with, in fils.
    pub amount: i64,
    /// The payment's reversal reason (`chargeback`, `bank_return`...).
    pub reversal_reason: Option<String>,
    /// The day it was reopened.
    pub reopened_on: NaiveDate,
    /// The day it was paid again; `None` while it is outstanding.
    pub settled_on: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Chargeback {
    /// The row of `invoice_id`, if the invoice was ever reopened.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_invoice(db: &Db, invoice_id: i64) -> Result<Option<Chargeback>> {
        Chargeback::query()
            .where_(Chargeback::INVOICE_ID.eq(invoice_id))
            .first(db)
            .await
    }

    /// The invoices of `enrolment_id` that are reopened and not paid again,
    /// oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn outstanding_of(db: &Db, enrolment_id: i64) -> Result<Vec<Chargeback>> {
        Chargeback::query()
            .where_(Chargeback::ENROLMENT_ID.eq(enrolment_id))
            .where_(Chargeback::SETTLED_ON.is_null())
            .order_by(Chargeback::ID)
            .get(db)
            .await
    }
}
