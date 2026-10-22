//! The pay account pages and actions (wl-hr §4 *Pay account*, §5.7,
//! `HRM.ACL.05`, issues #209 to #211).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /hr/pay-accounts/{id}` | `wl_hr.pay_account` | `pay_accounts.approve`, `employees.manage` or `payroll.approve` |
//! | `POST /hr/pay-accounts/{id}/transitions/{name}` | `wl_hr.pay_account_transitions.*` | the transition's permission (`approve`, `reject`, `stop`) |
//! | `POST /hr/employees/{id}/pay-accounts` | `wl_hr.pay_account_request` | `employees.manage` in the employee's scope |
//! | `POST /hr/pay-accounts/{id}/waive` | `wl_hr.pay_account_waive` | `payroll.approve`; an MD approves |
//! | `POST /hr/pay-accounts/{id}/reached` | `wl_hr.pay_account_reached` | `employees.manage` in the employee's scope |
//! | `POST /hr/portal/pay-accounts` | `wl_hr.portal_pay_account_request` | the employee, signed in |
//! | `POST /hr/portal/pay-accounts/{id}/stop` | `wl_hr.portal_pay_account_stop` | the employee, signed in |
//! | `GET`, `POST /hr/pay-accounts/{id}/stop` | `wl_hr.pay_account_stop` | the signed link of the SMS, no login |
//!
//! The stop link is signed and expires with the cooling period; it works
//! once, because the account is no longer `cooling` after it. The page shows
//! only the last 4 digits of the account.

use std::collections::BTreeMap;
use std::time::Duration;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::signed::ValidSignature;
use renox::state_machine::StateMachine;
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::ApprovalRequest;
use wl_kernel::documents::today;

use crate::employee::Employee;
use crate::pay_account::{
    self, NewPayAccount, PayAccount, PayAccountStatus, REACH_METHODS, ReachedInput,
};
use crate::{permissions, texts};

/// The texts a template reads as `tx['pay_account.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The routes of the pay account pages.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            // The signed link carries its own credential.
            Routes::new()
                .get("/hr/pay-accounts/{id}/stop", stop_page)
                .name("wl_hr.pay_account_stop")
                .post("/hr/pay-accounts/{id}/stop", stop_by_link),
        )
        .merge(
            Routes::new()
                .get("/hr/pay-accounts/{id}", show)
                .name("wl_hr.pay_account")
                .group(
                    "/hr/pay-accounts",
                    "wl_hr.pay_account_transitions.",
                    wl_kernel::machines::routes::<PayAccount>(),
                )
                .post("/hr/employees/{id}/pay-accounts", request)
                .name("wl_hr.pay_account_request")
                .post("/hr/pay-accounts/{id}/waive", waive)
                .name("wl_hr.pay_account_waive")
                .post("/hr/pay-accounts/{id}/reached", reached)
                .name("wl_hr.pay_account_reached")
                .post("/hr/portal/pay-accounts", portal_request)
                .name("wl_hr.portal_pay_account_request")
                .post("/hr/portal/pay-accounts/{id}/stop", portal_stop)
                .name("wl_hr.portal_pay_account_stop")
                .require_auth(),
        )
}

/// The package text `key` (without the `wl_hr.` prefix).
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_hr.{key}"))
}

/// The badge kind of a status.
fn kind(status: PayAccountStatus) -> &'static str {
    match status {
        PayAccountStatus::PendingApproval => "warning",
        PayAccountStatus::Cooling => "info",
        PayAccountStatus::Active => "success",
        PayAccountStatus::Retired | PayAccountStatus::Rejected => "info",
    }
}

/// An account as the pages show it: never the full number.
#[derive(Serialize, Debug, Clone)]
pub struct AccountView {
    /// The primary key.
    pub id: i64,
    /// The employee's name.
    pub employee: String,
    /// `…1234`.
    pub number: String,
    /// The channel in the page's language.
    pub channel: String,
    /// The routing code.
    pub routing_code: String,
    /// The holder's name.
    pub holder: String,
    /// Why the holder's name differs from the employee's.
    pub explanation: String,
    /// The status in the page's language.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
    /// The day the account may be used from.
    pub cooling_ends: String,
    /// What is known of the notice to the worker.
    pub notice: String,
    /// Who reached the worker, how and when.
    pub reached: String,
    /// Why it was rejected or waived.
    pub reason: String,
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

/// The pay account page.
#[derive(Serialize, ViewData)]
#[view(template = "wl-hr/pay_account.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    account: AccountView,
    actions: Vec<ActionView>,
    can_waive: bool,
    waive_pending: bool,
    can_record_reached: bool,
    methods: Vec<(String, String)>,
    method_selected: String,
    today: String,
}

/// The account `id` as a view.
pub(crate) async fn view_of(
    state: &AppState,
    lang: &Lang,
    account: &PayAccount,
) -> Result<AccountView> {
    let employee = match Employee::find(&state.db, account.employee_id).await? {
        Some(employee) => crate::pages::employees::employee_name(state, lang, &employee).await,
        None => format!("#{}", account.employee_id),
    };
    let notice = if account.notice_undelivered_at.is_some() {
        text(lang, "pay_account.notice_undelivered_state")
    } else if account.notice_sent_to.is_some() {
        text(lang, "pay_account.notice_sent")
    } else {
        String::new()
    };
    let reached = match (
        &account.reached_by,
        &account.reached_method,
        account.reached_on,
    ) {
        (Some(by), Some(method), Some(on)) => format!(
            "{by}, {}, {on}",
            text(lang, &format!("pay_account.method.{method}"))
        ),
        _ => String::new(),
    };
    Ok(AccountView {
        id: account.id,
        employee,
        number: account.masked(),
        channel: text(lang, &format!("pay_account.channel.{}", account.channel)),
        routing_code: account.routing_code.clone().unwrap_or_default(),
        holder: account.holder_name.clone(),
        explanation: account.mismatch_explanation.clone().unwrap_or_default(),
        status: text(
            lang,
            &format!("pay_account.status.{}", account.status.as_str()),
        ),
        status_kind: kind(account.status),
        cooling_ends: account
            .cooling_ends_on
            .map(|day| day.to_string())
            .unwrap_or_default(),
        notice,
        reached,
        reason: account
            .reject_reason
            .clone()
            .or_else(|| account.waive_reason.clone())
            .unwrap_or_default(),
    })
}

/// `GET /hr/pay-accounts/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    let sees = [
        permissions::PAY_ACCOUNTS_APPROVE,
        permissions::EMPLOYEES_MANAGE,
        permissions::PAYROLL_APPROVE,
    ]
    .iter()
    .any(|permission| user.has_permission(permission));
    if !sees {
        return Err(Error::Forbidden);
    }
    let account = PayAccount::find_or_404(&state.db, id).await?;
    let employee = Employee::find_or_404(&state.db, account.employee_id).await?;
    let mut actions = Vec::new();
    for action in account.available(&user, &state.db).await? {
        actions.push(ActionView {
            name: action.name.clone(),
            label: action.label.clone(),
            tone: action.tone.clone().unwrap_or_default(),
            needs_reason: action.needs_reason,
        });
    }
    let cooling = account.status == PayAccountStatus::Cooling;
    let waive_pending = ApprovalRequest::pending_about(&state.db, &account.subject())
        .await?
        .iter()
        .any(|request| request.transition == pay_account::WAIVE_COOLING);
    let manages = crate::exits::holds_manage(&state, user.id, &employee).await?;
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        account: view_of(&state, &lang, &account).await?,
        actions,
        can_waive: cooling && !waive_pending && user.has_permission(permissions::PAYROLL_APPROVE),
        waive_pending,
        can_record_reached: cooling && manages,
        methods: REACH_METHODS
            .iter()
            .map(|m| {
                (
                    (*m).to_owned(),
                    text(&lang, &format!("pay_account.method.{m}")),
                )
            })
            .collect(),
        method_selected: REACH_METHODS[0].to_owned(),
        today: today().to_string(),
    }))
}

/// The form that asks for a new pay account.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct RequestForm {
    /// `bank_account` or `salary_card`.
    #[validate(max = 20)]
    #[serde(default)]
    pub channel: String,
    /// The routing code.
    #[validate(max = 20)]
    #[serde(default)]
    pub routing_code: String,
    /// The IBAN or card number.
    #[validate(max = 60)]
    #[serde(default)]
    pub account_number: String,
    /// The holder's name.
    #[validate(max = 200)]
    #[serde(default)]
    pub holder_name: String,
    /// Why the name differs from the employee's.
    #[validate(max = 1000)]
    #[serde(default)]
    pub mismatch_explanation: String,
}

impl RequestForm {
    fn into_input(self) -> NewPayAccount {
        NewPayAccount {
            channel: self.channel.trim().to_owned(),
            routing_code: self.routing_code,
            account_number: self.account_number,
            holder_name: self.holder_name,
            mismatch_explanation: Some(self.mismatch_explanation)
                .filter(|text| !text.trim().is_empty()),
            mismatch_document: None,
        }
    }
}

/// `POST /hr/employees/{id}/pay-accounts`: HR asks for a new account.
async fn request(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back: renox::Back,
    Path(id): Path<i64>,
    Valid(form): Valid<RequestForm>,
) -> Result<Response> {
    pay_account::request(&state, &user, id, form.into_input()).await?;
    Ok((Toast::success(text(&lang, "pay_account.requested")), back).into_response())
}

/// `POST /hr/portal/pay-accounts`: the employee asks for a new account.
async fn portal_request(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back: renox::Back,
    Valid(form): Valid<RequestForm>,
) -> Result<Response> {
    pay_account::request_as_employee(&state, &user, form.into_input()).await?;
    Ok((Toast::success(text(&lang, "pay_account.requested")), back).into_response())
}

/// A reason.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct ReasonForm {
    /// Why.
    #[validate(max = 1000)]
    #[serde(default)]
    pub reason: String,
}

/// `POST /hr/pay-accounts/{id}/waive`: the payroll user asks to waive the
/// cooling; an MD decides.
async fn waive(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back: renox::Back,
    Path(id): Path<i64>,
    Valid(form): Valid<ReasonForm>,
) -> Result<Response> {
    pay_account::waive_cooling(&state, &user, id, &form.reason).await?;
    Ok((Toast::info(text(&lang, "pay_account.waive_sent")), back).into_response())
}

/// The form that records the worker reached.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct ReachedForm {
    /// Who reached the worker.
    #[validate(max = 200)]
    #[serde(default)]
    pub by: String,
    /// `call`, `visit` or `in_person`.
    #[validate(max = 20)]
    #[serde(default)]
    pub method: String,
    /// The day, today when blank.
    #[validate(max = 20)]
    #[serde(default)]
    pub on: String,
}

/// `POST /hr/pay-accounts/{id}/reached`: HR records that the worker was
/// reached another way.
async fn reached(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back: renox::Back,
    Path(id): Path<i64>,
    Valid(form): Valid<ReachedForm>,
) -> Result<Response> {
    let on = if form.on.trim().is_empty() {
        today()
    } else {
        form.on
            .trim()
            .parse::<NaiveDate>()
            .map_err(|_| Error::BadRequest("on".to_owned()))?
    };
    let input = ReachedInput {
        by: form.by,
        method: form.method,
        on: Some(on),
    };
    pay_account::record_worker_reached(&state, &user, id, input).await?;
    Ok((
        Toast::success(text(&lang, "pay_account.reached_recorded")),
        back,
    )
        .into_response())
}

/// `POST /hr/portal/pay-accounts/{id}/stop`: the employee stops the change.
async fn portal_stop(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back: renox::Back,
    Path(id): Path<i64>,
) -> Result<Response> {
    pay_account::stop_in_portal(&state, &user, id).await?;
    Ok((Toast::success(text(&lang, "pay_account.stopped")), back).into_response())
}

/// The page of the signed link.
#[derive(Serialize, ViewData)]
#[view(template = "wl-hr/pay_account_stop.html")]
pub(crate) struct StopPage {
    tx: Texts,
    /// `ask`, `done` or `closed`.
    step: &'static str,
    number: String,
    date: String,
    action: String,
}

/// The signed address the stop form posts to, valid until the cooling ends.
fn action_of(state: &AppState, account: &PayAccount) -> Result<String> {
    let days = account
        .cooling_ends_on
        .map_or(1, |end| (end - today()).num_days().max(1));
    let seconds = days.unsigned_abs() * 24 * 3600;
    state.signed_url(
        "wl_hr.pay_account_stop",
        &[&account.id],
        Duration::from_secs(seconds),
    )
}

fn stop_view(
    state: &AppState,
    lang: &Lang,
    account: &PayAccount,
    step: &'static str,
) -> Result<View> {
    Ok(View::typed(StopPage {
        tx: texts::all(lang),
        step,
        number: account.masked(),
        date: account
            .cooling_ends_on
            .map(|day| day.to_string())
            .unwrap_or_default(),
        action: action_of(state, account)?,
    }))
}

/// `GET /hr/pay-accounts/{id}/stop`: the worker opens the link of the SMS.
async fn stop_page(
    _signature: ValidSignature,
    State(state): State<AppState>,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    let account = PayAccount::find_or_404(&state.db, id).await?;
    let step = if account.status == PayAccountStatus::Cooling {
        "ask"
    } else {
        "closed"
    };
    stop_view(&state, &lang, &account, step)
}

/// `POST /hr/pay-accounts/{id}/stop`: the worker stops the change, once.
async fn stop_by_link(
    _signature: ValidSignature,
    State(state): State<AppState>,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    let account = PayAccount::find_or_404(&state.db, id).await?;
    if account.status != PayAccountStatus::Cooling {
        return stop_view(&state, &lang, &account, "closed");
    }
    let account = pay_account::stop_by_worker(&state, id).await?;
    stop_view(&state, &lang, &account, "done")
}
