//! The staff pages and actions on COD remittances (03 Logistics §7.1 *COD
//! remittance*; BL-LOG-14, BL-LOG-15).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /remittances` | `logistics.remittances` | `remittances.view` |
//! | `GET /remittances/accounts` | `logistics.account_changes` | `remittances.view`: the bank account changes waiting |
//! | `GET /remittances/{id}` | `logistics.remittance` | `remittances.view` |
//! | `POST /remittances/build` | `logistics.remittance_build` | `remittances.prepare` |
//! | `POST /remittances/{id}/approve` | `logistics.remittance_approve` | `remittances.approve` |
//! | `POST /remittances/{id}/reopen` | `logistics.remittance_reopen` | `remittances.approve` |
//! | `POST /remittances/{id}/pay` | `logistics.remittance_pay` | `remittances.pay` (field `block_exception`: ask the company admin for an exception while the client is blocked) |
//! | `POST /remittances/{id}/return-payment` | `logistics.remittance_return_payment` | `remittances.pay` (fields `advice`, `reason`) |
//! | `POST /remittances/{id}/repay` | `logistics.remittance_repay` | `remittances.pay` (field `payment_reference`) |
//! | `POST /remittances/{id}/confirm-account` | `logistics.remittance_confirm_account` | `remittances.pay`, or a client user of the company (field `note`) |
//! | `POST /remittances/accounts` | `logistics.account_request` | `remittances.request_account` (fields `client_party_id`, `account`) |
//! | `POST /remittances/accounts/{id}/approve` | `logistics.account_approve` | `remittances.approve` |
//! | `POST /remittances/accounts/{id}/reject` | `logistics.account_reject` | `remittances.approve` (field `note`) |
//! | `POST /remittances/accounts/{id}/withdraw` | `logistics.account_withdraw` | `remittances.request_account` |
//! | `GET /remittances/{id}/statement` | `logistics.remittance_statement` | `remittances.view`, or a client user of the company (JSON) |
//! | `POST /remittances/adjustments` | `logistics.adjustment_add` | `remittances.prepare` |
//! | `POST /remittances/adjustments/{id}/approve` | `logistics.adjustment_approve` | `remittances.approve` |
//! | `POST /remittances/adjustments/{id}/reject` | `logistics.adjustment_reject` | `remittances.approve` |
//! | `POST /remittances/adjustments/{id}/resubmit` | `logistics.adjustment_resubmit` | `remittances.prepare` |
//! | `POST /remittances/adjustments/{id}/delete` | `logistics.adjustment_delete` | `remittances.prepare` |
//!
//! A successful action answers a redirect (303) with a toast. A refused
//! action answers 422 with the reason (403 without the permission).

use renox::grid::{Action, Column, Grid, GridPage, GridRequest, Summary};
use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::Serialize;
use wl_kernel::PartyId;
use wl_kernel::block_guard::{self, ActionKind, BlockRefusalView, GuardOutcome};

use crate::account::{self, AccountChange, AccountChangeStatus};
use crate::adjustment::{self, AdjustmentLine, AdjustmentStatus, NewAdjustment};
use crate::booking;
use crate::pages::{Fields, Texts, checked, field, tx};
use crate::permissions;
use crate::remit;
use crate::remittance::{Remittance, RemittanceStatus};
use crate::shipment::Shipment;
use crate::statement;
use crate::texts::t;

/// The templates, compiled in.
pub(crate) const VIEWS: &[(&str, &str)] = &[
    (
        "logistics/remittances.html",
        include_str!("../views/logistics/remittances.html"),
    ),
    (
        "logistics/remittance.html",
        include_str!("../views/logistics/remittance.html"),
    ),
    (
        "logistics/account_changes.html",
        include_str!("../views/logistics/account_changes.html"),
    ),
];

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .get("/remittances", index)
        .name("logistics.remittances")
        .get("/remittances/accounts", account_changes)
        .name("logistics.account_changes")
        .get("/remittances/{id}", show)
        .name("logistics.remittance")
        .post("/remittances/build", build)
        .name("logistics.remittance_build")
        .post("/remittances/{id}/approve", approve)
        .name("logistics.remittance_approve")
        .post("/remittances/{id}/reopen", reopen)
        .name("logistics.remittance_reopen")
        .post("/remittances/{id}/pay", pay)
        .name("logistics.remittance_pay")
        .post("/remittances/{id}/return-payment", return_payment)
        .name("logistics.remittance_return_payment")
        .post("/remittances/{id}/repay", repay)
        .name("logistics.remittance_repay")
        .post("/remittances/{id}/confirm-account", confirm_account)
        .name("logistics.remittance_confirm_account")
        .post("/remittances/accounts", request_account)
        .name("logistics.account_request")
        .post("/remittances/accounts/{id}/approve", approve_account)
        .name("logistics.account_approve")
        .post("/remittances/accounts/{id}/reject", reject_account)
        .name("logistics.account_reject")
        .post("/remittances/accounts/{id}/withdraw", withdraw_account)
        .name("logistics.account_withdraw")
        .get("/remittances/{id}/statement", statement_of)
        .name("logistics.remittance_statement")
        .post("/remittances/adjustments", add_adjustment)
        .name("logistics.adjustment_add")
        .post("/remittances/adjustments/{id}/approve", approve_adjustment)
        .name("logistics.adjustment_approve")
        .post("/remittances/adjustments/{id}/reject", reject_adjustment)
        .name("logistics.adjustment_reject")
        .post(
            "/remittances/adjustments/{id}/resubmit",
            resubmit_adjustment,
        )
        .name("logistics.adjustment_resubmit")
        .post("/remittances/adjustments/{id}/delete", delete_adjustment)
        .name("logistics.adjustment_delete")
        .require_auth()
}

/// The word of a status.
pub(crate) fn status_word(lang: &Lang, status: RemittanceStatus) -> String {
    t(
        lang,
        &format!("logistics.screen.remit_status.{}", status.as_str()),
    )
}

/// The badge colour of a status.
pub(crate) fn status_kind(status: RemittanceStatus) -> &'static str {
    match status {
        RemittanceStatus::Draft => "info",
        RemittanceStatus::Approved => "warning",
        RemittanceStatus::Paid => "success",
        RemittanceStatus::PaymentReturned => "error",
    }
}

fn answer(error: TransitionError) -> Error {
    error.into_error()
}

/// A redirect to the remittance's page with a toast.
fn back_to(lang: &Lang, key: &str, id: i64) -> Result<Response> {
    Ok((
        Toast::success(t(lang, key)),
        Redirect::route("logistics.remittance", &[&id])?,
    )
        .into_response())
}

/// The remittances grid (draft, approved, paid and returned in one list):
/// search on the number and client, a filter by status, sums of the money.
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    let text = |key: &str| crate::pages::text(lang, key);
    let statuses: Vec<(String, String)> = [
        RemittanceStatus::Draft,
        RemittanceStatus::Approved,
        RemittanceStatus::Paid,
        RemittanceStatus::PaymentReturned,
    ]
    .into_iter()
    .map(|status| (status.as_str().to_owned(), status_word(lang, status)))
    .collect();
    let grid = Grid::new("logistics-remittances")
        .title(&text("remit_title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("number", &text("remit_number"))
                .frozen()
                .mobile()
                .searchable()
                .copyable()
                .link("/remittances/{id}"),
        )
        .column(
            Column::related(
                "client",
                &text("client"),
                "wl_parties_parties",
                "client_party_id",
                "display_name",
            )
            .mobile()
            .searchable(),
        )
        .column(Column::date("period_end", &text("remit_period")).hidden())
        .column(
            Column::money("cod_total", &wl_kernel::money_label(&text("remit_cod")))
                .hidden()
                .summary(Summary::Sum),
        )
        .column(
            Column::money("fee_total", &wl_kernel::money_label(&text("remit_fees")))
                .hidden()
                .summary(Summary::Sum),
        )
        .column(
            Column::money(
                "adjustment_total",
                &wl_kernel::money_label(&text("remit_adjustments")),
            )
            .hidden()
            .summary(Summary::Sum),
        )
        .column(
            Column::money("net", &wl_kernel::money_label(&text("remit_net")))
                .mobile()
                .summary(Summary::Sum),
        )
        .column(
            Column::select("status", &text("state"), statuses)
                .badges(&[
                    ("draft", "info"),
                    ("approved", "warning"),
                    ("paid", "success"),
                    ("payment_returned", "danger"),
                ])
                .mobile(),
        )
        .sort_by("-number")
        .row_url("/remittances/{id}")
        .cards_on_mobile()
        .empty_state(&text("remit_empty"), Some(&text("remit_empty_text")));
    if can_export { grid.exports() } else { grid }
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "logistics/remittances.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    remittances: GridPage<Remittance>,
    open_account_changes: u64,
    can_build: bool,
    can_add_adjustment: bool,
    can_request_account: bool,
}

async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    if !user.has_permission(permissions::REMITTANCES_VIEW) {
        return Err(Error::Forbidden);
    }
    let can_export = user.has_permission(permissions::REMITTANCES_PREPARE);
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    let grid = grid(&lang, can_export);
    if let Some(file) = grid.export(Remittance::query(), &request).await? {
        return Ok(file);
    }
    let remittances = grid.page(Remittance::query(), &request).await?;
    // The bank account changes waiting have a page of their own; the header
    // links to it with their number.
    let open_account_changes = AccountChange::query()
        .where_(AccountChange::STATUS.is_in(AccountChangeStatus::OPEN))
        .count(&state.db)
        .await?;
    Ok(View::typed(IndexPage {
        tx: tx(&lang),
        remittances,
        open_account_changes,
        can_request_account: user.has_permission(permissions::REMITTANCES_REQUEST_ACCOUNT),
        can_build: user.has_permission(permissions::REMITTANCES_PREPARE),
        can_add_adjustment: user.has_permission(permissions::REMITTANCES_PREPARE),
    })
    .into_response())
}

/// The grid of the bank account changes waiting (pending, approved or
/// confirmed): search on the client and the account, a filter by state. The
/// decisions are in each row's menu, for the roles that may take them; the
/// server still refuses one the user may not take on that row (their own
/// request). Not exported: the rows carry bank accounts. A change has no
/// page of its own, so the rows link nowhere.
pub(crate) fn account_grid(lang: &Lang, can_decide: bool, can_withdraw: bool) -> Grid {
    let text = |key: &str| crate::pages::text(lang, key);
    let statuses: Vec<(String, String)> = AccountChangeStatus::OPEN
        .iter()
        .map(|status| {
            (
                status.as_str().to_owned(),
                t(
                    lang,
                    &format!("logistics.screen.account_status.{}", status.as_str()),
                ),
            )
        })
        .collect();
    let mut grid = Grid::new("logistics-account-changes")
        .title(&text("account_changes"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::related(
                "client",
                &text("client"),
                "wl_parties_parties",
                "client_party_id",
                "display_name",
            )
            .frozen()
            .mobile()
            .searchable(),
        )
        .column(
            Column::text("account", &text("remit_account"))
                .mobile()
                .searchable()
                .copyable(),
        )
        .column(
            Column::select("status", &text("state"), statuses)
                .badges(&[
                    ("pending", "warning"),
                    ("approved", "info"),
                    ("confirmed", "info"),
                ])
                .mobile(),
        )
        .column(Column::text("via", &text("account_via")).hidden())
        .column(Column::datetime("requested_at", &text("account_requested_at")).mobile())
        .column(Column::datetime("expires_at", &text("account_expires_at")).hidden())
        .sort_by("-id")
        .cards_on_mobile()
        .empty_state(
            &text("account_changes_empty"),
            Some(&text("account_changes_empty_text")),
        );
    if can_decide {
        grid = grid
            .row_action(
                Action::new(&text("account_approve"), "/remittances/accounts/{id}/approve")
                    .confirm(&text("account_approve_message")),
            )
            .row_action(
                Action::new(&text("account_reject"), "/remittances/accounts/{id}/reject")
                    .sheet("reject-account"),
            );
    }
    if can_withdraw {
        grid = grid.row_action(
            Action::new(&text("account_withdraw"), "/remittances/accounts/{id}/withdraw")
                .confirm(&text("account_withdraw_message"))
                .danger(),
        );
    }
    grid
}

/// What the page of the bank account changes needs.
#[derive(Serialize, ViewData)]
#[view(template = "logistics/account_changes.html")]
pub(crate) struct AccountChangesPage {
    tx: Texts,
    changes: GridPage<AccountChange>,
    can_decide: bool,
    can_request_account: bool,
}

/// `GET /remittances/accounts`: the bank account changes waiting.
async fn account_changes(user: AuthUser, lang: Lang, request: GridRequest) -> Result<Response> {
    if !user.has_permission(permissions::REMITTANCES_VIEW) {
        return Err(Error::Forbidden);
    }
    let can_decide = user.has_permission(permissions::REMITTANCES_APPROVE);
    let can_request_account = user.has_permission(permissions::REMITTANCES_REQUEST_ACCOUNT);
    let grid = account_grid(&lang, can_decide, can_request_account);
    let query =
        AccountChange::query().where_(AccountChange::STATUS.is_in(AccountChangeStatus::OPEN));
    let changes = grid.page(query, &request).await?;
    Ok(View::typed(AccountChangesPage {
        tx: tx(&lang),
        changes,
        can_decide,
        can_request_account,
    })
    .into_response())
}

/// A shipment line of the page.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct LineRow {
    shipment: String,
    shipment_url: String,
    cod_amount: i64,
    absorbed: i64,
    absorbed_reason: String,
    collected: i64,
    fee: i64,
    amount: i64,
}

/// An adjustment line of the page.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct AdjustmentRow {
    id: i64,
    amount: i64,
    reason: String,
    source: String,
    status: String,
    counts: bool,
    can_decide: bool,
    can_delete: bool,
}

/// An invoice offset of the page.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct OffsetRow {
    invoice: String,
    due_date: String,
    amount: i64,
    payment: String,
}

/// A return of the transfer, as the page lists it.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ReturnRow {
    advice: String,
    reason: String,
    bank_account: String,
    payment_reference: String,
}

/// What the remittance's page needs.
#[derive(Serialize, ViewData)]
#[view(template = "logistics/remittance.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    remittance: Remittance,
    status_label: String,
    status_kind: &'static str,
    lines: Vec<LineRow>,
    offsets: Vec<OffsetRow>,
    returns: Vec<ReturnRow>,
    /// The client is blocked and its payment waits: the refusal with the
    /// "Request exception" form (`PTY.ACL.03`).
    refusal: Option<BlockRefusalView>,
    can_dispute: bool,
    can_resolve: bool,
    can_return: bool,
    can_repay: bool,
    can_confirm_account: bool,
    adjustments: Vec<AdjustmentRow>,
    can_approve: bool,
    can_reopen: bool,
    can_pay: bool,
}

async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    if !user.has_permission(permissions::REMITTANCES_VIEW) {
        return Err(Error::Forbidden);
    }
    let remittance = Remittance::find_or_404(&state.db, id).await?;
    let mut lines = Vec::new();
    for line in remittance.lines(&state.db).await? {
        let number = Shipment::find(&state.db, line.shipment_id)
            .await?
            .map(|s| s.number)
            .unwrap_or_default();
        lines.push(LineRow {
            shipment_url: format!("/shipments/{}", line.shipment_id),
            shipment: number,
            cod_amount: line.cod_amount,
            absorbed: line.absorbed,
            absorbed_reason: line.absorbed_reason.clone(),
            collected: line.collected_for_client(),
            fee: line.fee,
            amount: line.amount,
        });
    }
    let offsets = remittance
        .offsets(&state.db)
        .await?
        .into_iter()
        .map(|offset| OffsetRow {
            invoice: offset.invoice_number,
            due_date: offset
                .due_date
                .map(|day| day.to_string())
                .unwrap_or_default(),
            amount: offset.amount,
            payment: offset.payment_number.unwrap_or_default(),
        })
        .collect();
    let returns = remittance
        .returns(&state.db)
        .await?
        .into_iter()
        .map(|returned| ReturnRow {
            advice: returned.advice,
            reason: returned.reason,
            bank_account: returned.bank_account.unwrap_or_default(),
            payment_reference: returned.payment_reference.unwrap_or_default(),
        })
        .collect();
    let adjustments = remittance
        .adjustments(&state.db)
        .await?
        .iter()
        .map(|line: &AdjustmentLine| AdjustmentRow {
            id: line.id,
            amount: line.amount,
            reason: line.reason.clone(),
            source: line.source.clone(),
            status: t(
                &lang,
                &format!(
                    "logistics.screen.adjustment_status.{}",
                    line.status.as_str()
                ),
            ),
            counts: line.counts(),
            can_decide: line.status == AdjustmentStatus::Pending
                && user.has_permission(permissions::REMITTANCES_APPROVE)
                && line.prepared_by != Some(user.id),
            can_delete: line.status != AdjustmentStatus::Approved
                && user.has_permission(permissions::REMITTANCES_PREPARE)
                && line.prepared_by == Some(user.id),
        })
        .collect();
    let waiting_to_pay = matches!(
        remittance.status,
        RemittanceStatus::Approved | RemittanceStatus::PaymentReturned
    );
    let refusal = if waiting_to_pay
        && !remittance.block_exception_granted
        && user.has_permission(permissions::REMITTANCES_PAY)
    {
        let mut tx = state.db.begin().await?;
        let outcome = block_guard::guard(&state)
            .check_in(
                &mut tx,
                Some(&user),
                remittance.client(),
                ActionKind::MoneyOut,
            )
            .await?;
        match outcome {
            GuardOutcome::Allowed => None,
            GuardOutcome::Blocked(refused) => {
                let step = if remittance.status == RemittanceStatus::Approved {
                    "pay"
                } else {
                    "repay"
                };
                Some(refused.view(
                    &lang,
                    &format!("/remittances/{}/{step}", remittance.id),
                    &[],
                ))
            }
        }
    } else {
        None
    };
    let draft = remittance.status == RemittanceStatus::Draft;
    let approved = remittance.status == RemittanceStatus::Approved;
    Ok(View::typed(ShowPage {
        tx: tx(&lang),
        status_label: status_word(&lang, remittance.status),
        status_kind: status_kind(remittance.status),
        lines,
        offsets,
        returns,
        refusal,
        can_dispute: matches!(
            remittance.status,
            RemittanceStatus::Approved | RemittanceStatus::Paid
        ) && user.has_permission(permissions::REMITTANCES_DISPUTE),
        can_resolve: remittance.disputed && user.has_permission(permissions::REMITTANCES_APPROVE),
        can_return: remittance.status == RemittanceStatus::Paid
            && user.has_permission(permissions::REMITTANCES_PAY),
        can_repay: remittance.status == RemittanceStatus::PaymentReturned
            && user.has_permission(permissions::REMITTANCES_PAY),
        can_confirm_account: remittance.status == RemittanceStatus::PaymentReturned
            && user.has_permission(permissions::REMITTANCES_PAY),
        adjustments,
        can_approve: draft
            && user.has_permission(permissions::REMITTANCES_APPROVE)
            && remittance.prepared_by != Some(user.id),
        can_reopen: approved && user.has_permission(permissions::REMITTANCES_APPROVE),
        can_pay: approved && user.has_permission(permissions::REMITTANCES_PAY),
        remittance,
    })
    .into_response())
}

/// `GET /remittances/{id}/statement`: the latest published statement as JSON,
/// for staff with `remittances.view` and for a client user of the company (the
/// portal screen is `POR.SLF.01`); none until the remittance is paid.
async fn statement_of(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Response> {
    let remittance = Remittance::find_or_404(&state.db, id).await?;
    let staff = user.has_permission(permissions::REMITTANCES_VIEW);
    if !staff && !booking::sees_for(&state, &user, remittance.client()).await? {
        return Err(Error::Forbidden);
    }
    match statement::view(&state.db, &remittance).await? {
        Some(found) => Ok(Json(found).into_response()),
        None => Err(Error::NotFound),
    }
}

/// `POST /remittances/build`: `client_party_id`.
async fn build(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Fields,
) -> Result<Response> {
    let client = field(&form, "client_party_id")
        .trim()
        .parse::<i64>()
        .map_err(|_| {
            Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                t(&lang, "logistics.bad_input"),
            )
        })?;
    let remittance = remit::build(&state, &user, PartyId(client))
        .await
        .map_err(answer)?;
    back_to(&lang, "logistics.remittance_built", remittance.id)
}

/// `POST /remittances/{id}/approve`.
async fn approve(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    remit::approve(&state, &user, id).await.map_err(answer)?;
    back_to(&lang, "logistics.remittance_approved", id)
}

/// `POST /remittances/{id}/reopen`.
async fn reopen(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    remit::reopen(&state, &user, id).await.map_err(answer)?;
    back_to(&lang, "logistics.remittance_reopened", id)
}

/// `POST /remittances/{id}/pay`: `payment_reference`, the bank reference.
async fn pay(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    if checked(&form, "block_exception") {
        remit::ask_exception(&state, &user, id)
            .await
            .map_err(answer)?;
        return back_to(&lang, "logistics.remittance_exception_asked", id);
    }
    remit::pay(&state, &user, id, &field(&form, "payment_reference"))
        .await
        .map_err(answer)?;
    back_to(&lang, "logistics.remittance_paid", id)
}

/// `POST /remittances/{id}/return-payment`: `advice` (the bank's return
/// advice) and `reason`.
async fn return_payment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    remit::return_payment(
        &state,
        &user,
        id,
        &field(&form, "advice"),
        &field(&form, "reason"),
    )
    .await
    .map_err(answer)?;
    back_to(&lang, "logistics.remittance_payment_returned", id)
}

/// `POST /remittances/{id}/repay`: `payment_reference`, the new bank
/// reference; `block_exception` asks for an exception while the client is
/// blocked.
async fn repay(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    if checked(&form, "block_exception") {
        remit::ask_exception(&state, &user, id)
            .await
            .map_err(answer)?;
        return back_to(&lang, "logistics.remittance_exception_asked", id);
    }
    remit::repay(&state, &user, id, &field(&form, "payment_reference"))
        .await
        .map_err(answer)?;
    back_to(&lang, "logistics.remittance_repaid", id)
}

/// `POST /remittances/{id}/confirm-account`: `note`, how the client
/// confirmed that the account is correct.
async fn confirm_account(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    remit::confirm_account(&state, &user, id, &field(&form, "note"))
        .await
        .map_err(answer)?;
    back_to(&lang, "logistics.remittance_account_confirmed", id)
}

/// `POST /remittances/accounts`: `client_party_id` and `account`, the new
/// IBAN.
async fn request_account(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Fields,
) -> Result<Response> {
    let client = field(&form, "client_party_id")
        .trim()
        .parse::<i64>()
        .map_err(|_| bad_input(&lang))?;
    account::request(&state, &user, PartyId(client), &field(&form, "account"))
        .await
        .map_err(answer)?;
    back_to_changes(&lang, "logistics.account_requested")
}

/// `POST /remittances/accounts/{id}/approve`.
async fn approve_account(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    account::approve_change(&state, &user, id)
        .await
        .map_err(answer)?;
    back_to_changes(&lang, "logistics.account_approved")
}

/// `POST /remittances/accounts/{id}/reject`: `note`.
async fn reject_account(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    account::reject_change(&state, &user, id, &field(&form, "note"))
        .await
        .map_err(answer)?;
    back_to_changes(&lang, "logistics.account_rejected")
}

/// `POST /remittances/accounts/{id}/withdraw`.
async fn withdraw_account(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    account::withdraw_change(&state, &user, id)
        .await
        .map_err(answer)?;
    back_to_changes(&lang, "logistics.account_withdrawn")
}

/// A whole number from a form; blank is none, anything else that is not a
/// number is `Err`.
fn id_in(
    form: &std::collections::BTreeMap<String, String>,
    name: &str,
) -> std::result::Result<Option<i64>, ()> {
    match field(form, name).trim() {
        "" => Ok(None),
        text => text.parse::<i64>().map(Some).map_err(|_| ()),
    }
}

fn bad_input(lang: &Lang) -> Error {
    Error::Status(
        StatusCode::UNPROCESSABLE_ENTITY,
        t(lang, "logistics.bad_input"),
    )
}

/// A redirect to the remittance list with a toast.
/// A redirect to the bank account changes, with a toast `key`.
fn back_to_changes(lang: &Lang, key: &str) -> Result<Response> {
    Ok((
        Toast::success(t(lang, key)),
        Redirect::route("logistics.account_changes", &[])?,
    )
        .into_response())
}

fn back_to_list(lang: &Lang, key: &str) -> Result<Response> {
    Ok((
        Toast::success(t(lang, key)),
        Redirect::route("logistics.remittances", &[])?,
    )
        .into_response())
}

/// `POST /remittances/adjustments`: `client_party_id`, `amount` (dirhams,
/// minus to take back), `reason`, `source` (blank: manual), and optionally
/// `shipment_id`, `corrects_remittance_id` and `remittance_id` (a draft to
/// put it on now).
async fn add_adjustment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Fields,
) -> Result<Response> {
    let client = field(&form, "client_party_id")
        .trim()
        .parse::<i64>()
        .map_err(|_| bad_input(&lang))?;
    let amount = adjustment::parse_signed_dirhams(&field(&form, "amount"))
        .ok_or_else(|| bad_input(&lang))?;
    let new = NewAdjustment {
        client: PartyId(client),
        amount,
        reason: field(&form, "reason"),
        source: field(&form, "source"),
        shipment_id: id_in(&form, "shipment_id").map_err(|()| bad_input(&lang))?,
        corrects_remittance_id: id_in(&form, "corrects_remittance_id")
            .map_err(|()| bad_input(&lang))?,
        remittance_id: id_in(&form, "remittance_id").map_err(|()| bad_input(&lang))?,
    };
    let line = adjustment::add(&state, &user, &new).await.map_err(answer)?;
    match line.remittance_id {
        Some(id) => back_to(&lang, "logistics.adjustment_added", id),
        None => back_to_list(&lang, "logistics.adjustment_added"),
    }
}

/// Where a decision on `line` goes back to.
fn back_after(lang: &Lang, key: &str, line: &AdjustmentLine) -> Result<Response> {
    match line.remittance_id {
        Some(id) => back_to(lang, key, id),
        None => back_to_list(lang, key),
    }
}

/// `POST /remittances/adjustments/{id}/approve`: `note`.
async fn approve_adjustment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let line = adjustment::approve_line(&state, &user, id, &field(&form, "note"))
        .await
        .map_err(answer)?;
    back_after(&lang, "logistics.adjustment_approved", &line)
}

/// `POST /remittances/adjustments/{id}/reject`: `note`.
async fn reject_adjustment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let line = adjustment::reject_line(&state, &user, id, &field(&form, "note"))
        .await
        .map_err(answer)?;
    back_after(&lang, "logistics.adjustment_rejected", &line)
}

/// `POST /remittances/adjustments/{id}/resubmit`: `amount` and `reason`, the
/// preparer's corrected line.
async fn resubmit_adjustment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let amount = adjustment::parse_signed_dirhams(&field(&form, "amount"))
        .ok_or_else(|| bad_input(&lang))?;
    let line = adjustment::resubmit_line(&state, &user, id, amount, &field(&form, "reason"))
        .await
        .map_err(answer)?;
    back_after(&lang, "logistics.adjustment_resubmitted", &line)
}

/// `POST /remittances/adjustments/{id}/delete`.
async fn delete_adjustment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    adjustment::delete_line(&state, &user, id)
        .await
        .map_err(answer)?;
    back_to_list(&lang, "logistics.adjustment_deleted")
}
