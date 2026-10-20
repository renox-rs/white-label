//! The refund pages (wl-payments §6, A10):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /refunds` | `wl_payments.refunds.index` | `payments.view` |
//! | `GET /refunds/{id}` | `wl_payments.refunds.show` | `payments.view` |
//! | `POST /refunds/{id}/fail` | `wl_payments.refunds.fail` | `refunds.pay` |
//! | `POST /refunds/{id}/retry` | `wl_payments.refunds.retry` | `refunds.pay` |
//! | `POST /refunds/{id}/change-method` | `wl_payments.refunds.change_method` | `refunds.pay` (fields `method`, `details`) |
//! | `POST /refunds/{id}/cancel` | `wl_payments.refunds.cancel` | `refunds.approve` (field `reason`) |
//! | `POST /refunds/{id}/transitions/{name}` | `wl_payments.refunds.transitions.*` | `payments.view` and the transition's permission |
//!
//! `fail` has its own route because it takes a kind and a reason, and so do
//! `retry`, `change_method` and `cancel` of a failed refund (they do more
//! than move the status: a gateway call, the party's notice, the accountant's
//! task); the kernel's transition route serves `<rx-transition-actions>` for
//! `approve` and `reject`. A refused guard answers 422 under the field.

use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{self, Available, StateMachine, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::invoicing::CREDIT_NOTE_SUBJECT_TYPE;
use wl_kernel::payments::{
    DEPOSIT_SUBJECT_TYPE, PAYMENT_SUBJECT_TYPE, PaymentMethod, RefundFailureKind,
};

use crate::followup::money_text;
use crate::pages::{Text, Texts, filled, invalid, options, party_name, texts_in};
use crate::permissions;
use crate::refund::{Refund, RefundStatus, fail};
use crate::refund_attempt::{AttemptOutcome, RefundAttempt};
use crate::refund_fail::FAIL;
use crate::refund_machine::PAY;
use crate::refund_recover::{
    self, CANCEL, CHANGE_METHOD, CHANGE_METHOD_FOR_APPROVAL, CHANGE_METHODS, RETRY,
};
use crate::texts;

/// The kinds a user may give, by code.
fn manual_kind_codes() -> Vec<&'static str> {
    RefundFailureKind::ALL
        .iter()
        .filter(|kind| kind.is_manual())
        .map(|kind| kind.as_str())
        .collect()
}

/// The module's refund routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .merge(wl_kernel::scopes::anywhere(
            Routes::new()
                .get("/refunds", index)
                .name("wl_payments.refunds.index")
                .get("/refunds/{id}", show)
                .name("wl_payments.refunds.show"),
            permissions::VIEW,
        ))
        .merge(
            Routes::new()
                .group(
                    "/refunds",
                    "wl_payments.refunds.transitions.",
                    wl_kernel::machines::routes::<Refund>(),
                )
                .require_permission(permissions::VIEW),
        )
        .merge(
            Routes::new()
                .post("/refunds/{id}/fail", store_fail)
                .name("wl_payments.refunds.fail")
                .post("/refunds/{id}/retry", store_retry)
                .name("wl_payments.refunds.retry")
                .post("/refunds/{id}/change-method", store_change_method)
                .name("wl_payments.refunds.change_method")
                .require_permission(permissions::REFUNDS_PAY),
        )
        .merge(
            Routes::new()
                .post("/refunds/{id}/cancel", store_cancel)
                .name("wl_payments.refunds.cancel")
                .require_permission(permissions::REFUNDS_APPROVE),
        )
}

/// The badge colour of a status.
pub(crate) fn status_kind(status: RefundStatus) -> &'static str {
    match status {
        RefundStatus::Requested => "warning",
        RefundStatus::Approved => "info",
        RefundStatus::Paid => "success",
        RefundStatus::Failed => "error",
        RefundStatus::Rejected | RefundStatus::Cancelled => "neutral",
    }
}

/// Every refund status.
const STATUSES: [RefundStatus; 6] = [
    RefundStatus::Requested,
    RefundStatus::Approved,
    RefundStatus::Paid,
    RefundStatus::Failed,
    RefundStatus::Rejected,
    RefundStatus::Cancelled,
];

/// The refund status codes.
fn status_codes() -> Vec<&'static str> {
    STATUSES.iter().map(|s| s.as_str()).collect()
}

/// The source kind codes.
pub(crate) const SOURCE_KINDS: [&str; 7] = [
    "payment",
    "credit_note",
    "deposit",
    "party_credit",
    "payer_credit",
    "order",
    "booking",
];

/// The refunds grid: party, source, amount, method, status; filters by
/// status, source and method.
pub fn grid(lang: &Lang) -> Grid {
    grid_with(&texts_in(lang))
}

/// [`grid`] with the texts of `t`.
fn grid_with(t: Text<'_>) -> Grid {
    let methods: Vec<&str> = PaymentMethod::ALL.iter().map(|m| m.as_str()).collect();
    Grid::new("wl-payment-refunds")
        .title(&t("refunds.title"))
        .column(
            Column::number("id", "#")
                .frozen()
                .mobile()
                .link("/refunds/{id}"),
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
        .column(Column::select(
            "source_kind",
            &t("refunds.source"),
            options(&t, "refunds.source_kind", &SOURCE_KINDS),
        ))
        .column(Column::money("amount", &t("fields.amount")).currency_from("currency"))
        .column(Column::select(
            "method",
            &t("fields.method"),
            options(&t, "method", &methods),
        ))
        .column(
            Column::select(
                "status",
                &t("fields.status"),
                options(&t, "refunds.status", &status_codes()),
            )
            .badges(&[
                ("requested", "warning"),
                ("approved", "info"),
                ("paid", "success"),
                ("failed", "danger"),
                ("rejected", "neutral"),
                ("cancelled", "neutral"),
            ])
            .mobile(),
        )
        .column(Column::datetime("created_at", &t("fields.created_at")).hidden())
        .sort_by("-created_at")
        .row_url("/refunds/{id}")
        .cards_on_mobile()
        .empty_state(&t("refunds.empty"), Some(&t("refunds.empty_hint")))
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-payments/refunds.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    refunds: GridPage<Refund>,
}

/// `GET /refunds`: the grid.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<View> {
    // The refunds of the payments in the user's scope.
    let reach = wl_kernel::scopes::reach(&state.db, &user, permissions::VIEW).await?;
    let query = match crate::scoped::refund_ids(&state.db, &reach).await? {
        None => Refund::query(),
        Some(ids) => Refund::query().where_in("id", ids),
    };
    let refunds = grid(&lang).page(query, &request).await?;
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        refunds,
    }))
}

/// One attempt as the refund page shows it.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct AttemptRow {
    id: i64,
    number: i32,
    method: String,
    reference: Option<String>,
    gateway_ref: Option<String>,
    outcome: String,
    outcome_kind: &'static str,
    failure_kind: String,
    failure_reason: String,
    failed_on: String,
}

/// The row for `attempt`.
fn attempt_row(t: Text<'_>, attempt: &RefundAttempt) -> AttemptRow {
    AttemptRow {
        id: attempt.id,
        number: attempt.number,
        method: t(&format!("method.{}", attempt.method.as_str())),
        reference: attempt.reference.clone(),
        gateway_ref: attempt.gateway_ref.clone(),
        outcome: t(&format!("refunds.outcome.{}", attempt.outcome.as_str())),
        outcome_kind: match attempt.outcome {
            AttemptOutcome::Pending => "info",
            AttemptOutcome::Paid => "success",
            AttemptOutcome::Failed => "error",
        },
        failure_kind: attempt
            .failure_kind
            .map(|kind| t(&format!("refunds.kind.{}", kind.as_str())))
            .unwrap_or_default(),
        failure_reason: attempt.failure_reason.clone().unwrap_or_default(),
        failed_on: attempt
            .failed_on
            .map(|day| day.format("%Y-%m-%d").to_string())
            .unwrap_or_default(),
    }
}

/// The back-office page of a source, when this module knows it.
fn source_link(source_type: &str, source_id: i64) -> Option<String> {
    match source_type {
        PAYMENT_SUBJECT_TYPE => Some(format!("/payments/{source_id}")),
        DEPOSIT_SUBJECT_TYPE => Some(format!("/deposits/{source_id}")),
        CREDIT_NOTE_SUBJECT_TYPE => Some(format!("/credit-notes/{source_id}")),
        _ => None,
    }
}

/// What the refund page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-payments/refund.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    refund: Refund,
    title: String,
    status_label: String,
    status_kind: &'static str,
    party_name: String,
    amount: String,
    method_label: String,
    source_label: String,
    source_link: Option<String>,
    attempts: Vec<AttemptRow>,
    timeline: Vec<Value>,
    actions: Vec<Available>,
    can_fail: bool,
    /// The kinds a user may give, `(code, label)`.
    kinds: Vec<(String, String)>,
    /// Whether "Retry" is offered: failed, and retries are left.
    can_retry: bool,
    /// Whether "Change method" is offered: failed.
    can_change: bool,
    /// Whether "Cancel" is offered: failed, for an approver.
    can_cancel: bool,
    /// The methods a failed refund can be paid by instead, `(code, label)`.
    change_methods: Vec<(String, String)>,
    /// How many times the refund was retried, and the most.
    retries: String,
}

/// `GET /refunds/{id}`: the card, the link to the source, the attempts, the
/// fail sheet and the history.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(refund): Found<Refund>,
) -> Result<View> {
    let reach = wl_kernel::scopes::reach(&state.db, &user, permissions::VIEW).await?;
    if let Some(ids) = crate::scoped::refund_ids(&state.db, &reach).await?
        && !ids.contains(&refund.id)
    {
        return Err(Error::Forbidden);
    }
    let t = texts_in(&lang);
    let db = state.db.clone();
    let attempts = RefundAttempt::of_refund(&db, refund.id)
        .await?
        .iter()
        .map(|attempt| attempt_row(&t, attempt))
        .collect();
    let history = state_machine::history(&db, Refund::TABLE, refund.id).await?;
    let mut timeline = state_machine::timeline(&history, Some(&state_machine::info_of::<Refund>()));
    timeline.sort_by(|a, b| a["time"].as_str().cmp(&b["time"].as_str()));
    // `fail` has its own sheet, and `pay` its own input.
    let actions = refund
        .available(&user, &db)
        .await?
        .into_iter()
        .filter(|a| {
            ![
                FAIL,
                PAY,
                RETRY,
                CHANGE_METHOD,
                CHANGE_METHOD_FOR_APPROVAL,
                CANCEL,
            ]
            .contains(&a.name.as_str())
        })
        .collect();
    let failed = refund.status == RefundStatus::Failed;
    let retries = refund_recover::retries_of(&db, refund.id).await?;
    let max = crate::config_of(&state).refund_max_retries;
    let can_fail = matches!(refund.status, RefundStatus::Approved | RefundStatus::Paid)
        && user.has_permission(permissions::REFUNDS_PAY);
    let source_label = format!(
        "{} #{}",
        t(&format!("refunds.source_kind.{}", refund.source_kind)),
        refund.source_id
    );
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        title: format!("{} #{}", t("refunds.heading"), refund.id),
        status_label: t(&format!("refunds.status.{}", refund.status.as_str())),
        status_kind: status_kind(refund.status),
        party_name: party_name(&state, refund.party()).await?,
        amount: money_text(&refund.currency, refund.amount),
        method_label: refund
            .method
            .map(|m| t(&format!("method.{}", m.as_str())))
            .unwrap_or_default(),
        source_link: source_link(&refund.source_type, refund.source_id),
        source_label,
        attempts,
        timeline,
        actions,
        can_fail,
        kinds: options(&t, "refunds.kind", &manual_kind_codes()),
        can_retry: failed && retries < max && user.has_permission(permissions::REFUNDS_PAY),
        can_change: failed && user.has_permission(permissions::REFUNDS_PAY),
        can_cancel: failed && user.has_permission(permissions::REFUNDS_APPROVE),
        change_methods: options(
            &t,
            "method",
            &CHANGE_METHODS
                .iter()
                .map(|m| m.as_str())
                .collect::<Vec<_>>(),
        ),
        retries: format!("{retries} / {max}"),
        refund,
    }))
}

/// The fail sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct FailForm {
    /// The kind code: `bank_return` or `card_unavailable`.
    #[validate(max = 32)]
    pub kind: Option<String>,
    /// The reason.
    #[validate(max = 2000)]
    pub reason: Option<String>,
}

/// The kind with code `code`, if a user may give it.
fn kind_of(code: &str) -> Option<RefundFailureKind> {
    RefundFailureKind::ALL
        .iter()
        .copied()
        .find(|k| k.as_str() == code.trim() && k.is_manual())
}

/// `POST /refunds/{id}/fail`: fails the refund and reloads the page. A
/// refused guard (no kind, a blank reason) answers 422 under its field; a
/// second `fail` is refused too.
async fn store_fail(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(refund): Found<Refund>,
    Valid(form): Valid<FailForm>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let Some(kind) = filled(&form.kind).and_then(kind_of) else {
        return Err(invalid("kind", t("refunds.fail.kind_required"), &form));
    };
    let reason = form.reason.clone().unwrap_or_default();
    match fail(&state, &user, &refund, kind, &reason).await {
        Ok(_) => Ok((Toast::success(t("refunds.fail.done")), renox::HxRefresh).into_response()),
        Err(TransitionError::Guard(message)) => Err(invalid("reason", message, &form)),
        Err(other) => Err(other.into_error()),
    }
}

/// `POST /refunds/{id}/retry`: retries the failed refund with the same method
/// and details; a gateway refund goes to the gateway again at once.
async fn store_retry(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(refund): Found<Refund>,
) -> Result<Response> {
    refund_recover::retry(&state, &user, &refund)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(texts::t(&lang, "wl_payments.refunds.retry.done")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The change-method sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct ChangeMethodForm {
    /// The method code: `gateway`, `transfer`, `cheque` or `cash`.
    #[validate(max = 32)]
    pub method: Option<String>,
    /// The new bank details of a transfer, or the name on a cheque that is
    /// not the party's.
    #[validate(max = 500)]
    pub details: Option<String>,
}

/// The method with code `code`, if a failed refund can be paid by it.
fn change_method_of(code: &str) -> Option<PaymentMethod> {
    CHANGE_METHODS
        .iter()
        .copied()
        .find(|method| method.as_str() == code.trim())
}

/// `POST /refunds/{id}/change-method`: pays the failed refund another way.
/// New bank details or another payee send it back to `requested` for a new
/// approval; a refusal answers 422 under its field.
async fn store_change_method(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(refund): Found<Refund>,
    Valid(form): Valid<ChangeMethodForm>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let Some(method) = filled(&form.method).and_then(change_method_of) else {
        return Err(invalid(
            "method",
            t("refunds.change.method_required"),
            &form,
        ));
    };
    match refund_recover::change_method(&state, &user, &refund, method, form.details.as_deref())
        .await
    {
        Ok(_) => Ok((Toast::success(t("refunds.change.done")), renox::HxRefresh).into_response()),
        Err(TransitionError::Guard(message)) => {
            let field = if message.contains("bank details") || message.contains("payee") {
                "details"
            } else {
                "method"
            };
            Err(invalid(field, message, &form))
        }
        Err(other) => Err(other.into_error()),
    }
}

/// The cancel sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct CancelForm {
    /// Why the refund is given up.
    #[validate(max = 2000)]
    pub reason: Option<String>,
}

/// `POST /refunds/{id}/cancel`: cancels the failed refund for a reason; the
/// amount stays on the party's credit balance.
async fn store_cancel(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(refund): Found<Refund>,
    Valid(form): Valid<CancelForm>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let reason = form.reason.clone().unwrap_or_default();
    match refund_recover::cancel(&state, &user, &refund, &reason).await {
        Ok(_) => Ok((Toast::success(t("refunds.cancel.done")), renox::HxRefresh).into_response()),
        Err(TransitionError::ReasonRequired) => {
            Err(invalid("reason", t("refunds.cancel.required"), &form))
        }
        Err(other) => Err(other.into_error()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_refund_can_be_paid_by_four_methods() {
        for code in ["gateway", "transfer", "cheque", "cash"] {
            assert!(change_method_of(code).is_some(), "{code}");
        }
        assert_eq!(change_method_of(" cash "), Some(PaymentMethod::Cash));
        for code in ["card", "wallet", "set_off", ""] {
            assert_eq!(change_method_of(code), None, "{code}");
        }
    }

    fn english_text(key: &str) -> String {
        texts::english(&format!("wl_payments.{key}")).to_owned()
    }

    #[test]
    fn a_user_cannot_give_the_gateway_kind() {
        assert_eq!(kind_of("bank_return"), Some(RefundFailureKind::BankReturn));
        assert_eq!(
            kind_of("card_unavailable"),
            Some(RefundFailureKind::CardUnavailable)
        );
        assert_eq!(kind_of("gateway_declined"), None);
        assert_eq!(kind_of(""), None);
    }

    #[test]
    fn the_grid_lists_the_refund_columns() {
        let grid = grid_with(&english_text);
        let keys: Vec<&str> = grid.columns().iter().map(Column::key).collect();
        assert_eq!(
            keys,
            [
                "id",
                "party_name",
                "source_kind",
                "amount",
                "method",
                "status",
                "created_at"
            ]
        );
    }

    #[test]
    fn sources_link_to_their_pages() {
        assert_eq!(
            source_link(PAYMENT_SUBJECT_TYPE, 4).as_deref(),
            Some("/payments/4")
        );
        assert_eq!(
            source_link(DEPOSIT_SUBJECT_TYPE, 2).as_deref(),
            Some("/deposits/2")
        );
        assert_eq!(source_link("orders.order", 2), None);
    }

    #[test]
    fn every_status_outcome_kind_and_source_has_a_label() {
        let mut keys: Vec<String> = Vec::new();
        keys.extend(
            STATUSES
                .iter()
                .map(|s| format!("refunds.status.{}", s.as_str())),
        );
        keys.extend(
            [
                AttemptOutcome::Pending,
                AttemptOutcome::Paid,
                AttemptOutcome::Failed,
            ]
            .iter()
            .map(|s| format!("refunds.outcome.{}", s.as_str())),
        );
        keys.extend(
            RefundFailureKind::ALL
                .iter()
                .map(|s| format!("refunds.kind.{}", s.as_str())),
        );
        keys.extend(
            SOURCE_KINDS
                .iter()
                .map(|s| format!("refunds.source_kind.{s}")),
        );
        for key in keys {
            assert_ne!(english_text(&key), format!("wl_payments.{key}"), "{key}");
        }
    }
}
