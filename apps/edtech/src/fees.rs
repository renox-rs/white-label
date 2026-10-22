//! The fees of an enrolment (10 §4.3 steps 1-2, `FIN.BIL.01`, `FIN.SUB.01`):
//! the first invoice, the instalment plan, the invoices of the later
//! instalments on their due dates, the package sold for a package enrolment
//! and the subscription of a membership.
//!
//! Everything goes through the kernel's contracts: [`Invoicer`] for the
//! invoices, [`PackageSales`] for the package, [`Subscriptions`] for the
//! membership. Without `invoicing`, `sales` or the subscriptions the
//! enrolment is still created, without an invoice, package or subscription.
//!
//! Decisions:
//!
//! - Amounts on the enrolment are fils with VAT; the invoice lines carry the
//!   amount before VAT ([`crate::refund::net_of_gross`], half-up), VAT at 5%.
//! - The course fee is split in equal instalments, the remainder on the
//!   first; the first falls due on the day of the enrolment and is invoiced
//!   with the registration fee, the others monthly after it.
//! - The invoices are drafted and then issued as the system. Where the
//!   invoicing module is set not to issue that source automatically (the
//!   default), the draft stays for the accountant to issue and the enrolment
//!   waits for the payment as usual.

use renox::chrono::{Months, NaiveDate};
use renox::db::now;
use renox::prelude::*;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, InvoiceId, VatCategory, invoicer};
use wl_kernel::sales::{SellPackage, package_sales};
use wl_kernel::subscriptions::{StartSubscription, subscriptions};

use crate::enrolment::{Enrolment, EnrolmentKind, EnrolmentStatus, Instalment, InstalmentStatus};
use crate::refund::net_of_gross;
use crate::texts::english;

/// The scheduled task and command that invoice the instalments that fall due.
pub const TASK: &str = "edtech:instalments";
/// The time of day it runs.
pub const TASK_AT: &str = "00:40";
/// The time zone of [`TASK_AT`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// The amounts of `count` instalments of `total`: equal, the remainder on the
/// first.
pub fn split(total: i64, count: i64) -> Vec<i64> {
    let count = count.max(1);
    let each = total.max(0) / count;
    let rest = total.max(0) - each * count;
    (0..count)
        .map(|n| if n == 0 { each + rest } else { each })
        .collect()
}

/// The due date of instalment `number` (1-based) of a plan that starts on
/// `start`: monthly.
pub fn due_date(start: NaiveDate, number: i64) -> NaiveDate {
    let months = u32::try_from((number - 1).max(0)).unwrap_or(0);
    start.checked_add_months(Months::new(months)).unwrap_or(start)
}

/// One line of an enrolment's invoice: `gross` with VAT.
fn line(description: &str, gross: i64, enrolment: &Enrolment) -> DraftLine {
    DraftLine::new(description, net_of_gross(gross), VatCategory::Standard)
        .origin(enrolment.subject())
}

/// Drafts, and tries to issue, an invoice to the payer of `enrolment` with
/// `lines`; `None` when invoicing is off or there is nothing to bill.
async fn bill(
    state: &AppState,
    enrolment: &Enrolment,
    lines: Vec<DraftLine>,
) -> Result<Option<InvoiceId>> {
    if lines.is_empty() {
        return Ok(None);
    }
    let mut draft = InvoiceDraft::new(enrolment.payer(), enrolment.subject());
    if let (Some(kind), Some(id)) = (&enrolment.campus_type, enrolment.campus_id) {
        draft = draft.scope(wl_kernel::Subject::new(kind, id));
    }
    for line in lines {
        draft = draft.line(line);
    }
    let books = invoicer(state);
    let Some(id) = books.draft(&state.db, draft).await? else {
        return Ok(None);
    };
    // The invoicing module issues a draft as the system only where it is set
    // to; otherwise the accountant issues it.
    let _issued = books.issue(state, id, None).await;
    Ok(Some(id))
}

/// Raises the fees of a new enrolment: the instalment plan and the first
/// invoice of a course, the invoice and the package of a package enrolment,
/// the registration fee of a membership. Returns the enrolment as it is after.
///
/// # Errors
///
/// The database's errors and the contracts'.
pub async fn raise(state: &AppState, enrolment: &Enrolment) -> Result<Enrolment> {
    let name = enrolment.item_name.clone().unwrap_or_default();
    let mut first_invoice: Option<InvoiceId> = None;
    let mut package: Option<i64> = None;
    match enrolment.kind {
        EnrolmentKind::Course => {
            let amounts = split(enrolment.fee(), enrolment.instalments);
            let start = today();
            let total = i64::try_from(amounts.len()).unwrap_or(1);
            let mut lines = Vec::new();
            if let Some(first) = amounts.first().copied().filter(|a| *a > 0) {
                lines.push(line(&instalment_text(&name, 1, total), first, enrolment));
            }
            if enrolment.registration_fee > 0 {
                lines.push(line(
                    &english("edtech.fees.registration", &[]),
                    enrolment.registration_fee,
                    enrolment,
                ));
            }
            first_invoice = bill(state, enrolment, lines).await?;
            for (index, amount) in amounts.iter().enumerate() {
                let number = i64::try_from(index).unwrap_or(0) + 1;
                let mut row = Instalment {
                    enrolment_id: enrolment.id,
                    number,
                    due_date: due_date(start, number),
                    amount: *amount,
                    invoice_id: (number == 1).then(|| first_invoice.map(|i| i.0)).flatten(),
                    status: if number == 1 && first_invoice.is_some() {
                        InstalmentStatus::Invoiced
                    } else {
                        InstalmentStatus::Scheduled
                    },
                    ..Instalment::default()
                };
                row.save(&state.db).await?;
            }
        }
        EnrolmentKind::Package => {
            let mut lines = vec![line(
                &english("edtech.fees.package", &[("item", &name)]),
                enrolment.fee(),
                enrolment,
            )];
            if enrolment.registration_fee > 0 {
                lines.push(line(
                    &english("edtech.fees.registration", &[]),
                    enrolment.registration_fee,
                    enrolment,
                ));
            }
            first_invoice = bill(state, enrolment, lines).await?;
            let mut sale = SellPackage::new(
                enrolment.student(),
                enrolment.item_code.as_deref().unwrap_or_default(),
                enrolment.sessions_total,
                enrolment.fee(),
            )
            .item_name(&name)
            .payer(enrolment.payer());
            if let Some(invoice) = first_invoice {
                sale = sale.invoice(invoice.0);
            }
            if let Some(user) = enrolment.counsellor_id {
                sale = sale.sold_by(user);
            }
            package = package_sales(state)
                .sell(state, sale)
                .await?
                .map(|id| id.0);
        }
        EnrolmentKind::Membership => {
            // The subscription starts when the enrolment activates; only the
            // registration fee is billed now.
            if enrolment.registration_fee > 0 {
                let lines = vec![line(
                    &english("edtech.fees.registration", &[]),
                    enrolment.registration_fee,
                    enrolment,
                )];
                first_invoice = bill(state, enrolment, lines).await?;
            }
        }
    }
    Enrolment::query()
        .where_(Enrolment::ID.eq(enrolment.id))
        .update(
            &state.db,
            &[
                ("first_invoice_id", &first_invoice.map(|i| i.0)),
                ("package_id", &package),
                ("updated_at", &now()),
            ],
        )
        .await?;
    let stored = Enrolment::find_or_404(&state.db, enrolment.id).await?;
    // The first payment is asked through a payment link that expires with the
    // seat hold (`EDU.RUL.11`); without payments there is none.
    let Some(invoice) = first_invoice else {
        return Ok(stored);
    };
    let Some(summary) = invoicer(state).find(&state.db, invoice).await? else {
        return Ok(stored);
    };
    crate::lapse::open_link(state, &stored, summary.gross_total).await
}

/// The description of instalment `number` of `total`.
fn instalment_text(name: &str, number: i64, total: i64) -> String {
    if total <= 1 {
        english("edtech.fees.course", &[("item", &name)])
    } else {
        english(
            "edtech.fees.instalment",
            &[("item", &name), ("number", &number), ("total", &total)],
        )
    }
}

/// Starts the subscription of a membership enrolment, the day it activates.
/// Returns the enrolment as it is after; one that has a subscription already
/// is left as it is.
///
/// # Errors
///
/// The database's errors and the contract's.
pub async fn start_membership(state: &AppState, enrolment: &Enrolment) -> Result<Enrolment> {
    if enrolment.kind != EnrolmentKind::Membership || enrolment.subscription_id.is_some() {
        return Ok(enrolment.clone());
    }
    let plan = enrolment
        .item_name
        .clone()
        .unwrap_or_else(|| english("edtech.fees.membership", &[]));
    let request = StartSubscription::new(
        enrolment.payer(),
        enrolment.subject(),
        plan,
        net_of_gross(enrolment.fee()),
        today(),
    );
    if let Some(id) = subscriptions(state).start(state, request).await? {
        Enrolment::query()
            .where_(Enrolment::ID.eq(enrolment.id))
            .update(
                &state.db,
                &[("subscription_id", &Some(id.0)), ("updated_at", &now())],
            )
            .await?;
    }
    Enrolment::find_or_404(&state.db, enrolment.id).await
}

/// Invoices the instalments that fall due on or before `on`, for the enrolments
/// that are `active` (a frozen or ended enrolment is not billed). Returns the
/// number invoiced.
///
/// # Errors
///
/// The database's errors and the contract's.
pub async fn invoice_due(state: &AppState, on: NaiveDate) -> Result<usize> {
    let due = Instalment::query()
        .where_(Instalment::STATUS.eq(InstalmentStatus::Scheduled))
        .where_(Instalment::DUE_DATE.lte(on))
        .order_by(Instalment::ID)
        .get(&state.db)
        .await?;
    let mut invoiced = 0;
    for row in due {
        let Some(enrolment) = Enrolment::find(&state.db, row.enrolment_id).await? else {
            continue;
        };
        if enrolment.status != EnrolmentStatus::Active {
            continue;
        }
        let total = Instalment::of_enrolment(&state.db, enrolment.id).await?.len();
        let name = enrolment.item_name.clone().unwrap_or_default();
        let text = instalment_text(&name, row.number, i64::try_from(total).unwrap_or(1));
        let lines = vec![line(&text, row.amount, &enrolment)];
        let Some(invoice) = bill(state, &enrolment, lines).await? else {
            continue;
        };
        Instalment::query()
            .where_(Instalment::ID.eq(row.id))
            .update(
                &state.db,
                &[
                    ("invoice_id", &Some(invoice.0)),
                    ("status", &InstalmentStatus::Invoiced),
                    ("updated_at", &now()),
                ],
            )
            .await?;
        invoiced += 1;
    }
    Ok(invoiced)
}

/// Registers the daily run and its command.
pub(crate) fn register(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            invoice_due(&state, today()).await.map(|_| ())
        })
        .timezone(TASK_ZONE);
    app.command(
        TASK,
        "Invoice the instalments of the active enrolments that fall due",
        |_args, state: AppState| async move {
            let invoiced = invoice_due(&state, today()).await?;
            println!("{invoiced} instalments invoiced.");
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fee_is_split_in_equal_instalments_with_the_remainder_on_the_first() {
        assert_eq!(split(252_000, 2), [126_000, 126_000]);
        assert_eq!(split(100, 3), [34, 33, 33]);
        assert_eq!(split(100, 1), [100]);
        assert_eq!(split(100, 0), [100]);
        assert_eq!(split(0, 2), [0, 0]);
    }

    #[test]
    fn instalments_fall_due_a_month_apart() {
        let start = NaiveDate::from_ymd_opt(2026, 11, 30).unwrap();
        assert_eq!(due_date(start, 1), start);
        assert_eq!(due_date(start, 2), NaiveDate::from_ymd_opt(2026, 12, 30).unwrap());
        assert_eq!(due_date(start, 4), NaiveDate::from_ymd_opt(2027, 2, 28).unwrap());
    }
}
