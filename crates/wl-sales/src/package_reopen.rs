//! A package whose payment was reversed (wl-sales §4.4 "Payment reversed after
//! activation", §6; BL-SAL-16).
//!
//! When a payment for a package is reversed or charged back, wl-invoicing
//! reopens its invoice (`InvoiceReopened`). **The package does not change**:
//! it stays `active` (or `completed`, or `expired`), and the credits already
//! used stay used. What follows, once per reversal:
//!
//! - the package remembers the reopened balance ([`Package::reopened_balance`]);
//! - the staff who sold it get the task *collect before next use* (the seller
//!   user, else the role of the setting, else the company admin);
//! - the holder (and the payer, when it is another party) is told the balance
//!   is due.
//!
//! With the setting *suspend consumption while unpaid* on, `consume` is
//! refused until the invoice is paid ([`crate::package_machine`] guard
//! `paid_up`). The refusal is recorded ([`PackageRefusal`]) and added to the
//! seller's task as a comment; it is not replayed when the balance is paid,
//! the seller consumes it by hand. `InvoicePaid` ends the suspension.
//!
//! `PaymentReversed` is handled only for a package with **no invoice**, one
//! paid in advance by a payment link: its payment names it as the prepaid
//! subject and nothing else will reopen anything. A package with an invoice
//! is handled from `InvoiceReopened`, which carries the real balance; the
//! reversal is the idempotency key of both ([`PackageReopening`]).
//!
//! Decision: a package that is still `pending_payment` (never activated), or
//! `cancelled` or `refunded`, is left alone: there is no credit to protect.
//! A future instalment not yet due is not a reopened balance: only
//! `InvoiceReopened` makes one.

use renox::db::DateTime;
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};
use wl_kernel::invoicing::{InvoicePaid, InvoiceReopened};
use wl_kernel::notify::{self, NotifyRequest};
use wl_kernel::payments::PaymentReversed;
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::sales::PACKAGE_SUBJECT_TYPE;
use wl_kernel::tickets::{
    Assignee, OpenTicket, TASK_CATEGORY, TaskComment, TicketId, task_creator,
};
use wl_kernel::{COMPANY_ADMIN_ROLE, PartyId, Subject};

use crate::config::config_of;
use crate::notices::{PACKAGE_BALANCE_DUE, aed};
use crate::package::{Package, PackageStatus};
use crate::packages::of_invoice;
use crate::texts::english;

/// One reversal that reopened a package's invoice. The unique (package, money
/// source) makes an event that is delivered twice open one task.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_package_reopenings",
    unique(package_id, money_source)
)]
pub struct PackageReopening {
    /// The primary key.
    pub id: i64,
    /// The package.
    #[model(references = "wl_sales_packages")]
    pub package_id: i64,
    /// The record the money came from, e.g. `payments.payment:42`.
    pub money_source: String,
    /// The invoice reopened.
    pub invoice_id: Option<i64>,
    /// The balance after the reopening, in fils.
    pub balance: i64,
    /// The seller's task.
    pub ticket_id: Option<i64>,
    /// When the invoice was paid again; the row no longer counts.
    pub settled_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A consumption refused while consumption is suspended for an unpaid
/// reopened balance.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_sales_package_refusals", index(package_id))]
pub struct PackageRefusal {
    /// The primary key.
    pub id: i64,
    /// The package.
    #[model(references = "wl_sales_packages")]
    pub package_id: i64,
    /// The type of the record that wanted the credit.
    pub source_type: String,
    /// The id of the record that wanted the credit.
    pub source_id: i64,
    /// The credits it wanted.
    pub quantity: i64,
    /// Why: `unpaid`.
    pub reason: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// What [`reopen`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reopened {
    /// The task is open and the holder told.
    Handled,
    /// This reversal was handled before; nothing changed.
    Already,
    /// The package has no credit to protect (never activated, cancelled or
    /// refunded); nothing changed.
    Ignored,
}

/// The source key of the seller's task for the reversal `money_source`.
pub fn task_key(package: i64, money_source: &Subject) -> String {
    format!("sales.package_collect:{package}:{money_source}")
}

/// Handles a reversal that reopened `balance` fils on the package `package`:
/// remembers the balance, opens the seller's task once and tells the holder.
/// The package keeps its status and its used credits.
pub async fn reopen(
    state: &AppState,
    package: i64,
    money_source: &Subject,
    invoice: Option<i64>,
    balance: i64,
) -> Result<Reopened> {
    let package = Package::find_or_404(&state.db, package).await?;
    if !matches!(
        package.status,
        PackageStatus::Active | PackageStatus::Completed | PackageStatus::Expired
    ) {
        return Ok(Reopened::Ignored);
    }
    let key = money_source.to_string();
    let seen = PackageReopening::query()
        .where_(PackageReopening::PACKAGE_ID.eq(package.id))
        .where_(PackageReopening::MONEY_SOURCE.eq(key.clone()))
        .exists(&state.db)
        .await?;
    if seen {
        return Ok(Reopened::Already);
    }
    // The row comes first: it is what makes the event idempotent.
    let row = PackageReopening::create(
        &state.db,
        PackageReopening {
            package_id: package.id,
            money_source: key,
            invoice_id: invoice,
            balance,
            ..PackageReopening::default()
        },
    )
    .await?;
    refresh_balance(state, &package).await?;

    let ticket = open_collect_task(state, &package, money_source, balance).await?;
    if let Some(ticket) = ticket {
        PackageReopening::query()
            .where_(PackageReopening::ID.eq(row.id))
            .update(&state.db, &[("ticket_id", &Some(ticket.0))])
            .await?;
        Package::query()
            .where_(Package::ID.eq(package.id))
            .update(&state.db, &[("collect_ticket_id", &Some(ticket.0))])
            .await?;
    }
    tell_balance_due(state, &package, balance).await?;
    Ok(Reopened::Handled)
}

/// Opens the seller's task *collect before next use*; the seller user when
/// the package has one, else the holder of the configured role.
async fn open_collect_task(
    state: &AppState,
    package: &Package,
    money_source: &Subject,
    balance: i64,
) -> Result<Option<TicketId>> {
    let due = format!("AED {}", aed(balance));
    let (title, description) = {
        let params: [(&str, &dyn std::fmt::Display); 2] =
            [("balance", &due), ("package", &package.item_name)];
        (
            english("wl_sales.packages.collect_title", &params),
            english("wl_sales.packages.collect_description", &params),
        )
    };
    let key = task_key(package.id, money_source);
    let subject = package.subject();
    if let Some(user) = package.sold_by {
        let creator = task_creator(state);
        if let Some(found) = creator.find_open(state, &key).await? {
            return Ok(Some(found.id));
        }
        let ticket = OpenTicket::new(subject, TASK_CATEGORY, &title)
            .description(&description)
            .source_key(&key)
            .assign_to(Assignee::user(user));
        return Ok(creator.open(state, ticket).await?.map(|t| t.id));
    }
    let config = config_of(state);
    let role = if config.package_seller_role.is_empty() {
        COMPANY_ADMIN_ROLE
    } else {
        config.package_seller_role.as_str()
    };
    let task = RoleTask::new(subject, role, &title, &key).description(&description);
    Ok(role_tasks::open(state, task).await?.map(|t| t.id))
}

/// Tells the holder, and the payer when it is another party, that the balance
/// is due.
async fn tell_balance_due(state: &AppState, package: &Package, balance: i64) -> Result {
    let mut parties = vec![PartyId(package.holder_party_id)];
    if let Some(payer) = package.payer_party_id.map(PartyId)
        && !parties.contains(&payer)
    {
        parties.push(payer);
    }
    for party in parties {
        let request = NotifyRequest::new(PACKAGE_BALANCE_DUE, package.subject())
            .to_party(party)
            .data(json!({
                "package": package.item_name,
                "balance": aed(balance),
            }));
        notify::notifier(state).notify(state, request).await?;
    }
    Ok(())
}

/// Records a consumption that was refused because the balance is unpaid, and
/// adds it to the seller's task.
pub(crate) async fn record_refusal(
    state: &AppState,
    package: &Package,
    source: &Subject,
    quantity: i64,
) -> Result {
    PackageRefusal {
        package_id: package.id,
        source_type: source.subject_type.clone(),
        source_id: source.subject_id,
        quantity,
        reason: "unpaid".to_owned(),
        ..PackageRefusal::default()
    }
    .insert(&state.db)
    .await?;
    if let Some(ticket) = package.collect_ticket_id {
        let text = {
            let params: [(&str, &dyn std::fmt::Display); 2] =
                [("quantity", &quantity), ("source", source)];
            english("wl_sales.packages.refused_comment", &params)
        };
        task_creator(state)
            .comment(state, TicketId(ticket), TaskComment::internal(&text))
            .await?;
    }
    Ok(())
}

/// The refusals recorded for `package`, oldest first.
pub async fn refusals(db: &Db, package: i64) -> Result<Vec<PackageRefusal>> {
    PackageRefusal::query()
        .where_(PackageRefusal::PACKAGE_ID.eq(package))
        .order_by(PackageRefusal::ID)
        .get(db)
        .await
}

/// `InvoiceReopened` of a package's invoice.
async fn on_invoice_reopened(state: &AppState, event: &InvoiceReopened) -> Result {
    let by_source =
        (event.source.subject_type == PACKAGE_SUBJECT_TYPE).then_some(event.source.subject_id);
    let package = match by_source {
        Some(id) => Some(id),
        None => of_invoice(&state.db, event.invoice).await?.map(|p| p.id),
    };
    let Some(package) = package else {
        return Ok(());
    };
    reopen(
        state,
        package,
        &event.money_source,
        Some(event.invoice.0),
        event.balance,
    )
    .await
    .map(|_| ())
}

/// `PaymentReversed` of a payment made in advance for a package with no
/// invoice.
async fn on_payment_reversed(state: &AppState, event: &PaymentReversed) -> Result {
    let mut targets: Vec<(i64, i64)> = Vec::new();
    if let Some(prepaid) = &event.prepaid
        && prepaid.subject_type == PACKAGE_SUBJECT_TYPE
    {
        targets.push((prepaid.subject_id, 0));
    }
    for removed in &event.removed {
        if removed.target.subject_type == PACKAGE_SUBJECT_TYPE {
            targets.push((removed.target.subject_id, removed.amount));
        }
    }
    for (id, amount) in targets {
        let Some(package) = Package::find(&state.db, id).await? else {
            continue;
        };
        // A package with an invoice is handled from `InvoiceReopened`.
        if package.invoice_id.is_some() {
            continue;
        }
        let balance = if amount > 0 {
            amount
        } else {
            package.price_paid
        };
        reopen(state, id, &event.payment.subject(), None, balance).await?;
    }
    Ok(())
}

/// The invoice is paid: a reopened balance of the package is paid, so
/// consumption is no longer suspended.
pub(crate) async fn on_invoice_paid(state: &AppState, event: &InvoicePaid) -> Result {
    let by_source =
        (event.source.subject_type == PACKAGE_SUBJECT_TYPE).then_some(event.source.subject_id);
    let package = match by_source {
        Some(id) => Package::find(&state.db, id).await?,
        None => of_invoice(&state.db, event.invoice).await?,
    };
    let Some(package) = package else {
        return Ok(());
    };
    // Only what this invoice reopened is paid; another invoice of the
    // package may still be open.
    PackageReopening::query()
        .where_(PackageReopening::PACKAGE_ID.eq(package.id))
        .where_(PackageReopening::INVOICE_ID.eq(event.invoice.0))
        .where_(PackageReopening::SETTLED_AT.is_null())
        .update(&state.db, &[("settled_at", &Some(renox::db::now()))])
        .await?;
    refresh_balance(state, &package).await
}

/// Sets the package's reopened balance to what is still open: for each
/// invoice the balance of its latest reopening (an invoice reopened again
/// has the new balance, not the sum), plus each reopening that names no
/// invoice; a paid invoice counts 0.
///
/// Decision: a package normally has one invoice, but an event names the
/// package as its source, so a second invoice for the same package adds its
/// own balance instead of replacing the first one's.
async fn refresh_balance(state: &AppState, package: &Package) -> Result {
    let rows = PackageReopening::query()
        .where_(PackageReopening::PACKAGE_ID.eq(package.id))
        .order_by(PackageReopening::ID)
        .get(&state.db)
        .await?;
    let mut per_invoice: std::collections::BTreeMap<i64, i64> = std::collections::BTreeMap::new();
    let mut without_invoice = 0_i64;
    for row in &rows {
        let open = if row.settled_at.is_some() { 0 } else { row.balance };
        match row.invoice_id {
            Some(invoice) => {
                per_invoice.insert(invoice, open);
            }
            None => without_invoice += open,
        }
    }
    // Reversals with no invoice never reopen more than the package cost.
    if package.price_paid > 0 {
        without_invoice = without_invoice.min(package.price_paid);
    }
    let total = per_invoice.values().sum::<i64>() + without_invoice;
    let since = if total > 0 {
        Some(package.reopened_at.unwrap_or_else(renox::db::now))
    } else {
        None
    };
    Package::query()
        .where_(Package::ID.eq(package.id))
        .update(
            &state.db,
            &[("reopened_balance", &total), ("reopened_at", &since)],
        )
        .await?;
    Ok(())
}

/// Registers the listeners.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: InvoiceReopened, state: AppState| async move {
        on_invoice_reopened(&state, &event).await
    });
    app.listen(|event: PaymentReversed, state: AppState| async move {
        on_payment_reversed(&state, &event).await
    });
    app.listen(|event: InvoicePaid, state: AppState| async move {
        on_invoice_paid(&state, &event).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_key_names_the_package_and_the_reversal() {
        let key = task_key(9, &Subject::new("payments.payment", 42));
        assert_eq!(key, "sales.package_collect:9:payments.payment:42");
    }
}
