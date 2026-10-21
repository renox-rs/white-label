//! Netting open invoices against the COD in a remittance (03 Logistics §4.1
//! row *Client owes invoices*, §7.1 rows `pay` and `return_payment`, §7.2
//! `LOG.RUL.06`, `LOG.COD.01`, `FIN.PAY.01`; BL-LOG-16).
//!
//! - **Which invoices.** When the client's contract allows netting
//!   ([`crate::reweigh::ClientTerms::netting_allowed`], off unless set), the
//!   draft offsets the client's open invoices that are not disputed: issued,
//!   partly paid or overdue with a balance, read through the kernel's
//!   `Invoicer::open_of` and kept when they are delivery invoices: the
//!   invoices of the monthly runs ([`crate::billing::BillingRun`]). The
//!   kernel summary has no source, so the runs name them; an invoice for
//!   other services (a freight job, a supplementary invoice) is never set
//!   off against COD.
//! - **Order and cap.** Oldest due date first, an invoice offset in part when
//!   the amount before offsets runs out, so the net is never negative
//!   ([`plan`]).
//! - **Re-check at `pay`.** An invoice that became disputed, paid or written
//!   off since the draft is removed and the remittance goes back to a draft
//!   with a message ([`changes`]); `pay` never pays a silently smaller
//!   amount.
//! - **At `pay`** each offset becomes one incoming payment of the client,
//!   allocated in full to its invoice ([`crate::remit::pay`]). Nothing is
//!   recorded before.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, InvoiceSummary, invoicer};

use crate::billing::BillingRun;
use crate::remittance::Remittance;
use crate::reweigh;

/// The currency remittances are paid in.
const CURRENCY: &str = "AED";

/// One open invoice offset against the COD of a remittance.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_remittance_offsets", index(remittance_id))]
pub struct Offset {
    /// The primary key.
    pub id: i64,
    /// The remittance.
    #[model(references = "logistics_remittances")]
    pub remittance_id: i64,
    /// The invoice in wl-invoicing.
    pub invoice_id: i64,
    /// Its number (`INV-n` while it has none).
    pub invoice_number: String,
    /// Its due date, when known.
    pub due_date: Option<NaiveDate>,
    /// What is offset, in fils.
    pub amount: i64,
    /// The payment (method *COD netting*) allocated to the invoice at `pay`.
    pub payment_id: Option<i64>,
    /// Its number.
    pub payment_number: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The offsets of remittance `remittance`, in the order they were taken.
///
/// # Errors
///
/// Database errors.
pub async fn offsets_of(db: &Db, remittance: i64) -> Result<Vec<Offset>> {
    Offset::query()
        .where_(Offset::REMITTANCE_ID.eq(remittance))
        .order_by(Offset::ID)
        .get(db)
        .await
}

/// The number of an invoice: its own, or `INV-n` while it has none.
pub fn number_of(invoice: &InvoiceSummary) -> String {
    invoice
        .number
        .clone()
        .unwrap_or_else(|| format!("INV-{}", invoice.id.0))
}

/// The offsets taken from `available` fils: the open, undisputed invoices
/// with a balance, oldest due date first (then oldest invoice), each for its
/// balance or what is left of `available`. Their sum never exceeds
/// `available`.
pub fn plan(available: i64, open: &[InvoiceSummary]) -> Vec<(InvoiceSummary, i64)> {
    let mut candidates: Vec<&InvoiceSummary> = open
        .iter()
        .filter(|invoice| {
            invoice.status.is_open()
                && !invoice.disputed
                && invoice.balance > 0
                && invoice.currency.eq_ignore_ascii_case(CURRENCY)
        })
        .collect();
    candidates.sort_by_key(|invoice| (invoice.due_date, invoice.id.0));
    let mut left = available.max(0);
    let mut taken = Vec::new();
    for invoice in candidates {
        if left <= 0 {
            break;
        }
        let amount = invoice.balance.min(left);
        taken.push((invoice.clone(), amount));
        left -= amount;
    }
    taken
}

/// The client's open invoices to offset: none unless its contract allows
/// netting.
///
/// # Errors
///
/// Database errors and the invoicer's.
pub async fn open_invoices(state: &AppState, client: PartyId) -> Result<Vec<InvoiceSummary>> {
    let terms = reweigh::terms(&state.db, client).await?;
    if !terms.netting_allowed {
        return Ok(Vec::new());
    }
    let delivery_invoices: Vec<i64> = BillingRun::query()
        .where_(BillingRun::CLIENT_PARTY_ID.eq(client.0))
        .where_(BillingRun::INVOICE_ID.is_not_null())
        .get(&state.db)
        .await?
        .into_iter()
        .filter_map(|run| run.invoice_id)
        .collect();
    let mut open = invoicer(state).open_of(&state.db, client).await?;
    open.retain(|invoice| delivery_invoices.contains(&invoice.id.0));
    Ok(open)
}

/// Replaces the offsets of the draft `remittance` with `taken`, inside `tx`;
/// returns their sum.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn store(
    tx: &mut Transaction,
    remittance: i64,
    taken: &[(InvoiceSummary, i64)],
) -> Result<i64> {
    Offset::query()
        .where_(Offset::REMITTANCE_ID.eq(remittance))
        .delete(&mut *tx)
        .await?;
    let mut total = 0;
    for (invoice, amount) in taken {
        let mut offset = Offset {
            remittance_id: remittance,
            invoice_id: invoice.id.0,
            invoice_number: number_of(invoice),
            due_date: invoice.due_date,
            amount: *amount,
            ..Offset::default()
        };
        offset.insert(&mut *tx).await?;
        total += amount;
    }
    Ok(total)
}

/// Why `offset` can no longer be paid from `now`, the invoice as it is now
/// (`None`: it is gone): `disputed`, `written_off`, `paid` or `changed`.
/// `None` when the offset still holds.
pub fn change_of(offset: &Offset, now: Option<&InvoiceSummary>) -> Option<&'static str> {
    let Some(invoice) = now else {
        return Some("changed");
    };
    if invoice.disputed {
        Some("disputed")
    } else if invoice.status == InvoiceStatus::WrittenOff {
        Some("written_off")
    } else if invoice.status == InvoiceStatus::Paid || invoice.balance <= 0 {
        Some("paid")
    } else if !invoice.status.is_open() || invoice.balance < offset.amount {
        Some("changed")
    } else {
        None
    }
}

/// The offsets of `remittance` that no longer hold, each with the reason
/// ([`change_of`]).
///
/// # Errors
///
/// Database errors and the invoicer's.
pub async fn changes(
    state: &AppState,
    remittance: &Remittance,
) -> Result<Vec<(Offset, &'static str)>> {
    let invoices = invoicer(state);
    let mut changed = Vec::new();
    for offset in remittance.offsets(&state.db).await? {
        let now = invoices
            .find(&state.db, InvoiceId(offset.invoice_id))
            .await?;
        if let Some(reason) = change_of(&offset, now.as_ref()) {
            changed.push((offset, reason));
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invoice(id: i64, balance: i64, due: (i32, u32, u32), disputed: bool) -> InvoiceSummary {
        InvoiceSummary {
            id: InvoiceId(id),
            number: Some(format!("INV-{id}")),
            status: InvoiceStatus::Issued,
            party: PartyId(501),
            legal_entity_id: Some(1),
            currency: "AED".to_owned(),
            gross_total: balance,
            balance,
            due_date: NaiveDate::from_ymd_opt(due.0, due.1, due.2),
            disputed,
            chasing_paused_until: None,
        }
    }

    #[test]
    fn the_oldest_due_date_is_offset_first_and_the_last_in_part() {
        let open = vec![
            invoice(2, 180_000, (2026, 11, 1), false),
            invoice(1, 50_000, (2026, 10, 1), false),
        ];
        let taken = plan(200_000, &open);
        let amounts: Vec<(i64, i64)> = taken.iter().map(|(i, a)| (i.id.0, *a)).collect();
        assert_eq!(amounts, vec![(1, 50_000), (2, 150_000)]);
    }

    #[test]
    fn a_disputed_invoice_is_never_offset() {
        let open = vec![
            invoice(1, 50_000, (2026, 10, 1), true),
            invoice(2, 180_000, (2026, 11, 1), false),
        ];
        let taken = plan(200_000, &open);
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].0.id.0, 2);
        assert_eq!(taken[0].1, 180_000);
    }

    #[test]
    fn the_offsets_never_exceed_what_is_available() {
        let open = vec![invoice(1, 500_000, (2026, 10, 1), false)];
        assert_eq!(plan(0, &open).len(), 0);
        assert_eq!(plan(-5, &open).len(), 0);
        assert_eq!(plan(120_000, &open)[0].1, 120_000);
    }

    #[test]
    fn an_offset_that_no_longer_holds_says_why() {
        let offset = Offset {
            amount: 150_000,
            ..Offset::default()
        };
        let fine = invoice(2, 180_000, (2026, 11, 1), false);
        assert_eq!(change_of(&offset, Some(&fine)), None);
        let disputed = invoice(2, 180_000, (2026, 11, 1), true);
        assert_eq!(change_of(&offset, Some(&disputed)), Some("disputed"));
        let mut paid = fine.clone();
        paid.status = InvoiceStatus::Paid;
        paid.balance = 0;
        assert_eq!(change_of(&offset, Some(&paid)), Some("paid"));
        let mut off = fine.clone();
        off.status = InvoiceStatus::WrittenOff;
        assert_eq!(change_of(&offset, Some(&off)), Some("written_off"));
        let mut less = fine.clone();
        less.balance = 100_000;
        assert_eq!(change_of(&offset, Some(&less)), Some("changed"));
        assert_eq!(change_of(&offset, None), Some("changed"));
    }
}
