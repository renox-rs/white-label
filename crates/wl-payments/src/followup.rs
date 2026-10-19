//! Follow-up tasks (wl-payments A10): work the module leaves for a role.
//!
//! The first kind is `raise_charge`: after a reversal, a party whose credit
//! was already refunded owes money, and the accountant gets a task "Raise a
//! charge for AED 500.00" with the payment and the refunds linked. It is a
//! payments record, not a ticket (`TaskCreator` cannot close a ticket, and
//! ticketing may be off). No invoice or charge is ever created from it;
//! the accountant raises the charge and marks the task done. The task is
//! unique per kind and payment.

use renox::db::Json;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::payments::PAYMENT_SUBJECT_TYPE;

use crate::credit;
use crate::notices;
use crate::payment::Payment;
use crate::permissions;
use crate::refund::{Refund, RefundStatus};

/// The role that gets the `raise_charge` task.
pub const ACCOUNTANT_ROLE: &str = "accountant";

/// What a follow-up task asks for.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FollowupKind {
    /// Raise a charge for money the party owes after a reversal.
    #[default]
    RaiseCharge,
}

/// Whether a follow-up task is still to do.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FollowupStatus {
    /// To do.
    #[default]
    Open,
    /// Done by a user.
    Done,
}

/// A task for a role. Money is `i64` fils of [`FollowupTask::currency`].
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_payments_followup_tasks",
    unique(kind, payment_id),
    index(role, status)
)]
pub struct FollowupTask {
    /// The primary key.
    pub id: i64,
    /// What it asks for.
    pub kind: FollowupKind,
    /// The party it is about.
    pub party_id: i64,
    /// The payment it follows.
    pub payment_id: Option<i64>,
    /// The amount, in fils.
    pub amount: i64,
    /// The ISO currency code, e.g. `AED`.
    pub currency: String,
    /// The refunds it links.
    pub refund_ids: Json<Vec<i64>>,
    /// The text shown to the role.
    pub text: String,
    /// The role that does it, e.g. `accountant`.
    pub role: String,
    /// To do or done.
    pub status: FollowupStatus,
    /// The user who marked it done.
    pub done_by: Option<i64>,
    /// When it was marked done.
    pub done_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// `fils` of `currency` as text, e.g. `AED 3,000.00`.
pub(crate) fn money_text(currency: &str, fils: i64) -> String {
    let sign = if fils < 0 { "-" } else { "" };
    let fils = fils.unsigned_abs();
    let whole = (fils / 100).to_string();
    let mut grouped = String::new();
    for (i, digit) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("{currency} {sign}{grouped}.{:02}", fils % 100)
}

impl FollowupTask {
    /// An open `raise_charge` task for the `deficit` fils the party of
    /// `payment` owes, linking `refund_ids`, for the accountant.
    pub fn raise_charge(payment: &Payment, deficit: i64, refund_ids: Vec<i64>) -> Self {
        FollowupTask {
            kind: FollowupKind::RaiseCharge,
            party_id: payment.party_id,
            payment_id: Some(payment.id),
            amount: deficit,
            currency: payment.currency.clone(),
            refund_ids: Json(refund_ids),
            text: format!(
                "Raise a charge for {}",
                money_text(&payment.currency, deficit)
            ),
            role: ACCOUNTANT_ROLE.to_owned(),
            status: FollowupStatus::Open,
            ..Default::default()
        }
    }

    /// The party it is about.
    pub fn party(&self) -> PartyId {
        PartyId(self.party_id)
    }

    /// Whether it is still to do.
    pub fn is_open(&self) -> bool {
        self.status == FollowupStatus::Open
    }

    /// The open tasks of `role`, oldest first.
    pub async fn open_for(db: &Db, role: &str) -> Result<Vec<FollowupTask>> {
        FollowupTask::query()
            .where_(FollowupTask::ROLE.eq(role))
            .where_(FollowupTask::STATUS.eq(FollowupStatus::Open))
            .order_by(FollowupTask::ID)
            .get(db)
            .await
    }

    /// The tasks that follow `payment`, oldest first.
    pub async fn of_payment(db: &Db, payment_id: i64) -> Result<Vec<FollowupTask>> {
        FollowupTask::query()
            .where_(FollowupTask::PAYMENT_ID.eq(payment_id))
            .order_by(FollowupTask::ID)
            .get(db)
            .await
    }

    /// Saves `task` unless its payment already has a task of its kind
    /// (unique per payment); returns the new task, or `None` for the repeat.
    pub async fn create_once(db: &Db, mut task: FollowupTask) -> Result<Option<FollowupTask>> {
        if let Some(payment_id) = task.payment_id {
            let existing = FollowupTask::query()
                .where_(FollowupTask::KIND.eq(task.kind))
                .where_(FollowupTask::PAYMENT_ID.eq(payment_id))
                .count(db)
                .await?;
            if existing > 0 {
                return Ok(None);
            }
        }
        task.insert(db).await?;
        Ok(Some(task))
    }
}

/// The refunds a deficit of `deficit` fils links: the `own` refunds (paid
/// out of the reversed payment), then the `others` (the party's other paid
/// credit refunds), newest first, taking others only until the deficit is
/// covered. Each is `(id, amount)`.
pub(crate) fn linked_refunds(deficit: i64, own: &[(i64, i64)], others: &[(i64, i64)]) -> Vec<i64> {
    let mut ids: Vec<i64> = own.iter().map(|(id, _)| *id).collect();
    let mut covered: i64 = own.iter().map(|(_, amount)| amount).sum();
    for (id, amount) in others {
        if covered >= deficit {
            break;
        }
        ids.push(*id);
        covered += amount;
    }
    ids
}

/// After a reversal of `payment`: when the party's credit balance in the
/// payment's currency is below 0, the credit was already refunded, and one
/// `raise_charge` task for the deficit is created for the accountant role
/// (unique per payment), who is then told. No invoice, charge or draft is
/// ever created.
pub(crate) async fn check_deficit(state: &AppState, payment: &Payment) -> Result {
    let balance = credit::credit_balance(&state.db, payment.party(), &payment.currency).await?;
    if balance >= 0 {
        return Ok(());
    }
    let refunds = Refund::query()
        .where_(Refund::PARTY_ID.eq(payment.party_id))
        .where_(Refund::CURRENCY.eq(payment.currency.as_str()))
        .where_(Refund::STATUS.eq(RefundStatus::Paid))
        .order_by_desc(Refund::ID)
        .get(&state.db)
        .await?;
    let source = payment.subject();
    let own: Vec<(i64, i64)> = refunds
        .iter()
        .filter(|r| r.source_subject() == source)
        .map(|r| (r.id, r.amount))
        .collect();
    let others: Vec<(i64, i64)> = refunds
        .iter()
        .filter(|r| r.source_type != PAYMENT_SUBJECT_TYPE)
        .map(|r| (r.id, r.amount))
        .collect();
    let ids = linked_refunds(-balance, &own, &others);
    let task = FollowupTask::raise_charge(payment, -balance, ids);
    if let Some(task) = FollowupTask::create_once(&state.db, task).await? {
        notices::followup_created(state, payment, &task).await?;
    }
    Ok(())
}

/// Why `user` cannot mark `task` done, if so: a user needs
/// `payments.record` (the accountant has it) and the task must be open.
pub(crate) fn done_problem(task: &FollowupTask, may: bool) -> Option<Error> {
    if !may {
        return Some(Error::Forbidden);
    }
    if !task.is_open() {
        return Some(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            "The task is already done.".to_owned(),
        ));
    }
    None
}

/// `user` marks task `id` done.
///
/// # Errors
///
/// 404 for an unknown task; `Forbidden` without `payments.record`; 422 when
/// the task is already done.
pub async fn mark_done(db: &Db, user: &AuthUser, id: i64) -> Result<FollowupTask> {
    let mut task = FollowupTask::find_or_404(db, id).await?;
    let may = user.has_permission(permissions::RECORD);
    if let Some(problem) = done_problem(&task, may) {
        return Err(problem);
    }
    task.status = FollowupStatus::Done;
    task.done_by = Some(user.id);
    task.done_at = Some(renox::db::now());
    task.save(db).await?;
    Ok(task)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payment() -> Payment {
        Payment {
            id: 5,
            party_id: 7,
            currency: "AED".into(),
            ..Default::default()
        }
    }

    #[test]
    fn money_is_shown_with_grouped_thousands_and_two_decimals() {
        assert_eq!(money_text("AED", 50_000), "AED 500.00");
        assert_eq!(money_text("AED", 300_000), "AED 3,000.00");
        assert_eq!(money_text("AED", 123_456_789), "AED 1,234,567.89");
        assert_eq!(money_text("AED", 5), "AED 0.05");
        assert_eq!(money_text("USD", -1_050), "USD -10.50");
    }

    #[test]
    fn the_raise_charge_task_names_the_amount_the_payment_and_the_refunds() {
        let task = FollowupTask::raise_charge(&payment(), 50_000, vec![9, 11]);
        assert_eq!(task.kind, FollowupKind::RaiseCharge);
        assert_eq!(task.text, "Raise a charge for AED 500.00");
        assert_eq!(task.role, "accountant");
        assert_eq!(task.payment_id, Some(5));
        assert_eq!(task.party(), PartyId(7));
        assert_eq!(task.amount, 50_000);
        assert_eq!(task.refund_ids.0, [9, 11]);
        assert!(task.is_open());
    }

    #[test]
    fn the_deficit_links_the_own_refunds_then_others_until_covered() {
        // The own refund covers it: no others.
        assert_eq!(linked_refunds(500, &[(9, 500)], &[(5, 100)]), [9]);
        // It does not: others, newest first, until covered.
        assert_eq!(
            linked_refunds(500, &[(9, 200)], &[(8, 200), (7, 300), (6, 900)]),
            [9, 8, 7]
        );
        assert_eq!(
            linked_refunds(100, &[], &[(4, 60), (3, 60), (2, 60)]),
            [4, 3]
        );
        assert!(linked_refunds(100, &[], &[]).is_empty());
    }

    #[test]
    fn only_an_open_task_is_marked_done_and_only_by_a_user_allowed_to() {
        let mut task = FollowupTask::raise_charge(&payment(), 100, vec![]);
        assert!(done_problem(&task, true).is_none());
        assert!(matches!(done_problem(&task, false), Some(Error::Forbidden)));
        task.status = FollowupStatus::Done;
        assert!(matches!(
            done_problem(&task, true),
            Some(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
        ));
    }
}
