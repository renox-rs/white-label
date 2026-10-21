//! Follow-up tasks (wl-payments A10): work the module leaves for a role.
//!
//! The first kind is `raise_charge`: after a reversal, a party whose credit
//! was already refunded owes money, and the accountant gets a task "Raise a
//! charge for AED 500.00" with the payment and the refunds linked. It is a
//! payments record, not a ticket (`TaskCreator` cannot close a ticket, and
//! ticketing may be off). No invoice or charge is ever created from it;
//! the accountant raises the charge and marks the task done. The task is
//! unique per kind and payment.

use renox::chrono::NaiveDate;
use renox::db::Json;
use renox::prelude::*;
use renox::serde_json::{self, Value};
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::payments::PAYMENT_SUBJECT_TYPE;

use crate::cheque::Cheque;
use crate::credit;
use crate::notices;
use crate::payment::Payment;
use crate::permissions;
use crate::refund::{Refund, RefundStatus};

/// The role that gets the `raise_charge` task.
pub const ACCOUNTANT_ROLE: &str = "accountant";

/// The role told when a bounced cheque is referred to a legal case.
pub const MANAGER_ROLE: &str = "finance_manager";

/// What a follow-up task asks for.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FollowupKind {
    /// Raise a charge for money the party owes after a reversal.
    #[default]
    RaiseCharge,
    /// Pay a failed refund another way (`FIN.RUL.21`): a transfer (new bank
    /// details need an approval), a cheque in the party's name or cash.
    ChangeMethod,
    /// Follow up a bounced received cheque (`FIN.RUL.04`): re-present,
    /// settle, replace, refer to legal or write off.
    FollowUpCheque,
    /// Replace a bounced issued cheque: a new issued cheque or an outgoing
    /// payment covers the open amount.
    ReplaceCheque,
    /// Raise the bounce fee charge: invoicing could not schedule it.
    RaiseBounceFee,
    /// The follow-up of a legal case, for its case owner, with the evidence
    /// pack (`refer_to_legal`).
    LegalCase,
    /// Hand the original of a replaced cheque back to the party, with a
    /// receipt, once its replacement cleared.
    HandBack,
    /// An invoice write-off a cheque `write_off` asked for was rejected or
    /// failed: the cheque stays where it is (`write_off`).
    WriteOffFailed,
    /// Decide what to do with a bounced cheque that stayed unpaid M days
    /// (`FIN.RUL.18`): re-present it, refer it to legal or write it off.
    DecideCheque,
    /// Raise the top-up charge of a deposit whose amount grew: nothing could
    /// schedule it (`adjust`).
    RaiseTopUp,
    /// The gateway reported another amount than the payment's: look at it
    /// in the gateway (`late_confirm`, `confirm`).
    AmountMismatch,
    /// Submit the evidence of a chargeback by the gateway's deadline
    /// (`FIN.RUL.20`).
    ChargebackEvidence,
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
    /// The refund a `change_method` task is about.
    pub refund_id: Option<i64>,
    /// The cheque a cheque task is about.
    pub cheque_id: Option<i64>,
    /// The user a task is assigned to (the case owner of a legal case); the
    /// role still sees it.
    pub assignee_id: Option<i64>,
    /// The day the task is due: the evidence deadline of a chargeback.
    pub due_on: Option<NaiveDate>,
    /// The day the accountant was reminded of the deadline.
    pub reminded_on: Option<NaiveDate>,
    /// What a task carries for its reader: the evidence pack of a legal case.
    pub evidence: Json<Value>,
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

    /// The evidence task of the chargeback on `payment` (`FIN.RUL.20`): due
    /// `due_on`, the links to what the platform holds in `evidence`, `text`
    /// says what to do.
    pub fn chargeback_evidence(
        payment: &Payment,
        due_on: NaiveDate,
        evidence: Value,
        text: String,
    ) -> Self {
        FollowupTask {
            kind: FollowupKind::ChargebackEvidence,
            party_id: payment.party_id,
            payment_id: Some(payment.id),
            amount: payment.amount,
            currency: payment.currency.clone(),
            evidence: Json(evidence),
            text,
            role: ACCOUNTANT_ROLE.to_owned(),
            status: FollowupStatus::Open,
            due_on: Some(due_on),
            ..Default::default()
        }
    }

    /// The task to look into a gateway confirmation of `reported` fils that
    /// does not match `payment`.
    pub fn amount_mismatch(payment: &Payment, reported: i64) -> Self {
        FollowupTask {
            kind: FollowupKind::AmountMismatch,
            party_id: payment.party_id,
            payment_id: Some(payment.id),
            amount: reported,
            currency: payment.currency.clone(),
            text: format!(
                "The gateway confirmed {} for payment #{} of {}",
                money_text(&payment.currency, reported),
                payment.id,
                money_text(&payment.currency, payment.amount)
            ),
            role: ACCOUNTANT_ROLE.to_owned(),
            status: FollowupStatus::Open,
            ..Default::default()
        }
    }

    /// An open `change_method` task for the failed `refund`, for the
    /// accountant: pay it by another method (`FIN.RUL.21`).
    pub fn change_method(refund: &Refund) -> Self {
        FollowupTask {
            kind: FollowupKind::ChangeMethod,
            party_id: refund.party_id,
            payment_id: None,
            amount: refund.amount,
            currency: refund.currency.clone(),
            refund_ids: Json(vec![refund.id]),
            refund_id: Some(refund.id),
            text: format!(
                "Change the method of refund #{} ({})",
                refund.id,
                money_text(&refund.currency, refund.amount)
            ),
            role: ACCOUNTANT_ROLE.to_owned(),
            status: FollowupStatus::Open,
            ..Default::default()
        }
    }

    /// A cheque task of `kind` for the accountant, about `cheque`, with
    /// `text`.
    fn for_cheque(kind: FollowupKind, cheque: &Cheque, amount: i64, text: String) -> Self {
        FollowupTask {
            kind,
            party_id: cheque.party_id,
            payment_id: None,
            amount,
            currency: cheque.currency.clone(),
            cheque_id: Some(cheque.id),
            text,
            role: ACCOUNTANT_ROLE.to_owned(),
            status: FollowupStatus::Open,
            evidence: Json(serde_json::json!({})),
            ..Default::default()
        }
    }

    /// The task to follow up the bounced received `cheque` (`FIN.RUL.04`).
    pub fn follow_up_cheque(cheque: &Cheque) -> Self {
        Self::for_cheque(
            FollowupKind::FollowUpCheque,
            cheque,
            cheque.open_amount,
            format!(
                "Follow up bounced cheque {} ({} open)",
                cheque.number,
                money_text(&cheque.currency, cheque.open_amount)
            ),
        )
    }

    /// The task to replace the bounced issued `cheque`.
    pub fn replace_cheque(cheque: &Cheque) -> Self {
        Self::for_cheque(
            FollowupKind::ReplaceCheque,
            cheque,
            cheque.open_amount,
            format!(
                "Replace bounced cheque {} ({} open)",
                cheque.number,
                money_text(&cheque.currency, cheque.open_amount)
            ),
        )
    }

    /// The task to raise the bounce fee of `cheque`: `fee` fils before VAT.
    pub fn raise_bounce_fee(cheque: &Cheque, fee: i64) -> Self {
        Self::for_cheque(
            FollowupKind::RaiseBounceFee,
            cheque,
            fee,
            format!(
                "Raise a bounce fee charge of {} for cheque {}",
                money_text(&cheque.currency, fee),
                cheque.number
            ),
        )
    }

    /// The task to hand the original of the replaced `cheque` back.
    pub fn hand_back(cheque: &Cheque) -> Self {
        Self::for_cheque(
            FollowupKind::HandBack,
            cheque,
            cheque.amount,
            format!(
                "Hand back cheque {} to the party with a receipt",
                cheque.number
            ),
        )
    }

    /// The task to raise a top-up charge of `amount` fils for `deposit`, after
    /// the amount grew with version `source_version` of its source, due `due`.
    pub fn raise_top_up(
        deposit: &crate::deposit::Deposit,
        amount: i64,
        source_version: i32,
        due: renox::chrono::NaiveDate,
    ) -> Self {
        FollowupTask {
            kind: FollowupKind::RaiseTopUp,
            party_id: deposit.party_id,
            payment_id: None,
            amount,
            currency: deposit.currency.clone(),
            text: format!(
                "Raise a top-up charge of {} for deposit {} (version {source_version} of {}), due {}",
                money_text(&deposit.currency, amount),
                deposit.id,
                deposit.source(),
                due.format("%Y-%m-%d")
            ),
            role: ACCOUNTANT_ROLE.to_owned(),
            status: FollowupStatus::Open,
            evidence: Json(serde_json::json!({})),
            ..Default::default()
        }
    }

    /// The decision task of `FIN.RUL.18` for `role`: re-present, refer to
    /// legal or write off the bounced `cheque`, which stayed unpaid `days`
    /// days.
    pub fn decide_cheque(cheque: &Cheque, role: &str, days: i64) -> Self {
        let mut task = Self::for_cheque(
            FollowupKind::DecideCheque,
            cheque,
            cheque.open_amount,
            format!(
                "Decide on bounced cheque {} ({} open for {days} days): re-present, refer to legal or write off",
                cheque.number,
                money_text(&cheque.currency, cheque.open_amount)
            ),
        );
        task.role = role.to_owned();
        task
    }

    /// The task that a write-off request of the `cheque` was rejected or
    /// failed, with `text` saying which and why.
    pub fn write_off_failed(cheque: &Cheque, text: String) -> Self {
        Self::for_cheque(
            FollowupKind::WriteOffFailed,
            cheque,
            cheque.open_amount,
            text,
        )
    }

    /// The follow-up of the legal case of `cheque` for its case owner
    /// `owner`, carrying the `evidence` pack. The accountant role sees it
    /// too.
    pub fn legal_case(cheque: &Cheque, owner: i64, evidence: Value) -> Self {
        let mut task = Self::for_cheque(
            FollowupKind::LegalCase,
            cheque,
            cheque.open_amount,
            format!(
                "Follow up the legal case of cheque {} ({} open)",
                cheque.number,
                money_text(&cheque.currency, cheque.open_amount)
            ),
        );
        task.assignee_id = Some(owner);
        task.evidence = Json(evidence);
        task
    }

    /// The open tasks about the cheque `cheque_id`, oldest first.
    pub async fn open_of_cheque(db: &Db, cheque_id: i64) -> Result<Vec<FollowupTask>> {
        FollowupTask::query()
            .where_(FollowupTask::CHEQUE_ID.eq(cheque_id))
            .where_(FollowupTask::STATUS.eq(FollowupStatus::Open))
            .order_by(FollowupTask::ID)
            .get(db)
            .await
    }

    /// The tasks about the cheque `cheque_id`, done ones too, oldest first.
    pub async fn of_cheque(db: &Db, cheque_id: i64) -> Result<Vec<FollowupTask>> {
        FollowupTask::query()
            .where_(FollowupTask::CHEQUE_ID.eq(cheque_id))
            .order_by(FollowupTask::ID)
            .get(db)
            .await
    }

    /// Marks the open tasks of `kinds` about the cheque `cheque_id` done, by
    /// nobody: the work is no longer to do. Returns how many were closed.
    pub async fn close_for_cheque(
        db: &Db,
        cheque_id: i64,
        kinds: &[FollowupKind],
    ) -> Result<usize> {
        let open = Self::open_of_cheque(db, cheque_id).await?;
        let mut closed = 0;
        for mut task in open {
            if !kinds.contains(&task.kind) {
                continue;
            }
            task.status = FollowupStatus::Done;
            task.done_at = Some(renox::db::now());
            task.save(db).await?;
            closed += 1;
        }
        Ok(closed)
    }

    /// The open tasks of `role` and those assigned to user `user_id`, oldest
    /// first.
    pub async fn open_for_user(db: &Db, role: &str, user_id: i64) -> Result<Vec<FollowupTask>> {
        let mut tasks = Self::open_for(db, role).await?;
        let assigned = FollowupTask::query()
            .where_(FollowupTask::ASSIGNEE_ID.eq(user_id))
            .where_(FollowupTask::STATUS.eq(FollowupStatus::Open))
            .order_by(FollowupTask::ID)
            .get(db)
            .await?;
        for task in assigned {
            if !tasks.iter().any(|t| t.id == task.id) {
                tasks.push(task);
            }
        }
        tasks.sort_by_key(|t| t.id);
        Ok(tasks)
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

    /// The open tasks about the refund `refund_id`, oldest first.
    pub async fn open_of_refund(db: &Db, refund_id: i64) -> Result<Vec<FollowupTask>> {
        FollowupTask::query()
            .where_(FollowupTask::REFUND_ID.eq(refund_id))
            .where_(FollowupTask::STATUS.eq(FollowupStatus::Open))
            .order_by(FollowupTask::ID)
            .get(db)
            .await
    }

    /// Marks the open tasks about the refund `refund_id` done, by nobody: the
    /// refund left `failed`, so the work is no longer to do. Returns how
    /// many were closed.
    pub async fn close_for_refund(db: &Db, refund_id: i64) -> Result<usize> {
        let open = Self::open_of_refund(db, refund_id).await?;
        let closed = open.len();
        for mut task in open {
            task.status = FollowupStatus::Done;
            task.done_at = Some(renox::db::now());
            task.save(db).await?;
        }
        Ok(closed)
    }

    /// Saves `task` unless its payment already has a task of its kind
    /// (unique per payment), or its refund an open one; returns the new task,
    /// or `None` for the repeat.
    pub async fn create_once(db: &Db, mut task: FollowupTask) -> Result<Option<FollowupTask>> {
        if let Some(refund_id) = task.refund_id {
            let open = FollowupTask::query()
                .where_(FollowupTask::KIND.eq(task.kind))
                .where_(FollowupTask::REFUND_ID.eq(refund_id))
                .where_(FollowupTask::STATUS.eq(FollowupStatus::Open))
                .count(db)
                .await?;
            if open > 0 {
                return Ok(None);
            }
        }
        if let Some(cheque_id) = task.cheque_id {
            // One open task of a kind per cheque; a hand-back only once.
            let mut same = FollowupTask::query()
                .where_(FollowupTask::KIND.eq(task.kind))
                .where_(FollowupTask::CHEQUE_ID.eq(cheque_id))
                .where_(FollowupTask::ROLE.eq(task.role.as_str()));
            if task.kind != FollowupKind::HandBack {
                same = same.where_(FollowupTask::STATUS.eq(FollowupStatus::Open));
            }
            if same.count(db).await? > 0 {
                return Ok(None);
            }
        }
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
/// `payments.record` (the accountant has it) or be the task's assignee, and
/// the task must be open.
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
    // The case owner of a legal case may close it without `payments.record`.
    let may = user.has_permission(permissions::RECORD)
        || task.assignee_id == Some(user.id)
        || (task.role != ACCOUNTANT_ROLE && user.has_role(&task.role));
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
