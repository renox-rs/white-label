//! Recording the payer's remittance advice on a payer bill, and holding an
//! overpayment for a second person (wl-invoicing §3 *Remittance advice*, §4
//! *Payer bill* `record_remittance`, §5.8 step 7, `FIN.ACL.16`).
//!
//! The reconciler (`payer_bills.reconcile`) enters what the payer paid and
//! denied per line of an `issued` bill. The machine has no app state, so
//! [`record`] matches the advice to the lines ([`plan`]) and gathers the
//! result into a [`RemittanceInput`]. The bill moves to `paid` (every line
//! paid as billed), `denied` (nothing paid on any line) or `partially_paid`,
//! which are three transitions, [`transition_name`], each with a system twin
//! `..._approved` the kernel runs after an approval.
//!
//! **Overpayment.** What the payer paid above what is open on a line (less
//! what it denied on it) is held: the whole remittance waits as an approval
//! request (`FIN.ACL.16`, never the reconciler's own approval, SoD) and
//! nothing is recorded until a second person approves. The input's `amount`
//! is the overpayment, so the policy (any amount above 0) applies to
//! overpaying remittances only. A rejected request records nothing; the
//! reconciler corrects the advice. On approval the kernel runs the
//! `..._approved` twin with the stored input, and its guards and effect
//! check the lines again, so a bill that changed meanwhile fails the
//! request.
//!
//! The effect, in the transition's transaction: the [`PayerRemittance`] (its
//! reference is unique per payer, so the same advice is not recorded twice),
//! the paid and denied amounts and the payer's denial codes on the lines, one
//! [`PayerAllocation`] per paid line, the totals of the bill and, for an
//! approved overpayment, a movement of the payer credit balance
//! ([`crate::payer_credit`]). After the commit a listener emits
//! `PayerRemittanceRecorded`.
//!
//! Decisions:
//!
//! - One advice is recorded on one bill; a multi-bill import is the
//!   remittance import (Later), which runs one remittance per bill.
//! - The money of the advice is the payer's receipt in wl-payments; the
//!   application to lines is recorded here (the allocations below), and the
//!   unallocated rest stays the payer's party credit there. `payment` on the
//!   advice names the receipt.
//! - A system remittance (an import) is not offered: only a user's run is
//!   intercepted by an approval, and an overpayment may not skip one.

use renox::chrono::NaiveDate;
use renox::db::Json;
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{
    Outcome, StateMachine, Transition, TransitionCtx, TransitionError, Transitioned,
};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals;
use wl_kernel::machines::approved;
use wl_kernel::payer_bills::{
    PayerAnswer, PayerBillStatus, PayerRemittanceRecorded, Remittance, RemittanceLine,
};

use crate::payer_bill::{PayerBill, PayerBillLine, lines_of, write_totals_in};
use crate::payer_credit::{self, PayerCreditMovement};
use crate::permissions;
use crate::texts::english;

/// The prefix of the three remittance transitions.
pub const RECORD_REMITTANCE: &str = "record_remittance";

/// The approval policies' action key of moving money to or from the payer
/// credit balance (`FIN.ACL.16`): a remittance's overpayment and a
/// recoupment.
pub const ACTION: &str = "invoicing.payer_money";

/// The states a remittance leads to.
pub const OUTCOMES: [PayerBillStatus; 3] = [
    PayerBillStatus::Paid,
    PayerBillStatus::PartiallyPaid,
    PayerBillStatus::Denied,
];

/// The name of the remittance transition that leads to `outcome`:
/// `record_remittance_paid`, or its `_approved` twin the kernel runs.
pub fn transition_name(outcome: PayerBillStatus, approved_twin: bool) -> String {
    let name = format!("{RECORD_REMITTANCE}_{}", outcome.as_str());
    if approved_twin { approved(&name) } else { name }
}

/// A remittance advice recorded on a payer bill. Row of
/// `wl_invoicing_payer_remittances`.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_payer_remittances",
    unique(payer_party_id, reference),
    index(payer_bill_id)
)]
pub struct PayerRemittance {
    /// The primary key.
    pub id: i64,
    /// The payer bill it was recorded on.
    pub payer_bill_id: i64,
    /// The payer (a party).
    pub payer_party_id: i64,
    /// The legal entity whose books it belongs to.
    pub legal_entity_id: i64,
    /// The payer's reference; unique per payer.
    pub reference: String,
    /// The advice's date.
    pub advice_date: NaiveDate,
    /// The wl-payments payment that carried the money, if recorded there.
    pub payment_id: Option<i64>,
    /// What was applied to the lines, in fils.
    pub paid_total: i64,
    /// What the payer denied, in fils.
    pub denied_total: i64,
    /// What went to the payer credit balance, in fils.
    pub overpayment: i64,
    /// The lines as the payer sent them.
    pub lines: Json<Vec<RemittanceLine>>,
    /// The approval request of an overpayment, if one was needed.
    pub approval_request_id: Option<i64>,
    /// The user who recorded it.
    pub recorded_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// Money of a remittance applied to one payer bill line (the application
/// side of a payment). A reversed payment marks its rows removed.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_payer_allocations",
    index(payer_bill_id),
    index(payment_id)
)]
pub struct PayerAllocation {
    /// The primary key.
    pub id: i64,
    /// The payer bill.
    pub payer_bill_id: i64,
    /// The line it was applied to.
    pub payer_bill_line_id: i64,
    /// The remittance that carried it.
    pub remittance_id: i64,
    /// The wl-payments payment, if the advice names one.
    pub payment_id: Option<i64>,
    /// The amount applied, in fils.
    pub amount: i64,
    /// When a reversed payment or a recoupment took it back.
    pub removed_at: Option<DateTime>,
    /// Why it was taken back.
    pub removed_reason: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// One line of an advice, matched to a payer bill line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedLine {
    /// The payer bill line.
    pub line: i64,
    /// What is applied to the line's open amount.
    pub applied: i64,
    /// What the payer paid above it.
    pub over: i64,
    /// What the payer denied.
    pub denied: i64,
    /// The payer's denial code.
    pub code: Option<String>,
    /// The payer's denial reason.
    pub reason: Option<String>,
}

/// What a remittance run carries to its guards and effect, gathered by
/// [`record`]; it is also the approval request's payload, which the policy's
/// condition reads (`amount`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct RemittanceInput {
    /// The advice.
    pub remittance: Option<Remittance>,
    /// The advice matched to the bill's lines.
    pub plan: Vec<PlannedLine>,
    /// Why the advice cannot be recorded, if it cannot.
    pub problem: Option<String>,
    /// The total applied to the lines.
    pub applied: i64,
    /// The total paid above the open amounts.
    pub over: i64,
    /// The total denied.
    pub denied: i64,
    /// What needs a second person's approval: the overpayment, 0 for none.
    pub amount: i64,
    /// The user a system run acts for.
    pub by: Option<i64>,
}

/// The run's input, if it is a [`RemittanceInput`].
fn input(ctx: &TransitionCtx) -> Option<RemittanceInput> {
    ctx.input_as::<RemittanceInput>().ok()
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// Matches `remittance` to `lines`: what is applied, over and denied per
/// line, and the state the bill is left in. The reason it cannot be
/// recorded otherwise.
pub fn plan(
    lines: &[PayerBillLine],
    remittance: &Remittance,
) -> std::result::Result<(Vec<PlannedLine>, PayerBillStatus), String> {
    if remittance.reference.trim().is_empty() {
        return Err(english("wl_invoicing.payer_bill.errors.remit_reference").to_owned());
    }
    if remittance.lines.is_empty() {
        return Err(english("wl_invoicing.payer_bill.errors.remit_lines").to_owned());
    }
    let mut planned: Vec<PlannedLine> = Vec::new();
    for entry in &remittance.lines {
        let Some(line) = lines.iter().find(|l| l.id == entry.line) else {
            return Err(english("wl_invoicing.payer_bill.errors.remit_unmatched").to_owned());
        };
        if planned.iter().any(|p| p.line == line.id) {
            return Err(english("wl_invoicing.payer_bill.errors.remit_twice").to_owned());
        }
        if entry.paid < 0 || entry.denied < 0 || entry.paid + entry.denied == 0 {
            return Err(english("wl_invoicing.payer_bill.errors.remit_amount").to_owned());
        }
        if entry.denied > line.open() {
            return Err(english("wl_invoicing.payer_bill.errors.remit_denied").to_owned());
        }
        if entry.denied > 0 && entry.code.as_deref().is_none_or(|c| c.trim().is_empty()) {
            return Err(english("wl_invoicing.payer_bill.errors.remit_code").to_owned());
        }
        let room = line.open() - entry.denied;
        let over = (entry.paid - room).max(0);
        planned.push(PlannedLine {
            line: line.id,
            applied: entry.paid - over,
            over,
            denied: entry.denied,
            code: entry.code.clone(),
            reason: entry.reason.clone(),
        });
    }
    let outcome = outcome_after(lines, &planned);
    Ok((planned, outcome))
}

/// The state the bill is in once `planned` is applied to `lines`: `paid`
/// when every line is paid as billed, `denied` when nothing is paid on any
/// line, else `partially_paid`.
fn outcome_after(lines: &[PayerBillLine], planned: &[PlannedLine]) -> PayerBillStatus {
    let paid_after = |line: &PayerBillLine| {
        line.paid
            + planned
                .iter()
                .find(|p| p.line == line.id)
                .map_or(0, |p| p.applied)
    };
    if lines.iter().all(|l| paid_after(l) == l.claimed) {
        PayerBillStatus::Paid
    } else if lines.iter().all(|l| paid_after(l) == 0) {
        PayerBillStatus::Denied
    } else {
        PayerBillStatus::PartiallyPaid
    }
}

/// The three remittance transitions and their approved twins, for the
/// machine.
pub(crate) fn transitions() -> Vec<Transition<PayerBill>> {
    let mut all = Vec::new();
    for outcome in OUTCOMES {
        all.push(rules(
            Transition::new(transition_name(outcome, false))
                .label("Record remittance")
                .tone("primary")
                .permission(permissions::PAYER_BILLS_RECONCILE),
            outcome,
        ));
        all.push(rules(
            Transition::new(transition_name(outcome, true))
                .label("Record remittance (approved)")
                .system(),
            outcome,
        ));
    }
    all
}

/// What the transitions share: the issued state, the guards and the effect.
fn rules(transition: Transition<PayerBill>, outcome: PayerBillStatus) -> Transition<PayerBill> {
    transition
        .from([PayerBillStatus::Issued])
        .to(outcome)
        .guard("remittance_matched", |_bill: &PayerBill, ctx| {
            let problem = match input(ctx) {
                Some(RemittanceInput {
                    problem: Some(problem),
                    ..
                }) => Some(problem),
                Some(RemittanceInput {
                    remittance: Some(_),
                    ref plan,
                    ..
                }) if !plan.is_empty() => None,
                _ => Some(english("wl_invoicing.payer_bill.errors.remit_lines").to_owned()),
            };
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|bill: &PayerBill, ctx, tx| {
            let bill = bill.clone();
            let gathered = input(ctx);
            let approval = approval_of(ctx.meta());
            let by = ctx
                .actor()
                .map(|user| user.id)
                .or_else(|| gathered.as_ref().and_then(|i| i.by));
            Box::pin(async move {
                let gathered = gathered.ok_or_else(|| {
                    Error::from(renox::anyhow::anyhow!("a remittance run without its input"))
                })?;
                write_remittance_in(tx, &bill, gathered, approval, by).await
            })
        })
}

/// The approval request a run's history metadata names, if any.
pub(crate) fn approval_of(meta: &Value) -> Option<i64> {
    meta.get("approval_request").and_then(Value::as_i64)
}

/// What a remittance writes besides the status, in the run's transaction.
async fn write_remittance_in(
    tx: &mut renox::db::Transaction,
    bill: &PayerBill,
    input: RemittanceInput,
    approval: Option<i64>,
    by: Option<i64>,
) -> Result {
    let remittance = input.remittance.ok_or_else(|| {
        Error::from(renox::anyhow::anyhow!(
            "a remittance run without its advice"
        ))
    })?;
    let mut lines = lines_of(&mut *tx, bill.id).await?;
    // The lines may have changed since the advice was matched (an approval
    // can wait): the plan must still fit.
    for planned in &input.plan {
        let fits = lines
            .iter()
            .find(|l| l.id == planned.line)
            .is_some_and(|l| planned.applied + planned.denied <= l.open());
        if !fits {
            return Err(refused(english(
                "wl_invoicing.payer_bill.errors.remit_changed",
            )));
        }
    }
    let mut row = PayerRemittance {
        payer_bill_id: bill.id,
        payer_party_id: bill.payer_party_id,
        legal_entity_id: bill.legal_entity_id,
        reference: remittance.reference.clone(),
        advice_date: remittance.date,
        payment_id: remittance.payment,
        paid_total: input.applied,
        denied_total: input.denied,
        overpayment: input.over,
        lines: Json(remittance.lines.clone()),
        approval_request_id: approval,
        recorded_by: by,
        ..Default::default()
    };
    row.insert(&mut *tx).await?;
    for planned in &input.plan {
        let Some(line) = lines.iter_mut().find(|l| l.id == planned.line) else {
            continue;
        };
        line.paid += planned.applied;
        line.denied = planned.denied;
        if planned.denied > 0 {
            line.denial_code = planned.code.clone();
            line.denial_reason = planned.reason.clone();
        }
        PayerBillLine::query()
            .where_(PayerBillLine::ID.eq(line.id))
            .update(
                &mut *tx,
                &[
                    ("paid", &line.paid),
                    ("denied", &line.denied),
                    ("denial_code", &line.denial_code),
                    ("denial_reason", &line.denial_reason),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
        if planned.applied > 0 {
            PayerAllocation {
                payer_bill_id: bill.id,
                payer_bill_line_id: line.id,
                remittance_id: row.id,
                payment_id: remittance.payment,
                amount: planned.applied,
                ..Default::default()
            }
            .insert(&mut *tx)
            .await?;
        }
    }
    write_totals_in(tx, bill.id, &lines).await?;
    if input.over > 0 {
        PayerCreditMovement {
            payer_party_id: bill.payer_party_id,
            legal_entity_id: bill.legal_entity_id,
            currency: bill.currency.clone(),
            kind: payer_credit::OVERPAYMENT.to_owned(),
            amount: input.over,
            payer_bill_id: Some(bill.id),
            remittance_id: Some(row.id),
            ..Default::default()
        }
        .insert(&mut *tx)
        .await?;
    }
    let open: i64 = lines.iter().map(PayerBillLine::open).sum();
    PayerBill::query()
        .where_(PayerBill::ID.eq(bill.id))
        .update(
            &mut *tx,
            &[
                ("has_remittance", &true),
                ("last_remittance_on", &Some(remittance.date)),
                ("in_ageing", &(open > 0)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// What a call that may wait for approval came to.
pub(crate) fn answer_of(outcome: &Outcome<PayerBill>) -> PayerAnswer {
    match outcome {
        Outcome::Done(_) => PayerAnswer::Done,
        Outcome::Pending(pending) => PayerAnswer::Pending {
            request: pending
                .data
                .get(approvals::submit::REQUEST_KEY)
                .and_then(Value::as_i64)
                .unwrap_or_default(),
        },
    }
}

/// Records `remittance` on the issued payer bill `bill_id` as `user`, in its
/// own transaction. An advice that overpays a line waits as an approval
/// request (`FIN.ACL.16`): the answer is then [`PayerAnswer::Pending`], the
/// request is announced and nothing is recorded.
///
/// # Errors
///
/// `Forbidden` without `payer_bills.reconcile`; `NotFrom` unless the bill is
/// `issued`; `Guard` (422) for an advice without a reference or lines, a
/// line that is not on the bill, repeated, without an amount, denied above
/// what is open or denied without the payer's code, or a reference already
/// recorded for the payer. Nothing is written then.
pub async fn record(
    state: &AppState,
    user: &AuthUser,
    bill_id: i64,
    remittance: Remittance,
) -> std::result::Result<(PayerBill, PayerAnswer), TransitionError> {
    let bill = PayerBill::find_or_404(&state.db, bill_id).await?;
    let lines = lines_of(&state.db, bill.id).await?;
    let reference = remittance.reference.trim().to_owned();
    let used = PayerRemittance::query()
        .where_(PayerRemittance::PAYER_PARTY_ID.eq(bill.payer_party_id))
        .where_(PayerRemittance::REFERENCE.eq(reference))
        .count(&state.db)
        .await?
        > 0;
    let mut gathered = RemittanceInput {
        remittance: Some(remittance.clone()),
        by: Some(user.id),
        ..Default::default()
    };
    let mut outcome = PayerBillStatus::PartiallyPaid;
    match plan(&lines, &remittance) {
        Ok((planned, after)) => {
            gathered.applied = planned.iter().map(|p| p.applied).sum();
            gathered.over = planned.iter().map(|p| p.over).sum();
            gathered.denied = planned.iter().map(|p| p.denied).sum();
            gathered.amount = gathered.over;
            gathered.plan = planned;
            outcome = after;
        }
        Err(problem) => gathered.problem = Some(problem),
    }
    if used && gathered.problem.is_none() {
        gathered.problem = Some(english("wl_invoicing.payer_bill.errors.remit_used").to_owned());
    }
    let name = transition_name(outcome, false);
    let ran = bill
        .transition(&name)
        .by(user)
        .meta(json!({}))
        .input(&gathered)
        .run(state)
        .await?;
    approvals::announce(state, &ran)
        .await
        .map_err(TransitionError::Failed)?;
    Ok((
        PayerBill::find_or_404(&state.db, bill_id).await?,
        answer_of(&ran),
    ))
}

/// After the commit of a remittance (recorded at once or after the
/// approval): emits `PayerRemittanceRecorded`.
pub(crate) async fn on_transition(state: &AppState, event: &Transitioned<PayerBill>) -> Result {
    if !event.transition.starts_with(RECORD_REMITTANCE) {
        return Ok(());
    }
    let bill = PayerBill::find_or_404(&state.db, event.key).await?;
    let Some(row) = PayerRemittance::query()
        .where_(PayerRemittance::PAYER_BILL_ID.eq(bill.id))
        .order_by_desc(PayerRemittance::ID)
        .first(&state.db)
        .await?
    else {
        return Ok(());
    };
    state
        .emit(PayerRemittanceRecorded {
            payer_bill: bill.payer_bill_id(),
            payer: bill.payer(),
            source: bill.source(),
            claimed: bill.claimed_total,
            paid: bill.paid_total,
            denied: bill.denied_total,
            open: bill.balance,
            reference: row.reference,
            overpayment: row.overpayment,
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: i64, claimed: i64, paid: i64) -> PayerBillLine {
        PayerBillLine {
            id,
            position: id,
            claimed,
            paid,
            ..Default::default()
        }
    }

    fn day() -> NaiveDate {
        "2026-12-10".parse().unwrap()
    }

    #[test]
    fn the_three_outcomes_have_a_user_and_an_approved_transition() {
        let machine = PayerBill::machine();
        for outcome in OUTCOMES {
            let user = machine.find(&transition_name(outcome, false)).unwrap();
            assert!(!user.is_system());
            assert_eq!(user.from_states(), ["issued"]);
            assert_eq!(user.to_state(), Some(outcome.as_str()));
            let twin = machine.find(&transition_name(outcome, true)).unwrap();
            assert!(twin.is_system());
            assert_eq!(twin.to_state(), Some(outcome.as_str()));
        }
        assert_eq!(
            transition_name(PayerBillStatus::PartiallyPaid, true),
            "record_remittance_partially_paid_approved"
        );
    }

    #[test]
    fn a_line_paid_in_full_leaves_the_bill_paid() {
        let lines = [line(1, 40_000, 0)];
        let advice = Remittance::new("ADV-1", day()).line(RemittanceLine::paid(1, 40_000));
        let (planned, outcome) = plan(&lines, &advice).unwrap();
        assert_eq!(outcome, PayerBillStatus::Paid);
        assert_eq!(planned[0].applied, 40_000);
        assert_eq!(planned[0].over, 0);
    }

    #[test]
    fn what_is_paid_above_the_open_amount_is_the_overpayment() {
        let lines = [line(1, 40_000, 0)];
        let advice = Remittance::new("ADV-1", day()).line(RemittanceLine::paid(1, 45_000));
        let (planned, outcome) = plan(&lines, &advice).unwrap();
        assert_eq!((planned[0].applied, planned[0].over), (40_000, 5_000));
        assert_eq!(outcome, PayerBillStatus::Paid);
        // What the payer denied counts against the room on the line.
        let advice = Remittance::new("ADV-2", day()).line(RemittanceLine::paid(1, 35_000).denied(
            10_000,
            "R12",
            "not covered",
        ));
        let (planned, outcome) = plan(&lines, &advice).unwrap();
        assert_eq!((planned[0].applied, planned[0].over), (30_000, 5_000));
        assert_eq!(outcome, PayerBillStatus::PartiallyPaid);
    }

    #[test]
    fn a_bill_with_nothing_paid_is_denied() {
        let lines = [line(1, 40_000, 0), line(2, 10_000, 0)];
        let advice = Remittance::new("ADV-1", day())
            .line(RemittanceLine::paid(1, 0).denied(40_000, "R01", "no cover"))
            .line(RemittanceLine::paid(2, 0).denied(10_000, "R01", "no cover"));
        let (planned, outcome) = plan(&lines, &advice).unwrap();
        assert_eq!(outcome, PayerBillStatus::Denied);
        assert_eq!(planned.iter().map(|p| p.denied).sum::<i64>(), 50_000);
    }

    #[test]
    fn an_advice_that_does_not_match_the_bill_is_refused() {
        let lines = [line(1, 40_000, 0)];
        let bad = |advice: Remittance| plan(&lines, &advice).unwrap_err();
        assert!(!bad(Remittance::new(" ", day()).line(RemittanceLine::paid(1, 1))).is_empty());
        assert!(!bad(Remittance::new("A", day())).is_empty());
        assert!(!bad(Remittance::new("A", day()).line(RemittanceLine::paid(9, 1))).is_empty());
        assert!(
            !bad(Remittance::new("A", day())
                .line(RemittanceLine::paid(1, 1))
                .line(RemittanceLine::paid(1, 1)))
            .is_empty()
        );
        assert!(!bad(Remittance::new("A", day()).line(RemittanceLine::paid(1, 0))).is_empty());
        // Denied above what is open, or without a code.
        assert!(
            !bad(Remittance::new("A", day())
                .line(RemittanceLine::paid(1, 0).denied(50_000, "R1", "x")))
            .is_empty()
        );
        let mut no_code = RemittanceLine::paid(1, 0);
        no_code.denied = 100;
        assert!(!bad(Remittance::new("A", day()).line(no_code)).is_empty());
    }

    #[renox::test]
    async fn the_guard_reads_the_gathered_advice() {
        let machine = PayerBill::machine();
        let transition = machine
            .find(&transition_name(PayerBillStatus::Paid, false))
            .unwrap();
        let ctx = |input: &RemittanceInput| {
            TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).unwrap())
        };
        let bill = PayerBill::default();
        let ready = RemittanceInput {
            remittance: Some(Remittance::new("A", day())),
            plan: vec![PlannedLine {
                line: 1,
                applied: 1,
                over: 0,
                denied: 0,
                code: None,
                reason: None,
            }],
            ..Default::default()
        };
        assert!(transition.check_guards(&bill, &ctx(&ready)).await.is_ok());
        let problem = RemittanceInput {
            problem: Some("no".into()),
            ..ready.clone()
        };
        assert!(
            transition
                .check_guards(&bill, &ctx(&problem))
                .await
                .is_err()
        );
        assert!(
            transition
                .check_guards(&bill, &TransitionCtx::new(None))
                .await
                .is_err()
        );
    }
}
