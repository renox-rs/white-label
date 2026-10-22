//! A card payment charged back (10 §4.3 row *A card payment is charged back*,
//! §7.1 *Enrolment*, BL-EDU-06, `FIN.RUL.20`).
//!
//! When wl-invoicing reopens a paid invoice of an enrolment because its payment
//! was reversed (`InvoiceReopened`, reason `payment_reversed`), **the enrolment
//! does not change**: it is not frozen, withdrawn or blocked. The counsellor
//! gets one task to collect the money again or withdraw the student:
//!
//! - The invoices of an enrolment's course fee, instalments, package and
//!   registration fee have the enrolment as their source; a membership's cycle
//!   invoices have the subscription (its `past_due` is wl-invoicing's) and make no
//!   task here.
//! - The task goes to the enrolment's counsellor; when that user is no longer
//!   active (an employee who left, or a user without `enrolments.manage`) it goes
//!   to the counsellors of the enrolment's campus (the task to the first, the
//!   others told), or to the holder the kernel picks when there are none. It is
//!   due in two working days and names the amount, the payer and the two links:
//!   *send payment link* ([`collect_path`]) and *withdraw* (the enrolment page).
//! - Each reopened invoice is a row ([`Chargeback`]); reopening another invoice of
//!   the same enrolment, or the same one again, **updates the one open task** with
//!   a comment.
//! - When the invoice is paid again (the chargeback was won, or the payer paid)
//!   the row is settled; when none is left the task closes. A withdrawal closes it
//!   too: the credit note and the refund take what was actually paid.
//!
//! *Send payment link* ([`send_link`]) makes a payment link of wl-payments for
//! what is outstanding, replaces the enrolment's earlier link and sends the path
//! to the payer. While money is outstanding the enrolment is an active prepaid
//! subject for wl-payments ([`crate::lapse::enrolment_active`]), so a payment made
//! on the link is not refunded as a late one.

use renox::auth::permissions::{Scope, users_with_permission_in, users_with_role_in};
use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use wl_kernel::calendar::calendar;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceId, InvoicePaid, InvoiceReopened, ReopenReason, invoicer};
use wl_kernel::payments::{DEFAULT_CURRENCY, PaymentLinkRequest, payment_links};
use wl_kernel::role_tasks;
use wl_kernel::tickets::{Assignee, OpenTicket, TASK_CATEGORY, TaskComment, task_creator};
use wl_kernel::workforce::workforce;
use wl_kernel::{LegalEntity, Subject};

use crate::chargeback::Chargeback;
use crate::db::refused;
use crate::enrolment::{ENROLMENT_SUBJECT_TYPE, Enrolment, EnrolmentStatus};
use crate::enrolments::party_name;
use crate::money_text;
use crate::notices;
use crate::permissions;
use crate::texts::english;

/// The days a payment link for the outstanding money stays open.
pub const LINK_DAYS: i64 = 7;

/// The working days the task is due in.
pub const DUE_WORKING_DAYS: u32 = 2;

/// The key the one open task of `enrolment` is found by.
pub fn task_key(enrolment: i64) -> String {
    format!("edtech.chargeback:{enrolment}")
}

/// The page that sends the payment link for the money outstanding on
/// `enrolment`.
pub fn collect_path(enrolment: i64) -> String {
    format!("/edtech/enrolments/{enrolment}/collect")
}

/// The page where the student is withdrawn.
pub fn withdraw_path(enrolment: i64) -> String {
    format!("/edtech/enrolments/{enrolment}")
}

/// The enrolment states in which a reopened invoice is still the centre's to
/// collect: a withdrawn, transferred or lapsed enrolment was settled by its
/// own end.
fn collectable(status: EnrolmentStatus) -> bool {
    matches!(
        status,
        EnrolmentStatus::PendingPayment
            | EnrolmentStatus::Active
            | EnrolmentStatus::Frozen
            | EnrolmentStatus::Completed
    )
}

/// What is outstanding on `enrolment` in fils: the balance of each invoice that
/// was reopened and not paid again, read from the books now.
///
/// # Errors
///
/// The database's and the invoicing contract's errors.
pub async fn outstanding(state: &AppState, enrolment: i64) -> Result<i64> {
    let books = invoicer(state);
    let mut total = 0;
    for row in Chargeback::outstanding_of(&state.db, enrolment).await? {
        total += match books.find(&state.db, InvoiceId(row.invoice_id)).await? {
            Some(summary) => summary.balance.max(0),
            None => row.amount,
        };
    }
    Ok(total)
}

/// Whether anything is outstanding on `enrolment` (a row not settled).
///
/// # Errors
///
/// The database's errors.
pub async fn is_outstanding(db: &Db, enrolment: i64) -> Result<bool> {
    Ok(!Chargeback::outstanding_of(db, enrolment).await?.is_empty())
}

/// The scope of the campus of `enrolment`: the campus subject's registered scope,
/// else a scope of its own type and id; `None` without a campus.
async fn campus_scope(db: &Db, enrolment: &Enrolment) -> Result<Option<Scope>> {
    let (Some(kind), Some(id)) = (&enrolment.campus_type, enrolment.campus_id) else {
        return Ok(None);
    };
    let registered = wl_kernel::scopes::scope_of(db, &Subject::new(kind, id)).await?;
    Ok(Some(if registered.is_global() {
        Scope::new(kind, id)
    } else {
        registered
    }))
}

/// Whether `user` can still be given counsellor work in `scope`: the employee
/// behind the account is still listed by wl-hr (not exited or absconded), and
/// the user still holds `enrolments.manage` there.
async fn counsellor_active(state: &AppState, user: i64, scope: &Scope) -> Result<bool> {
    let people = workforce(state);
    if let Some(employee) = people.employee_of_user(&state.db, user).await?
        && let Some(profile) = people.staff(state, employee).await?
        && !profile.listed
    {
        return Ok(false);
    }
    Ok(
        users_with_permission_in(&state.db, permissions::ENROLMENTS_MANAGE, scope)
            .await?
            .iter()
            .any(|holder| holder.id == user),
    )
}

/// The users who get the task of `enrolment`: its counsellor while active, else
/// the counsellors of its campus, else the holder the kernel picks.
///
/// # Errors
///
/// The database's and the workforce contract's errors.
pub async fn recipients(state: &AppState, enrolment: &Enrolment) -> Result<Vec<i64>> {
    let scope = campus_scope(&state.db, enrolment)
        .await?
        .unwrap_or_else(Scope::global);
    if let Some(user) = enrolment.counsellor_id
        && counsellor_active(state, user, &scope).await?
    {
        return Ok(vec![user]);
    }
    let mut found = Vec::new();
    for user in users_with_role_in(&state.db, permissions::COUNSELLOR_ROLE, &scope).await? {
        if counsellor_active(state, user.id, &scope).await? {
            found.push(user.id);
        }
    }
    if found.is_empty()
        && let Some(user) = role_tasks::assignee(&state.db, permissions::COUNSELLOR_ROLE).await?
    {
        found.push(user);
    }
    Ok(found)
}

/// The invoices outstanding on `enrolment` as text: `INV-5 (AED 2,520.00)`.
async fn invoices_text(state: &AppState, rows: &[Chargeback]) -> Result<String> {
    let books = invoicer(state);
    let mut parts = Vec::new();
    for row in rows {
        let summary = books.find(&state.db, InvoiceId(row.invoice_id)).await?;
        let (number, balance) = match summary {
            Some(found) => (
                found.number.unwrap_or_else(|| format!("#{}", row.invoice_id)),
                found.balance.max(0),
            ),
            None => (format!("#{}", row.invoice_id), row.amount),
        };
        parts.push(format!("{number} ({})", money_text(balance)));
    }
    Ok(parts.join(", "))
}

/// Opens the one task of `enrolment`, or adds what changed to the one that is
/// open. Without the ticketing module there is no task.
async fn raise_task(state: &AppState, enrolment: &Enrolment) -> Result<()> {
    let rows = Chargeback::outstanding_of(&state.db, enrolment.id).await?;
    if rows.is_empty() {
        return Ok(());
    }
    let amount = money_text(outstanding(state, enrolment.id).await?);
    let invoices = invoices_text(state, &rows).await?;
    let number = enrolment.number();
    let creator = task_creator(state);
    let key = task_key(enrolment.id);
    if let Some(open) = creator.find_open(state, &key).await? {
        let text = english(
            "edtech.chargeback.update_comment",
            &[
                ("enrolment", &number),
                ("amount", &amount),
                ("invoices", &invoices),
            ],
        );
        creator
            .comment(state, open.id, TaskComment::internal(&text))
            .await?;
        return Ok(());
    }
    let student = party_name(state, "en", enrolment.student_party_id).await;
    let payer = party_name(state, "en", enrolment.payer_party_id).await;
    let title = english("edtech.chargeback.task_title", &[("enrolment", &number)]);
    let description = english(
        "edtech.chargeback.task_body",
        &[
            ("enrolment", &number),
            ("student", &student),
            ("payer", &payer),
            ("amount", &amount),
            ("invoices", &invoices),
            ("collect", &collect_path(enrolment.id)),
            ("withdraw", &withdraw_path(enrolment.id)),
        ],
    );
    let users = recipients(state, enrolment).await?;
    let mut ticket = OpenTicket::new(enrolment.subject(), TASK_CATEGORY, &title)
        .description(&description)
        .source_key(&key)
        .due_at(calendar(state).add_working_days(now(), DUE_WORKING_DAYS));
    if let Some(first) = users.first() {
        ticket = ticket.assign_to(Assignee::user(*first));
    }
    creator.open(state, ticket).await?;
    // The other counsellors of the campus are told; the task is the first's.
    if users.len() > 1 {
        notices::tell_users(state, enrolment.subject(), &number, &users, &title, &description)
            .await?;
    }
    Ok(())
}

/// `InvoiceReopened`: a paid invoice of an enrolment came back open because its
/// payment was reversed. Records it and opens or updates the counsellor's task.
/// Answers whether it was a new reopening.
///
/// # Errors
///
/// The database's, the invoicing, ticketing and workforce contracts' errors.
pub async fn on_reopened(state: &AppState, event: &InvoiceReopened) -> Result<bool> {
    if event.reason != ReopenReason::PaymentReversed
        || !event.new_status.is_open()
        || event.source.subject_type != ENROLMENT_SUBJECT_TYPE
    {
        return Ok(false);
    }
    let Some(enrolment) = Enrolment::find(&state.db, event.source.subject_id).await? else {
        return Ok(false);
    };
    if !collectable(enrolment.status) {
        return Ok(false);
    }
    let known = Chargeback::of_invoice(&state.db, event.invoice.0).await?;
    let fresh = match known {
        Some(row) if row.settled_on.is_none() && row.amount == event.balance => false,
        Some(row) => {
            Chargeback::query()
                .where_(Chargeback::ID.eq(row.id))
                .update(
                    &state.db,
                    &[
                        ("amount", &event.balance),
                        ("settled_on", &Option::<NaiveDate>::None),
                        ("reversal_reason", &event.reversal_reason),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
            true
        }
        None => {
            let mut row = Chargeback {
                enrolment_id: enrolment.id,
                invoice_id: event.invoice.0,
                amount: event.balance,
                reversal_reason: event.reversal_reason.clone(),
                reopened_on: today(),
                ..Chargeback::default()
            };
            row.save(&state.db).await?;
            true
        }
    };
    if fresh {
        raise_task(state, &enrolment).await?;
    }
    Ok(fresh)
}

/// `InvoicePaid`: a reopened invoice of an enrolment is paid again. Settles it;
/// when nothing is left outstanding the task closes. Answers whether it was a
/// reopened invoice.
///
/// # Errors
///
/// The database's and the ticketing contract's errors.
pub async fn on_paid(state: &AppState, event: &InvoicePaid) -> Result<bool> {
    if event.source.subject_type != ENROLMENT_SUBJECT_TYPE {
        return Ok(false);
    }
    let Some(row) = Chargeback::of_invoice(&state.db, event.invoice.0).await? else {
        return Ok(false);
    };
    if row.settled_on.is_some() {
        return Ok(false);
    }
    Chargeback::query()
        .where_(Chargeback::ID.eq(row.id))
        .update(
            &state.db,
            &[("settled_on", &Some(today())), ("updated_at", &now())],
        )
        .await?;
    let Some(enrolment) = Enrolment::find(&state.db, row.enrolment_id).await? else {
        return Ok(true);
    };
    if is_outstanding(&state.db, enrolment.id).await? {
        raise_task(state, &enrolment).await?;
    } else {
        close_task(state, &enrolment, "edtech.chargeback.closed_note").await?;
    }
    Ok(true)
}

/// Closes the open task of `enrolment` with the note `key`; the rows left are
/// settled when the enrolment is over.
async fn close_task(state: &AppState, enrolment: &Enrolment, key: &str) -> Result {
    let creator = task_creator(state);
    if let Some(open) = creator.find_open(state, &task_key(enrolment.id)).await? {
        let note = english(key, &[("enrolment", &enrolment.number())]);
        creator.complete(state, open.id, &note).await?;
    }
    Ok(())
}

/// After a withdrawal: the task of the enrolment closes, and what was
/// reopened is settled by the withdrawal's own credit note and refund.
///
/// # Errors
///
/// The database's and the ticketing contract's errors.
pub async fn on_withdrawn(state: &AppState, enrolment: &Enrolment) -> Result {
    if Chargeback::outstanding_of(&state.db, enrolment.id).await?.is_empty() {
        return Ok(());
    }
    Chargeback::query()
        .where_(Chargeback::ENROLMENT_ID.eq(enrolment.id))
        .where_(Chargeback::SETTLED_ON.is_null())
        .update(
            &state.db,
            &[("settled_on", &Some(today())), ("updated_at", &now())],
        )
        .await?;
    close_task(state, enrolment, "edtech.chargeback.withdrawn_note").await
}

/// *Send payment link*: makes a payment link for what is outstanding on
/// enrolment `id`, replaces the enrolment's earlier link and sends the path to
/// the family. Answers the path.
///
/// # Errors
///
/// A 403 without `enrolments.manage`; a 422 when nothing is outstanding or online
/// payment is not available.
pub async fn send_link(state: &AppState, by: &AuthUser, id: i64) -> Result<String> {
    if !by.has_permission(permissions::ENROLMENTS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let enrolment = Enrolment::find_or_404(&state.db, id).await?;
    let amount = outstanding(state, id).await?;
    if amount <= 0 || !collectable(enrolment.status) {
        return Err(refused(english("edtech.chargeback.problem.nothing", &[])));
    }
    let scope = enrolment.campus_type.as_deref().zip(enrolment.campus_id);
    let entity = LegalEntity::for_scope(&state.db, scope).await?;
    let request = PaymentLinkRequest {
        party: enrolment.payer(),
        legal_entity_id: entity.id,
        amount,
        currency: DEFAULT_CURRENCY.to_owned(),
        prepaid: enrolment.subject(),
        expires_at: now() + renox::chrono::Duration::days(LINK_DAYS),
        created_by: Some(by.id),
    };
    let Some(link) = payment_links(state).create(state, request).await? else {
        return Err(refused(english("edtech.chargeback.problem.no_link", &[])));
    };
    crate::lapse::cancel_link(state, &enrolment).await;
    Enrolment::query()
        .where_(Enrolment::ID.eq(id))
        .update(
            &state.db,
            &[
                ("payment_link_type", &Some(link.link.subject_type.clone())),
                ("payment_link_id", &Some(link.link.subject_id)),
                ("payment_link_path", &Some(link.path.clone())),
                ("updated_at", &now()),
            ],
        )
        .await?;
    let number = enrolment.number();
    let title = english("edtech.chargeback.link_title", &[("enrolment", &number)]);
    let body = english(
        "edtech.chargeback.link_body",
        &[
            ("enrolment", &number),
            ("amount", &money_text(amount)),
            ("link", &link.path),
        ],
    );
    notices::tell_family(state, &enrolment, &title, &body).await?;
    Ok(link.path)
}

/// Registers the listeners.
pub(crate) fn listen(app: &mut Registry) {
    app.listen(|event: InvoiceReopened, state: AppState| async move {
        on_reopened(&state, &event).await.map(|_| ())
    });
    app.listen(|event: InvoicePaid, state: AppState| async move {
        on_paid(&state, &event).await.map(|_| ())
    });
}
