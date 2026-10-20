//! What a user who holds a payments permission in some scopes (buildings)
//! sees of the module-wide lists: the payments, refunds, deposits and
//! cheques of the records in those scopes (`wl_kernel::scopes::Reach`).
//!
//! - a deposit or a cheque belongs to the agreement it was taken for (a
//!   lease), so to the agreement's scope;
//! - a payment belongs to the scope of the invoices it is allocated to, of
//!   the agreement it was prepaid for, or of the deposit it paid;
//! - a refund belongs to the scope of the payment it returns.

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::invoicing::INVOICE_SUBJECT_TYPE;
use wl_kernel::payments::{DEPOSIT_SUBJECT_TYPE, PAYMENT_SUBJECT_TYPE};
use wl_kernel::scopes::{self, Reach};

use crate::allocation::Allocation;
use crate::deposit::Deposit;
use crate::payment::Payment;
use crate::refund::Refund;

/// The ids of the payments in `scopes`.
pub(crate) async fn payment_ids(
    db: &Db,
    scopes: &[renox::auth::permissions::Scope],
) -> Result<Vec<i64>> {
    let reach = Reach::only(scopes.to_vec());
    let mut ids: Vec<i64> = reach
        .apply(db, Payment::query(), "prepaid_type", "prepaid_id")
        .await?
        .get(db)
        .await?
        .into_iter()
        .map(|payment| payment.id)
        .collect();
    let invoices = scopes::members_in(db, INVOICE_SUBJECT_TYPE, scopes).await?;
    if !invoices.is_empty() {
        ids.extend(
            Allocation::query()
                .where_eq("target_type", INVOICE_SUBJECT_TYPE)
                .where_in("target_id", invoices)
                .where_null("removed_at")
                .get(db)
                .await?
                .into_iter()
                .map(|allocation| allocation.payment_id),
        );
    }
    let deposits: Vec<Option<i64>> = reach
        .apply(db, Deposit::query(), "source_type", "source_id")
        .await?
        .get(db)
        .await?
        .into_iter()
        .map(|deposit| deposit.received_payment_id)
        .collect();
    ids.extend(deposits.into_iter().flatten());
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

/// Tells the kernel how to list the payments and deposits of some scopes.
pub(crate) fn register() {
    scopes::register_derived_members(DEPOSIT_SUBJECT_TYPE, |db, scopes| {
        Box::pin(async move {
            let reach = Reach::only(scopes.to_vec());
            Ok(reach
                .apply(db, Deposit::query(), "source_type", "source_id")
                .await?
                .get(db)
                .await?
                .into_iter()
                .map(|deposit| deposit.id)
                .collect())
        })
    });
    scopes::register_derived_members(PAYMENT_SUBJECT_TYPE, |db, scopes| {
        Box::pin(async move { payment_ids(db, scopes).await })
    });
}

/// Whether the payment `id` is in `reach`.
pub(crate) async fn payment_in_reach(db: &Db, reach: &Reach, id: i64) -> Result<bool> {
    Ok(match reach.ids(db, PAYMENT_SUBJECT_TYPE).await? {
        None => true,
        Some(ids) => ids.contains(&id),
    })
}

/// Whether the deposit's agreement is in `reach`.
pub(crate) async fn deposit_in_reach(db: &Db, reach: &Reach, deposit: &Deposit) -> Result<bool> {
    reach.contains(db, &deposit.source()).await
}

/// Whether the cheque's agreement is in `reach`; a cheque with none is for
/// global holders only.
pub(crate) async fn agreement_in_reach(
    db: &Db,
    reach: &Reach,
    agreement: Option<Subject>,
) -> Result<bool> {
    match agreement {
        Some(agreement) => reach.contains(db, &agreement).await,
        None => Ok(reach.is_all()),
    }
}

/// The ids of the refunds in `reach`, `None` for every one: a refund returns
/// a payment, a credit note or a deposit, and is in the scope of that source.
pub(crate) async fn refund_ids(db: &Db, reach: &Reach) -> Result<Option<Vec<i64>>> {
    if reach.is_all() {
        return Ok(None);
    }
    let mut ids: Vec<i64> = Vec::new();
    for source_type in [
        PAYMENT_SUBJECT_TYPE,
        wl_kernel::invoicing::CREDIT_NOTE_SUBJECT_TYPE,
        DEPOSIT_SUBJECT_TYPE,
    ] {
        let sources = reach.ids(db, source_type).await?.unwrap_or_default();
        if sources.is_empty() {
            continue;
        }
        ids.extend(
            Refund::query()
                .where_eq("source_type", source_type)
                .where_in("source_id", sources)
                .get(db)
                .await?
                .into_iter()
                .map(|refund| refund.id),
        );
    }
    ids.sort_unstable();
    ids.dedup();
    Ok(Some(ids))
}
