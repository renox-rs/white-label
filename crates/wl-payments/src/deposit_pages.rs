//! The deposit pages (wl-payments §6, A10):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /deposits` | `wl_payments.deposits.index` | `deposits.view` |
//! | `GET /deposits/{id}` | `wl_payments.deposits.show` | `deposits.view` |
//!
//! A `refunded` deposit whose refund `failed` shows a "Refund failed" badge
//! with a link to the refund; the badge is read from the refund, so the
//! deposit stays `refunded`.

use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{self};
use serde::Serialize;
use std::collections::BTreeMap;
use wl_kernel::payments::PaymentMethod;

use crate::deposit::{Deposit, DepositStatus};
use crate::followup::money_text;
use crate::pages::{Text, Texts, options, party_name, texts_in};
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

/// The module's deposit routes, behind `deposits.view`.
pub fn routes() -> Routes {
    Routes::new()
        .get("/deposits", index)
        .name("wl_payments.deposits.index")
        .get("/deposits/{id}", show)
        .name("wl_payments.deposits.show")
        .require_permission(permissions::DEPOSITS_VIEW)
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
}

/// `GET /deposits/{id}`: the card, the "Refund failed" badge with its link
/// and the history.
async fn show(
    State(state): State<AppState>,
    lang: Lang,
    Found(deposit): Found<Deposit>,
) -> Result<View> {
    let t = texts_in(&lang);
    let db = state.db.clone();
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
        deposit,
    }))
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
}
