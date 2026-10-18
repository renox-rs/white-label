//! The VAT summary pages (wl-invoicing §6, DESIGN-SYSTEM):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /vat-summaries` | `wl_invoicing.vat_summaries` | `vat.view` |
//! | `GET /vat-summaries/{id}` | `wl_invoicing.vat_summary` | `vat.view` |
//! | `POST /vat-summaries/{id}/transitions/{name}` | `wl_invoicing.vat_transitions.*` | `vat.view` and the transition's permission |
//! | `POST /vat-summaries/{id}/adjustments/{adjustment}/include` | `wl_invoicing.vat_include` | `vat.prepare` |
//! | `POST /vat-summaries/{id}/adjustments/{adjustment}/exclude` | `wl_invoicing.vat_exclude` | `vat.prepare` |
//!
//! The list is a `renox::grid`; the page of one summary shows its period,
//! who prepared, reviewed and locked it, the actions of its machine
//! (`<rx-transition-actions>`), its adjustments, and for each proposed one
//! an include/exclude sheet that asks for a reason (an empty one answers
//! 422 under the field). The templates are compiled in and can be replaced
//! by the app (`resources/views/wl-invoicing/…`).

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{self, Available, StateMachine, TransitionError};
use renox::validation::{Errors, ValidationError};
use serde::{Deserialize, Serialize};
use wl_kernel::LegalEntity;
use wl_kernel::notify::fill;

use crate::allocate::amount_text;
use crate::invoice::Invoice;
use crate::permissions;
use crate::texts;
use crate::vat_adjustment::{self, AdjustmentStatus, VatAdjustment};
use crate::vat_summary::{VatSummary, VatSummaryStatus};

/// The texts a template reads as `tx['fields.number']`.
type Texts = BTreeMap<&'static str, String>;

/// The package text `key` (without the `wl_invoicing.` prefix) in a language.
type Text<'a> = &'a dyn Fn(&str) -> String;

/// The package texts in `lang`.
fn texts_in(lang: &Lang) -> impl Fn(&str) -> String + '_ {
    move |key: &str| texts::t(lang, &format!("wl_invoicing.{key}"))
}

/// The module's VAT routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/vat-summaries", index)
                .name("wl_invoicing.vat_summaries")
                .get("/vat-summaries/{id}", show)
                .name("wl_invoicing.vat_summary")
                .group(
                    "/vat-summaries",
                    "wl_invoicing.vat_transitions.",
                    wl_kernel::machines::routes::<VatSummary>(),
                )
                .require_permission(permissions::VAT_VIEW),
        )
        .merge(
            Routes::new()
                .post(
                    "/vat-summaries/{id}/adjustments/{adjustment}/include",
                    include,
                )
                .name("wl_invoicing.vat_include")
                .post(
                    "/vat-summaries/{id}/adjustments/{adjustment}/exclude",
                    exclude,
                )
                .name("wl_invoicing.vat_exclude")
                .require_permission(permissions::VAT_PREPARE),
        )
}

/// The badge colour of a summary status.
fn status_kind(status: VatSummaryStatus) -> &'static str {
    match status {
        VatSummaryStatus::Open | VatSummaryStatus::Reviewed => "info",
        VatSummaryStatus::Prepared => "warning",
        VatSummaryStatus::Locked => "success",
    }
}

/// The badge colour of an adjustment status.
fn adjustment_kind_of(status: AdjustmentStatus) -> &'static str {
    match status {
        AdjustmentStatus::Pending => "neutral",
        AdjustmentStatus::Proposed => "info",
        AdjustmentStatus::Included => "success",
        AdjustmentStatus::Excluded => "warning",
    }
}

/// `2026-01-01 to 2026-03-31`.
pub(crate) fn period_text(start: NaiveDate, end: NaiveDate) -> String {
    format!("{} to {}", start.format("%Y-%m-%d"), end.format("%Y-%m-%d"))
}

/// What the invoice page adds to a bad-debt relief's status: the summary it
/// is on, and the reason of a decision.
pub(crate) fn relief_detail(
    t: Text<'_>,
    relief: &VatAdjustment,
    summary: Option<&VatSummary>,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(summary) = summary {
        parts.push(fill(
            &t("write_off.relief_on"),
            &json!({ "period": period_text(summary.period_start, summary.period_end) }),
        ));
    }
    if let Some(reason) = relief.decision_reason.as_deref().filter(|r| !r.is_empty()) {
        parts.push(reason.to_owned());
    }
    (!parts.is_empty()).then(|| parts.join(". "))
}

/// The VAT summaries grid: filters by entity and status.
pub fn grid(lang: &Lang) -> Grid {
    grid_with(&texts_in(lang))
}

/// [`grid`] with the texts of `t`.
fn grid_with(t: Text<'_>) -> Grid {
    let statuses: Vec<(String, String)> = VatSummaryStatus::ALL
        .iter()
        .map(|s| {
            (
                s.as_str().to_owned(),
                t(&format!("vat.status.{}", s.as_str())),
            )
        })
        .collect();
    Grid::new("wl-invoicing-vat-summaries")
        .title(&t("vat.title"))
        .column(
            Column::related(
                "entity",
                &t("vat.fields.entity"),
                "wl_legal_entities",
                "legal_entity_id",
                "code",
            )
            .frozen()
            .mobile(),
        )
        .column(
            Column::date("period_start", &t("vat.fields.period_start"))
                .mobile()
                .link("/vat-summaries/{id}"),
        )
        .column(Column::date("period_end", &t("vat.fields.period_end")).mobile())
        .column(
            Column::select("status", &t("vat.fields.status"), statuses)
                .badges(&[
                    ("open", "info"),
                    ("prepared", "warning"),
                    ("reviewed", "info"),
                    ("locked", "success"),
                ])
                .mobile(),
        )
        .column(Column::datetime("locked_at", &t("vat.fields.locked_at")).hidden())
        .sort_by("-period_start")
        .row_url("/vat-summaries/{id}")
        .cards_on_mobile()
        .empty_state(&t("vat.empty"), Some(&t("vat.empty_hint")))
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/vat_summaries.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    summaries: GridPage<VatSummary>,
}

/// `GET /vat-summaries`: the grid.
async fn index(lang: Lang, request: GridRequest) -> Result<View> {
    let summaries = grid(&lang).page(VatSummary::query(), &request).await?;
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        summaries,
    }))
}

/// One adjustment as the summary page shows it.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct AdjustmentRow {
    id: i64,
    kind: String,
    /// The source invoice's id and number, when the source is an invoice.
    invoice_id: Option<i64>,
    invoice: String,
    amount: String,
    status: String,
    status_kind: &'static str,
    reason: Option<String>,
    /// Whether the user may include or exclude it now.
    can_decide: bool,
}

/// What the summary page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/vat_summary.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    summary: VatSummary,
    title: String,
    entity: String,
    period: String,
    status_label: String,
    status_kind: &'static str,
    prepared_by: Option<String>,
    reviewed_by: Option<String>,
    locked_by: Option<String>,
    adjustments: Vec<AdjustmentRow>,
    timeline: Vec<Value>,
    /// The transitions this user may run (`<rx-transition-actions>`).
    actions: Vec<Available>,
}

/// Whether adjustments of a summary in `status` can still be decided.
fn decidable(status: VatSummaryStatus) -> bool {
    matches!(status, VatSummaryStatus::Open | VatSummaryStatus::Prepared)
}

/// The names of users `ids`.
async fn names(db: &Db, ids: Vec<i64>) -> Result<BTreeMap<i64, String>> {
    if ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let users = User::query().where_in("id", ids).get(db).await?;
    Ok(users.into_iter().map(|user| (user.id, user.name)).collect())
}

/// `GET /vat-summaries/{id}`: the period, who did what, the history, the
/// actions and the adjustments.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(summary): Found<VatSummary>,
) -> Result<View> {
    let t = texts_in(&lang);
    let db = state.db.clone();
    let entity = LegalEntity::find(&db, summary.legal_entity_id)
        .await?
        .map(|e| e.code)
        .unwrap_or_default();
    let who = names(
        &db,
        [summary.prepared_by, summary.reviewed_by, summary.locked_by]
            .into_iter()
            .flatten()
            .collect(),
    )
    .await?;
    let name_of = |id: Option<i64>| id.map(|id| who.get(&id).cloned().unwrap_or(format!("#{id}")));

    let rows = VatAdjustment::query()
        .where_(VatAdjustment::SUMMARY_ID.eq(summary.id))
        .order_by(VatAdjustment::ID)
        .get(&db)
        .await?;
    let invoice_type = Invoice::default().subject().subject_type;
    let invoice_ids: Vec<i64> = rows
        .iter()
        .filter(|a| a.source_type == invoice_type)
        .map(|a| a.source_id)
        .collect();
    let numbers: BTreeMap<i64, String> = if invoice_ids.is_empty() {
        BTreeMap::new()
    } else {
        Invoice::query()
            .where_(Invoice::ID.is_in(invoice_ids))
            .get(&db)
            .await?
            .into_iter()
            .map(|i| (i.id, i.number.unwrap_or_default()))
            .collect()
    };
    let can_decide = user.has_permission(permissions::VAT_PREPARE) && decidable(summary.status);
    let adjustments = rows
        .iter()
        .map(|a| {
            let is_invoice = a.source_type == invoice_type;
            AdjustmentRow {
                id: a.id,
                kind: t(&format!("adjustment_kind.{}", a.kind.as_str())),
                invoice_id: is_invoice.then_some(a.source_id),
                invoice: if is_invoice {
                    numbers.get(&a.source_id).cloned().unwrap_or_default()
                } else {
                    a.source().to_string()
                },
                amount: amount_text("AED", a.amount),
                status: t(&format!("adjustment_status.{}", a.status.as_str())),
                status_kind: adjustment_kind_of(a.status),
                reason: a.decision_reason.clone(),
                can_decide: can_decide && a.status == AdjustmentStatus::Proposed,
            }
        })
        .collect();

    let history = state_machine::history(&db, VatSummary::TABLE, summary.id).await?;
    let timeline = state_machine::timeline(&history, Some(&state_machine::info_of::<VatSummary>()));
    let actions = summary.available(&user, &db).await?;
    let period = period_text(summary.period_start, summary.period_end);
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        title: format!("{entity} {period}"),
        entity,
        period,
        status_label: t(&format!("vat.status.{}", summary.status.as_str())),
        status_kind: status_kind(summary.status),
        prepared_by: name_of(summary.prepared_by),
        reviewed_by: name_of(summary.reviewed_by),
        locked_by: name_of(summary.locked_by),
        adjustments,
        timeline,
        actions,
        summary,
    }))
}

/// The decision sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct DecisionForm {
    /// Why the adjustment is included or excluded.
    #[validate(required, max = 2000)]
    pub reason: String,
}

/// Includes (`include`) or excludes the proposed adjustment of a summary
/// and reloads the page. A refusal answers 422 under the reason.
async fn decide(
    state: &AppState,
    user: &AuthUser,
    lang: &Lang,
    ids: (i64, i64),
    include: bool,
    form: DecisionForm,
) -> Result<Response> {
    let (summary_id, adjustment_id) = ids;
    let adjustment = VatAdjustment::find(&state.db, adjustment_id)
        .await?
        .filter(|a| a.summary_id == Some(summary_id))
        .ok_or(Error::NotFound)?;
    let done = if include {
        vat_adjustment::include(state, user, &adjustment, &form.reason).await
    } else {
        vat_adjustment::exclude(state, user, &adjustment, &form.reason).await
    };
    match done {
        Ok(()) => {
            let key = if include {
                "wl_invoicing.vat.adjustments.included"
            } else {
                "wl_invoicing.vat.adjustments.excluded"
            };
            Ok((Toast::success(texts::t(lang, key)), renox::HxRefresh).into_response())
        }
        Err(TransitionError::Guard(message)) => {
            let mut errors = Errors::new();
            errors.add("reason", message);
            Err(Error::Validation(
                ValidationError::new(errors).with_input(&form),
            ))
        }
        Err(other) => Err(other.into_error()),
    }
}

/// `POST /vat-summaries/{id}/adjustments/{adjustment}/include`.
async fn include(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((id, adjustment)): Path<(i64, i64)>,
    Valid(form): Valid<DecisionForm>,
) -> Result<Response> {
    decide(&state, &user, &lang, (id, adjustment), true, form).await
}

/// `POST /vat-summaries/{id}/adjustments/{adjustment}/exclude`.
async fn exclude(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((id, adjustment)): Path<(i64, i64)>,
    Valid(form): Valid<DecisionForm>,
) -> Result<Response> {
    decide(&state, &user, &lang, (id, adjustment), false, form).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::texts::english;
    use crate::vat_adjustment::AdjustmentKind;

    fn english_text(key: &str) -> String {
        english(&format!("wl_invoicing.{key}")).to_owned()
    }

    #[test]
    fn the_grid_lists_the_summary_columns_and_statuses() {
        let grid = grid_with(&english_text);
        let keys: Vec<&str> = grid.columns().iter().map(Column::key).collect();
        assert_eq!(
            keys,
            [
                "entity",
                "period_start",
                "period_end",
                "status",
                "locked_at"
            ]
        );
        let status = grid
            .columns()
            .iter()
            .find(|c| c.key() == "status")
            .expect("status column");
        assert_eq!(status.options().len(), VatSummaryStatus::ALL.len());
    }

    #[test]
    fn periods_read_as_a_range() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 3, 31).unwrap();
        assert_eq!(period_text(start, end), "2026-01-01 to 2026-03-31");
    }

    #[test]
    fn every_status_and_kind_has_a_label() {
        for status in VatSummaryStatus::ALL {
            let key = format!("vat.status.{}", status.as_str());
            assert_ne!(english_text(&key), format!("wl_invoicing.{key}"));
        }
        for status in AdjustmentStatus::ALL {
            let key = format!("adjustment_status.{}", status.as_str());
            assert_ne!(english_text(&key), format!("wl_invoicing.{key}"));
        }
        for kind in AdjustmentKind::ALL {
            let key = format!("adjustment_kind.{}", kind.as_str());
            assert_ne!(english_text(&key), format!("wl_invoicing.{key}"));
        }
    }

    #[test]
    fn a_relief_detail_names_the_summary_and_the_reason() {
        let summary = VatSummary {
            period_start: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            period_end: NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
            ..Default::default()
        };
        let mut relief = VatAdjustment::default();
        assert_eq!(relief_detail(&english_text, &relief, None), None);
        assert_eq!(
            relief_detail(&english_text, &relief, Some(&summary)).as_deref(),
            Some("On the VAT summary for 2026-07-01 to 2026-09-30")
        );
        relief.decision_reason = Some("Customer paid".to_owned());
        assert_eq!(
            relief_detail(&english_text, &relief, Some(&summary)).as_deref(),
            Some("On the VAT summary for 2026-07-01 to 2026-09-30. Customer paid")
        );
    }
}
