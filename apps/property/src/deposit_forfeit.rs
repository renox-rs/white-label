//! Applying the deposit in full when the deductions exceed it (01 Property
//! §4.3 row "Deductions exceed the deposit", §7.1 *Deposit* `forfeit`,
//! `PRP.ACL.05`, §9 "Deposits and damages"; BL-PRP-17).
//!
//! When the approved deductions (unpaid rent plus the damage lines published to
//! the tenant) reach the deposit, the accountant forfeits it with the reason
//! *applied to deductions* ([`apply`]):
//!
//! - **Before:** a version of the deductions is published to the tenant, no
//!   line is disputed and its 14-day window has passed; the total deductions
//!   are at least the deposit (below it the settlement goes through `refund`
//!   and the reason is refused); a property manager of the building approves,
//!   and above AED 5,000 of total deductions (`PRP.ACL.05`) a holder of the
//!   finance manager or owner role approves too; the damages charge type is
//!   configured when there are damage lines ([`DAMAGES`], no platform default
//!   while A-06 is open).
//! - **In wl-payments** (the kernel's
//!   [`Deposits::apply_to_deductions`](wl_kernel::deposits::Deposits)): the
//!   deposit is forfeited and applied to the unpaid rent invoices, oldest
//!   first, as payments of method *deposit applied* (the rent keeps the VAT on
//!   its own invoice, the deposit itself carries none); the rest covers the
//!   damage lines in the order they were published.
//! - **After:** what is left of the damage lines is invoiced to the tenant as
//!   one damages invoice with one line per remaining damage, VAT from the
//!   charge type, due after the client's standard terms; the invoice follows
//!   the arrears flow like a rent invoice ([`crate::arrears`]). Unpaid rent the
//!   deposit did not cover stays open on its invoices. A settlement statement
//!   ([`DepositSettlement`]) keeps the figures and the invoice number.
//!
//! Decided where the spec is silent:
//!
//! - The unpaid rent is every invoice of a charge of the lease that is open
//!   with a balance and not disputed; a disputed invoice is left out.
//! - The damage lines are the lines of the latest published version; unpaid
//!   rent is not a line of it, because the tenant already has those invoices.
//! - A forfeiture is not money out, so a blocked tenant does not stop it.
//! - The accountant who forfeits holds `deposits.refund` (wl-payments'
//!   permission) in the lease's building; the approver is another user.

use renox::chrono::NaiveDate;
use renox::auth::permissions::{users_with_role, users_with_role_in};
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::VatCategory;
use wl_kernel::deposits::{ApplyDeposit, ApplyToInvoice, deposit_deductions, deposits};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, InvoiceId, InvoiceStatus, invoicer};

use crate::building::{BUILDING_SUBJECT_TYPE, refused};
use crate::charge::ChargeStatus;
use crate::charge_type::ChargeType;
use crate::lease::Lease;
use crate::permissions::{DEPOSITS_REFUND, PROPERTY_MANAGER_ROLE, SECOND_APPROVER_ROLES};
use crate::texts::english;

/// The rule that asks a second approver above a threshold.
pub const RULE: &str = "PRP.ACL.05";

/// The total of the deductions, in fils, above which a second approver is
/// needed (`PRP.ACL.05`, AED 5,000).
pub const SECOND_APPROVAL_ABOVE: i64 = 500_000;

/// The code of the charge type the damages are invoiced with.
pub const DAMAGES: &str = "damages";

/// The settlement statement of a deposit applied to the deductions.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "property_deposit_settlements", unique(deposit_id), index(root_id))]
pub struct DepositSettlement {
    /// The primary key.
    pub id: i64,
    /// The id of version 1: the lease the deposit was held for.
    pub root_id: i64,
    /// The version it was settled on.
    #[model(references = "property_leases")]
    pub lease_id: i64,
    /// The deposit.
    pub deposit_id: i64,
    /// What the deposit held, in fils.
    pub held: i64,
    /// The unpaid rent the deposit was set against, in fils.
    pub rent_due: i64,
    /// The damage lines, in fils.
    pub damages: i64,
    /// The part applied to the rent invoices.
    pub to_rent: i64,
    /// The part kept for the damage lines.
    pub to_damages: i64,
    /// The damages the deposit did not cover, invoiced to the tenant.
    pub excess: i64,
    /// The damages invoice.
    pub excess_invoice_id: Option<i64>,
    /// Its number, once issued.
    pub excess_invoice_number: Option<String>,
    /// The property manager who approved.
    pub approver_id: i64,
    /// The finance or owner approval above the threshold.
    pub second_approver_id: Option<i64>,
    /// The accountant who forfeited.
    pub applied_by: Option<i64>,
    /// The day.
    pub applied_on: NaiveDate,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl DepositSettlement {
    /// The settlement of the lease `root_id`, if its deposit was applied.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_root(db: &Db, root_id: i64) -> Result<Option<DepositSettlement>> {
        DepositSettlement::query()
            .where_(DepositSettlement::ROOT_ID.eq(root_id))
            .first(db)
            .await
    }

    /// The total deductions the statement shows: the unpaid rent and the
    /// damage lines.
    pub fn total_deductions(&self) -> i64 {
        self.rent_due + self.damages
    }
}

/// An unpaid invoice of the lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unpaid {
    /// The invoice.
    pub id: i64,
    /// Its number, once issued.
    pub number: Option<String>,
    /// What it is open for, in fils.
    pub open: i64,
    /// Its due date, else the due date of its charge.
    pub due: NaiveDate,
}

/// What the approvals of a forfeiture must be, for a total of `total` fils of
/// deductions: whether a second approver is needed (`PRP.ACL.05`, strictly
/// above AED 5,000).
pub fn second_approval_needed(total: i64) -> bool {
    total > SECOND_APPROVAL_ABOVE
}

/// The damage lines `(description, amount)` that the deposit did not cover,
/// when `kept` fils of it went to the damages: the lines are covered in the
/// order they were published, and what is left of each is invoiced.
pub fn uncovered(kept: i64, lines: &[(String, i64)]) -> Vec<(String, i64)> {
    let mut left = kept.max(0);
    let mut rest = Vec::new();
    for (description, amount) in lines {
        let covered = left.min(*amount);
        left -= covered;
        if amount - covered > 0 {
            rest.push((description.clone(), amount - covered));
        }
    }
    rest
}

/// What the user gives to forfeit the deposit of a lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyRequest {
    /// The property manager who approves.
    pub approver: i64,
    /// Finance or the owner, above the threshold.
    pub second_approver: Option<i64>,
}

impl ApplyRequest {
    /// A request approved by the property manager `approver`.
    pub fn new(approver: i64) -> Self {
        ApplyRequest {
            approver,
            second_approver: None,
        }
    }

    /// With the second approval of finance or the owner.
    pub fn second(mut self, user: i64) -> Self {
        self.second_approver = Some(user);
        self
    }
}

/// The unpaid invoices of the lease `root_id`, oldest due date first: the
/// invoices of its charges that are open, with a balance, and not disputed.
///
/// # Errors
///
/// The database's and the invoicer's errors.
pub async fn unpaid_invoices(state: &AppState, root_id: i64) -> Result<Vec<Unpaid>> {
    let mut found: Vec<Unpaid> = Vec::new();
    for version in Lease::versions_of(&state.db, root_id).await? {
        for charge in version.charges(&state.db).await? {
            let Some(id) = charge.invoice_id else {
                continue;
            };
            if charge.status != ChargeStatus::Invoiced || found.iter().any(|u| u.id == id) {
                continue;
            }
            let Some(summary) = invoicer(state).find(&state.db, InvoiceId(id)).await? else {
                continue;
            };
            let open = matches!(
                summary.status,
                InvoiceStatus::Issued | InvoiceStatus::PartiallyPaid | InvoiceStatus::Overdue
            );
            if open && summary.balance > 0 && !summary.disputed {
                found.push(Unpaid {
                    id,
                    number: summary.number.clone(),
                    open: summary.balance,
                    due: summary.due_date.unwrap_or(charge.due_date),
                });
            }
        }
    }
    found.sort_by_key(|u| (u.due, u.id));
    Ok(found)
}

/// Why the approvers of `request` are not acceptable for `total` fils of
/// deductions forfeited by `user`, if they are not.
async fn approvers_problem(
    state: &AppState,
    user: &AuthUser,
    lease: &Lease,
    request: &ApplyRequest,
    total: i64,
) -> Result<Option<String>> {
    let managers = users_with_role_in(&state.db, PROPERTY_MANAGER_ROLE, &lease.scope()).await?;
    if request.approver == user.id || !managers.iter().any(|m| m.id == request.approver) {
        return Ok(Some(english("property.deposit_forfeit.approver_wrong", &[])));
    }
    if !second_approval_needed(total) {
        return Ok(None);
    }
    let Some(second) = request.second_approver else {
        let limit = crate::money_text(SECOND_APPROVAL_ABOVE);
        return Ok(Some(english(
            "property.deposit_forfeit.second_required",
            &[("limit", &limit)],
        )));
    };
    if second == user.id || second == request.approver {
        return Ok(Some(english("property.deposit_forfeit.second", &[])));
    }
    for role in SECOND_APPROVER_ROLES {
        if users_with_role(&state.db, role)
            .await?
            .iter()
            .any(|u| u.id == second)
        {
            return Ok(None);
        }
    }
    Ok(Some(english("property.deposit_forfeit.second", &[])))
}

/// `user` forfeits the deposit of lease `lease_id`, applying it to the
/// deductions, approved by `request.approver` (and `request.second_approver`
/// above the threshold), and invoices the damages the deposit did not cover.
///
/// # Errors
///
/// 403 without `deposits.refund` in the lease's building; 422 when the
/// deposit was applied already, nothing was published to the tenant, a line is
/// disputed or the window has not passed, the approvers are not acceptable, the
/// damages charge type is not configured for damage lines, or wl-payments
/// refuses (the total deductions are below the deposit, no deposit is being
/// settled). Nothing is changed then.
pub async fn apply(
    state: &AppState,
    user: &AuthUser,
    lease_id: i64,
    request: ApplyRequest,
) -> Result<DepositSettlement> {
    let lease = Lease::find(&state.db, lease_id)
        .await?
        .ok_or(Error::NotFound)?;
    if !user.has_permission_in(DEPOSITS_REFUND, &lease.scope()) {
        return Err(Error::Forbidden);
    }
    if DepositSettlement::of_root(&state.db, lease.root_id)
        .await?
        .is_some()
    {
        return Err(refused(english("property.deposit_forfeit.done_already", &[])));
    }
    let published = deposit_deductions(state)
        .published_for(state, lease.tenant())
        .await?
        .into_iter()
        .find(|shown| shown.source == lease.agreement())
        .ok_or_else(|| refused(english("property.deposit_forfeit.unpublished", &[])))?;
    if !published.disputed_lines.is_empty() {
        return Err(refused(english("property.deposit_forfeit.disputed", &[])));
    }
    if let Some(end) = published.window_ends_on
        && today() <= end
    {
        let day = end.to_string();
        return Err(refused(english(
            "property.deposit_forfeit.window",
            &[("day", &day)],
        )));
    }
    let lines: Vec<(String, i64)> = published
        .lines
        .iter()
        .map(|line| (line.description.clone(), line.amount))
        .collect();
    let damages: i64 = lines.iter().map(|(_, amount)| *amount).sum();
    let unpaid = unpaid_invoices(state, lease.root_id).await?;
    let rent_due: i64 = unpaid.iter().map(|u| u.open).sum();
    let total = rent_due + damages;
    if let Some(problem) = approvers_problem(state, user, &lease, &request, total).await? {
        return Err(refused(problem));
    }
    let charge_type = ChargeType::active_by_code(&state.db, DAMAGES).await?;
    if damages > 0 && charge_type.is_none() {
        return Err(refused(english("property.deposit_forfeit.no_charge_type", &[])));
    }

    let applied = deposits(state)
        .apply_to_deductions(
            state,
            user,
            ApplyDeposit {
                source: lease.agreement(),
                approver: request.approver,
                rent: unpaid
                    .iter()
                    .map(|u| ApplyToInvoice {
                        invoice: InvoiceId(u.id),
                        amount: u.open,
                    })
                    .collect(),
            },
        )
        .await?;

    let rest = uncovered(applied.kept, &lines);
    let excess: i64 = rest.iter().map(|(_, amount)| *amount).sum();
    let mut invoice_id = None;
    if excess > 0 {
        let vat = charge_type.map_or(VatCategory::Exempt, |c| c.vat_category);
        let mut draft = InvoiceDraft::new(lease.tenant(), lease.subject())
            .scope(wl_kernel::Subject::new(
                BUILDING_SUBJECT_TYPE,
                lease.building_id,
            ))
            .date_of_supply(today());
        for (description, amount) in &rest {
            draft = draft.line(DraftLine::new(description, *amount, vat).charge_type(DAMAGES));
        }
        invoice_id = invoicer(state).draft(&state.db, draft).await?.map(|id| id.0);
    }
    let kept_for_damages = applied.kept.min(damages);
    DepositSettlement::create(
        &state.db,
        DepositSettlement {
            root_id: lease.root_id,
            lease_id: lease.id,
            deposit_id: applied.deposit.0,
            held: applied.held,
            rent_due,
            damages,
            to_rent: applied.to_invoices,
            to_damages: kept_for_damages,
            excess,
            excess_invoice_id: invoice_id,
            approver_id: request.approver,
            second_approver_id: request.second_approver,
            applied_by: Some(user.id),
            applied_on: today(),
            ..Default::default()
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(amounts: &[i64]) -> Vec<(String, i64)> {
        amounts
            .iter()
            .enumerate()
            .map(|(index, amount)| (format!("Damage {}", index + 1), *amount))
            .collect()
    }

    #[test]
    fn the_second_approver_is_needed_only_above_five_thousand_dirhams() {
        assert!(!second_approval_needed(500_000));
        assert!(second_approval_needed(500_001));
        assert!(second_approval_needed(650_000));
        assert!(!second_approval_needed(0));
    }

    #[test]
    fn the_damages_the_deposit_did_not_cover_are_what_is_left_of_each_line() {
        // AED 3,000.00 of the deposit covers the damages of AED 4,500.00.
        let rest = uncovered(300_000, &lines(&[450_000]));
        assert_eq!(rest, vec![("Damage 1".to_owned(), 150_000)]);
        // It covers the first line, then part of the second.
        let rest = uncovered(300_000, &lines(&[100_000, 400_000, 50_000]));
        assert_eq!(
            rest,
            vec![
                ("Damage 2".to_owned(), 200_000),
                ("Damage 3".to_owned(), 50_000)
            ]
        );
        // Everything is covered: nothing is invoiced.
        assert!(uncovered(450_000, &lines(&[450_000])).is_empty());
        assert!(uncovered(500_000, &lines(&[450_000])).is_empty());
        assert!(uncovered(0, &[]).is_empty());
    }

    #[test]
    fn a_settlement_adds_the_rent_and_the_damages() {
        let statement = DepositSettlement {
            rent_due: 200_000,
            damages: 450_000,
            ..Default::default()
        };
        assert_eq!(statement.total_deductions(), 650_000);
    }
}
