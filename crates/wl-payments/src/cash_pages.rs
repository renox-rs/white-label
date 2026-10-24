//! The cash handover pages (wl-payments §6, A10):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /cash` | `wl_payments.cash.index` | `cash.view` |
//! | `GET /cash/{id}` | `wl_payments.cash.show` | `cash.view` |
//! | `POST /cash/{id}/count` | `wl_payments.cash.count` | `cash.count` (fields `counted`, `absent`, `note`) |
//! | `POST /cash/{id}/recount` | `wl_payments.cash.recount` | `cash.count` (fields `counted`, `holder_present`, `absent`, `note`) |
//! | `POST /cash/{id}/recoveries` | `wl_payments.cash.recovery` | `cash.count` (fields `amount`, `note`) |
//! | `POST /cash/{id}/deductions` | `wl_payments.cash.deduction` | `cash.count` (fields `amount`, `note`) |
//! | `POST /cash/{id}/recover` | `wl_payments.cash.recover` | `cash.count` (field `amount`, optional cash) |
//! | `POST /cash/{id}/write-off` | `wl_payments.cash.write_off` | `cash.write_off` (field `reason`) |
//! | `POST /cash/shortages` | `wl_payments.cash.raise` | `cash.count` (fields `holder`, `amount`, `source_type`, `source_id`, `reason`) |
//! | `POST /cash/recoveries/{id}/acknowledge` | `wl_payments.cash.acknowledge` | the person the deduction is taken from |
//!
//! The deduction sheet is offered only while the client's
//! `cash_salary_deduction` setting is on (`FIN.ACL.19`, off by default), and
//! the route refuses a call while it is off. Amounts are typed in dirhams
//! (`2250.00`); a count may be `0`. A refused guard answers 422 under its
//! field.

use renox::grid::{Column, Grid, GridPage, GridRequest, Summary};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{self, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::custody::{HandoverKind, HandoverStatus};

use crate::cash_count;
use crate::cash_handover::{
    CashCount, CashEntry, CashHandover, CashPosting, CountKind, PostingKind,
};
use crate::cash_later::{self, RaiseShortage};
use crate::cash_notices::period_text;
use crate::cash_recovery::{self, CashRecovery, RecoveryKind, RecoveryStatus};
use crate::cash_settle::{self, WriteOffRequest};
use crate::config_of;
use crate::followup::money_text;
use crate::pages::{Text, Texts, filled, invalid, options, parse_amount, texts_in};
use crate::permissions;
use crate::texts;

/// The module's cash routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/cash", index)
                .name("wl_payments.cash.index")
                .get("/cash/{id}", show)
                .name("wl_payments.cash.show")
                .require_permission(permissions::CASH_VIEW),
        )
        .merge(
            Routes::new()
                .post("/cash/{id}/count", store_count)
                .name("wl_payments.cash.count")
                .post("/cash/{id}/recount", store_recount)
                .name("wl_payments.cash.recount")
                .post("/cash/{id}/recoveries", store_recovery)
                .name("wl_payments.cash.recovery")
                .post("/cash/{id}/deductions", store_deduction)
                .name("wl_payments.cash.deduction")
                .post("/cash/{id}/recover", store_recover)
                .name("wl_payments.cash.recover")
                .post("/cash/shortages", store_shortage)
                .name("wl_payments.cash.raise")
                .require_permission(permissions::CASH_COUNT),
        )
        .merge(
            Routes::new()
                .post("/cash/{id}/write-off", store_write_off)
                .name("wl_payments.cash.write_off")
                .require_permission(permissions::CASH_WRITE_OFF),
        )
        .merge(
            // The person the deduction is taken from: the route needs a
            // signed-in user, and the service checks it is them.
            Routes::new()
                .post("/cash/recoveries/{id}/acknowledge", store_acknowledge)
                .name("wl_payments.cash.acknowledge"),
        )
}

/// The badge colour of a status.
pub(crate) fn status_kind(status: HandoverStatus) -> &'static str {
    match status {
        HandoverStatus::Open => "warning",
        HandoverStatus::Counted | HandoverStatus::Recovered => "success",
        HandoverStatus::Short => "error",
        HandoverStatus::WrittenOff => "info",
    }
}

const STATUSES: [HandoverStatus; 5] = [
    HandoverStatus::Open,
    HandoverStatus::Counted,
    HandoverStatus::Short,
    HandoverStatus::Recovered,
    HandoverStatus::WrittenOff,
];

fn status_codes() -> Vec<&'static str> {
    STATUSES.iter().map(|s| s.as_str()).collect()
}

/// The handovers grid: holder, period, amounts, status; filters by status and
/// kind.
pub fn grid(lang: &Lang, can_export: bool) -> Grid {
    let grid = grid_with(&texts_in(lang));
    if can_export { grid.exports() } else { grid }
}

fn grid_with(t: Text<'_>) -> Grid {
    Grid::new("wl-payments-cash")
        .title(&t("cash.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::number("id", "#")
                .frozen()
                .mobile()
                .link("/cash/{id}"),
        )
        .column(
            Column::text("holder", &t("cash.holder"))
                .computed(
                    "COALESCE({T}.\"holder_till\", (SELECT \"users\".\"name\" FROM \"users\" WHERE \"users\".\"id\" = {T}.\"holder_user_id\"))",
                )
                .mobile()
                .searchable(),
        )
        .column(Column::select(
            "kind",
            &t("cash.kind"),
            options(&t, "cash.kind_label", &["period", "later_shortage"]),
        ))
        .column(Column::select(
            "period_kind",
            &t("cash.period"),
            options(&t, "cash.period", &["run", "shift", "day", "till_session"]),
        ))
        .column(Column::date("period_date", &t("fields.date")).mobile())
        .column(Column::money("expected", &t("cash.expected"))
                .currency_from("currency")
                .summary(Summary::Sum),
        )
        .column(Column::money("counted", &t("cash.counted"))
                .currency_from("currency")
                .summary(Summary::Sum),
        )
        .column(Column::money("difference", &t("cash.difference"))
                .currency_from("currency")
                .summary(Summary::Sum),
        )
        .column(
            Column::select(
                "status",
                &t("fields.status"),
                options(&t, "cash.status", &status_codes()),
            )
            .badges(&[
                ("open", "warning"),
                ("counted", "success"),
                ("short", "danger"),
                ("recovered", "success"),
                ("written_off", "info"),
            ])
            .mobile(),
        )
        .sort_by("-period_date")
        .row_url("/cash/{id}")
        .cards_on_mobile()
        .empty_state(&t("cash.empty"), Some(&t("cash.empty_hint")))
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-payments/cash.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    handovers: GridPage<CashHandover>,
    can_raise: bool,
}

/// `GET /cash`: the grid.
async fn index(lang: Lang, user: AuthUser, request: GridRequest) -> Result<Response> {
    // The export is for those who count the cash.
    let can_raise = user.has_permission(permissions::CASH_COUNT);
    let grid = grid(&lang, can_raise);
    if request.param("export").is_some() && !can_raise {
        return Err(Error::Forbidden);
    }
    if let Some(file) = grid.export(CashHandover::query(), &request).await? {
        return Ok(file);
    }
    let handovers = grid.page(CashHandover::query(), &request).await?;
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        handovers,
        can_raise,
    })
    .into_response())
}

#[derive(Serialize, Debug, Clone)]
pub(crate) struct EntryRow {
    id: i64,
    source: String,
    amount: String,
    note: String,
}

#[derive(Serialize, Debug, Clone)]
pub(crate) struct CountRow {
    id: i64,
    kind: String,
    amount: String,
    counter: String,
    holder: String,
    note: String,
}

#[derive(Serialize, Debug, Clone)]
pub(crate) struct RecoveryRow {
    id: i64,
    kind: String,
    amount: String,
    status: String,
    status_kind: &'static str,
    note: String,
    can_acknowledge: bool,
}

#[derive(Serialize, Debug, Clone)]
pub(crate) struct PostingRow {
    id: i64,
    kind: String,
    amount: String,
    location: String,
}

/// What the handover page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-payments/cash_handover.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    handover: CashHandover,
    title: String,
    status_label: String,
    status_kind: &'static str,
    holder_name: String,
    responsible_name: String,
    kind_label: String,
    period: String,
    source_label: String,
    expected: String,
    counted: String,
    difference: String,
    overage: String,
    recovered: String,
    rest: String,
    written_off: String,
    entries: Vec<EntryRow>,
    counts: Vec<CountRow>,
    recoveries: Vec<RecoveryRow>,
    postings: Vec<PostingRow>,
    timeline: Vec<Value>,
    presence: Vec<(String, String)>,
    can_count: bool,
    can_recount: bool,
    can_record: bool,
    can_deduct: bool,
    can_recover: bool,
    can_write_off: bool,
}

/// The name of user `id`.
async fn user_name(db: &Db, id: i64) -> Result<String> {
    Ok(User::find(db, id)
        .await?
        .map_or_else(|| format!("#{id}"), |user| user.name))
}

/// `GET /cash/{id}`: the card, the actions, what was collected, the counts,
/// the recoveries, the postings and the history.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(handover): Found<CashHandover>,
) -> Result<View> {
    let t = texts_in(&lang);
    let db = state.db.clone();
    let money = |fils: i64| money_text(&handover.currency, fils);
    let mut entries = Vec::new();
    for entry in CashEntry::of_handover(&db, handover.id).await? {
        entries.push(EntryRow {
            id: entry.id,
            source: format!("{} #{}", entry.source_type, entry.source_id),
            amount: money(entry.amount),
            note: entry.note.unwrap_or_default(),
        });
    }
    let mut counts = Vec::new();
    for count in CashCount::of_handover(&db, handover.id).await? {
        counts.push(CountRow {
            id: count.id,
            kind: t(match count.kind {
                CountKind::Count => "cash.count.kind_count",
                CountKind::Recount => "cash.count.kind_recount",
            }),
            amount: money(count.amount),
            counter: user_name(&db, count.counter_id).await?,
            holder: match (count.holder_absent, count.holder_present) {
                (true, _) => t("cash.absent_short"),
                (false, Some(true)) => t("cash.yes"),
                (false, Some(false)) => t("cash.no"),
                (false, None) => String::new(),
            },
            note: count.note.unwrap_or_default(),
        });
    }
    let mut recoveries = Vec::new();
    for recovery in CashRecovery::of_handover(&db, handover.id).await? {
        recoveries.push(RecoveryRow {
            id: recovery.id,
            kind: t(match recovery.kind {
                RecoveryKind::Cash => "cash.recovery.kind_cash",
                RecoveryKind::SalaryDeduction => "cash.recovery.kind_deduction",
            }),
            amount: money(recovery.amount),
            status: t(&format!(
                "cash.recovery.status.{}",
                recovery.status.as_str()
            )),
            status_kind: match recovery.status {
                RecoveryStatus::Recorded => "success",
                RecoveryStatus::Requested | RecoveryStatus::Authorised => "warning",
                RecoveryStatus::Rejected => "neutral",
            },
            note: recovery.note.clone().unwrap_or_default(),
            can_acknowledge: recovery.status == RecoveryStatus::Authorised
                && recovery.responsible_user_id == user.id,
        });
    }
    let mut postings = Vec::new();
    for posting in CashPosting::of_handover(&db, handover.id).await? {
        postings.push(PostingRow {
            id: posting.id,
            kind: t(match posting.kind {
                PostingKind::OverageLiability => "cash.posting.overage_liability",
                PostingKind::ShortageLoss => "cash.posting.shortage_loss",
            }),
            amount: money(posting.amount),
            location: posting.location.unwrap_or_default(),
        });
    }
    let history = state_machine::history(&db, CashHandover::TABLE, handover.id).await?;
    let mut timeline =
        state_machine::timeline(&history, Some(&state_machine::info_of::<CashHandover>()));
    timeline.sort_by(|a, b| a["time"].as_str().cmp(&b["time"].as_str()));
    let may_count = user.has_permission(permissions::CASH_COUNT)
        && user.id != handover.holder_user_id
        && user.id != handover.responsible_user_id;
    let short = handover.status == HandoverStatus::Short;
    let config = config_of(&state);
    let source_label = handover
        .source()
        .map(|s| format!("{} #{}", s.subject_type, s.subject_id))
        .unwrap_or_default();
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        title: format!("{} #{}", t("cash.heading"), handover.id),
        status_label: t(&format!("cash.status.{}", handover.status.as_str())),
        status_kind: status_kind(handover.status),
        holder_name: match &handover.holder_till {
            Some(till) => till.clone(),
            None => user_name(&db, handover.holder_user_id).await?,
        },
        responsible_name: if handover.holder_till.is_some() {
            user_name(&db, handover.responsible_user_id).await?
        } else {
            String::new()
        },
        kind_label: t(&format!("cash.kind_label.{}", handover.kind.as_str())),
        period: period_text(&handover),
        source_label,
        expected: money(handover.expected),
        counted: money(handover.counted),
        difference: money(handover.difference),
        overage: money(handover.overage),
        recovered: money(handover.recovered),
        rest: money(handover.rest()),
        written_off: money(handover.written_off),
        entries,
        counts,
        recoveries,
        postings,
        timeline,
        presence: vec![
            (String::new(), String::new()),
            ("yes".to_owned(), t("cash.yes")),
            ("no".to_owned(), t("cash.no")),
        ],
        can_count: may_count
            && handover.status == HandoverStatus::Open
            && handover.kind == HandoverKind::Period,
        can_recount: may_count && short,
        can_record: may_count && short,
        // The option is not offered while the client has not turned it on.
        can_deduct: may_count && short && config.cash_salary_deduction,
        can_recover: may_count && short,
        can_write_off: short && user.has_permission(permissions::CASH_WRITE_OFF),
        handover,
    }))
}

/// `text` as fils for a count: dirhams, `0` allowed.
fn parse_counted(text: &str) -> Option<i64> {
    if let Some(fils) = parse_amount(text) {
        return Some(fils);
    }
    let zero = text.trim().replace(',', "");
    let zero = zero.trim_end_matches(".0").trim_end_matches(".00");
    (!zero.is_empty() && zero.chars().all(|c| c == '0')).then_some(0)
}

fn truthy(value: &Option<String>) -> bool {
    filled(value).is_some_and(|v| matches!(v, "1" | "on" | "yes" | "true"))
}

fn presence(value: &Option<String>) -> Option<bool> {
    match filled(value) {
        Some("yes") => Some(true),
        Some("no") => Some(false),
        _ => None,
    }
}

/// The success answer: a toast and a refresh.
fn done(lang: &Lang, key: &str) -> Response {
    (Toast::success(texts::t(lang, key)), renox::HxRefresh).into_response()
}

/// A refused guard answers 422 under `field`.
fn refuse<F: Serialize>(error: TransitionError, field: &str, form: &F) -> Error {
    match error {
        TransitionError::Guard(message) => invalid(field, message, form),
        other => other.into_error(),
    }
}

/// The count sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct CountForm {
    /// What was counted, in dirhams.
    #[validate(max = 32)]
    pub counted: Option<String>,
    /// Whether the count was made without the holder.
    #[validate(max = 8)]
    pub absent: Option<String>,
    /// A note.
    #[validate(max = 2000)]
    pub note: Option<String>,
    /// At a recount: `yes` or `no`, whether the holder was present or notified.
    #[validate(max = 8)]
    pub holder_present: Option<String>,
}

/// `POST /cash/{id}/count`: counts the open handover.
async fn store_count(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(handover): Found<CashHandover>,
    Valid(form): Valid<CountForm>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let Some(counted) = filled(&form.counted).and_then(parse_counted) else {
        return Err(invalid("counted", t("cash.amount_required"), &form));
    };
    match cash_count::count(
        &state,
        &user,
        &handover,
        counted,
        truthy(&form.absent),
        filled(&form.note),
    )
    .await
    {
        Ok(_) => Ok(done(&lang, "wl_payments.cash.count.done")),
        Err(error) => Err(refuse(error, "counted", &form)),
    }
}

/// `POST /cash/{id}/recount`: recounts the short handover.
async fn store_recount(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(handover): Found<CashHandover>,
    Valid(form): Valid<CountForm>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let Some(counted) = filled(&form.counted).and_then(parse_counted) else {
        return Err(invalid("counted", t("cash.amount_required"), &form));
    };
    match cash_count::recount(
        &state,
        &user,
        &handover,
        counted,
        presence(&form.holder_present),
        truthy(&form.absent),
        filled(&form.note),
    )
    .await
    {
        Ok(_) => Ok(done(&lang, "wl_payments.cash.recount.done")),
        Err(error) => Err(refuse(error, "counted", &form)),
    }
}

/// The recovery sheets' form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct RecoveryForm {
    /// The amount, in dirhams.
    #[validate(max = 32)]
    pub amount: Option<String>,
    /// A note.
    #[validate(max = 2000)]
    pub note: Option<String>,
}

/// `POST /cash/{id}/recoveries`: records a part recovery in cash.
async fn store_recovery(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(handover): Found<CashHandover>,
    Valid(form): Valid<RecoveryForm>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let Some(amount) = filled(&form.amount).and_then(parse_amount) else {
        return Err(invalid("amount", t("cash.amount_required"), &form));
    };
    match cash_recovery::record_cash(&state, &user, &handover, amount, filled(&form.note)).await {
        Ok(_) => Ok(done(&lang, "wl_payments.cash.recovery.done")),
        Err(error) => Err(refuse(error, "amount", &form)),
    }
}

/// `POST /cash/{id}/deductions`: asks to recover by a salary deduction; the
/// option is refused while the client's setting is off (`FIN.ACL.19`).
async fn store_deduction(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(handover): Found<CashHandover>,
    Valid(form): Valid<RecoveryForm>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let Some(amount) = filled(&form.amount).and_then(parse_amount) else {
        return Err(invalid("amount", t("cash.amount_required"), &form));
    };
    match cash_recovery::request_deduction(&state, &user, &handover, amount, filled(&form.note))
        .await
    {
        Ok(_) => Ok(done(&lang, "wl_payments.cash.deduction.done")),
        Err(error) => Err(refuse(error, "amount", &form)),
    }
}

/// `POST /cash/{id}/recover`: recovers the short handover, recording the cash
/// given first.
async fn store_recover(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(handover): Found<CashHandover>,
    Valid(form): Valid<RecoveryForm>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let cash = match filled(&form.amount) {
        None => None,
        Some(text) => match parse_amount(text) {
            Some(amount) => Some(amount),
            None => return Err(invalid("amount", t("cash.amount_required"), &form)),
        },
    };
    match cash_settle::recover(&state, &user, &handover, cash).await {
        Ok(_) => Ok(done(&lang, "wl_payments.cash.recover.done")),
        Err(error) => Err(refuse(error, "amount", &form)),
    }
}

/// The write-off sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct WriteOffForm {
    /// Why the rest is written off.
    #[validate(max = 2000)]
    pub reason: Option<String>,
}

/// `POST /cash/{id}/write-off`: writes off the rest of the shortage; above
/// the limit it waits for another approver.
async fn store_write_off(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(handover): Found<CashHandover>,
    Valid(form): Valid<WriteOffForm>,
) -> Result<Response> {
    let reason = form.reason.clone().unwrap_or_default();
    match cash_settle::write_off(&state, &user, &handover, &reason).await {
        Ok(WriteOffRequest::WrittenOff) => Ok(done(&lang, "wl_payments.cash.write_off.done")),
        Ok(WriteOffRequest::Requested(_)) => {
            Ok(done(&lang, "wl_payments.cash.write_off.requested"))
        }
        Err(error) => Err(refuse(error, "reason", &form)),
    }
}

/// `POST /cash/recoveries/{id}/acknowledge`: the person a deduction is taken
/// from acknowledges it; it is passed to wl-hr.
async fn store_acknowledge(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(recovery): Found<CashRecovery>,
) -> Result<Response> {
    cash_recovery::acknowledge(&state, &user, &recovery)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(done(&lang, "wl_payments.cash.acknowledge.done"))
}

/// The raise sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct RaiseForm {
    /// The holder: the user's id.
    #[validate(max = 20)]
    pub holder: Option<String>,
    /// The amount, in dirhams.
    #[validate(max = 32)]
    pub amount: Option<String>,
    /// The type of the record that caused it, e.g. `logistics.shipment`.
    #[validate(max = 64)]
    pub source_type: Option<String>,
    /// The id of that record.
    #[validate(max = 20)]
    pub source_id: Option<String>,
    /// Why.
    #[validate(max = 2000)]
    pub reason: Option<String>,
}

/// `POST /cash/shortages`: raises a later shortage against a holder.
async fn store_shortage(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Valid(form): Valid<RaiseForm>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let Some(holder) = filled(&form.holder).and_then(|v| v.parse::<i64>().ok()) else {
        return Err(invalid("holder", t("cash.raise.holder_required"), &form));
    };
    let Some(amount) = filled(&form.amount).and_then(parse_amount) else {
        return Err(invalid("amount", t("cash.amount_required"), &form));
    };
    let source = match (filled(&form.source_type), filled(&form.source_id)) {
        (Some(kind), Some(id)) => format!("{kind}:{id}").parse::<Subject>().ok(),
        _ => None,
    };
    let Some(source) = source else {
        return Err(invalid(
            "source_type",
            t("cash.raise.source_required"),
            &form,
        ));
    };
    let request = RaiseShortage {
        holder_user: holder,
        amount,
        reason: form.reason.clone().unwrap_or_default(),
        source,
    };
    match cash_later::raise_shortage(&state, &user, &request).await {
        Ok(_) => Ok(done(&lang, "wl_payments.cash.raise.done")),
        Err(error) => Err(refuse(error, "amount", &form)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn english_text(key: &str) -> String {
        texts::english(&format!("wl_payments.{key}")).to_owned()
    }

    #[test]
    fn a_count_may_be_zero() {
        assert_eq!(parse_counted("2250.00"), Some(225_000));
        assert_eq!(parse_counted("0"), Some(0));
        assert_eq!(parse_counted("0.00"), Some(0));
        assert_eq!(parse_counted(" 0.0 "), Some(0));
        assert_eq!(parse_counted(""), None);
        assert_eq!(parse_counted("-5"), None);
        assert_eq!(parse_counted("abc"), None);
    }

    #[test]
    fn the_holders_presence_is_read_from_the_sheet() {
        assert_eq!(presence(&Some("yes".into())), Some(true));
        assert_eq!(presence(&Some("no".into())), Some(false));
        assert_eq!(presence(&Some("".into())), None);
        assert_eq!(presence(&None), None);
        assert!(truthy(&Some("on".into())));
        assert!(!truthy(&Some("".into())));
    }

    #[test]
    fn the_grid_lists_the_handover_columns() {
        let grid = grid_with(&english_text);
        let keys: Vec<&str> = grid.columns().iter().map(Column::key).collect();
        assert_eq!(
            keys,
            [
                "id",
                "holder",
                "kind",
                "period_kind",
                "period_date",
                "expected",
                "counted",
                "difference",
                "status"
            ]
        );
    }

    #[test]
    fn every_status_kind_and_period_has_a_label() {
        let mut keys: Vec<String> = Vec::new();
        keys.extend(
            STATUSES
                .iter()
                .map(|s| format!("cash.status.{}", s.as_str())),
        );
        keys.extend(
            ["period", "later_shortage"]
                .iter()
                .map(|k| format!("cash.kind_label.{k}")),
        );
        keys.extend(
            ["run", "shift", "day", "till_session"]
                .iter()
                .map(|k| format!("cash.period.{k}")),
        );
        for status in ["recorded", "requested", "authorised", "rejected"] {
            keys.push(format!("cash.recovery.status.{status}"));
        }
        for key in keys {
            assert_ne!(english_text(&key), format!("wl_payments.{key}"), "{key}");
        }
    }
}
