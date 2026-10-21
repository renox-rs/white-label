//! The statement of a COD remittance, as it is published to the client
//! portal (03 Logistics §6 *COD remittance*, §7.1 rows `pay` and `repay`;
//! BL-LOG-14, BL-LOG-16, BL-LOG-17, BL-LOG-18).
//!
//! A statement is a row written when the remittance is paid: its number
//! is the remittance's, its figures and bank reference are those of the
//! payment. `repay` publishes a new version with the new reference and date;
//! the earlier versions stay as history. The statement lists the adjustment
//! lines that count ([`crate::adjustment::statement_lines`]) and each invoice
//! offset with its number and amount ([`crate::netting`]). The PDF
//! (`DOC.GEN.01`) and the portal screen (`POR.SLF.01`) are not built: the
//! data is read at `GET /remittances/{id}/statement`.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::{dubai_date, today};

use crate::adjustment::{self, StatementLine};
use crate::remittance::Remittance;

/// A published version of a remittance's statement.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "logistics_statements",
    unique(remittance_id, version),
    index(remittance_id)
)]
pub struct Statement {
    /// The primary key.
    pub id: i64,
    /// The remittance.
    #[model(references = "logistics_remittances")]
    pub remittance_id: i64,
    /// 1 at `pay`, one more at each `repay`.
    pub version: i64,
    /// The bank reference of the payment this version shows.
    pub payment_reference: String,
    /// The day it was paid (Asia/Dubai).
    pub paid_on: NaiveDate,
    /// What was paid in cash, in fils.
    pub net: i64,
    /// What was offset against open invoices, in fils.
    pub offset_total: i64,
    /// When it was published.
    pub published_at: DateTime,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The versions published for remittance `id`, oldest first.
///
/// # Errors
///
/// Database errors.
pub async fn history(db: &Db, id: i64) -> Result<Vec<Statement>> {
    Statement::query()
        .where_(Statement::REMITTANCE_ID.eq(id))
        .order_by(Statement::VERSION)
        .get(db)
        .await
}

/// Publishes the next version of the statement of `remittance`, which has
/// just been paid (or repaid): the figures and the reference are the
/// remittance's now.
///
/// # Errors
///
/// Database errors.
pub async fn publish(db: &Db, remittance: &Remittance) -> Result<Statement> {
    let version = i64::try_from(history(db, remittance.id).await?.len()).unwrap_or(i64::MAX) + 1;
    let mut statement = Statement {
        remittance_id: remittance.id,
        version,
        payment_reference: remittance.payment_reference.clone().unwrap_or_default(),
        paid_on: remittance.paid_at.map_or_else(today, dubai_date),
        net: remittance.net,
        offset_total: remittance.offset_total,
        published_at: renox::db::now(),
        ..Statement::default()
    };
    statement.insert(db).await?;
    Ok(statement)
}

/// One invoice offset as the client reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OffsetLine {
    /// The invoice's number.
    pub invoice: String,
    /// What was offset, in fils.
    pub amount: i64,
}

/// What the client reads of a published statement.
#[derive(Debug, Clone, Serialize)]
pub struct StatementView {
    /// The remittance's number, also the statement's.
    pub number: String,
    /// The latest version published.
    pub version: i64,
    /// The bank reference of the latest payment.
    pub payment_reference: String,
    /// The day it was paid.
    pub paid_on: String,
    /// The COD of its shipments less the differences the client absorbed.
    pub cod_total: i64,
    /// The COD fees.
    pub fee_total: i64,
    /// The adjustment lines that count, signed.
    pub adjustment_total: i64,
    /// The open invoices offset.
    pub offset_total: i64,
    /// What was paid in cash.
    pub net: i64,
    /// Whether the client contests it.
    pub disputed: bool,
    /// The adjustment lines.
    pub adjustments: Vec<StatementLine>,
    /// The invoices offset, each with its number and amount.
    pub offsets: Vec<OffsetLine>,
}

/// The latest published statement of `remittance`; none until it is paid.
///
/// # Errors
///
/// Database errors.
pub async fn view(db: &Db, remittance: &Remittance) -> Result<Option<StatementView>> {
    let Some(latest) = history(db, remittance.id).await?.pop() else {
        return Ok(None);
    };
    let offsets = remittance
        .offsets(db)
        .await?
        .into_iter()
        .map(|offset| OffsetLine {
            invoice: offset.invoice_number,
            amount: offset.amount,
        })
        .collect();
    Ok(Some(StatementView {
        number: remittance.number.clone(),
        version: latest.version,
        payment_reference: latest.payment_reference,
        paid_on: latest.paid_on.to_string(),
        cod_total: remittance.cod_total,
        fee_total: remittance.fee_total,
        adjustment_total: remittance.adjustment_total,
        offset_total: latest.offset_total,
        net: latest.net,
        disputed: remittance.disputed,
        adjustments: adjustment::statement_lines(db, remittance).await?,
        offsets,
    }))
}
