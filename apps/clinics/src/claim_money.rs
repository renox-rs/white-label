//! The payer's money on a claim: the remittance, the overpayment held for a
//! second person and the recoupment (05 §4.3 "When it goes wrong", §7.1
//! *Claim* `record_remittance` and `record_recoupment`; `FIN.ACL.16`;
//! BL-CLN-08).
//!
//! The money is recorded on the claim's payer bill through the kernel's
//! `PayerBills`. wl-invoicing matches the advice to the bill's lines, holds a
//! remittance that pays more than is open on a line (and every recoupment
//! recorded by a user) as an approval request for a second person, never the
//! one who recorded it, and applies it when the approval is given. A reversed
//! wl-payments payment is recouped by wl-invoicing itself, without approval.
//!
//! **The claim follows its payer bill** ([`follow`]): whatever recorded the
//! money (the user at once, the approval later, the reversal of a payment),
//! the payer bill announces it (`PayerRemittanceRecorded`,
//! `PayerRecoupmentRecorded`) and the claim runs its system transition to the
//! state the payer bill reached: `record_remittance_*` from `submitted` or
//! `resubmitted`, `record_recoupment_*` from `paid`, `partially_paid` or
//! `written_off`. Following is idempotent: it compares the claim with the
//! payer bill and does nothing when they agree.
//!
//! While an advice waits for approval the claim stays as it is and shows it
//! (`held_*`): nothing is recorded. When the approval ends without a yes the
//! hold is cleared and the way it ended is kept (`held_result`); the
//! requester is told by the approvals module.
//!
//! Decisions:
//!
//! - The advice is recorded on one claim; a multi-claim import and a
//!   recoupment line on the advice of another claim are the remittance import
//!   (Later, `CLN.CLM.02`): a recoupment is recorded on the claim it takes
//!   money back from.
//! - The date the claim keeps for the resubmission window is the date of the
//!   advice (of the one that carried the recoupment, for a recoupment).
//! - A reversed payment's recoupment has no advice: the day of the reversal
//!   (today) is its date, and the recoupment code is `payment_reversed`.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::approvals::{ApprovalDecided, ApprovalEnd};
use wl_kernel::documents::today;
use wl_kernel::payer_bills::{
    PAYER_BILL_SUBJECT_TYPE, PayerAnswer, PayerBillStatus, PayerRecoupmentRecorded,
    PayerRemittanceRecorded, Recoupment, Remittance, RemittanceLine, payer_bills,
};

use crate::claim::{AnswerKind, Claim, ClaimLine, ClaimStatus};
use crate::claim_machine::{
    ClaimInput, LineMoney, MoneyInput, RECORD_RECOUPMENT_DENIED, RECORD_RECOUPMENT_PARTIALLY_PAID,
    RECORD_REMITTANCE_DENIED, RECORD_REMITTANCE_PAID, RECORD_REMITTANCE_PARTIALLY_PAID, clean,
};
use crate::claims::can_manage;
use crate::texts::english;

/// One line of a remittance advice, by the claim's line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdviceLine {
    /// The claim line.
    pub line: i64,
    /// What the payer paid on it in this advice, in fils.
    pub paid: i64,
    /// What the payer denied on it, in fils.
    pub denied: i64,
    /// The payer's denial code.
    pub code: Option<String>,
    /// The payer's reason.
    pub reason: Option<String>,
}

impl AdviceLine {
    /// `paid` fils paid on claim line `line`, nothing denied.
    pub fn paid(line: i64, paid: i64) -> Self {
        AdviceLine {
            line,
            paid,
            denied: 0,
            code: None,
            reason: None,
        }
    }

    /// With `denied` fils denied under `code` and `reason`.
    pub fn denied(mut self, denied: i64, code: &str, reason: &str) -> Self {
        self.denied = denied;
        self.code = Some(code.trim().to_owned()).filter(|c| !c.is_empty());
        self.reason = Some(reason.trim().to_owned()).filter(|r| !r.is_empty());
        self
    }
}

/// The payer's remittance advice for one claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advice {
    /// The payer's reference of the advice; unique per payer.
    pub reference: String,
    /// The date of the advice.
    pub date: NaiveDate,
    /// The wl-payments payment that carried the money, if recorded there.
    pub payment: Option<i64>,
    /// What it pays and denies, per claim line.
    pub lines: Vec<AdviceLine>,
}

impl Advice {
    /// The advice `reference` dated `date`, with no line yet.
    pub fn new(reference: &str, date: NaiveDate) -> Self {
        Advice {
            reference: reference.trim().to_owned(),
            date,
            payment: None,
            lines: Vec::new(),
        }
    }

    /// Carried by wl-payments payment `payment`.
    pub fn payment(mut self, payment: i64) -> Self {
        self.payment = Some(payment);
        self
    }

    /// With `line` added.
    pub fn line(mut self, line: AdviceLine) -> Self {
        self.lines.push(line);
        self
    }
}

/// Money a payer takes back from a claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Takeback {
    /// The payer's recoupment code, e.g. `R07`.
    pub code: String,
    /// The payer's reason, if it gave one.
    pub reason: Option<String>,
    /// The reference of the later advice that carried it.
    pub reference: Option<String>,
    /// The date of that advice.
    pub date: NaiveDate,
    /// The claim lines and the amounts taken back, in fils.
    pub lines: Vec<(i64, i64)>,
}

impl Takeback {
    /// A recoupment under `code` on the advice dated `date`, with no line
    /// yet.
    pub fn new(code: &str, date: NaiveDate) -> Self {
        Takeback {
            code: code.trim().to_owned(),
            reason: None,
            reference: None,
            date,
            lines: Vec::new(),
        }
    }

    /// With the payer's `reason`.
    pub fn reason(mut self, reason: &str) -> Self {
        self.reason = clean(&Some(reason.to_owned()));
        self
    }

    /// Carried by the advice `reference`.
    pub fn carried_by(mut self, reference: &str) -> Self {
        self.reference = clean(&Some(reference.to_owned()));
        self
    }

    /// With `amount` taken back from claim line `line`.
    pub fn line(mut self, line: i64, amount: i64) -> Self {
        self.lines.push((line, amount));
        self
    }
}

/// What recording money came to.
#[derive(Debug, Clone, PartialEq)]
pub enum Recorded {
    /// The payer bill recorded it and the claim follows.
    Done(Claim),
    /// A second person must approve first: nothing is recorded.
    Held(Claim),
}

/// What the payer bill announced of the money, for [`follow`].
#[derive(Debug, Clone, Default)]
pub struct Heard {
    /// The payer's reference of the advice.
    pub reference: Option<String>,
    /// The part that went to the payer credit balance, in fils.
    pub overpayment: i64,
    /// The recoupment code.
    pub code: Option<String>,
    /// Whether a reversed payment caused it.
    pub reversal: bool,
}

fn guard(message: String) -> TransitionError {
    TransitionError::Guard(message)
}

/// Notes on the claim what is being recorded on its payer bill, so that the
/// payer bill's announcement finds the advice's date and the user.
async fn mark(
    state: &AppState,
    id: i64,
    kind: &str,
    reference: Option<&str>,
    date: NaiveDate,
    by: i64,
) -> Result {
    let none: Option<String> = None;
    Claim::query()
        .where_(Claim::ID.eq(id))
        .update(
            &state.db,
            &[
                ("held_kind", &Some(kind.to_owned())),
                ("held_reference", &reference.map(str::to_owned)),
                ("held_date", &Some(date)),
                ("held_by", &Some(by)),
                ("held_result", &none),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// Clears what [`mark`] noted and any hold, optionally keeping how a hold
/// ended.
async fn clear(state: &AppState, id: i64, result: Option<&str>) -> Result {
    let none: Option<String> = None;
    let no_date: Option<NaiveDate> = None;
    let no_id: Option<i64> = None;
    Claim::query()
        .where_(Claim::ID.eq(id))
        .update(
            &state.db,
            &[
                ("held_kind", &none),
                ("held_request_id", &no_id),
                ("held_reference", &none),
                ("held_date", &no_date),
                ("held_by", &no_id),
                ("held_result", &result.map(str::to_owned)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// What happened to a call that may wait for a second person.
async fn settle(
    state: &AppState,
    id: i64,
    answer: PayerAnswer,
    heard: Heard,
) -> std::result::Result<Recorded, TransitionError> {
    match answer {
        PayerAnswer::Pending { request } => {
            Claim::query()
                .where_(Claim::ID.eq(id))
                .update(
                    &state.db,
                    &[
                        ("held_request_id", &Some(request)),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await?;
            Ok(Recorded::Held(Claim::find_or_404(&state.db, id).await?))
        }
        PayerAnswer::Done => {
            // The payer bill's announcement moved the claim already when
            // the listener ran; this finishes it otherwise.
            let claim = follow(state, id, heard).await?;
            if claim.held_kind.is_some() {
                clear(state, id, None).await?;
            }
            Ok(Recorded::Done(Claim::find_or_404(&state.db, id).await?))
        }
    }
}

/// Records the payer's remittance `advice` on claim `id`, as `by` (who needs
/// `claims.manage`). An advice that pays more than is open on a line waits
/// for a second person's approval (`FIN.ACL.16`): the answer is then
/// [`Recorded::Held`] and nothing is recorded; the claim moves when the
/// approval is given.
///
/// # Errors
///
/// `Forbidden` without `claims.manage`; `NotFrom` unless the claim is
/// `submitted` or `resubmitted`; `Guard` (422) for a line that is not on the
/// claim, while another advice waits, and the payer bill's refusals (an
/// advice that is not matched, a reference recorded already for the payer).
pub async fn record_remittance(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    advice: Advice,
) -> std::result::Result<Recorded, TransitionError> {
    if !can_manage(by) {
        return Err(TransitionError::Forbidden);
    }
    let claim = Claim::find_or_404(&state.db, id).await?;
    Claim::machine()
        .find(RECORD_REMITTANCE_PAID)?
        .check_from(claim.status.as_str())?;
    if claim.is_held() {
        return Err(guard(english("clinics.claim.problem.held", &[])));
    }
    let lines = ClaimLine::of_claim(&state.db, id).await?;
    let mut remittance = Remittance::new(&advice.reference, advice.date);
    if let Some(payment) = advice.payment {
        remittance = remittance.payment(payment);
    }
    for entry in &advice.lines {
        let bill_line = lines
            .iter()
            .find(|l| l.id == entry.line)
            .and_then(|l| l.payer_bill_line_id)
            .ok_or_else(|| guard(english("clinics.claim.problem.no_line", &[])))?;
        let mut line = RemittanceLine::paid(bill_line, entry.paid);
        if entry.denied > 0 {
            line = line.denied(
                entry.denied,
                entry.code.as_deref().unwrap_or(""),
                entry.reason.as_deref().unwrap_or(""),
            );
        }
        remittance = remittance.line(line);
    }
    mark(
        state,
        id,
        "remittance",
        Some(&advice.reference),
        advice.date,
        by.id,
    )
    .await?;
    let answer = match payer_bills(state)
        .record_remittance(state, claim.payer_bill(), remittance, by)
        .await
    {
        Ok(answer) => answer,
        Err(error) => {
            clear(state, id, None).await?;
            return Err(error.into());
        }
    };
    settle(
        state,
        id,
        answer,
        Heard {
            reference: Some(advice.reference.clone()),
            ..Heard::default()
        },
    )
    .await
}

/// Records money the payer takes back from claim `id`, as `by` (who needs
/// `claims.manage`), with a second person's approval (`FIN.ACL.16`): the
/// answer is [`Recorded::Held`] and nothing is recorded until it is given.
/// Then the recouped lines are open again as denied with the payer's code,
/// the amount is set off from the payer credit balance first, and the claim
/// is `denied` when nothing is paid on any line, else `partially_paid`.
///
/// # Errors
///
/// `Forbidden` without `claims.manage`; `NotFrom` unless the claim is `paid`,
/// `partially_paid` or `written_off`; `Guard` (422) without the payer's code,
/// for a line that is not on the claim, for an amount at 0 or above what the
/// line has been paid, and while another advice waits.
pub async fn record_recoupment(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    takeback: Takeback,
) -> std::result::Result<Recorded, TransitionError> {
    if !can_manage(by) {
        return Err(TransitionError::Forbidden);
    }
    let claim = Claim::find_or_404(&state.db, id).await?;
    Claim::machine()
        .find(RECORD_RECOUPMENT_DENIED)?
        .check_from(claim.status.as_str())?;
    if claim.is_held() {
        return Err(guard(english("clinics.claim.problem.held", &[])));
    }
    if takeback.code.trim().is_empty() {
        return Err(guard(english("clinics.claim.problem.recoup_code", &[])));
    }
    let lines = ClaimLine::of_claim(&state.db, id).await?;
    let mut recoupment = Recoupment::new(&takeback.code);
    if let Some(reason) = &takeback.reason {
        recoupment = recoupment.reason(reason);
    }
    if let Some(reference) = &takeback.reference {
        recoupment = recoupment.carried_by(reference, takeback.date);
    }
    if takeback.lines.is_empty() {
        return Err(guard(english("clinics.claim.problem.recoup_above", &[])));
    }
    for (line_id, amount) in &takeback.lines {
        let line = lines
            .iter()
            .find(|l| l.id == *line_id)
            .ok_or_else(|| guard(english("clinics.claim.problem.no_line", &[])))?;
        let bill_line = line
            .payer_bill_line_id
            .ok_or_else(|| guard(english("clinics.claim.problem.no_line", &[])))?;
        if *amount <= 0 || *amount > line.paid {
            return Err(guard(english("clinics.claim.problem.recoup_above", &[])));
        }
        recoupment = recoupment.line(bill_line, *amount);
    }
    mark(
        state,
        id,
        "recoupment",
        takeback.reference.as_deref(),
        takeback.date,
        by.id,
    )
    .await?;
    let answer = match payer_bills(state)
        .record_recoupment(state, claim.payer_bill(), recoupment, by)
        .await
    {
        Ok(answer) => answer,
        Err(error) => {
            clear(state, id, None).await?;
            return Err(error.into());
        }
    };
    settle(
        state,
        id,
        answer,
        Heard {
            reference: takeback.reference.clone(),
            code: Some(takeback.code.clone()),
            ..Heard::default()
        },
    )
    .await
}

/// Brings claim `id` to what its payer bill holds: the remittance recorded
/// on it moves a `submitted` or `resubmitted` claim to `paid`,
/// `partially_paid` or `denied`; money taken back moves a `paid`,
/// `partially_paid` or `written_off` claim to `partially_paid` or `denied`.
/// A claim that agrees with its payer bill is returned as it is.
///
/// # Errors
///
/// The database's and the payer bills' errors, and a refusal of the
/// transition other than the claim being in a state that does not take it.
pub async fn follow(
    state: &AppState,
    id: i64,
    heard: Heard,
) -> std::result::Result<Claim, TransitionError> {
    let claim = Claim::find_or_404(&state.db, id).await?;
    let Some(bill) = payer_bills(state).find(&state.db, claim.payer_bill()).await? else {
        return Ok(claim);
    };
    let kind_and_name = match claim.status {
        ClaimStatus::Submitted | ClaimStatus::Resubmitted => match bill.status {
            PayerBillStatus::Paid => Some((AnswerKind::Remittance, RECORD_REMITTANCE_PAID)),
            PayerBillStatus::PartiallyPaid => {
                Some((AnswerKind::Remittance, RECORD_REMITTANCE_PARTIALLY_PAID))
            }
            PayerBillStatus::Denied => Some((AnswerKind::Remittance, RECORD_REMITTANCE_DENIED)),
            _ => None,
        },
        ClaimStatus::Paid | ClaimStatus::PartiallyPaid | ClaimStatus::WrittenOff
            if bill.paid < claim.paid =>
        {
            match bill.status {
                PayerBillStatus::PartiallyPaid => {
                    Some((AnswerKind::Recoupment, RECORD_RECOUPMENT_PARTIALLY_PAID))
                }
                PayerBillStatus::Denied => Some((AnswerKind::Recoupment, RECORD_RECOUPMENT_DENIED)),
                _ => None,
            }
        }
        _ => None,
    };
    let Some((kind, name)) = kind_and_name else {
        return Ok(claim);
    };
    let claim_lines = ClaimLine::of_claim(&state.db, id).await?;
    let bill_lines = payer_bills(state).lines(&state.db, claim.payer_bill()).await?;
    let mut lines = Vec::with_capacity(claim_lines.len());
    for line in &claim_lines {
        let Some(on_bill) = line
            .payer_bill_line_id
            .and_then(|id| bill_lines.iter().find(|l| l.id == id))
        else {
            continue;
        };
        let taken = (line.paid - on_bill.paid).max(0);
        let (code, reason) = if taken > 0 {
            (
                on_bill
                    .recoupment_code
                    .clone()
                    .or_else(|| on_bill.denial_code.clone()),
                on_bill.denial_reason.clone(),
            )
        } else {
            (on_bill.denial_code.clone(), on_bill.denial_reason.clone())
        };
        lines.push(LineMoney {
            line: line.id,
            paid: on_bill.paid,
            denied: on_bill.denied,
            recouped: line.recouped + taken,
            code,
            reason,
        });
    }
    let recouped = match kind {
        AnswerKind::Recoupment => claim.paid - bill.paid,
        AnswerKind::Remittance => 0,
    };
    let input = ClaimInput {
        by: claim.held_by,
        money: Some(MoneyInput {
            kind,
            lines,
            paid: bill.paid,
            denied: bill.denied,
            reference: heard.reference.or_else(|| claim.held_reference.clone()),
            advice_date: Some(claim.held_date.unwrap_or_else(today)),
            overpayment: heard.overpayment,
            recouped,
            code: heard.code,
            reversal: heard.reversal,
            approval_request: claim.held_request_id,
            by: claim.held_by,
        }),
        ..ClaimInput::default()
    };
    match claim
        .transition(name)
        .by_system()
        .input(&input)
        .run(state)
        .await
    {
        Ok(_) | Err(TransitionError::NotFrom(_)) => {}
        Err(other) => return Err(other),
    }
    Ok(Claim::find_or_404(&state.db, id).await?)
}

/// `PayerRemittanceRecorded`: the claim of that payer bill follows it.
///
/// # Errors
///
/// The errors of [`follow`].
pub async fn on_remittance_recorded(state: &AppState, event: &PayerRemittanceRecorded) -> Result {
    let Some(claim) = Claim::of_payer_bill(&state.db, event.payer_bill.0).await? else {
        return Ok(());
    };
    follow(
        state,
        claim.id,
        Heard {
            reference: Some(event.reference.clone()),
            overpayment: event.overpayment,
            ..Heard::default()
        },
    )
    .await
    .map(|_| ())
    .map_err(TransitionError::into_error)
}

/// `PayerRecoupmentRecorded`: the claim of that payer bill follows it, also
/// when a reversed payment caused it (no approval was needed).
///
/// # Errors
///
/// The errors of [`follow`].
pub async fn on_recoupment_recorded(state: &AppState, event: &PayerRecoupmentRecorded) -> Result {
    let Some(claim) = Claim::of_payer_bill(&state.db, event.payer_bill.0).await? else {
        return Ok(());
    };
    follow(
        state,
        claim.id,
        Heard {
            code: Some(event.code.clone()),
            reversal: event.reversal,
            ..Heard::default()
        },
    )
    .await
    .map(|_| ())
    .map_err(TransitionError::into_error)
}

/// `ApprovalDecided` for the payer bill of a claim: when the request that
/// held its advice ended without a yes (rejected, returned, withdrawn,
/// expired, failed), nothing was recorded; the hold is cleared and the way it
/// ended is kept. A yes is followed by the payer bill's own announcement.
///
/// # Errors
///
/// The database's errors.
pub async fn on_approval_decided(state: &AppState, event: &ApprovalDecided) -> Result {
    if event.decision == ApprovalEnd::Approved
        || event.subject.subject_type != PAYER_BILL_SUBJECT_TYPE
    {
        return Ok(());
    }
    let Some(claim) = Claim::of_payer_bill(&state.db, event.subject.subject_id).await? else {
        return Ok(());
    };
    if claim.held_request_id == Some(event.request) {
        clear(state, claim.id, Some(event.decision.as_str())).await?;
    }
    Ok(())
}

/// Listens to the payer bill's announcements: the claim follows its money.
pub fn register(app: &mut Registry) {
    app.listen(
        |event: PayerRemittanceRecorded, state: AppState| async move {
            on_remittance_recorded(&state, &event).await
        },
    );
    app.listen(
        |event: PayerRecoupmentRecorded, state: AppState| async move {
            on_recoupment_recorded(&state, &event).await
        },
    );
    app.listen(|event: ApprovalDecided, state: AppState| async move {
        on_approval_decided(&state, &event).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        text.parse().expect("a date")
    }

    #[test]
    fn an_advice_is_built_with_its_builders() {
        let advice = Advice::new(" ADV-1 ", day("2026-12-10"))
            .payment(5)
            .line(AdviceLine::paid(1, 45_000))
            .line(AdviceLine::paid(2, 0).denied(10_000, " R12 ", "not covered"));
        assert_eq!(advice.reference, "ADV-1");
        assert_eq!(advice.payment, Some(5));
        assert_eq!(advice.lines[1].code.as_deref(), Some("R12"));
        assert_eq!(advice.lines[0].denied, 0);
    }

    #[test]
    fn a_takeback_is_built_with_its_builders() {
        let takeback = Takeback::new(" R07 ", day("2026-12-10"))
            .reason(" taken back ")
            .carried_by("ADV-10")
            .line(3, 40_000);
        assert_eq!(takeback.code, "R07");
        assert_eq!(takeback.reason.as_deref(), Some("taken back"));
        assert_eq!(takeback.reference.as_deref(), Some("ADV-10"));
        assert_eq!(takeback.lines, [(3, 40_000)]);
    }
}
