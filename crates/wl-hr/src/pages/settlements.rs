//! The final settlement pages and actions (wl-hr §3 *Final settlement*, §4,
//! §5.4, issues #212 to #214).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /hr/settlements/{id}` | `wl_hr.settlement` | `settlements.prepare`, `.approve` or `.pay` |
//! | `POST /hr/settlements/{id}/transitions/{name}` | `wl_hr.settlement_transitions.*` | the transition's permission (`approve`, `cancel`) |
//! | `POST /hr/settlements/{id}/amounts` | `wl_hr.settlement_amounts` | `settlements.prepare`; `draft` or `disputed` |
//! | `POST /hr/settlements/{id}/payments` | `wl_hr.settlement_payment` | `settlements.pay`; `approved` or `disputed` |
//! | `POST /hr/settlements/{id}/pay` | `wl_hr.settlement_pay` | `settlements.pay`; `approved` |
//! | `POST /hr/settlements/{id}/dispute` | `wl_hr.settlement_dispute` | `settlements.prepare`; `approved` or `paid` |
//! | `POST /hr/settlements/{id}/resolve` | `wl_hr.settlement_resolve` | `settlements.approve`; `disputed` |
//! | `POST /hr/settlements/{id}/acknowledgement` | `wl_hr.settlement_acknowledgement` | `settlements.pay` or `.prepare`; `paid` |
//!
//! Amounts are typed in dirhams (`12000.50`).

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;

use crate::config::config_of;
use crate::employee::Employee;
use crate::pages::payroll::{aed, parse_aed};
use crate::settlement::{FinalSettlement, PAYEE_ESTATE, SettlementStatus};
use crate::settlement_amounts::{AmountsInput, set_amounts};
use crate::settlement_close::acknowledge;
use crate::settlement_dispute::{
    DisputeInput, REVISED, ResolveInput, SOURCES, UPHELD, dispute_settlement, resolve_dispute,
};
use crate::settlement_pay::pay_settlement;
use crate::settlement_payment::{METHODS, PaymentInput, SettlementPayment, paid_total};
use crate::{permissions, texts};

/// The texts a template reads as `tx['settlement.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The routes of the settlement pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/hr/settlements/{id}", show)
        .name("wl_hr.settlement")
        .group(
            "/hr/settlements",
            "wl_hr.settlement_transitions.",
            wl_kernel::machines::routes::<FinalSettlement>(),
        )
        .post("/hr/settlements/{id}/amounts", amounts)
        .name("wl_hr.settlement_amounts")
        .post("/hr/settlements/{id}/payments", payment)
        .name("wl_hr.settlement_payment")
        .post("/hr/settlements/{id}/pay", pay)
        .name("wl_hr.settlement_pay")
        .post("/hr/settlements/{id}/dispute", dispute)
        .name("wl_hr.settlement_dispute")
        .post("/hr/settlements/{id}/resolve", resolve)
        .name("wl_hr.settlement_resolve")
        .post("/hr/settlements/{id}/acknowledgement", acknowledgement)
        .name("wl_hr.settlement_acknowledgement")
        .require_auth()
}

/// The package text `key` (without the `wl_hr.` prefix).
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_hr.{key}"))
}

/// The badge kind of a status.
fn kind(status: SettlementStatus) -> &'static str {
    match status {
        SettlementStatus::Draft | SettlementStatus::Cancelled => "info",
        SettlementStatus::Approved | SettlementStatus::Paid | SettlementStatus::Closed => "success",
        SettlementStatus::Disputed => "warning",
    }
}

/// A settlement in a list: on the employee's page.
#[derive(Serialize, Debug, Clone)]
pub struct RowView {
    /// The primary key.
    pub id: i64,
    /// The reason in words.
    pub reason: String,
    /// The status in the page's language.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
    /// The net, dirhams.
    pub net: String,
}

/// The row of `settlement`.
pub(crate) fn row_of(lang: &Lang, settlement: &FinalSettlement) -> RowView {
    RowView {
        id: settlement.id,
        reason: reason_text(&settlement.reason),
        status: text(
            lang,
            &format!("settlement.status.{}", settlement.status.as_str()),
        ),
        status_kind: kind(settlement.status),
        net: aed(settlement.net),
    }
}

/// A settlement as the page shows it, amounts in dirhams.
#[derive(Serialize, Debug, Clone)]
pub struct SettlementView {
    /// The primary key.
    pub id: i64,
    /// The employee's name.
    pub employee: String,
    /// The employee's id.
    pub employee_id: i64,
    /// The status in the page's language.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
    /// The reason.
    pub reason: String,
    /// `employee` or `estate`, in words.
    pub payee: String,
    /// Whether the payee is the estate.
    pub estate: bool,
    /// The estate's representative.
    pub representative: String,
    /// The document giving the right.
    pub right_document: String,
    /// The last working day.
    pub last_working_day: String,
    /// The legal deadline.
    pub deadline: String,
    /// Unpaid salary.
    pub unpaid_salary: String,
    /// Leave encashment.
    pub leave_encashment: String,
    /// Gratuity.
    pub gratuity: String,
    /// Notice pay.
    pub notice_pay: String,
    /// Other dues.
    pub other_dues: String,
    /// The dues.
    pub dues: String,
    /// Overpayment recoveries (supplementary runs).
    pub overpayment_recoveries: String,
    /// Advances and loans.
    pub advance_recoveries: String,
    /// Fines.
    pub fine_recoveries: String,
    /// Court debts.
    pub court_debt_recoveries: String,
    /// Damage.
    pub damage_recoveries: String,
    /// All recoveries.
    pub recoveries: String,
    /// The recoveries offset within the caps.
    pub offset: String,
    /// The net.
    pub net: String,
    /// What the employee owes.
    pub owed: String,
    /// What was paid.
    pub paid: String,
    /// What of the net is still unpaid.
    pub unpaid: String,
    /// The version of the statement.
    pub statement_version: i64,
    /// The source of the dispute in words.
    pub dispute_source: String,
    /// The reference of the dispute.
    pub dispute_reference: String,
    /// The undisputed amount.
    pub undisputed: String,
    /// The hearing date.
    pub hearing_on: String,
    /// The reply date.
    pub reply_due_on: String,
    /// The outcome in words.
    pub dispute_outcome: String,
    /// The decision document.
    pub resolution_document: String,
    /// The labour-case ticket.
    pub ticket: String,
    /// When the acknowledgement was signed.
    pub acknowledgement: String,
    /// When it closes without a dispute.
    pub closes_on: String,
}

/// One payment as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub struct PaymentView {
    /// The day.
    pub paid_on: String,
    /// The amount, dirhams.
    pub amount: String,
    /// The method in words.
    pub method: String,
    /// The bank's reference.
    pub reference: String,
}

/// One action the user may run.
#[derive(Serialize, Debug, Clone)]
pub struct ActionView {
    /// The transition's name.
    pub name: String,
    /// Its label.
    pub label: String,
    /// Its tone.
    pub tone: String,
    /// Whether it asks for a reason.
    pub needs_reason: bool,
}

/// The settlement page.
#[derive(Serialize, ViewData)]
#[view(template = "wl-hr/settlement.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    settlement: SettlementView,
    payments: Vec<PaymentView>,
    actions: Vec<ActionView>,
    can_edit_amounts: bool,
    can_record_payment: bool,
    can_pay: bool,
    can_dispute: bool,
    can_resolve: bool,
    can_acknowledge: bool,
    methods: Vec<(String, String)>,
    method_selected: String,
    sources: Vec<(String, String)>,
    source_selected: String,
    outcomes: Vec<(String, String)>,
    outcome_selected: String,
    today: String,
}

fn day(date: Option<NaiveDate>) -> String {
    date.map(|d| d.to_string()).unwrap_or_default()
}

/// The view of `settlement`.
async fn view_of(
    state: &AppState,
    lang: &Lang,
    settlement: &FinalSettlement,
) -> Result<SettlementView> {
    let name = match Employee::find(&state.db, settlement.employee_id).await? {
        Some(employee) => crate::pages::employees::employee_name(state, lang, &employee).await,
        None => format!("#{}", settlement.employee_id),
    };
    let paid = paid_total(&state.db, settlement.id).await?;
    let config = config_of(state);
    Ok(SettlementView {
        id: settlement.id,
        employee: name,
        employee_id: settlement.employee_id,
        status: text(
            lang,
            &format!("settlement.status.{}", settlement.status.as_str()),
        ),
        status_kind: kind(settlement.status),
        reason: reason_text(&settlement.reason),
        payee: text(lang, &format!("settlement.payee.{}", settlement.payee)),
        estate: settlement.payee == PAYEE_ESTATE,
        representative: settlement.representative.clone().unwrap_or_default(),
        right_document: settlement.right_document.clone().unwrap_or_default(),
        last_working_day: day(settlement.last_working_day),
        deadline: day(settlement.deadline(config.settlement_deadline_days)),
        unpaid_salary: aed(settlement.unpaid_salary),
        leave_encashment: aed(settlement.leave_encashment),
        gratuity: aed(settlement.gratuity),
        notice_pay: aed(settlement.notice_pay),
        other_dues: aed(settlement.other_dues),
        dues: aed(settlement.dues()),
        overpayment_recoveries: aed(settlement.recoveries),
        advance_recoveries: aed(settlement.advance_recoveries),
        fine_recoveries: aed(settlement.fine_recoveries),
        court_debt_recoveries: aed(settlement.court_debt_recoveries),
        damage_recoveries: aed(settlement.damage_recoveries),
        recoveries: aed(settlement.recoveries_total()),
        offset: aed(settlement.recoveries_offset),
        net: aed(settlement.net),
        owed: aed(settlement.owed_by_employee),
        paid: aed(paid),
        unpaid: aed((settlement.net - paid).max(0)),
        statement_version: settlement.statement_version,
        dispute_source: settlement
            .dispute_source
            .as_deref()
            .map(|s| text(lang, &format!("settlement.source.{s}")))
            .unwrap_or_default(),
        dispute_reference: settlement.dispute_reference.clone().unwrap_or_default(),
        undisputed: if settlement.dispute_source.is_some() {
            aed(settlement.undisputed_amount)
        } else {
            String::new()
        },
        hearing_on: day(settlement.hearing_on),
        reply_due_on: day(settlement.reply_due_on),
        dispute_outcome: settlement
            .dispute_outcome
            .as_deref()
            .map(|o| text(lang, &format!("settlement.outcome.{o}")))
            .unwrap_or_default(),
        resolution_document: settlement.resolution_document.clone().unwrap_or_default(),
        ticket: settlement
            .dispute_ticket
            .map(|t| format!("#{t}"))
            .unwrap_or_default(),
        acknowledgement: day(settlement.acknowledgement_signed_on),
        closes_on: day(settlement.closes_on),
    })
}

/// `GET /hr/settlements/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    let prepares = user.has_permission(permissions::SETTLEMENTS_PREPARE);
    let approves = user.has_permission(permissions::SETTLEMENTS_APPROVE);
    let pays = user.has_permission(permissions::SETTLEMENTS_PAY);
    if !(prepares || approves || pays) {
        return Err(Error::Forbidden);
    }
    let settlement = FinalSettlement::find_or_404(&state.db, id).await?;
    let status = settlement.status;
    let mut actions = Vec::new();
    for action in settlement.available(&user, &state.db).await? {
        // The others have a form of their own.
        if ![crate::settlement::APPROVE, crate::settlement::CANCEL].contains(&action.name.as_str())
        {
            continue;
        }
        actions.push(ActionView {
            name: action.name.clone(),
            label: action.label.clone(),
            tone: action.tone.clone().unwrap_or_default(),
            needs_reason: action.needs_reason,
        });
    }
    let payments = SettlementPayment::of_settlement(&state.db, id)
        .await?
        .into_iter()
        .map(|p| PaymentView {
            paid_on: p.paid_on.to_string(),
            amount: aed(p.amount),
            method: text(&lang, &format!("settlement.method.{}", p.method)),
            reference: p.reference,
        })
        .collect();
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        settlement: view_of(&state, &lang, &settlement).await?,
        payments,
        actions,
        can_edit_amounts: prepares
            && matches!(status, SettlementStatus::Draft | SettlementStatus::Disputed),
        can_record_payment: pays
            && matches!(
                status,
                SettlementStatus::Approved | SettlementStatus::Disputed
            ),
        can_pay: pays && status == SettlementStatus::Approved,
        can_dispute: prepares
            && matches!(status, SettlementStatus::Approved | SettlementStatus::Paid),
        can_resolve: approves && status == SettlementStatus::Disputed,
        can_acknowledge: (pays || prepares)
            && status == SettlementStatus::Paid
            && settlement.payee != PAYEE_ESTATE,
        methods: METHODS
            .iter()
            .map(|m| {
                (
                    (*m).to_owned(),
                    text(&lang, &format!("settlement.method.{m}")),
                )
            })
            .collect(),
        method_selected: METHODS[0].to_owned(),
        sources: SOURCES
            .iter()
            .map(|s| {
                (
                    (*s).to_owned(),
                    text(&lang, &format!("settlement.source.{s}")),
                )
            })
            .collect(),
        source_selected: SOURCES[0].to_owned(),
        outcomes: [UPHELD, REVISED]
            .iter()
            .map(|o| {
                (
                    (*o).to_owned(),
                    text(&lang, &format!("settlement.outcome.{o}")),
                )
            })
            .collect(),
        outcome_selected: UPHELD.to_owned(),
        today: today().to_string(),
    }))
}

/// Dirhams typed in a form, 0 when blank.
fn money(field: &str, value: &str) -> Result<i64> {
    if value.trim().is_empty() {
        return Ok(0);
    }
    parse_aed(value).ok_or_else(|| Error::BadRequest(field.to_owned()))
}

/// A date typed in a form, `None` when blank.
fn date(field: &str, value: &str) -> Result<Option<NaiveDate>> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    value
        .trim()
        .parse()
        .map(Some)
        .map_err(|_| Error::BadRequest(field.to_owned()))
}

fn some(value: &str) -> Option<String> {
    Some(value.trim().to_owned()).filter(|v| !v.is_empty())
}

/// The form that enters the amounts.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct AmountsForm {
    /// The last working day.
    #[validate(max = 20)]
    #[serde(default)]
    pub last_working_day: String,
    /// Unpaid salary, dirhams.
    #[validate(max = 20)]
    #[serde(default)]
    pub unpaid_salary: String,
    /// Leave encashment, dirhams.
    #[validate(max = 20)]
    #[serde(default)]
    pub leave_encashment: String,
    /// Gratuity, dirhams.
    #[validate(max = 20)]
    #[serde(default)]
    pub gratuity: String,
    /// Notice pay, dirhams.
    #[validate(max = 20)]
    #[serde(default)]
    pub notice_pay: String,
    /// Other dues, dirhams.
    #[validate(max = 20)]
    #[serde(default)]
    pub other_dues: String,
    /// Advances and loans, dirhams.
    #[validate(max = 20)]
    #[serde(default)]
    pub advance_recoveries: String,
    /// Fines, dirhams.
    #[validate(max = 20)]
    #[serde(default)]
    pub fine_recoveries: String,
    /// Court debts, dirhams.
    #[validate(max = 20)]
    #[serde(default)]
    pub court_debt_recoveries: String,
    /// Damage, dirhams.
    #[validate(max = 20)]
    #[serde(default)]
    pub damage_recoveries: String,
    /// The estate's representative.
    #[validate(max = 200)]
    #[serde(default)]
    pub representative: String,
    /// The document giving the right.
    #[validate(max = 200)]
    #[serde(default)]
    pub right_document: String,
}

/// `POST /hr/settlements/{id}/amounts`: the preparer enters the amounts (the
/// revised ones while `disputed`).
async fn amounts(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back: renox::Back,
    Path(id): Path<i64>,
    Valid(form): Valid<AmountsForm>,
) -> Result<Response> {
    let input = AmountsInput {
        last_working_day: date("last_working_day", &form.last_working_day)?,
        unpaid_salary: money("unpaid_salary", &form.unpaid_salary)?,
        leave_encashment: money("leave_encashment", &form.leave_encashment)?,
        gratuity: money("gratuity", &form.gratuity)?,
        notice_pay: money("notice_pay", &form.notice_pay)?,
        other_dues: money("other_dues", &form.other_dues)?,
        advance_recoveries: money("advance_recoveries", &form.advance_recoveries)?,
        fine_recoveries: money("fine_recoveries", &form.fine_recoveries)?,
        court_debt_recoveries: money("court_debt_recoveries", &form.court_debt_recoveries)?,
        damage_recoveries: money("damage_recoveries", &form.damage_recoveries)?,
        representative: some(&form.representative),
        right_document: some(&form.right_document),
    };
    set_amounts(&state, &user, id, input).await?;
    Ok((
        Toast::success(text(&lang, "settlement.amounts_saved")),
        back,
    )
        .into_response())
}

/// The form of a payment.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct PaymentForm {
    /// The amount, dirhams.
    #[validate(max = 20)]
    #[serde(default)]
    pub amount: String,
    /// `wps` or `transfer`.
    #[validate(max = 20)]
    #[serde(default)]
    pub method: String,
    /// The bank's reference.
    #[validate(max = 200)]
    #[serde(default)]
    pub reference: String,
}

impl PaymentForm {
    fn into_input(self) -> Result<PaymentInput> {
        Ok(PaymentInput {
            amount: money("amount", &self.amount)?,
            method: self.method,
            reference: self.reference,
        })
    }
}

/// `POST /hr/settlements/{id}/payments`: a payment against the settlement,
/// e.g. the undisputed amount while it is `disputed`.
async fn payment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back: renox::Back,
    Path(id): Path<i64>,
    Valid(form): Valid<PaymentForm>,
) -> Result<Response> {
    crate::settlement_payment::record(&state, &user, id, form.into_input()?).await?;
    Ok((
        Toast::success(text(&lang, "settlement.payment_recorded")),
        back,
    )
        .into_response())
}

/// `POST /hr/settlements/{id}/pay`: the rest of the net, then `pay`.
async fn pay(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back: renox::Back,
    Path(id): Path<i64>,
    Valid(form): Valid<PaymentForm>,
) -> Result<Response> {
    pay_settlement(&state, &user, id, form.into_input()?).await?;
    Ok((Toast::success(text(&lang, "settlement.paid")), back).into_response())
}

/// The form of a dispute.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct DisputeForm {
    /// `worker`, `mohre_complaint` or `court`.
    #[validate(max = 30)]
    #[serde(default)]
    pub source: String,
    /// The complaint or case reference.
    #[validate(max = 200)]
    #[serde(default)]
    pub reference: String,
    /// The undisputed amount, dirhams.
    #[validate(max = 20)]
    #[serde(default)]
    pub undisputed_amount: String,
    /// The hearing date.
    #[validate(max = 20)]
    #[serde(default)]
    pub hearing_on: String,
    /// The date a reply is due.
    #[validate(max = 20)]
    #[serde(default)]
    pub reply_due_on: String,
}

/// `POST /hr/settlements/{id}/dispute`.
async fn dispute(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back: renox::Back,
    Path(id): Path<i64>,
    Valid(form): Valid<DisputeForm>,
) -> Result<Response> {
    let undisputed_amount = if form.undisputed_amount.trim().is_empty() {
        None
    } else {
        Some(money("undisputed_amount", &form.undisputed_amount)?)
    };
    let input = DisputeInput {
        source: form.source,
        reference: form.reference,
        undisputed_amount,
        hearing_on: date("hearing_on", &form.hearing_on)?,
        reply_due_on: date("reply_due_on", &form.reply_due_on)?,
    };
    dispute_settlement(&state, &user, id, input).await?;
    Ok((Toast::info(text(&lang, "settlement.disputed")), back).into_response())
}

/// The form of a resolution.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct ResolveForm {
    /// `upheld` or `revised`.
    #[validate(max = 20)]
    #[serde(default)]
    pub outcome: String,
    /// The decision, agreement or judgment attached.
    #[validate(max = 500)]
    #[serde(default)]
    pub document: String,
}

/// `POST /hr/settlements/{id}/resolve`.
async fn resolve(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back: renox::Back,
    Path(id): Path<i64>,
    Valid(form): Valid<ResolveForm>,
) -> Result<Response> {
    let input = ResolveInput {
        outcome: form.outcome,
        document: form.document,
    };
    resolve_dispute(&state, &user, id, input).await?;
    Ok((Toast::success(text(&lang, "settlement.resolved")), back).into_response())
}

/// The form of the signed acknowledgement.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct AcknowledgementForm {
    /// The day it was signed, today when blank.
    #[validate(max = 20)]
    #[serde(default)]
    pub signed_on: String,
}

/// `POST /hr/settlements/{id}/acknowledgement`: HR records the signed
/// acknowledgement; the settlement closes.
async fn acknowledgement(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back: renox::Back,
    Path(id): Path<i64>,
    Valid(form): Valid<AcknowledgementForm>,
) -> Result<Response> {
    if !(user.has_permission(permissions::SETTLEMENTS_PAY)
        || user.has_permission(permissions::SETTLEMENTS_PREPARE))
    {
        return Err(Error::Forbidden);
    }
    acknowledge(&state, id, date("signed_on", &form.signed_on)?).await?;
    Ok((Toast::success(text(&lang, "settlement.acknowledged")), back).into_response())
}

/// The reason of an exit in words: `end_of_contract` as *End of contract*.
fn reason_text(reason: &str) -> String {
    let spaced = reason.replace('_', " ");
    let mut chars = spaced.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}
