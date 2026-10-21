//! The credit hold of a client contract (09 Manpower §4.1 *The client stops
//! paying*, §7.1 *Client contract* `hold`, §7.2 `MPW.RUL.10`, `MPW.ACL.14`;
//! BL-MPW-01).
//!
//! When a client stops paying while workers stay on site, the agency stops
//! taking on more exposure: the contract becomes `on_hold`, no request is
//! submitted and no deployment is activated or extended (`MPW.ACL.14`, see
//! [`crate::contract::ClientContract::takes_deployments`]); the workers on site
//! stay and are billed.
//!
//! - **`hold`** (`contracts.manage`) is the account manager's, with a reason:
//!   `overdue`, `bounced_cheque` or `other` with a text ([`hold_by_hand`]).
//!   **`auto_hold`** is the same transition for the system (`MPW.RUL.10`); a
//!   Renox transition is either a user's or the system's, so there are two.
//!   Both take a contract `active` or `terminating`, keep the state before the
//!   hold, the day and who held it, and add the hold reasons as rows
//!   ([`ContractHold`]) in the same transaction.
//! - A contract already `on_hold` has no second `hold`: a new cause is added
//!   to its open reasons ([`apply_reasons`]) and both managers are told once.
//! - **`MPW.RUL.10`** ([`run_contract`]) looks, daily at 02:00 Asia/Dubai
//!   ([`daily`]) and on a payment of the client (`PaymentReceived`,
//!   `InvoicePaid`), for an invoice issued under the contract that is open,
//!   not `disputed` nor `written_off`, with a balance and a due date more than
//!   N days back ([`crate::config::DEFAULT_HOLD_OVERDUE_DAYS`], 30). A
//!   `ChequeBounced` of the client holds each of its contracts at once
//!   ([`on_cheque_bounced`]). One reason row is made per invoice or cheque, so
//!   a second run with the same facts changes nothing and tells nobody.
//! - The account manager and the MD get one message with the overdue amount,
//!   the bounced cheques and the workers on site. The client gets nothing
//!   new: the `FIN.RUL.01` reminders go on as they do, and the hold is not
//!   shown in the portal. No deployment is ended, suspended or cancelled.
//!
//! Decisions where the spec is silent:
//!
//! - *An invoice issued under the contract* is one with a line whose origin is
//!   the contract (`manpower.client_contract:N`), found through the kernel's
//!   `Invoicer::billing`. The billing of the contract's hours puts that origin
//!   on its lines.
//! - The bounced cheques of the rule are the ones `ChequeBounced` announced
//!   (their reason rows), not a scan of the cheque register: the kernel's
//!   `Cheques` has no list by party. A cheque whose reason is open is looked
//!   up again for the release ([`crate::release`]).
//! - A hold has an open reason `other` until a manual `release`: the system
//!   never decides that a liquidation, say, is over.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::cheques::{ChequeBounced, cheques};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoicePaid, InvoiceSummary, invoicer};
use wl_kernel::payments::PaymentReceived;

use crate::config::DEFAULT_HOLD_OVERDUE_DAYS;
use crate::contract::{AUTO_HOLD, ClientContract, ContractStatus, HOLD};
use crate::db::{answer, refused, set_contract};
use crate::deployment::{Deployment, DeploymentStatus};
use crate::deployments::worker_name;
use crate::notices;
use crate::permissions;
use crate::site::Site;
use crate::texts::{english, money};

/// Why a contract is on hold.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HoldReason {
    /// An invoice is overdue more than N days.
    #[default]
    Overdue,
    /// A cheque of the client bounced.
    BouncedCheque,
    /// Another reason the account manager wrote.
    Other,
}

impl HoldReason {
    /// The reason stored as `text`.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "overdue" => Some(HoldReason::Overdue),
            "bounced_cheque" => Some(HoldReason::BouncedCheque),
            "other" => Some(HoldReason::Other),
            _ => None,
        }
    }
}

/// One reason a contract is on hold: an overdue invoice, a bounced cheque or
/// a text. It stays after the hold as the record of what held the contract.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "manpower_contract_holds", index(client_contract_id))]
pub struct ContractHold {
    /// The primary key.
    pub id: i64,
    /// The contract held.
    pub client_contract_id: i64,
    /// Why.
    pub reason: HoldReason,
    /// `other`: what the account manager wrote.
    pub reason_text: Option<String>,
    /// `overdue`: the invoice.
    pub invoice_id: Option<i64>,
    /// `bounced_cheque`: the cheque.
    pub cheque_id: Option<i64>,
    /// The invoice number or the cheque number, for the messages.
    pub label: Option<String>,
    /// The open balance or open amount, in fils.
    pub amount: i64,
    /// The user who added it; none when the rule did.
    pub held_by: Option<i64>,
    /// When the cause was gone or the hold released; none while open.
    pub resolved_at: Option<DateTime>,
    /// How: `payment`, `cheque_resolved`, `released`, `terminated`.
    pub resolution: Option<String>,
    /// Set when an MD-approved release covered it: the rule does not hold
    /// again for the same invoice or cheque.
    pub covered: bool,
    /// When the row was created: when the reason was added.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ContractHold {
    /// Whether the reason still holds the contract.
    pub fn is_open(&self) -> bool {
        self.resolved_at.is_none()
    }

    /// The reasons of contract `contract_id`, oldest first.
    ///
    /// # Errors
    ///
    /// A database error.
    pub async fn of_contract(db: &Db, contract_id: i64) -> Result<Vec<ContractHold>> {
        ContractHold::query()
            .where_(ContractHold::CLIENT_CONTRACT_ID.eq(contract_id))
            .order_by(ContractHold::ID)
            .get(db)
            .await
    }

    /// The open reasons of contract `contract_id`, oldest first.
    ///
    /// # Errors
    ///
    /// A database error.
    pub async fn open_of(db: &Db, contract_id: i64) -> Result<Vec<ContractHold>> {
        Ok(ContractHold::of_contract(db, contract_id)
            .await?
            .into_iter()
            .filter(ContractHold::is_open)
            .collect())
    }
}

/// A reason to add to a hold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewReason {
    /// Why.
    pub kind: HoldReason,
    /// `other`: the text.
    #[serde(default)]
    pub text: Option<String>,
    /// `overdue`: the invoice.
    #[serde(default)]
    pub invoice_id: Option<i64>,
    /// `bounced_cheque`: the cheque.
    #[serde(default)]
    pub cheque_id: Option<i64>,
    /// The invoice number or the cheque number.
    #[serde(default)]
    pub label: Option<String>,
    /// The open balance or open amount, in fils.
    #[serde(default)]
    pub amount: i64,
}

impl NewReason {
    /// A reason of the account manager's: `overdue`, `bounced_cheque` or
    /// `other` with `text`.
    pub fn by_hand(kind: HoldReason, text: Option<&str>) -> Self {
        NewReason {
            kind,
            text: text
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned),
            invoice_id: None,
            cheque_id: None,
            label: None,
            amount: 0,
        }
    }

    /// An invoice overdue more than N days.
    fn invoice(summary: &InvoiceSummary) -> Self {
        NewReason {
            kind: HoldReason::Overdue,
            text: None,
            invoice_id: Some(summary.id.0),
            cheque_id: None,
            label: Some(
                summary
                    .number
                    .clone()
                    .unwrap_or_else(|| format!("#{}", summary.id)),
            ),
            amount: summary.balance,
        }
    }

    /// The history line of the transition: what the reason is.
    fn history(&self) -> String {
        match self.kind {
            HoldReason::Overdue => english(
                "manpower.hold.history_overdue",
                &[("what", &self.label.clone().unwrap_or_default())],
            ),
            HoldReason::BouncedCheque => english(
                "manpower.hold.history_bounced_cheque",
                &[("what", &self.label.clone().unwrap_or_default())],
            ),
            HoldReason::Other => english(
                "manpower.hold.history_other",
                &[("text", &self.text.clone().unwrap_or_default())],
            ),
        }
    }

    /// The row.
    fn row(&self, contract_id: i64, by: Option<i64>) -> ContractHold {
        ContractHold {
            client_contract_id: contract_id,
            reason: self.kind,
            reason_text: self.text.clone(),
            invoice_id: self.invoice_id,
            cheque_id: self.cheque_id,
            label: self.label.clone(),
            amount: self.amount,
            held_by: by,
            ..ContractHold::default()
        }
    }
}

/// What a `hold` or `auto_hold` run carries to its guard and effect.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldInput {
    /// The reasons that hold the contract.
    #[serde(default)]
    pub reasons: Vec<NewReason>,
    /// The user who holds it; none for the rule.
    #[serde(default)]
    pub by: Option<i64>,
}

/// The input of a `hold` run.
pub(crate) fn hold_input(ctx: &TransitionCtx) -> HoldInput {
    ctx.input_as::<HoldInput>().unwrap_or_default()
}

/// Why `input` cannot hold a contract, if it cannot: a reason is needed and
/// `other` needs its text.
pub fn hold_problem(input: &HoldInput) -> Option<String> {
    if input.reasons.is_empty() {
        return Some(english("manpower.hold.reason_required", &[]));
    }
    if input
        .reasons
        .iter()
        .any(|reason| reason.kind == HoldReason::Other && reason.text.is_none())
    {
        return Some(english("manpower.hold.text_required", &[]));
    }
    None
}

/// Adds `reasons` as rows of contract `contract_id` in `tx`.
pub(crate) async fn add_reasons(
    tx: &mut Transaction,
    contract_id: i64,
    reasons: &[NewReason],
    by: Option<i64>,
) -> Result {
    for reason in reasons {
        ContractHold::create(&mut *tx, reason.row(contract_id, by)).await?;
    }
    Ok(())
}

/// The effect of `hold` and `auto_hold` inside their transaction: the state
/// before the hold, the day and who held it are kept and the reasons added.
pub(crate) async fn hold_effects(
    tx: &mut Transaction,
    contract: &ClientContract,
    input: &HoldInput,
) -> Result {
    let before = Some(contract.status.as_str().to_owned());
    set_contract(
        &mut *tx,
        contract.id,
        &[
            ("status_before_hold", &before),
            ("held_at", &Some(now())),
            ("held_by", &input.by),
        ],
    )
    .await?;
    add_reasons(&mut *tx, contract.id, &input.reasons, input.by).await
}

/// The contract `id`, or 404.
async fn load(state: &AppState, id: i64) -> Result<ClientContract> {
    ClientContract::find_or_404(&state.db, id).await
}

/// Whether `summary` counts for `MPW.RUL.10` on `on`: open, not `disputed`
/// (a `written_off` or paid invoice is not open), with a balance and a due
/// date more than `days` back.
pub fn counts_as_overdue(summary: &InvoiceSummary, on: NaiveDate, days: i64) -> bool {
    summary.status.is_open()
        && summary.balance > 0
        && !summary.disputed
        && summary
            .due_date
            .is_some_and(|due| (on - due).num_days() > days)
}

/// The invoices issued under `contract` that count for `MPW.RUL.10` on `on`,
/// oldest first.
///
/// # Errors
///
/// The invoicing contract's errors.
pub async fn overdue_invoices(
    state: &AppState,
    contract: &ClientContract,
    on: NaiveDate,
) -> Result<Vec<InvoiceSummary>> {
    let invoicing = invoicer(state);
    let mut found = Vec::new();
    for id in invoicing.billing(&state.db, &contract.subject()).await? {
        if let Some(summary) = invoicing.find(&state.db, id).await?
            && counts_as_overdue(&summary, on, DEFAULT_HOLD_OVERDUE_DAYS)
        {
            found.push(summary);
        }
    }
    Ok(found)
}

/// Holds `contract` with `reasons`, or adds them when it is already
/// `on_hold`, and tells the account manager and the MD once. `by` is the user
/// who holds it by hand; none for the rule. A contract that is not `active`,
/// `terminating` or `on_hold` is left alone and answers `None`.
///
/// # Errors
///
/// `Forbidden` when `by` lacks `contracts.manage` for the client; a 422 for a
/// missing reason or text.
pub async fn apply_reasons(
    state: &AppState,
    id: i64,
    reasons: Vec<NewReason>,
    by: Option<&AuthUser>,
) -> Result<Option<ClientContract>> {
    let input = HoldInput {
        reasons,
        by: by.map(|user| user.id),
    };
    if let Some(problem) = hold_problem(&input) {
        return Err(refused(problem));
    }
    let contract = load(state, id).await?;
    if let Some(user) = by
        && !user.has_permission_in(permissions::CONTRACTS_MANAGE, &contract.scope())
    {
        return Err(Error::Forbidden);
    }
    let history = input
        .reasons
        .iter()
        .map(NewReason::history)
        .collect::<Vec<_>>()
        .join("; ");
    match contract.status {
        ContractStatus::Active | ContractStatus::Terminating => {
            let name = if by.is_some() { HOLD } else { AUTO_HOLD };
            let mut tx = state.db.begin().await?;
            let run = contract.transition(name).reason(&history).input(&input);
            let run = match by {
                Some(user) => run.by(user),
                None => run.by_system(),
            };
            let outcome = match run.run_in(&mut tx).await {
                Ok(outcome) => outcome,
                // Another run held, ended or released it first.
                Err(TransitionError::NotFrom(_) | TransitionError::Conflict) if by.is_none() => {
                    return Ok(None);
                }
                Err(other) => return Err(answer(other)),
            };
            tx.commit().await?;
            outcome.emit(state).await?;
            let held = load(state, id).await?;
            tell(state, &held, false).await?;
            Ok(Some(held))
        }
        ContractStatus::OnHold => {
            let mut tx = state.db.begin().await?;
            add_reasons(&mut tx, id, &input.reasons, input.by).await?;
            set_contract(&mut tx, id, &[]).await?;
            tx.commit().await?;
            let held = load(state, id).await?;
            tell(state, &held, true).await?;
            Ok(Some(held))
        }
        ContractStatus::Draft | ContractStatus::Ended | ContractStatus::Cancelled => {
            if by.is_some() {
                Err(refused(english("manpower.hold.not_holdable", &[])))
            } else {
                Ok(None)
            }
        }
    }
}

/// `by` puts contract `id` on credit hold with `reason` (`overdue`,
/// `bounced_cheque` or `other`) and, for `other`, the `text`. On a contract
/// that is already `on_hold` the reason is added.
///
/// # Errors
///
/// `Forbidden` without `contracts.manage` for the client; a 422 without a
/// reason, for a reason that is not one of the three, for `other` without its
/// text, or unless the contract is `active`, `terminating` or `on_hold`.
pub async fn hold_by_hand(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
    text: Option<&str>,
) -> Result<ClientContract> {
    let kind = HoldReason::parse(reason)
        .ok_or_else(|| refused(english("manpower.hold.reason_required", &[])))?;
    apply_reasons(state, id, vec![NewReason::by_hand(kind, text)], Some(by))
        .await?
        .ok_or_else(|| refused(english("manpower.hold.not_holdable", &[])))
}

/// The people told of a hold: the account manager of the client and the MD.
async fn people(state: &AppState, contract: &ClientContract) -> Result<Vec<i64>> {
    let mut users =
        notices::account_managers(state, contract.client(), contract.account_manager_user_id)
            .await?;
    for user in notices::md_users(state, contract.client()).await? {
        if !users.contains(&user) {
            users.push(user);
        }
    }
    Ok(users)
}

/// The workers on site (`active` or `suspended` deployments) of `contract`,
/// one line each.
async fn workers_on_site(state: &AppState, contract: &ClientContract) -> Result<Vec<String>> {
    let deployments = Deployment::query()
        .where_(Deployment::CLIENT_CONTRACT_ID.eq(contract.id))
        .where_(Deployment::STATUS.is_in([DeploymentStatus::Active, DeploymentStatus::Suspended]))
        .order_by(Deployment::ID)
        .get(&state.db)
        .await?;
    let mut lines = Vec::new();
    for deployment in &deployments {
        let worker = worker_name(state, "en", deployment.employee()).await;
        let site = Site::find(&state.db, deployment.site_id)
            .await?
            .map(|site| site.name)
            .unwrap_or_default();
        lines.push(english(
            "manpower.hold.worker",
            &[
                ("worker", &worker),
                ("role", &deployment.role),
                ("site", &site),
            ],
        ));
    }
    Ok(lines)
}

/// The lines that say what holds `contract` now: the overdue amount, each
/// bounced cheque and each other reason.
pub(crate) fn reason_lines(open: &[ContractHold]) -> Vec<String> {
    let mut lines = Vec::new();
    let overdue: i64 = open
        .iter()
        .filter(|row| row.reason == HoldReason::Overdue)
        .map(|row| row.amount)
        .sum();
    if open.iter().any(|row| row.reason == HoldReason::Overdue) {
        let invoices = open
            .iter()
            .filter(|row| row.reason == HoldReason::Overdue)
            .filter_map(|row| row.label.clone())
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(english(
            "manpower.hold.line_overdue",
            &[("amount", &money(overdue)), ("invoices", &invoices)],
        ));
    }
    for row in open
        .iter()
        .filter(|row| row.reason == HoldReason::BouncedCheque)
    {
        lines.push(english(
            "manpower.hold.line_cheque",
            &[
                ("cheque", &row.label.clone().unwrap_or_default()),
                ("amount", &money(row.amount)),
            ],
        ));
    }
    for row in open.iter().filter(|row| row.reason == HoldReason::Other) {
        lines.push(english(
            "manpower.hold.line_other",
            &[("text", &row.reason_text.clone().unwrap_or_default())],
        ));
    }
    lines
}

/// Tells the account manager and the MD, in one message, that `contract` is
/// on hold (`added`: another reason was added to a hold that was there).
async fn tell(state: &AppState, contract: &ClientContract, added: bool) -> Result {
    let open = ContractHold::open_of(&state.db, contract.id).await?;
    let workers = workers_on_site(state, contract).await?;
    let client = wl_kernel::parties::directory(state)
        .display_name(&state.db, contract.client(), "en")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("#{}", contract.client_party_id));
    let count = workers.len();
    let reasons = reason_lines(&open).join("\n");
    let workers = workers.join("\n");
    let params: [(&str, &(dyn std::fmt::Display + Sync)); 5] = [
        ("contract", &contract.reference),
        ("client", &client),
        ("reasons", &reasons),
        ("count", &count),
        ("workers", &workers),
    ];
    let key = if added {
        "manpower.hold.added"
    } else {
        "manpower.hold.held"
    };
    let users = people(state, contract).await?;
    notices::tell(
        state,
        contract.subject(),
        &contract.reference,
        &users,
        &english(&format!("{key}_title"), &params),
        &english(&format!("{key}_body"), &params),
    )
    .await?;
    Ok(())
}

/// `MPW.RUL.10` for contract `id` on `on`: holds it, or adds a reason, for
/// each invoice that is overdue and not yet a reason (open, or covered by an
/// MD-approved release). Returns the reasons made; none changes nothing and
/// tells nobody. A contract that is not `active`, `terminating` or `on_hold`
/// does nothing.
///
/// # Errors
///
/// The database's and the contracts' errors.
pub async fn run_contract(state: &AppState, id: i64, on: NaiveDate) -> Result<Vec<NewReason>> {
    let contract = load(state, id).await?;
    if !matches!(
        contract.status,
        ContractStatus::Active | ContractStatus::Terminating | ContractStatus::OnHold
    ) {
        return Ok(Vec::new());
    }
    let known: Vec<i64> = ContractHold::of_contract(&state.db, id)
        .await?
        .iter()
        .filter(|row| row.is_open() || row.covered)
        .filter_map(|row| row.invoice_id)
        .collect();
    let reasons: Vec<NewReason> = overdue_invoices(state, &contract, on)
        .await?
        .iter()
        .filter(|summary| !known.contains(&summary.id.0))
        .map(NewReason::invoice)
        .collect();
    if reasons.is_empty() {
        return Ok(reasons);
    }
    apply_reasons(state, id, reasons.clone(), None).await?;
    Ok(reasons)
}

/// The contracts of `client` that `MPW.RUL.10` looks at.
async fn contracts_of(state: &AppState, client: Option<PartyId>) -> Result<Vec<ClientContract>> {
    let mut query = ClientContract::query().where_(ClientContract::STATUS.is_in([
        ContractStatus::Active,
        ContractStatus::Terminating,
        ContractStatus::OnHold,
    ]));
    if let Some(client) = client {
        query = query.where_(ClientContract::CLIENT_PARTY_ID.eq(client.0));
    }
    query.order_by(ClientContract::ID).get(&state.db).await
}

/// `MPW.RUL.10` for every contract of `client` on `on` (a payment of the
/// client changed an invoice balance). Returns the contracts it held or added
/// a reason to.
///
/// # Errors
///
/// As [`run_contract`].
pub async fn run_client(state: &AppState, client: PartyId, on: NaiveDate) -> Result<Vec<i64>> {
    run_all(state, contracts_of(state, Some(client)).await?, on).await
}

/// `MPW.RUL.10` for the given contracts.
async fn run_all(
    state: &AppState,
    contracts: Vec<ClientContract>,
    on: NaiveDate,
) -> Result<Vec<i64>> {
    let mut touched = Vec::new();
    for contract in contracts {
        if !run_contract(state, contract.id, on).await?.is_empty() {
            touched.push(contract.id);
        }
    }
    Ok(touched)
}

/// The daily run at 02:00 Asia/Dubai: `MPW.RUL.10` for every contract.
///
/// # Errors
///
/// As [`run_contract`].
pub async fn daily(state: &AppState) -> Result {
    run_all(state, contracts_of(state, None).await?, today()).await?;
    Ok(())
}

/// A cheque of the client bounced: each of its contracts `active` or
/// `terminating` is held with the cheque as the reason; one that is
/// `on_hold` gets the reason added. A cheque that already is a reason (open,
/// or covered by an MD-approved release) changes nothing.
///
/// # Errors
///
/// As [`run_contract`].
pub async fn on_cheque_bounced(state: &AppState, event: &ChequeBounced) -> Result {
    let label = cheques(state)
        .find(&state.db, event.cheque)
        .await?
        .map_or_else(|| format!("#{}", event.cheque), |cheque| cheque.number);
    for contract in contracts_of(state, Some(event.party)).await? {
        let known = ContractHold::of_contract(&state.db, contract.id)
            .await?
            .iter()
            .any(|row| row.cheque_id == Some(event.cheque.0) && (row.is_open() || row.covered));
        if known {
            continue;
        }
        let reason = NewReason {
            kind: HoldReason::BouncedCheque,
            text: None,
            invoice_id: None,
            cheque_id: Some(event.cheque.0),
            label: Some(label.clone()),
            amount: event.open_amount,
        };
        apply_reasons(state, contract.id, vec![reason], None).await?;
    }
    Ok(())
}

/// The rule's listeners: a bounced cheque holds, a payment of the client is a
/// reason to look again.
pub(crate) fn listen(app: &mut Registry) {
    app.listen(|event: ChequeBounced, state: AppState| async move {
        on_cheque_bounced(&state, &event).await
    });
    app.listen(|event: PaymentReceived, state: AppState| async move {
        run_client(&state, event.party, today()).await?;
        Ok(())
    });
    app.listen(|event: InvoicePaid, state: AppState| async move {
        run_client(&state, event.party, today()).await?;
        Ok(())
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::invoicing::{InvoiceId, InvoiceStatus};

    fn summary(status: InvoiceStatus, balance: i64, due: Option<NaiveDate>) -> InvoiceSummary {
        InvoiceSummary {
            id: InvoiceId(1),
            number: Some("INV-1".into()),
            status,
            party: PartyId(5),
            legal_entity_id: None,
            currency: "AED".into(),
            gross_total: 4_200_000,
            balance,
            due_date: due,
            disputed: false,
            chasing_paused_until: None,
        }
    }

    fn day(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, day).unwrap()
    }

    #[test]
    fn an_invoice_is_overdue_more_than_thirty_days_the_day_after_the_thirtieth() {
        let open = summary(InvoiceStatus::Overdue, 4_200_000, Some(day(10, 1)));
        assert!(!counts_as_overdue(&open, day(10, 31), 30));
        assert!(counts_as_overdue(&open, day(11, 1), 30));
    }

    #[test]
    fn a_disputed_paid_or_written_off_invoice_does_not_count() {
        let due = Some(day(9, 1));
        let mut disputed = summary(InvoiceStatus::Overdue, 100, due);
        disputed.disputed = true;
        assert!(!counts_as_overdue(&disputed, day(11, 1), 30));
        for status in [
            InvoiceStatus::WrittenOff,
            InvoiceStatus::Paid,
            InvoiceStatus::Credited,
        ] {
            assert!(!counts_as_overdue(
                &summary(status, 100, due),
                day(11, 1),
                30
            ));
        }
        assert!(!counts_as_overdue(
            &summary(InvoiceStatus::Overdue, 0, due),
            day(11, 1),
            30
        ));
        assert!(counts_as_overdue(
            &summary(InvoiceStatus::PartiallyPaid, 100, due),
            day(11, 1),
            30
        ));
    }

    #[test]
    fn a_hold_needs_a_reason_and_other_needs_its_text() {
        assert!(hold_problem(&HoldInput::default()).is_some());
        let other = NewReason::by_hand(HoldReason::Other, Some("  "));
        let input = HoldInput {
            reasons: vec![other],
            by: Some(1),
        };
        assert!(hold_problem(&input).is_some_and(|p| p.contains("other reason")));
        let input = HoldInput {
            reasons: vec![NewReason::by_hand(
                HoldReason::Other,
                Some("client in liquidation"),
            )],
            by: Some(1),
        };
        assert_eq!(hold_problem(&input), None);
    }

    #[test]
    fn a_reason_is_stored_and_read_by_its_name() {
        for reason in [
            HoldReason::Overdue,
            HoldReason::BouncedCheque,
            HoldReason::Other,
        ] {
            assert_eq!(HoldReason::parse(reason.as_str()), Some(reason));
        }
        assert_eq!(HoldReason::parse("bored"), None);
    }
}
