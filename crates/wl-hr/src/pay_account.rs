//! Pay accounts and their change (wl-hr §3 *Pay account*, §4 *Pay account*,
//! §5.7, `HRM.ACL.05`, issues #209 to #211). A change of pay account is the
//! classic payroll fraud, so a new account is never used at once.
//!
//! ```text
//! [*] --request--> pending_approval --approve--> cooling --activate--> active --retire--> retired
//!                  pending_approval --reject---> rejected
//!                  cooling --stop | stop_by_worker--> rejected
//!                  cooling --waive_cooling | waive_cooling_approved--> active
//! ```
//!
//! - **`request`** is not a transition (the account does not exist yet): the
//!   services [`request`] (HR, `employees.manage`) and [`request_as_employee`]
//!   (the employee, in the portal) check the format of the IBAN or card and of
//!   the routing code, that the holder name matches the employee's name or the
//!   mismatch is explained, and that the employee has no other account
//!   `pending_approval` or `cooling`; they take the worker's mobile number
//!   **as it is on record now** ([`PayAccount::notice_to`]), so a number
//!   changed in the same request is never used, and tell the approvers.
//! - **`approve`** (`pay_accounts.approve`, `HRM.ACL.05`): not the requester.
//!   Cooling ends after N days (default 5, [`crate::HrConfig::cooling_days`],
//!   kept on the row from the request). The notice to the worker follows after
//!   the commit ([`crate::pay_account_events`]); with no number on record the
//!   notice counts as undelivered at once.
//! - **`activate`** (system, [`activate_due`], daily at 00:00 Asia/Dubai): the
//!   cooling has ended and the notice was not given up, or HR recorded that
//!   the worker was reached ([`record_worker_reached`]). The previous active
//!   account is `retired` and `PayAccountChanged` follows.
//! - **`waive_cooling`** (`payroll.approve`) needs an MD: the kernel's
//!   interceptor turns it into an approval request with one step for the MD
//!   role (policy `hr.waive_cooling`); the twin `waive_cooling_approved` runs at
//!   the approval. The user who asks and the requester of the change never
//!   approve ([`register`] names the second to the kernel); with no other MD
//!   the request waits. The reason is part of the payload, so the twin sees it.
//! - **`stop`** (`employees.manage`) and **`stop_by_worker`** (system, after
//!   the worker's own signed link or the portal): the old account stays, a
//!   security ticket is opened and the MD told.
//!
//! Decisions on what the spec leaves open: a salary card number has
//! [`CARD_LENGTH`] digits (the exchange house's `SalaryFileFormat` is not built);
//! the routing code is 9 digits and is looked up in
//! [`crate::HrConfig::routing_codes`] when that list is not empty. The account
//! number is stored in the row and shown only as its last 4 digits.

use renox::chrono::{Duration, NaiveDate};
use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{self, ApprovalPolicy, PolicyDefault};
use wl_kernel::documents::today;
use wl_kernel::workforce::PAY_ACCOUNT_SUBJECT_TYPE;

use crate::config::config_of;
use crate::employee::Employee;
use crate::permissions;
use crate::texts::english;

/// `approve` (`pay_accounts.approve`).
pub const APPROVE: &str = "approve";
/// `reject` (`pay_accounts.approve`).
pub const REJECT: &str = "reject";
/// `stop` by HR (`employees.manage`).
pub const STOP: &str = "stop";
/// `stop` by the worker, run by the system for the signed link or the portal.
pub const STOP_BY_WORKER: &str = "stop_by_worker";
/// `activate` (system).
pub const ACTIVATE: &str = "activate";
/// `waive_cooling` (`payroll.approve`, then an MD).
pub const WAIVE_COOLING: &str = "waive_cooling";
/// The twin of [`WAIVE_COOLING`] the kernel runs at the MD's approval.
pub const WAIVE_COOLING_APPROVED: &str = "waive_cooling_approved";
/// `retire` (system).
pub const RETIRE: &str = "retire";

/// The approval policy's action key of [`WAIVE_COOLING`].
pub const WAIVE_ACTION: &str = "hr.waive_cooling";

/// The channel of an IBAN.
pub const BANK_ACCOUNT: &str = "bank_account";
/// The channel of an exchange house's salary card.
pub const SALARY_CARD: &str = "salary_card";

/// The digits of a salary card number (Default, spec silent on the detail).
pub const CARD_LENGTH: usize = 16;

/// The digits of a routing code.
pub const ROUTING_LENGTH: usize = 9;

/// The ways HR can have reached the worker (`worker reached`).
pub const REACH_METHODS: [&str; 3] = ["call", "visit", "in_person"];

/// The daily job, and the command, that run [`activate_due`].
pub const TASK: &str = "hr:pay-accounts";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "00:00";

/// Where a pay account stands (§4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PayAccountStatus {
    /// Requested by HR or by the employee; waits for another user.
    #[default]
    PendingApproval,
    /// Approved and the worker told; not used by payroll yet.
    Cooling,
    /// Used by the next payroll run submitted.
    Active,
    /// Replaced by a newer account.
    Retired,
    /// Not approved, or stopped.
    Rejected,
}

/// Where an employee's salary is paid.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_hr_pay_accounts",
    state_machine,
    index(employee_id, status)
)]
pub struct PayAccount {
    /// The primary key.
    pub id: i64,
    /// The employee.
    #[model(references = "wl_hr_employees")]
    pub employee_id: i64,
    /// [`BANK_ACCOUNT`] or [`SALARY_CARD`].
    pub channel: String,
    /// The bank or agent routing code.
    pub routing_code: Option<String>,
    /// The IBAN or card number (`SEN`).
    pub account_number: String,
    /// Its last 4 characters: the only part shown or sent.
    pub last4: String,
    /// The name of the holder.
    pub holder_name: String,
    /// Why the holder name differs from the employee's name.
    pub mismatch_explanation: Option<String>,
    /// A supporting document (wl-documents), when attached.
    pub mismatch_document: Option<i64>,
    /// The user who asked: the employee's own user in the portal.
    pub requested_by: Option<i64>,
    /// Whether the employee asked in the portal.
    pub requested_by_employee: bool,
    /// The user who approved it.
    pub approved_by: Option<i64>,
    /// The day it was approved.
    pub approved_on: Option<NaiveDate>,
    /// Why it was rejected.
    pub reject_reason: Option<String>,
    /// The worker's mobile number on record before the request: the notice
    /// goes only to it.
    pub notice_to: Option<String>,
    /// The worker's e-mail address on record before the request.
    pub notice_email: Option<String>,
    /// The number the notice was sent to.
    pub notice_sent_to: Option<String>,
    /// The notice's message in wl-notify.
    pub notice_message: Option<i64>,
    /// When the notice was found undelivered (`MessageGaveUp`, or no number
    /// on record at `approve`).
    pub notice_undelivered_at: Option<DateTime>,
    /// Who reached the worker another way (*worker reached*).
    pub reached_by: Option<String>,
    /// How: call, visit or in person.
    pub reached_method: Option<String>,
    /// When.
    pub reached_on: Option<NaiveDate>,
    /// The user who recorded it.
    pub reached_recorded_by: Option<i64>,
    /// The cooling period in days, as set when the request was made.
    pub cooling_days: i64,
    /// The day the account may be used from (00:00 Asia/Dubai).
    pub cooling_ends_on: Option<NaiveDate>,
    /// The day the bank reported the account closed.
    pub closed_reported_on: Option<NaiveDate>,
    /// Why the cooling was waived.
    pub waive_reason: Option<String>,
    /// The user who stopped it.
    pub stopped_by: Option<i64>,
    /// Whether the worker stopped it (link or portal).
    pub stopped_by_worker: bool,
    /// Where it stands; changed only by the machine.
    pub status: PayAccountStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PayAccount {
    /// The account as a [`Subject`](wl_kernel::Subject).
    pub fn subject(&self) -> wl_kernel::Subject {
        wl_kernel::Subject::new(PAY_ACCOUNT_SUBJECT_TYPE, self.id)
    }

    /// The last 4 digits as shown: `…1234`.
    pub fn masked(&self) -> String {
        format!("…{}", self.last4)
    }

    /// The accounts of `employee_id`, oldest first.
    pub async fn of_employee(db: &Db, employee_id: i64) -> Result<Vec<PayAccount>> {
        PayAccount::query()
            .where_(PayAccount::EMPLOYEE_ID.eq(employee_id))
            .order_by(PayAccount::ID)
            .get(db)
            .await
    }

    /// The account payroll uses for `employee_id` now, inside `tx`.
    pub async fn active_in(tx: &mut Transaction, employee_id: i64) -> Result<Option<PayAccount>> {
        PayAccount::query()
            .where_(PayAccount::EMPLOYEE_ID.eq(employee_id))
            .where_(PayAccount::STATUS.eq(PayAccountStatus::Active))
            .order_by_desc(PayAccount::ID)
            .first(&mut *tx)
            .await
    }

    /// The account of `employee_id` that is `pending_approval` or `cooling`.
    pub async fn open_for(db: &Db, employee_id: i64) -> Result<Option<PayAccount>> {
        PayAccount::query()
            .where_(PayAccount::EMPLOYEE_ID.eq(employee_id))
            .where_(
                PayAccount::STATUS
                    .is_in([PayAccountStatus::PendingApproval, PayAccountStatus::Cooling]),
            )
            .order_by(PayAccount::ID)
            .first(db)
            .await
    }

    /// Whether the cooling period has ended on `day`.
    pub fn cooling_ended(&self, day: NaiveDate) -> bool {
        self.cooling_ends_on.is_some_and(|end| day >= end)
    }

    /// Whether the notice was given up and HR has not reached the worker.
    pub fn blocked_by_notice(&self) -> bool {
        self.notice_undelivered_at.is_some() && self.reached_on.is_none()
    }
}

fn refuse(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]))
}

// ---- format ----------------------------------------------------------------

/// `text` without spaces and in upper case: how an IBAN is compared.
pub fn compact(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_uppercase()
}

/// Whether `iban` is a UAE IBAN: `AE`, 2 check digits and 19 digits (23
/// characters), and the ISO 13616 mod-97 check passes. Spaces are ignored.
pub fn iban_valid(iban: &str) -> bool {
    let iban = compact(iban);
    if iban.len() != 23 || !iban.starts_with("AE") || !iban.is_ascii() {
        return false;
    }
    if !iban[2..].chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let rearranged = format!("{}{}", &iban[4..], &iban[..4]);
    let mut remainder: u32 = 0;
    for c in rearranged.chars() {
        let Some(value) = c.to_digit(36) else {
            return false;
        };
        let digits = if value >= 10 { 2 } else { 1 };
        remainder = (remainder * 10_u32.pow(digits) + value) % 97;
    }
    remainder == 1
}

/// Whether `card` is a salary card number: digits only, [`CARD_LENGTH`] of them.
pub fn card_valid(card: &str) -> bool {
    let card = compact(card);
    card.len() == CARD_LENGTH && card.chars().all(|c| c.is_ascii_digit())
}

/// Whether `code` is a routing code: [`ROUTING_LENGTH`] digits, and in `known`
/// when that list is not empty.
pub fn routing_valid(code: &str, known: &[String]) -> bool {
    let code = code.trim();
    code.len() == ROUTING_LENGTH
        && code.chars().all(|c| c.is_ascii_digit())
        && (known.is_empty() || known.iter().any(|k| k == code))
}

/// A name as compared: letters and digits in lower case, so case and spacing
/// do not matter.
pub fn name_key(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

// ---- the request -----------------------------------------------------------

/// What a request carries.
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq, Eq)]
#[serde(default)]
pub struct NewPayAccount {
    /// [`BANK_ACCOUNT`] or [`SALARY_CARD`].
    pub channel: String,
    /// The routing code of the bank or agent.
    pub routing_code: String,
    /// The IBAN or card number.
    pub account_number: String,
    /// The name of the holder.
    pub holder_name: String,
    /// Why the name differs from the employee's, when it does.
    pub mismatch_explanation: Option<String>,
    /// A supporting document, when attached.
    pub mismatch_document: Option<i64>,
}

/// The employee's name, when the party directory knows it.
async fn known_name(state: &AppState, employee: &Employee) -> Option<String> {
    wl_kernel::parties::directory(state)
        .display_name(&state.db, employee.party(), "en")
        .await
        .ok()
        .flatten()
}

/// The mobile number and the e-mail address on record for the employee now.
async fn contacts_on_record(
    state: &AppState,
    employee: &Employee,
) -> Result<(Option<String>, Option<String>)> {
    use wl_kernel::notify::NotifyChannel;
    let contacts = wl_kernel::parties::directory(state)
        .contacts(&state.db, employee.party())
        .await?;
    Ok(match contacts {
        Some(contacts) => (
            contacts
                .point_for(NotifyChannel::Sms)
                .map(|p| p.address.clone()),
            contacts
                .point_for(NotifyChannel::Email)
                .map(|p| p.address.clone()),
        ),
        None => (None, None),
    })
}

/// Checks `input` and gives the account number as it is stored.
async fn checked(
    state: &AppState,
    employee: &Employee,
    input: &NewPayAccount,
) -> Result<(String, String)> {
    let number = compact(&input.account_number);
    match input.channel.as_str() {
        BANK_ACCOUNT if iban_valid(&number) => {}
        BANK_ACCOUNT => return Err(refuse("wl_hr.pay_account.iban_invalid")),
        SALARY_CARD if card_valid(&number) => {}
        SALARY_CARD => return Err(refuse("wl_hr.pay_account.card_invalid")),
        _ => return Err(refuse("wl_hr.pay_account.channel_invalid")),
    }
    if !routing_valid(&input.routing_code, &config_of(state).routing_codes) {
        return Err(refuse("wl_hr.pay_account.routing_invalid"));
    }
    let holder = input.holder_name.trim();
    if holder.is_empty() {
        return Err(refuse("wl_hr.pay_account.holder_required"));
    }
    let matches = known_name(state, employee)
        .await
        .is_some_and(|name| name_key(&name) == name_key(holder));
    let explained = input
        .mismatch_explanation
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty());
    if !matches && !explained {
        return Err(refuse("wl_hr.pay_account.explanation_required"));
    }
    let last4: String = number
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    Ok((number, last4))
}

/// Makes the account `pending_approval` and tells the approvers.
async fn create(
    state: &AppState,
    employee: &Employee,
    by: &AuthUser,
    by_employee: bool,
    input: NewPayAccount,
) -> Result<PayAccount> {
    let (number, last4) = checked(state, employee, &input).await?;
    if PayAccount::open_for(&state.db, employee.id)
        .await?
        .is_some()
    {
        return Err(refuse("wl_hr.pay_account.already_open"));
    }
    let (mobile, email) = contacts_on_record(state, employee).await?;
    let explanation = input
        .mismatch_explanation
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned);
    let account = PayAccount::create(
        &state.db,
        PayAccount {
            employee_id: employee.id,
            channel: input.channel.clone(),
            routing_code: Some(input.routing_code.trim().to_owned()),
            account_number: number,
            last4,
            holder_name: input.holder_name.trim().to_owned(),
            mismatch_explanation: explanation,
            mismatch_document: input.mismatch_document,
            requested_by: Some(by.id),
            requested_by_employee: by_employee,
            notice_to: mobile,
            notice_email: email,
            cooling_days: config_of(state).cooling_days,
            ..PayAccount::default()
        },
    )
    .await?;
    renox::audit::record(
        &state.db,
        renox::audit::Entry::new("hr.pay_account_requested")
            .user(by.id)
            .subject(PayAccount::TABLE, account.id)
            .data(json!({
                "employee": employee.id,
                "last4": account.last4,
                "by_employee": by_employee,
            })),
    )
    .await?;
    crate::pay_account_events::requested(state, employee, &account).await?;
    Ok(account)
}

/// `by` (a holder of `employees.manage` for the employee) asks for a new pay
/// account for `employee_id`.
///
/// # Errors
///
/// `Forbidden` for a user who may not manage the employee; 422 with the text
/// of the problem for an invalid account, a name mismatch without an
/// explanation, or while another account is `pending_approval` or `cooling`.
pub async fn request(
    state: &AppState,
    by: &AuthUser,
    employee_id: i64,
    input: NewPayAccount,
) -> Result<PayAccount> {
    let employee = Employee::find_or_404(&state.db, employee_id).await?;
    if !crate::exits::holds_manage(state, by.id, &employee).await? {
        return Err(Error::Forbidden);
    }
    create(state, &employee, by, false, input).await
}

/// The employee whose account is the user `user_id`.
pub async fn employee_of_user(db: &Db, user_id: i64) -> Result<Option<Employee>> {
    Employee::query()
        .where_(Employee::USER_ID.eq(user_id))
        .order_by(Employee::ID)
        .first(db)
        .await
}

/// The employee `by` asks for a new pay account for themselves, in the
/// portal.
///
/// # Errors
///
/// `Forbidden` for a user who is no employee; the same 422 as [`request`].
pub async fn request_as_employee(
    state: &AppState,
    by: &AuthUser,
    input: NewPayAccount,
) -> Result<PayAccount> {
    let employee = employee_of_user(&state.db, by.id)
        .await?
        .ok_or(Error::Forbidden)?;
    create(state, &employee, by, true, input).await
}

// ---- the machine -----------------------------------------------------------

impl StateMachine for PayAccount {
    type State = PayAccountStatus;
    const COLUMN: renox::db::Col<PayAccount, PayAccountStatus> = PayAccount::STATUS;

    fn machine() -> Machine<Self> {
        use PayAccountStatus::{Active, Cooling, PendingApproval, Rejected, Retired};
        Machine::new()
            .initial([PendingApproval])
            .state(PendingApproval)
            .tone("warning")
            .state(Cooling)
            .tone("info")
            .state(Active)
            .tone("success")
            .state(Retired)
            .tone("neutral")
            .terminal()
            .state(Rejected)
            .tone("neutral")
            .terminal()
            .transition(approve())
            .transition(reject())
            .transition(stop(false))
            .transition(stop(true))
            .transition(activate())
            .transition(waive(false))
            .transition(waive(true))
            .transition(
                Transition::new(RETIRE)
                    .label("Retire")
                    .system()
                    .from([Active])
                    .to(Retired),
            )
    }
}

/// `approve`: not the requester; cooling ends after N days.
fn approve() -> Transition<PayAccount> {
    Transition::new(APPROVE)
        .label("Approve")
        .tone("primary")
        .permission(permissions::PAY_ACCOUNTS_APPROVE)
        .from([PayAccountStatus::PendingApproval])
        .to(PayAccountStatus::Cooling)
        .guard("not_the_requester", |account: &PayAccount, ctx| {
            let same = matches!(
                (ctx.actor(), account.requested_by),
                (Some(user), Some(requester)) if user.id == requester
            );
            Box::pin(async move {
                if same {
                    Err(english("wl_hr.pay_account.same_user", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .effect(|account: &PayAccount, ctx, tx| {
            let id = account.id;
            let days = account.cooling_days;
            let no_number = account.notice_to.is_none();
            let by = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                let day = today();
                let undelivered: Option<DateTime> = no_number.then(now);
                PayAccount::query()
                    .where_(PayAccount::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("approved_by", &by),
                            ("approved_on", &Some(day)),
                            ("cooling_ends_on", &Some(day + Duration::days(days))),
                            ("notice_undelivered_at", &undelivered),
                            ("updated_at", &now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// `reject`: with a reason.
fn reject() -> Transition<PayAccount> {
    Transition::new(REJECT)
        .label("Reject")
        .tone("danger")
        .permission(permissions::PAY_ACCOUNTS_APPROVE)
        .from([PayAccountStatus::PendingApproval])
        .to(PayAccountStatus::Rejected)
        .reason_required()
        .effect(|account: &PayAccount, ctx, tx| {
            let id = account.id;
            let reason = ctx.reason().map(str::trim).map(str::to_owned);
            Box::pin(async move {
                PayAccount::query()
                    .where_(PayAccount::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[("reject_reason", &reason), ("updated_at", &now())],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// `stop`, by HR or (as the system) for the worker.
fn stop(by_worker: bool) -> Transition<PayAccount> {
    let transition: Transition<PayAccount> = if by_worker {
        Transition::new(STOP_BY_WORKER).system()
    } else {
        Transition::new(STOP)
            .tone("danger")
            .permission(permissions::EMPLOYEES_MANAGE)
    };
    transition
        .label("Stop")
        .from([PayAccountStatus::Cooling])
        .to(PayAccountStatus::Rejected)
        .effect(move |account: &PayAccount, ctx, tx| {
            let id = account.id;
            let by = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                PayAccount::query()
                    .where_(PayAccount::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("stopped_by", &by),
                            ("stopped_by_worker", &by_worker),
                            ("updated_at", &now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// `activate`: the cooling ended and the notice was not given up, or the
/// worker was reached.
fn activate() -> Transition<PayAccount> {
    Transition::new(ACTIVATE)
        .label("Activate")
        .system()
        .from([PayAccountStatus::Cooling])
        .to(PayAccountStatus::Active)
        .guard("cooling_ended", |account: &PayAccount, _ctx| {
            let ended = account.cooling_ended(today());
            Box::pin(async move {
                if ended {
                    Ok(())
                } else {
                    Err(english("wl_hr.pay_account.cooling_not_ended", &[]))
                }
            })
        })
        .guard("notice_delivered", |account: &PayAccount, _ctx| {
            let blocked = account.blocked_by_notice();
            Box::pin(async move {
                if blocked {
                    Err(english("wl_hr.pay_account.notice_undelivered", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .effect(|account: &PayAccount, _ctx, tx| {
            let (id, employee) = (account.id, account.employee_id);
            Box::pin(async move { retire_previous_in(tx, id, employee).await })
        })
}

/// What `waive_cooling` carries: the reason, part of the payload the approval
/// request keeps, so the twin sees it.
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq, Eq)]
#[serde(default)]
pub struct WaiveInput {
    /// Why the cooling is waived.
    pub reason: String,
}

/// `waive_cooling` and its twin: with a reason in the input; the user's run is
/// deferred as an approval request with one step for the MD.
fn waive(approved: bool) -> Transition<PayAccount> {
    let transition: Transition<PayAccount> = if approved {
        Transition::new(WAIVE_COOLING_APPROVED).system()
    } else {
        Transition::new(WAIVE_COOLING)
            .tone("danger")
            .permission(permissions::PAYROLL_APPROVE)
    };
    transition
        .label("Waive cooling")
        .from([PayAccountStatus::Cooling])
        .to(PayAccountStatus::Active)
        .guard(
            "reason_given",
            |_account: &PayAccount, ctx: &TransitionCtx| {
                let given = ctx
                    .input_as::<WaiveInput>()
                    .is_ok_and(|input| !input.reason.trim().is_empty());
                Box::pin(async move {
                    if given {
                        Ok(())
                    } else {
                        Err(english("wl_hr.pay_account.waive_reason_required", &[]))
                    }
                })
            },
        )
        .effect(|account: &PayAccount, ctx, tx| {
            let (id, employee) = (account.id, account.employee_id);
            let reason = ctx
                .input_as::<WaiveInput>()
                .map(|input| input.reason.trim().to_owned())
                .unwrap_or_default();
            Box::pin(async move {
                PayAccount::query()
                    .where_(PayAccount::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[("waive_reason", &Some(reason)), ("updated_at", &now())],
                    )
                    .await?;
                retire_previous_in(tx, id, employee).await
            })
        })
}

/// The account `id` of `employee_id` became active: the one payroll used
/// before is `retired`.
async fn retire_previous_in(tx: &mut Transaction, id: i64, employee_id: i64) -> Result {
    let previous = PayAccount::query()
        .where_(PayAccount::EMPLOYEE_ID.eq(employee_id))
        .where_(PayAccount::STATUS.eq(PayAccountStatus::Active))
        .where_(PayAccount::ID.ne(id))
        .order_by(PayAccount::ID)
        .get(&mut *tx)
        .await?;
    for old in previous {
        let _outcome = old
            .transition(RETIRE)
            .by_system()
            .run_in(&mut *tx)
            .await
            .map_err(TransitionError::into_error)?;
    }
    Ok(())
}

// ---- actions ---------------------------------------------------------------

/// Payroll user `by` asks to waive the cooling of account `id` for `reason`
/// (`HRM.ACL.05`). The kernel defers it as an approval request with one step
/// for the MD role; the account stays `cooling` until an MD approves.
///
/// # Errors
///
/// `Forbidden` without `payroll.approve`; 422 without a reason, when the
/// account is not `cooling`, or when no approval policy would check it.
pub async fn waive_cooling(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
) -> Result<wl_kernel::ApprovalRequest> {
    let account = PayAccount::find_or_404(&state.db, id).await?;
    let input = WaiveInput {
        reason: reason.trim().to_owned(),
    };
    let payload = renox::serde_json::to_value(&input).map_err(|e| Error::Internal(e.into()))?;
    if ApprovalPolicy::for_action(&state.db, WAIVE_ACTION, &payload)
        .await?
        .is_none()
    {
        return Err(refuse("wl_hr.exits.no_policy"));
    }
    let outcome = account
        .transition(WAIVE_COOLING)
        .by(by)
        .reason(input.reason.clone())
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    approvals::announce(state, &outcome).await?;
    let request = wl_kernel::ApprovalRequest::pending_about(&state.db, &account.subject())
        .await?
        .into_iter()
        .find(|request| request.transition == WAIVE_COOLING)
        .ok_or_else(|| refuse("wl_hr.exits.no_policy"))?;
    renox::audit::record(
        &state.db,
        renox::audit::Entry::new("hr.pay_account_waive_requested")
            .user(by.id)
            .subject(PayAccount::TABLE, account.id)
            .data(json!({
                "approval_request": request.id,
                "reason": input.reason,
                "requested_by": account.requested_by,
            })),
    )
    .await?;
    Ok(request)
}

/// What *worker reached* carries: by whom, how, when.
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq, Eq)]
#[serde(default)]
pub struct ReachedInput {
    /// Who reached the worker.
    pub by: String,
    /// `call`, `visit` or `in_person`.
    pub method: String,
    /// The day.
    pub on: Option<NaiveDate>,
}

/// `by` (a holder of `employees.manage` for the employee) records that the
/// worker was reached another way, on the `cooling` account `id`. A field
/// update, not a transition; audited. When the cooling has ended already the
/// account activates at once, else on its end date.
///
/// # Errors
///
/// `Forbidden` for a user who may not manage the employee; 422 for an account
/// that is not `cooling`, or when by whom, the method or the date is missing,
/// or the date is after today.
pub async fn record_worker_reached(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    input: ReachedInput,
) -> Result<PayAccount> {
    let account = PayAccount::find_or_404(&state.db, id).await?;
    let employee = Employee::find_or_404(&state.db, account.employee_id).await?;
    if !crate::exits::holds_manage(state, by.id, &employee).await? {
        return Err(Error::Forbidden);
    }
    if account.status != PayAccountStatus::Cooling {
        return Err(refuse("wl_hr.pay_account.not_cooling"));
    }
    let day = today();
    let whom = input.by.trim();
    let valid = !whom.is_empty()
        && REACH_METHODS.contains(&input.method.trim())
        && input.on.is_some_and(|on| on <= day);
    if !valid {
        return Err(refuse("wl_hr.pay_account.reached_invalid"));
    }
    PayAccount::query()
        .where_(PayAccount::ID.eq(id))
        .update(
            &state.db,
            &[
                ("reached_by", &Some(whom.to_owned())),
                ("reached_method", &Some(input.method.trim().to_owned())),
                ("reached_on", &input.on),
                ("reached_recorded_by", &Some(by.id)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    renox::audit::record(
        &state.db,
        renox::audit::Entry::new("hr.pay_account_worker_reached")
            .user(by.id)
            .subject(PayAccount::TABLE, id)
            .data(json!({
                "reached_by": whom,
                "method": input.method.trim(),
                "on": input.on,
            })),
    )
    .await?;
    let account = PayAccount::find_or_404(&state.db, id).await?;
    if account.cooling_ended(day) {
        activate_account(state, id).await?;
    }
    PayAccount::find_or_404(&state.db, id).await
}

/// Runs `activate` on account `id` as the system.
async fn activate_account(state: &AppState, id: i64) -> std::result::Result<(), TransitionError> {
    let account = PayAccount::find_or_404(&state.db, id)
        .await
        .map_err(TransitionError::Failed)?;
    let _outcome = account.transition(ACTIVATE).by_system().run(state).await?;
    Ok(())
}

/// The worker stops the change of account `id` (the signed link, or the
/// portal once `user` was checked): it is `rejected`, the old account stays.
///
/// # Errors
///
/// 422 when the account is no longer `cooling`.
pub async fn stop_by_worker(state: &AppState, id: i64) -> Result<PayAccount> {
    let account = PayAccount::find_or_404(&state.db, id).await?;
    if account.status != PayAccountStatus::Cooling {
        return Err(refuse("wl_hr.pay_account.not_cooling"));
    }
    let _outcome = account
        .transition(STOP_BY_WORKER)
        .by_system()
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    PayAccount::find_or_404(&state.db, id).await
}

/// The employee `user` stops their own change of account `id`, in the portal.
///
/// # Errors
///
/// `Forbidden` when the account is not the user's; 422 as [`stop_by_worker`].
pub async fn stop_in_portal(state: &AppState, user: &AuthUser, id: i64) -> Result<PayAccount> {
    let account = PayAccount::find_or_404(&state.db, id).await?;
    let employee = employee_of_user(&state.db, user.id).await?;
    if employee.is_none_or(|e| e.id != account.employee_id) {
        return Err(Error::Forbidden);
    }
    stop_by_worker(state, id).await
}

/// What a run of [`activate_due`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Accounts that became `active`.
    pub activated: usize,
    /// Accounts past their cooling end that stay `cooling` because the
    /// notice was not delivered and HR has not reached the worker.
    pub waiting: usize,
}

/// `activate` for every `cooling` account whose cooling ended on or before
/// `day`; an account whose notice gave up stays (the guard refuses it).
/// Idempotent: a second run finds nothing more.
pub async fn activate_due(state: &AppState, day: NaiveDate) -> Result<Report> {
    let due = PayAccount::query()
        .where_(PayAccount::STATUS.eq(PayAccountStatus::Cooling))
        .where_(PayAccount::COOLING_ENDS_ON.is_not_null())
        .order_by(PayAccount::ID)
        .get(&state.db)
        .await?;
    let mut report = Report::default();
    for account in due.into_iter().filter(|a| a.cooling_ended(day)) {
        if account.blocked_by_notice() {
            report.waiting += 1;
            continue;
        }
        match activate_account(state, account.id).await {
            Ok(()) => report.activated += 1,
            Err(TransitionError::Guard(_) | TransitionError::NotFrom(_)) => report.waiting += 1,
            Err(error) => return Err(error.into_error()),
        }
    }
    Ok(report)
}

// ---- registration ------------------------------------------------------------

/// The machine's entry in the kernel registry: `waive_cooling` is deferred as
/// an approval request under the action key [`WAIVE_ACTION`].
pub fn subject_machine() -> wl_kernel::SubjectMachine {
    wl_kernel::SubjectMachine::of::<PayAccount>(PAY_ACCOUNT_SUBJECT_TYPE).build()
}

/// The default policy of `waive_cooling`: one step for `md_role`, segregation
/// of duties on, no expiry (a request waits for the MD).
pub fn waive_policy(md_role: &str) -> PolicyDefault {
    PolicyDefault::new(WAIVE_ACTION)
        .step([md_role])
        .expiry_hours(None)
}

/// Registers the machine, the policy, the second user the MD must not be,
/// the listeners and the daily job.
pub(crate) fn register(app: &mut Registry, md_role: &str) {
    subject_machine().register();
    renox::state_machine::register::<PayAccount>();
    waive_policy(md_role).register();
    // The requester of the change may not approve its waiver, besides the
    // user who asks for it (the request's own requester).
    approvals::register_excluded_approvers(PAY_ACCOUNT_SUBJECT_TYPE, |db, request| {
        Box::pin(async move {
            let account = PayAccount::find(db, request.subject_id).await?;
            Ok(account
                .and_then(|account| account.requested_by)
                .into_iter()
                .collect())
        })
    });
    crate::pay_account_events::register(app);
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            activate_due(&state, today()).await.map(|_| ())
        })
        .timezone(crate::wps_alerts::TASK_ZONE);
    app.command(
        TASK,
        "Activate the pay accounts whose cooling period ended (--date YYYY-MM-DD checks another day)",
        |args, state: AppState| async move {
            let on = match args.value("--date") {
                None => today(),
                Some(text) => text.parse().map_err(|_| {
                    Error::Status(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        format!("--date must be YYYY-MM-DD, got `{text}`"),
                    )
                })?,
            };
            let report = activate_due(&state, on).await?;
            println!(
                "{} accounts activated, {} waiting for the worker to be reached.",
                report.activated, report.waiting
            );
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_uae_iban_passes_the_length_prefix_and_mod_97_checks() {
        assert!(iban_valid("AE07 0331 2345 6789 0123 456"));
        assert!(iban_valid("ae550331234567890121234"));
        // A wrong check digit, a wrong length, another country, letters.
        assert!(!iban_valid("AE08 0331 2345 6789 0123 456"));
        assert!(!iban_valid("AE0703312345678901234"));
        assert!(!iban_valid("GB82WEST12345698765432"));
        assert!(!iban_valid("AE07033123456789012345A"));
    }

    #[test]
    fn a_card_number_is_digits_only() {
        assert!(card_valid("4111 1111 1111 1111"));
        assert!(!card_valid("4111 1111 1111 111A"));
        assert!(!card_valid("411111111111"));
    }

    #[test]
    fn a_routing_code_is_nine_digits_and_in_the_list_when_there_is_one() {
        assert!(routing_valid("123456789", &[]));
        assert!(!routing_valid("12345", &[]));
        assert!(!routing_valid("12345678A", &[]));
        let known = vec!["033000001".to_owned()];
        assert!(routing_valid("033000001", &known));
        assert!(!routing_valid("123456789", &known));
    }

    #[test]
    fn names_are_compared_without_case_and_spacing() {
        assert_eq!(name_key("Ravi  K."), name_key("ravi k"));
        assert_ne!(name_key("Ravi K."), name_key("Ravi Kumar"));
    }

    #[test]
    fn the_transitions_move_between_the_spec_states() {
        let machine = PayAccount::machine();
        let table = [
            (APPROVE, false, "pending_approval", "cooling"),
            (REJECT, false, "pending_approval", "rejected"),
            (STOP, false, "cooling", "rejected"),
            (STOP_BY_WORKER, true, "cooling", "rejected"),
            (ACTIVATE, true, "cooling", "active"),
            (WAIVE_COOLING, false, "cooling", "active"),
            (WAIVE_COOLING_APPROVED, true, "cooling", "active"),
            (RETIRE, true, "active", "retired"),
        ];
        for (name, system, from, to) in table {
            let transition = machine.find(name).unwrap();
            assert_eq!(transition.is_system(), system, "{name}");
            assert_eq!(transition.from_states(), [from], "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
        }
    }

    #[test]
    fn the_cooling_ends_on_its_end_date_not_before() {
        let account = PayAccount {
            cooling_ends_on: NaiveDate::from_ymd_opt(2026, 11, 7),
            ..PayAccount::default()
        };
        assert!(!account.cooling_ended(NaiveDate::from_ymd_opt(2026, 11, 6).unwrap()));
        assert!(account.cooling_ended(NaiveDate::from_ymd_opt(2026, 11, 7).unwrap()));
    }
}
