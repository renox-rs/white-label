//! The deposit pages (wl-payments §6, A10):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /deposits` | `wl_payments.deposits.index` | `deposits.view` |
//! | `GET /deposits/{id}` | `wl_payments.deposits.show` | `deposits.view` |
//! | `POST /deposits/{id}/deductions` | `wl_payments.deposits.deductions` | `deposits.manage` (fields `lines`, `forfeiture_reason`) |
//! | `POST /deposits/{id}/answer` | `wl_payments.deposits.answer` | `deposits.manage` (fields `response`, `evidence`, `changes`) |
//! | `POST /deposits/{id}/legal_case/open` and `/close` | `wl_payments.deposits.legal_case_open` and `_close` | `deposits.manage` |
//! | `POST /deposits/{id}/refund` | `wl_payments.deposits.refund` | `deposits.refund` (field `method`) |
//! | `POST /deposits/{id}/forfeit` | `wl_payments.deposits.forfeit` | `deposits.refund` (fields `reason`, `approver`) |
//!
//! A `refunded` deposit whose refund `failed` shows a "Refund failed" badge
//! with a link to the refund; the badge is read from the refund, so the
//! deposit stays `refunded`.

use renox::auth::permissions::users_with_permission;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{self, TransitionError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use wl_kernel::deposits::DeductionKind;
use wl_kernel::payments::PaymentMethod;

use crate::deposit::{Deposit, DepositStatus, ForfeitRequest};
use crate::deposit_deductions::{
    self, AnswerRequest, DepositDeduction, DepositDispute, DisputeStatus, LineChange, NewLine,
    PublishRequest,
};
use crate::followup::money_text;
use crate::pages::{Text, Texts, filled, invalid, options, parse_amount, party_name, texts_in};
use crate::permissions;
use crate::texts;

/// Every deposit status.
const STATUSES: [DepositStatus; 5] = [
    DepositStatus::Held,
    DepositStatus::SettlementPending,
    DepositStatus::Refunded,
    DepositStatus::Forfeited,
    DepositStatus::Transferred,
];

/// The module's deposit routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/deposits", index)
                .name("wl_payments.deposits.index")
                .get("/deposits/{id}", show)
                .name("wl_payments.deposits.show")
                .require_permission(permissions::DEPOSITS_VIEW),
        )
        .merge(
            Routes::new()
                .post("/deposits/{id}/deductions", store_deductions)
                .name("wl_payments.deposits.deductions")
                .post("/deposits/{id}/answer", store_answer)
                .name("wl_payments.deposits.answer")
                .post("/deposits/{id}/legal_case/open", store_legal_case_open)
                .name("wl_payments.deposits.legal_case_open")
                .post("/deposits/{id}/legal_case/close", store_legal_case_close)
                .name("wl_payments.deposits.legal_case_close")
                .require_permission(permissions::DEPOSITS_MANAGE),
        )
        .merge(
            Routes::new()
                .post("/deposits/{id}/refund", store_refund)
                .name("wl_payments.deposits.refund")
                .post("/deposits/{id}/forfeit", store_forfeit)
                .name("wl_payments.deposits.forfeit")
                .require_permission(permissions::DEPOSITS_REFUND),
        )
}

/// The badge colour of a status.
fn status_kind(status: DepositStatus) -> &'static str {
    match status {
        DepositStatus::Held => "info",
        DepositStatus::SettlementPending => "warning",
        DepositStatus::Refunded => "success",
        DepositStatus::Forfeited => "error",
        DepositStatus::Transferred => "neutral",
    }
}

/// The deposits grid: party, source, amounts, status and whether the
/// refund failed.
pub fn grid(lang: &Lang) -> Grid {
    grid_with(&texts_in(lang))
}

/// [`grid`] with the texts of `t`.
fn grid_with(t: Text<'_>) -> Grid {
    let codes: Vec<&str> = STATUSES.iter().map(|s| s.as_str()).collect();
    Grid::new("wl-payment-deposits")
        .title(&t("deposits.title"))
        .column(
            Column::number("id", "#")
                .frozen()
                .mobile()
                .link("/deposits/{id}"),
        )
        .column(
            Column::text("party_name", &t("fields.party"))
                .computed(
                    "(SELECT \"wl_parties_parties\".\"display_name\" FROM \"wl_parties_parties\" \
                     WHERE \"wl_parties_parties\".\"id\" = {T}.\"party_id\")",
                )
                .mobile()
                .searchable(),
        )
        .column(Column::money("amount", &t("fields.amount")).currency_from("currency"))
        .column(Column::money("held_amount", &t("deposits.held")).currency_from("currency"))
        .column(
            Column::select(
                "status",
                &t("fields.status"),
                options(&t, "deposits.status", &codes),
            )
            .badges(&[
                ("held", "info"),
                ("settlement_pending", "warning"),
                ("refunded", "success"),
                ("forfeited", "danger"),
                ("transferred", "neutral"),
            ])
            .mobile(),
        )
        .column(
            Column::select(
                "refund_state",
                &t("deposits.refund_state"),
                vec![("failed".to_owned(), t("deposits.refund_failed"))],
            )
            .computed(
                "CASE WHEN EXISTS (SELECT 1 FROM \"wl_payments_refunds\" \"r\" \
                 WHERE \"r\".\"id\" = {T}.\"refund_id\" AND \"r\".\"status\" = 'failed') \
                 THEN 'failed' ELSE '' END",
            )
            .badges(&[("failed", "danger")])
            .mobile(),
        )
        .column(Column::datetime("created_at", &t("fields.created_at")).hidden())
        .sort_by("-created_at")
        .row_url("/deposits/{id}")
        .cards_on_mobile()
        .empty_state(&t("deposits.empty"), Some(&t("deposits.empty_hint")))
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-payments/deposits.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    deposits: GridPage<Deposit>,
}

/// `GET /deposits`: the grid.
async fn index(lang: Lang, request: GridRequest) -> Result<View> {
    let deposits = grid(&lang).page(Deposit::query(), &request).await?;
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        deposits,
    }))
}

/// What the deposit page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-payments/deposit.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    deposit: Deposit,
    title: String,
    status_label: String,
    status_kind: &'static str,
    party_name: String,
    source_label: String,
    method_label: String,
    /// Amount and held amount, formatted.
    amounts: BTreeMap<&'static str, String>,
    /// Whether the linked refund `failed`.
    refund_failed: bool,
    refund_label: String,
    refund_link: Option<String>,
    timeline: Vec<Value>,
    /// The published versions of the deductions, newest first.
    versions: Vec<VersionRow>,
    /// The disputes of the party, oldest first.
    disputes: Vec<DisputeRow>,
    /// Whether the deductions dispute flag is set: a dispute is open or a
    /// legal case holds it.
    dispute_flag: bool,
    /// Whether "Publish deductions" is offered.
    can_publish: bool,
    /// Whether "Answer the dispute" is offered.
    can_answer: bool,
    /// Whether the legal case buttons are offered, and which one.
    can_legal_case: bool,
    /// Whether "Refund" and "Forfeit" are offered.
    can_refund: bool,
    can_forfeit: bool,
    /// Why they wait, as text; empty when they do not.
    waiting_reason: String,
    /// The methods a deposit is refunded by, `(code, label)`.
    refund_methods: Vec<(String, String)>,
    /// The users who may approve a forfeiture, `(id, name)`.
    approvers: Vec<(String, String)>,
}

/// One line of a published version.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct LineRow {
    id: i64,
    description: String,
    amount: String,
    evidence: String,
    disputed: bool,
}

/// One published version of the deductions.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct VersionRow {
    version: i32,
    kind: String,
    reason: String,
    published_on: String,
    window_ends_on: String,
    total: String,
    latest: bool,
    lines: Vec<LineRow>,
}

/// One dispute of a line.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct DisputeRow {
    id: i64,
    line: String,
    comment: String,
    raised_on: String,
    status: String,
    status_kind: &'static str,
    response: String,
}

/// The rows of the versions of `deposit`, newest first.
async fn version_rows(
    db: &Db,
    t: Text<'_>,
    deposit: &Deposit,
    open_lines: &[i64],
) -> Result<Vec<VersionRow>> {
    let versions = DepositDeduction::of_deposit(db, deposit.id).await?;
    let newest = versions.last().map(|v| v.version);
    let mut rows = Vec::new();
    for version in versions.iter().rev() {
        let lines = version
            .lines(db)
            .await?
            .iter()
            .map(|line| LineRow {
                id: line.id,
                description: line.description.clone(),
                amount: money_text(&deposit.currency, line.amount),
                evidence: line.evidence.clone(),
                disputed: open_lines.contains(&line.id),
            })
            .collect();
        rows.push(VersionRow {
            version: version.version,
            kind: t(match version.kind {
                DeductionKind::Deductions => "deposits.deductions.kind.deductions",
                DeductionKind::Forfeiture => "deposits.deductions.kind.forfeiture",
            }),
            reason: version.reason.clone().unwrap_or_default(),
            published_on: version.published_on.format("%Y-%m-%d").to_string(),
            window_ends_on: version
                .window_ends_on
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_default(),
            total: money_text(&deposit.currency, version.total),
            latest: Some(version.version) == newest,
            lines,
        });
    }
    Ok(rows)
}

/// The rows of the disputes of `deposit`.
async fn dispute_rows(db: &Db, t: Text<'_>, deposit: &Deposit) -> Result<Vec<DisputeRow>> {
    let mut rows = Vec::new();
    for dispute in DepositDispute::of_deposit(db, deposit.id).await? {
        let line = deposit_deductions::DeductionLine::find(db, dispute.line_id)
            .await?
            .map_or_else(|| format!("#{}", dispute.line_id), |l| l.description);
        rows.push(DisputeRow {
            id: dispute.id,
            line,
            comment: dispute.comment.clone(),
            raised_on: dispute.raised_on.format("%Y-%m-%d").to_string(),
            status: t(&format!("deposits.dispute.status.{}", dispute.status.as_str())),
            status_kind: match dispute.status {
                DisputeStatus::Open => "warning",
                DisputeStatus::Revised => "info",
                DisputeStatus::Upheld => "success",
            },
            response: dispute.response.clone().unwrap_or_default(),
        });
    }
    Ok(rows)
}

/// `GET /deposits/{id}`: the card, the "Refund failed" badge with its link
/// and the history.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(deposit): Found<Deposit>,
) -> Result<View> {
    let t = texts_in(&lang);
    let db = state.db.clone();
    let open = DepositDispute::open_of_deposit(&db, deposit.id).await?;
    let open_lines: Vec<i64> = open.iter().map(|d| d.line_id).collect();
    let pending = deposit.status == DepositStatus::SettlementPending;
    let manage = user.has_permission(permissions::DEPOSITS_MANAGE);
    let refunding = user.has_permission(permissions::DEPOSITS_REFUND);
    let gate = deposit_deductions::gate_of(&state, &deposit).await?;
    let waiting_reason = if pending {
        deposit_deductions::gate_problem(&gate, false).unwrap_or_default()
    } else {
        String::new()
    };
    let approvers: Vec<(String, String)> =
        users_with_permission(&db, permissions::DEPOSITS_REFUND)
            .await?
            .into_iter()
            .filter(|u| u.id != user.id)
            .map(|u| (u.id.to_string(), u.name))
            .collect();
    let refund = deposit.linked_refund(&db).await?;
    let refund_failed = deposit.refund_failed(&db).await?;
    let history = state_machine::history(&db, Deposit::TABLE, deposit.id).await?;
    let mut timeline =
        state_machine::timeline(&history, Some(&state_machine::info_of::<Deposit>()));
    timeline.sort_by(|a, b| a["time"].as_str().cmp(&b["time"].as_str()));
    let source_label = format!("{} #{}", deposit.source_type, deposit.source_id);
    let amounts = BTreeMap::from([
        ("amount", money_text(&deposit.currency, deposit.amount)),
        ("held", money_text(&deposit.currency, deposit.held_amount)),
    ]);
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        title: format!("{} #{}", t("deposits.heading"), deposit.id),
        status_label: t(&format!("deposits.status.{}", deposit.status.as_str())),
        status_kind: status_kind(deposit.status),
        party_name: party_name(&state, deposit.party()).await?,
        source_label,
        method_label: deposit
            .refund_method
            .map(|m: PaymentMethod| t(&format!("method.{}", m.as_str())))
            .unwrap_or_default(),
        amounts,
        refund_failed,
        refund_label: refund
            .as_ref()
            .map(|r| {
                format!(
                    "{} #{} ({})",
                    t("refunds.heading"),
                    r.id,
                    t(&format!("refunds.status.{}", r.status.as_str()))
                )
            })
            .unwrap_or_default(),
        refund_link: refund.as_ref().map(|r| format!("/refunds/{}", r.id)),
        timeline,
        versions: version_rows(&db, &t, &deposit, &open_lines).await?,
        disputes: dispute_rows(&db, &t, &deposit).await?,
        dispute_flag: !open.is_empty() || deposit.dispute_legal_case,
        can_publish: manage && pending && open.is_empty(),
        can_answer: manage && pending && !open.is_empty(),
        can_legal_case: manage && pending,
        can_refund: refunding && pending && deposit.refund_id.is_none(),
        can_forfeit: refunding && pending,
        waiting_reason,
        refund_methods: options(&t, "method", &["transfer", "cheque"]),
        approvers,
        deposit,
    }))
}

/// The deductions sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct DeductionsForm {
    /// One deduction per line: `description | amount | evidence`, the amount
    /// in dirhams like `2500.00`.
    #[validate(max = 8000)]
    pub lines: Option<String>,
    /// Set to publish a forfeiture instead, with its reason; with no lines it
    /// is one line for the whole deposit.
    #[validate(max = 2000)]
    pub forfeiture_reason: Option<String>,
}

/// The lines in `text`, one `description | amount | evidence` per row; or the
/// message about the first row that is wrong.
pub(crate) fn parse_lines(text: &str) -> std::result::Result<Vec<NewLine>, String> {
    let mut lines = Vec::new();
    for (i, row) in text.lines().enumerate() {
        if row.trim().is_empty() {
            continue;
        }
        let parts: Vec<&str> = row.split('|').map(str::trim).collect();
        let [description, amount, evidence] = parts.as_slice() else {
            return Err(format!(
                "Line {}: write it as description | amount | evidence.",
                i + 1
            ));
        };
        let Some(amount) = parse_amount(amount) else {
            return Err(format!("Line {}: the amount is not a number above 0.", i + 1));
        };
        lines.push(NewLine {
            description: (*description).to_owned(),
            amount,
            evidence: (*evidence).to_owned(),
            billable: false,
        });
    }
    Ok(lines)
}

/// `POST /deposits/{id}/deductions`: publishes the next version of the
/// deductions (or the forfeiture) to the party and reloads the page.
async fn store_deductions(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(deposit): Found<Deposit>,
    Valid(form): Valid<DeductionsForm>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let lines = parse_lines(filled(&form.lines).unwrap_or_default())
        .map_err(|message| invalid("lines", message, &form))?;
    let request = PublishRequest {
        lines,
        forfeiture_reason: filled(&form.forfeiture_reason).map(str::to_owned),
    };
    match deposit_deductions::publish(&state, &user, &deposit, &request).await {
        Ok(_) => Ok((Toast::success(t("deposits.deductions.done")), renox::HxRefresh).into_response()),
        Err(TransitionError::Guard(message)) => Err(invalid("lines", message, &form)),
        Err(other) => Err(other.into_error()),
    }
}

/// The answer sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct AnswerForm {
    /// The answer to the party.
    #[validate(max = 4000)]
    pub response: Option<String>,
    /// The evidence reference.
    #[validate(max = 2000)]
    pub evidence: Option<String>,
    /// One revised line per row: `line id | new amount`, in dirhams (0
    /// removes the line). Empty: the deductions are upheld.
    #[validate(max = 2000)]
    pub changes: Option<String>,
}

/// The changes in `text`, one `line id | new amount` per row.
pub(crate) fn parse_changes(text: &str) -> std::result::Result<Vec<LineChange>, String> {
    let mut changes = Vec::new();
    for (i, row) in text.lines().enumerate() {
        if row.trim().is_empty() {
            continue;
        }
        let parts: Vec<&str> = row.split('|').map(str::trim).collect();
        let [id, amount] = parts.as_slice() else {
            return Err(format!("Row {}: write it as line id | new amount.", i + 1));
        };
        let Ok(line_id) = id.parse::<i64>() else {
            return Err(format!("Row {}: the line id is not a number.", i + 1));
        };
        let fils = if amount.trim_matches(|c| c == '0' || c == '.' || c == ',').is_empty() {
            Some(0)
        } else {
            parse_amount(amount)
        };
        let Some(amount) = fils else {
            return Err(format!("Row {}: the amount is not a number.", i + 1));
        };
        changes.push(LineChange {
            line_id,
            amount,
            ..Default::default()
        });
    }
    Ok(changes)
}

/// `POST /deposits/{id}/answer`: answers the open disputes (revising the
/// changed lines into a new version, upholding the rest) and reloads the page.
async fn store_answer(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(deposit): Found<Deposit>,
    Valid(form): Valid<AnswerForm>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let changes = parse_changes(filled(&form.changes).unwrap_or_default())
        .map_err(|message| invalid("changes", message, &form))?;
    let request = AnswerRequest {
        response: filled(&form.response).unwrap_or_default().to_owned(),
        evidence: filled(&form.evidence).unwrap_or_default().to_owned(),
        changes,
    };
    match deposit_deductions::answer(&state, &user, &deposit, &request).await {
        Ok(_) => Ok((Toast::success(t("deposits.answer.done")), renox::HxRefresh).into_response()),
        Err(TransitionError::Guard(message)) => {
            let field = if message.contains("evidence") {
                "evidence"
            } else if message.contains("line") || message.contains("amount") {
                "changes"
            } else {
                "response"
            };
            Err(invalid(field, message, &form))
        }
        Err(other) => Err(other.into_error()),
    }
}

/// Records that a legal case is open on the source agreement (`open`), or
/// that its decision is recorded.
async fn legal_case(
    state: &AppState,
    user: &AuthUser,
    lang: &Lang,
    deposit: &Deposit,
    open: bool,
) -> Result<Response> {
    deposit_deductions::set_legal_case(state, user, deposit, open)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(texts::t(lang, "wl_payments.deposits.legal_case.done")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /deposits/{id}/legal_case/open`: a legal case is open on the source
/// agreement; the dispute flag stays until its decision is recorded.
async fn store_legal_case_open(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(deposit): Found<Deposit>,
) -> Result<Response> {
    legal_case(&state, &user, &lang, &deposit, true).await
}

/// `POST /deposits/{id}/legal_case/close`: the decision of the legal case is
/// recorded.
async fn store_legal_case_close(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(deposit): Found<Deposit>,
) -> Result<Response> {
    legal_case(&state, &user, &lang, &deposit, false).await
}

/// The refund form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct RefundForm {
    /// `transfer` or `cheque`.
    #[validate(max = 32)]
    pub method: Option<String>,
}

/// `POST /deposits/{id}/refund`: refunds the deposit by transfer or cheque. A
/// refused guard (a dispute open, the window not over) answers 422.
async fn store_refund(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(deposit): Found<Deposit>,
    Valid(form): Valid<RefundForm>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let method = filled(&form.method).and_then(|code| {
        [PaymentMethod::Transfer, PaymentMethod::Cheque]
            .into_iter()
            .find(|m| m.as_str() == code)
    });
    match crate::deposit::refund(&state, &user, &deposit, method).await {
        Ok(_) => Ok((Toast::success(t("deposits.refund.done")), renox::HxRefresh).into_response()),
        Err(TransitionError::Guard(message)) => Err(invalid("method", message, &form)),
        Err(other) => Err(other.into_error()),
    }
}

/// The forfeit form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct ForfeitForm {
    /// Why the deposit is kept.
    #[validate(max = 2000)]
    pub reason: Option<String>,
    /// The second user who approves it.
    pub approver: Option<i64>,
}

/// `POST /deposits/{id}/forfeit`: forfeits the deposit. A refused guard (no
/// reason or approver, a dispute open, the forfeiture not published or its
/// window not over) answers 422.
async fn store_forfeit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(deposit): Found<Deposit>,
    Valid(form): Valid<ForfeitForm>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let request = ForfeitRequest {
        reason: filled(&form.reason).unwrap_or_default().to_owned(),
        approver: form.approver,
    };
    match crate::deposit::forfeit(&state, &user, &deposit, &request).await {
        Ok(_) => Ok((Toast::success(t("deposits.forfeit.done")), renox::HxRefresh).into_response()),
        Err(TransitionError::Guard(message)) => {
            let field = if message.contains("approval") {
                "approver"
            } else {
                "reason"
            };
            Err(invalid(field, message, &form))
        }
        Err(other) => Err(other.into_error()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn english_text(key: &str) -> String {
        texts::english(&format!("wl_payments.{key}")).to_owned()
    }

    #[test]
    fn the_grid_lists_the_deposit_columns() {
        let grid = grid_with(&english_text);
        let keys: Vec<&str> = grid.columns().iter().map(Column::key).collect();
        assert_eq!(
            keys,
            [
                "id",
                "party_name",
                "amount",
                "held_amount",
                "status",
                "refund_state",
                "created_at"
            ]
        );
    }

    #[test]
    fn every_status_has_a_label() {
        for status in STATUSES {
            let key = format!("deposits.status.{}", status.as_str());
            assert_ne!(english_text(&key), format!("wl_payments.{key}"), "{key}");
        }
    }

    #[test]
    fn deduction_lines_are_read_one_per_row() {
        let lines = parse_lines(
            "Repainting | 2,500.00 | photos-1\n\nCleaning|300|photos-2\n",
        )
        .expect("ok");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].description, "Repainting");
        assert_eq!(lines[0].amount, 250_000);
        assert_eq!(lines[1].evidence, "photos-2");
        assert!(parse_lines("Repainting | 2500").is_err());
        assert!(parse_lines("Repainting | lots | photos").is_err());
        assert!(parse_lines("Repainting | 0 | photos").is_err());
        assert!(parse_lines("").expect("ok").is_empty());
    }

    #[test]
    fn changes_are_read_as_line_id_and_new_amount() {
        let changes = parse_changes("12 | 800\n13 | 0").expect("ok");
        assert_eq!(changes.len(), 2);
        assert_eq!((changes[0].line_id, changes[0].amount), (12, 80_000));
        assert_eq!((changes[1].line_id, changes[1].amount), (13, 0));
        assert!(parse_changes("x | 800").is_err());
        assert!(parse_changes("12").is_err());
        assert!(parse_changes("12 | lots").is_err());
    }

    #[test]
    fn the_texts_of_the_deductions_exist() {
        for key in [
            "deposits.dispute.status.open",
            "deposits.dispute.status.revised",
            "deposits.dispute.status.upheld",
            "deposits.deductions.kind.deductions",
            "deposits.deductions.kind.forfeiture",
        ] {
            assert_ne!(english_text(key), format!("wl_payments.{key}"), "{key}");
        }
    }
}
