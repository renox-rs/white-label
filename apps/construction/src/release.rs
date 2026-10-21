//! The release of the retention (`make_due`, `invoice`, `settle`,
//! `CON.ACL.11`, 02 Construction §4.6 steps 3 to 6, §7.1 *Retention*, §9
//! "VAT"; BL-CON-07).
//!
//! Retention withheld on certified IPAs is not invoiced with them. Each
//! release part is `held` until its release condition is met, then `due`,
//! then invoiced on its own tax invoice at 5% VAT, and `settled` when that
//! invoice is paid, credited or written off.
//!
//! - **Due.** The system makes the first part due at the practical completion
//!   and the final part at the close of the project or the end of its defects
//!   liability period ([`daily`]), both at a termination
//!   ([`crate::termination`]). A user with `retentions.manage` makes a part
//!   due when a retention bond that is `active` replaces it
//!   ([`make_due_with_bond`]); the bond is linked to the part first
//!   ([`link_bond`]) and a bond below the held amount is a warning, not a
//!   refusal. The accountant is told once, after the commit
//!   ([`announce_due`]).
//! - **Invoice.** The accountant (`invoices.issue`) invoices a `due` part
//!   ([`invoice`]): one line for the remaining amount at 5% VAT, with the
//!   part as its source. A part is invoiced only when its release condition
//!   is recorded (`CON.ACL.11`): `due` says so, and a terminated project's
//!   parts wait for the termination effective date ([`invoice_problem`]).
//!   The draft id is kept on the part the moment it exists, so a run that
//!   stopped half-way resumes without making a second invoice.
//! - **Settle.** The retention invoice paid, credited or written off settles
//!   the part ([`settle_for_invoice`], run from the kernel's invoice events).
//!   `settled` is terminal: a reopened invoice follows its own lifecycle and
//!   the retention report reads its state.
//! - **Cap.** The accountant is told once when the retention withheld on the
//!   project reaches its cap ([`after_withheld`]).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{
    DraftLine, InvoiceCredited, InvoiceDraft, InvoiceId, InvoicePaid, InvoiceStatus,
    InvoiceWrittenOff, VatCategory, invoicer,
};

use crate::guarantee::{Guarantee, GuaranteeKind, GuaranteeStatus};
use crate::notices::{aed, tell_role};
use crate::permissions;
use crate::project::{Project, ProjectStatus};
use crate::retention::{
    INVOICE, MAKE_DUE_BOND, ReleaseInput, Retention, RetentionPart, RetentionStatus, SETTLE,
    make_part_due_in,
};
use crate::texts::english;

/// A 422 with `message`.
fn guard(message: String) -> TransitionError {
    TransitionError::Guard(message)
}

/// A part linked to a bond, or made due by it, and the warning that came
/// with it.
#[derive(Debug)]
pub struct Linked {
    /// The part as it is stored now.
    pub part: Retention,
    /// Set when the bond is below the amount held: a warning, not a refusal.
    pub warning: Option<String>,
}

/// The warning that `bond` is below what `part` holds, if it is.
pub fn bond_warning(part: &Retention, bond: &Guarantee) -> Option<String> {
    (bond.amount < part.amount).then(|| {
        english(
            "construction.retention.bond_below",
            &[
                ("bond", &aed(bond.amount)),
                ("held", &aed(part.amount)),
                ("part", &part.part.as_str()),
            ],
        )
    })
}

/// Why `part` cannot be made due by `bond`, if it cannot: a retention bond
/// is linked to it and it is `active`.
pub fn bond_problem(part: &Retention, bond: Option<&Guarantee>) -> Option<String> {
    let part_name = part.part.as_str();
    match bond {
        None => Some(english(
            "construction.retention.no_bond",
            &[("part", &part_name)],
        )),
        Some(bond) if bond.status != GuaranteeStatus::Active => Some(english(
            "construction.retention.bond_not_active",
            &[("part", &part_name)],
        )),
        Some(_) => None,
    }
}

/// Why `part` cannot be invoiced on `today`, if it cannot: the release
/// condition is recorded when the part is `due`, and the parts of a
/// terminated project wait for the termination effective date
/// (`CON.ACL.11`). A part with nothing left is not invoiced either.
pub fn invoice_problem(project: &Project, part: &Retention, today: NaiveDate) -> Option<String> {
    if part.amount <= 0 {
        return Some(english("construction.retention.nothing_held", &[]));
    }
    match project.termination_effective_on {
        Some(effective) if project.status == ProjectStatus::Terminated && effective > today => {
            Some(english(
                "construction.retention.before_termination",
                &[("date", &effective)],
            ))
        }
        _ => None,
    }
}

/// `user` links the retention bond `guarantee_id` to the part `id`, so that
/// it can replace it. A bond below the held amount links with a warning.
///
/// # Errors
///
/// `Forbidden` without `retentions.manage` in the project, `Guard` (422)
/// when the part is finished, the guarantee is not a retention bond of the
/// project or is released or expired.
pub async fn link_bond(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    guarantee_id: i64,
) -> std::result::Result<Linked, TransitionError> {
    let part = Retention::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    if !user.has_permission_in(permissions::RETENTIONS_MANAGE, &part.scope()) {
        return Err(TransitionError::Forbidden);
    }
    let bond = Guarantee::find(&state.db, guarantee_id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    if !matches!(part.status, RetentionStatus::Held | RetentionStatus::Due) {
        return Err(guard(english("construction.retention.finished", &[])));
    }
    if bond.kind != GuaranteeKind::RetentionBond || bond.project_id != part.project_id {
        return Err(guard(english("construction.retention.not_a_bond", &[])));
    }
    if matches!(
        bond.status,
        GuaranteeStatus::Released | GuaranteeStatus::Expired
    ) {
        return Err(guard(english(
            "construction.retention.bond_not_active",
            &[("part", &part.part.as_str())],
        )));
    }
    let linked = Some(bond.id);
    Retention::query()
        .where_(Retention::ID.eq(part.id))
        .update(&state.db, &[("bond_id", &linked)])
        .await?;
    let part = Retention::find_or_404(&state.db, id).await?;
    Ok(Linked {
        warning: bond_warning(&part, &bond),
        part,
    })
}

/// `user` makes the `held` part `id` due because the retention bond linked
/// to it is `active` (`make_due_bond`).
///
/// # Errors
///
/// `Forbidden` without `retentions.manage` in the project, `NotFrom` unless
/// the part is `held`, `Guard` (422) when no bond is linked to it or the
/// bond is not `active`.
pub async fn make_due_with_bond(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Linked, TransitionError> {
    let part = Retention::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    let bond = match part.bond_id {
        Some(bond) => Guarantee::find(&state.db, bond).await?,
        None => None,
    };
    let input = ReleaseInput {
        checked: true,
        problem: bond_problem(&part, bond.as_ref()),
    };
    let _outcome = part
        .transition(MAKE_DUE_BOND)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    announce_due(state, part.project_id).await?;
    let part = Retention::find_or_404(&state.db, id).await?;
    let warning = bond.as_ref().and_then(|bond| bond_warning(&part, bond));
    Ok(Linked { part, warning })
}

/// Tells the accountant, once per part, that a part of `project_id` is due.
/// Run after the commit of whatever made it due. Returns how many parts it
/// told about.
///
/// # Errors
///
/// A database error and the notifier's errors.
pub async fn announce_due(state: &AppState, project_id: i64) -> Result<usize> {
    let project = Project::find_or_404(&state.db, project_id).await?;
    let parts = Retention::query()
        .where_(Retention::PROJECT_ID.eq(project_id))
        .where_(Retention::STATUS.eq(RetentionStatus::Due))
        .where_(Retention::ANNOUNCED_AT.is_null())
        .order_by(Retention::ID)
        .get(&state.db)
        .await?;
    let mut told = 0;
    for part in parts {
        let now = Some(renox::db::now());
        let claimed = Retention::query()
            .where_(Retention::ID.eq(part.id))
            .where_(Retention::ANNOUNCED_AT.is_null())
            .update(&state.db, &[("announced_at", &now)])
            .await?;
        if claimed == 0 {
            continue;
        }
        let title = english(
            "construction.retention.due_title",
            &[("part", &part.part.as_str()), ("project", &project.name)],
        );
        let body = english(
            "construction.retention.due_body",
            &[
                ("part", &part.part.as_str()),
                ("project", &project.name),
                ("amount", &aed(part.amount)),
            ],
        );
        tell_role(
            state,
            &project,
            part.subject(),
            permissions::ACCOUNTANT_ROLE,
            &title,
            &body,
        )
        .await?;
        told += 1;
    }
    Ok(told)
}

/// Keeps the retention invoice `value` on the part `id`.
async fn keep_invoice(db: &Db, id: i64, value: i64) -> Result {
    let value = Some(value);
    Retention::query()
        .where_(Retention::ID.eq(id))
        .update(db, &[("invoice_id", &value)])
        .await?;
    Ok(())
}

/// `user` (the accountant) invoices the `due` part `id`: one tax invoice at
/// 5% VAT for the remaining amount, issued, and the part is `invoiced`.
///
/// # Errors
///
/// `Forbidden` without `invoices.issue` in the project, `NotFrom` unless the
/// part is `due`, `Guard` (422) when the project is terminated and its
/// effective date is not reached, nothing is held, or invoicing is off. What
/// was made stays on the part, and a new run resumes from it.
pub async fn invoice(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Retention, TransitionError> {
    let part = Retention::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    if !user.has_permission_in(permissions::INVOICES_ISSUE, &part.scope()) {
        return Err(TransitionError::Forbidden);
    }
    if part.status != RetentionStatus::Due {
        return Err(TransitionError::NotFrom(part.status.as_str().to_owned()));
    }
    let project = Project::find_or_404(&state.db, part.project_id).await?;
    if let Some(problem) = invoice_problem(&project, &part, today()) {
        return Err(guard(problem));
    }
    let invoices = invoicer(state);

    // The tax invoice for the remaining amount, issued.
    let invoice_id = match part.invoice_id {
        Some(made) => InvoiceId(made),
        None => {
            let description = english(
                "construction.retention.invoice_line",
                &[("part", &part.part.as_str()), ("project", &project.name)],
            );
            let line = DraftLine::new(&description, part.amount, VatCategory::Standard)
                .origin(part.subject());
            let mut draft =
                InvoiceDraft::new(project.client(), part.subject()).scope(project.subject());
            let supplied = match part.part {
                RetentionPart::First => project.completed_on,
                RetentionPart::Final => project.defects_liability_ends_on,
            };
            if let Some(on) = supplied {
                draft = draft.date_of_supply(on);
            }
            let made = invoices
                .draft(&state.db, draft.line(line))
                .await?
                .ok_or_else(|| guard(english("construction.invoice.invoicing_off", &[])))?;
            keep_invoice(&state.db, part.id, made.0).await?;
            made
        }
    };
    let summary = invoices
        .find(&state.db, invoice_id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    if summary.status == InvoiceStatus::Draft {
        invoices.issue(state, invoice_id, Some(user)).await?;
    }

    // The part is invoiced; `CON.RUL.07` stops with it.
    let fresh = Retention::find_or_404(&state.db, id).await?;
    let input = ReleaseInput {
        checked: true,
        problem: None,
    };
    let _outcome = fresh
        .transition(INVOICE)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    Ok(Retention::find_or_404(&state.db, id).await?)
}

/// Settles the part whose retention invoice is `invoice` when that invoice
/// is `paid`, `credited` or `written_off`. Returns the part it settled.
///
/// # Errors
///
/// A database error and the invoicing contract's errors.
pub async fn settle_for_invoice(state: &AppState, invoice: InvoiceId) -> Result<Option<Retention>> {
    let Some(part) = Retention::query()
        .where_(Retention::INVOICE_ID.eq(invoice.0))
        .first(&state.db)
        .await?
    else {
        return Ok(None);
    };
    if part.status != RetentionStatus::Invoiced {
        return Ok(None);
    }
    let Some(found) = invoicer(state).find(&state.db, invoice).await? else {
        return Ok(None);
    };
    if !matches!(
        found.status,
        InvoiceStatus::Paid | InvoiceStatus::Credited | InvoiceStatus::WrittenOff
    ) {
        return Ok(None);
    }
    let _outcome = part
        .transition(SETTLE)
        .by_system()
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(Some(Retention::find_or_404(&state.db, part.id).await?))
}

/// Tells the accountant, once, that the retention withheld on `project_id`
/// reached its cap. Run after the commit of the IPA invoice that withheld it.
/// Returns whether it told.
///
/// # Errors
///
/// A database error and the notifier's errors.
pub async fn after_withheld(state: &AppState, project_id: i64) -> Result<bool> {
    let project = Project::find_or_404(&state.db, project_id).await?;
    let Some(cap) = project.retention_cap else {
        return Ok(false);
    };
    let held: i64 = Retention::query()
        .where_(Retention::PROJECT_ID.eq(project_id))
        .get(&state.db)
        .await?
        .iter()
        .map(|part| part.amount)
        .sum();
    if held < cap {
        return Ok(false);
    }
    let now = Some(renox::db::now());
    let claimed = Project::query()
        .where_(Project::ID.eq(project_id))
        .where_(Project::RETENTION_CAP_NOTIFIED_AT.is_null())
        .update(&state.db, &[("retention_cap_notified_at", &now)])
        .await?;
    if claimed == 0 {
        return Ok(false);
    }
    let title = english(
        "construction.retention.cap_title",
        &[("project", &project.name)],
    );
    let body = english(
        "construction.retention.cap_body",
        &[("project", &project.name), ("cap", &aed(cap))],
    );
    tell_role(
        state,
        &project,
        project.subject(),
        permissions::ACCOUNTANT_ROLE,
        &title,
        &body,
    )
    .await?;
    Ok(true)
}

/// Makes the final part of every project whose defects liability period has
/// ended before `day` due, as the system, and tells the accountant. Returns
/// the projects it made a part due for.
///
/// # Errors
///
/// The database's errors and the notifier's.
pub async fn make_final_due_at_defects_end(state: &AppState, day: NaiveDate) -> Result<Vec<i64>> {
    let projects = Project::query()
        .where_(Project::STATUS.eq(ProjectStatus::PracticalCompletion))
        .order_by(Project::ID)
        .get(&state.db)
        .await?;
    let mut made = Vec::new();
    for project in projects {
        if !project
            .defects_liability_ends_on
            .is_some_and(|end| day > end)
        {
            continue;
        }
        let mut tx = state.db.begin().await?;
        let done = make_part_due_in(&mut tx, project.id, RetentionPart::Final).await?;
        tx.commit().await?;
        if done {
            announce_due(state, project.id).await?;
            made.push(project.id);
        }
    }
    Ok(made)
}

/// The daily run at 02:00 Asia/Dubai (§7.2 intro): the final parts whose
/// defects liability period ended, then the VAT notices of `CON.RUL.07`.
///
/// # Errors
///
/// The database's errors and the notifier's.
pub async fn daily(state: &AppState) -> Result {
    let day = today();
    make_final_due_at_defects_end(state, day).await?;
    crate::vat_notice::run_on(state, day).await?;
    Ok(())
}

/// Settles the retention part of an invoice when the invoice reaches an end.
pub(crate) fn listen(app: &mut Registry) {
    app.listen(|event: InvoicePaid, state: AppState| async move {
        let _settled = settle_for_invoice(&state, event.invoice).await?;
        Ok(())
    });
    app.listen(|event: InvoiceWrittenOff, state: AppState| async move {
        let _settled = settle_for_invoice(&state, event.invoice).await?;
        Ok(())
    });
    app.listen(|event: InvoiceCredited, state: AppState| async move {
        let _settled = settle_for_invoice(&state, event.invoice).await?;
        Ok(())
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn part(amount: i64) -> Retention {
        Retention::new(1, RetentionPart::Final, amount)
    }

    fn bond(amount: i64, status: GuaranteeStatus) -> Guarantee {
        Guarantee {
            status,
            ..Guarantee::new(1, GuaranteeKind::RetentionBond, amount)
        }
    }

    #[test]
    fn a_bond_below_the_held_amount_is_a_warning() {
        let held = part(5_000_000);
        assert_eq!(
            bond_warning(&held, &bond(5_000_000, GuaranteeStatus::Active)),
            None
        );
        let warning = bond_warning(&held, &bond(4_000_000, GuaranteeStatus::Active));
        assert!(warning.is_some_and(|text| text.contains("AED 40,000.00")));
    }

    #[test]
    fn only_an_active_linked_bond_replaces_a_part() {
        let held = part(5_000_000);
        assert!(bond_problem(&held, None).is_some_and(|text| text.contains("final")));
        assert!(
            bond_problem(&held, Some(&bond(1, GuaranteeStatus::Draft)))
                .is_some_and(|text| text.contains("not active"))
        );
        assert_eq!(
            bond_problem(&held, Some(&bond(1, GuaranteeStatus::Active))),
            None
        );
    }

    #[test]
    fn a_terminated_project_waits_for_its_effective_date() {
        let mut project = Project::new("TWR-01", "Tower", wl_kernel::PartyId(1));
        let due = part(5_000_000);
        assert_eq!(invoice_problem(&project, &due, day(2027, 6, 1)), None);
        project.status = ProjectStatus::Terminated;
        project.termination_effective_on = Some(day(2027, 5, 31));
        assert_eq!(invoice_problem(&project, &due, day(2027, 5, 31)), None);
        assert!(
            invoice_problem(&project, &due, day(2027, 5, 30))
                .is_some_and(|text| text.contains("2027-05-31"))
        );
        assert!(invoice_problem(&project, &part(0), day(2027, 6, 1)).is_some());
    }
}
