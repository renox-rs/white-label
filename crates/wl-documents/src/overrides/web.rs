//! The override pages (wl-documents §6, #50 A10):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /documents/overrides` | `wl_documents.overrides` | signed in |
//! | `GET /documents/overrides/new` | `wl_documents.overrides.new` | signed in |
//! | `POST /documents/overrides` | `wl_documents.overrides.store` | signed in |
//! | `GET /documents/overrides/{id}` | `wl_documents.override` | the requester, a step approver or `documents.view` in the override's scope |
//!
//! The list has two parts: the requests waiting for the viewer's approval
//! (the viewer is among the current step's approvers) and the viewer's own
//! requests. The details page includes the kernel's approval card, so the
//! approvers decide there; the card's buttons post to the kernel's routes,
//! which keep segregation of duties. Times are shown in Asia/Dubai.

use std::collections::{BTreeMap, HashMap};

use renox::axum::extract::Query;
use renox::chrono::FixedOffset;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine;
use renox::validation::{Errors, ValidationError};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::approvals::{self, ApprovalCard, ApprovalRequest};
use wl_kernel::documents::OverrideMode;

use super::model::{BlockOverride, BlockOverrideStatus};
use crate::documents::Documents;
use crate::kinds::DocumentKind;
use crate::permissions;
use crate::texts;

/// The texts a template reads as `tx['fields.kind']`.
type Texts = BTreeMap<&'static str, String>;

/// The module's override routes.
pub fn routes() -> Routes {
    Routes::new()
        .get("/documents/overrides", index)
        .name("wl_documents.overrides")
        .get("/documents/overrides/new", new)
        .name("wl_documents.overrides.new")
        .post("/documents/overrides", store)
        .name("wl_documents.overrides.store")
        .get("/documents/overrides/{id}", show)
        .name("wl_documents.override")
        .require_auth()
}

/// The package text `key` (without the `wl_documents.` prefix) in `lang`.
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_documents.{key}"))
}

/// `at` as `YYYY-MM-DD HH:MM` in Asia/Dubai (UTC+4, no daylight saving).
pub(crate) fn dubai(at: DateTime) -> String {
    let zone = FixedOffset::east_opt(4 * 3600).expect("a valid offset");
    at.with_timezone(&zone).format("%Y-%m-%d %H:%M").to_string()
}

/// The status key an override shows at `at`: an approved override past its
/// `valid_until` shows as `expired`.
pub(crate) fn shown_status(o: &BlockOverride, at: DateTime) -> &'static str {
    match o.status {
        BlockOverrideStatus::Requested => "requested",
        BlockOverrideStatus::Approved if o.valid_until.is_some_and(|until| at > until) => "expired",
        BlockOverrideStatus::Approved => "approved",
        BlockOverrideStatus::Refused => "refused",
        BlockOverrideStatus::Used => "used",
    }
}

/// The badge colour of a shown status.
fn status_kind(status: &str) -> &'static str {
    match status {
        "requested" => "warning",
        "approved" => "success",
        "refused" => "error",
        _ => "neutral",
    }
}

/// Names of users and kinds, looked up once per page.
struct Names<'a> {
    db: &'a Db,
    lang: &'a Lang,
    users: HashMap<i64, String>,
    kinds: HashMap<i64, String>,
}

impl<'a> Names<'a> {
    fn new(db: &'a Db, lang: &'a Lang) -> Self {
        Self {
            db,
            lang,
            users: HashMap::new(),
            kinds: HashMap::new(),
        }
    }

    async fn user(&mut self, id: i64) -> Result<String> {
        if let Some(name) = self.users.get(&id) {
            return Ok(name.clone());
        }
        let name = User::find(self.db, id)
            .await?
            .map_or_else(|| format!("#{id}"), |user| user.name);
        self.users.insert(id, name.clone());
        Ok(name)
    }

    async fn kind(&mut self, id: i64) -> Result<String> {
        if let Some(name) = self.kinds.get(&id) {
            return Ok(name.clone());
        }
        let locale = &self.lang.locale;
        let name = DocumentKind::find(self.db, id)
            .await?
            .map_or_else(|| format!("#{id}"), |kind| kind.name(locale).to_owned());
        self.kinds.insert(id, name.clone());
        Ok(name)
    }
}

/// The overrides grid: the requests waiting for the viewer's approval and
/// the viewer's own, told apart by the "list" filter. Search on kind, subject,
/// guarded transition and reason, filters by status and requester.
///
/// The subject reads in words ("Lease #3", never `property.lease #3`) for the
/// types of `subject_types`.
pub fn grid(lang: &Lang, user_id: i64, can_export: bool, subject_types: &[String]) -> Grid {
    let statuses: Vec<(String, String)> = ["requested", "approved", "refused", "used"]
        .into_iter()
        .map(|key| {
            (
                key.to_owned(),
                text(lang, &format!("overrides.status.{key}")),
            )
        })
        .collect();
    let lists = vec![
        ("waiting".to_owned(), text(lang, "overrides.waiting")),
        ("mine".to_owned(), text(lang, "overrides.mine")),
    ];
    let kind_sql = if lang.locale == "ar" {
        "(SELECT COALESCE(NULLIF(\"name_ar\", ''), \"name_en\") FROM \"wl_documents_kinds\" WHERE \"id\" = {T}.\"kind_id\")"
    } else {
        "(SELECT \"name_en\" FROM \"wl_documents_kinds\" WHERE \"id\" = {T}.\"kind_id\")"
    };
    // `user_id` is the signed-in user's number, never a request value.
    let list_sql =
        format!("CASE WHEN {{T}}.\"requested_by\" = {user_id} THEN 'mine' ELSE 'waiting' END");
    let grid = Grid::new("wl-documents-overrides")
        .title(&text(lang, "overrides.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("kind", &text(lang, "fields.kind"))
                .computed(kind_sql)
                .frozen()
                .mobile()
                .searchable()
                .link("/documents/overrides/{id}"),
        )
        .column(
            Column::text("subject", &text(lang, "fields.subject"))
                .computed(&wl_kernel::subject::words_sql(
                    "subject_type",
                    "subject_id",
                    subject_types,
                ))
                .searchable()
                .mobile(),
        )
        .column(Column::text("guarded", &text(lang, "fields.guarded")).searchable())
        .column(Column::number("record_id", &text(lang, "overrides.fields.record")).hidden())
        .column(
            Column::text(
                "requested_name",
                &text(lang, "overrides.fields.requested_by"),
            )
            .computed("(SELECT \"name\" FROM \"users\" WHERE \"id\" = {T}.\"requested_by\")")
            .searchable(),
        )
        .column(Column::text("reason", &text(lang, "overrides.fields.reason")).searchable())
        .column(
            Column::select("status", &text(lang, "overrides.fields.status"), statuses)
                .badges(&[
                    ("requested", "warning"),
                    ("approved", "success"),
                    ("refused", "danger"),
                    ("used", "neutral"),
                ])
                .mobile(),
        )
        .column(Column::datetime(
            "valid_until",
            &text(lang, "overrides.fields.valid_until"),
        ))
        .column(
            Column::select("list", &text(lang, "overrides.list"), lists)
                .computed(&list_sql)
                .hidden(),
        )
        // The requests waiting for the viewer first, then the newest.
        .sort_by("-list,-id")
        .row_url("/documents/overrides/{id}")
        .cards_on_mobile()
        .empty_state(
            &text(lang, "overrides.waiting_empty"),
            Some(&text(lang, "overrides.subtitle")),
        );
    if can_export { grid.exports() } else { grid }
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-documents/overrides.html")]
pub struct OverridesPage {
    tx: Texts,
    overrides: GridPage<BlockOverride>,
}

/// Whether `user` is among the current step's approvers of the pending
/// request of `o`.
async fn is_step_approver(db: &Db, user: &User, o: &BlockOverride) -> Result<bool> {
    let Some(id) = o.approval_request_id else {
        return Ok(false);
    };
    let Some(request) = ApprovalRequest::find(db, id).await? else {
        return Ok(false);
    };
    if !request.is_pending() {
        return Ok(false);
    }
    let approvers = approvals::notify::step_approvers(db, &request).await?;
    Ok(approvers.iter().any(|approver| approver.id == user.id))
}

/// `GET /documents/overrides`: the requests waiting for the viewer's
/// approval and the viewer's own requests, as one grid (newest first).
async fn index(
    State(db): State<Db>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    let mut ids: Vec<i64> = Vec::new();
    for o in BlockOverride::query()
        .where_(BlockOverride::STATUS.eq(BlockOverrideStatus::Requested))
        .get(&db)
        .await?
    {
        if o.requested_by != user.id && is_step_approver(&db, &user, &o).await? {
            ids.push(o.id);
        }
    }
    ids.extend(
        BlockOverride::query()
            .where_(BlockOverride::REQUESTED_BY.eq(user.id))
            .get(&db)
            .await?
            .into_iter()
            .map(|o| o.id),
    );
    let visible = BlockOverride::query().where_(BlockOverride::ID.is_in(if ids.is_empty() {
        vec![-1]
    } else {
        ids
    }));
    let can_export = user.has_permission(permissions::MANAGE);
    let mut subject_types: Vec<String> = BlockOverride::query()
        .pluck::<String, _>(&db, "subject_type")
        .await?;
    subject_types.sort();
    subject_types.dedup();
    let grid = grid(&lang, user.id, can_export, &subject_types);
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    if let Some(file) = grid.export(visible.clone(), &request).await? {
        return Ok(file);
    }
    Ok(View::typed(OverridesPage {
        tx: texts::all(&lang),
        overrides: grid.page(visible, &request).await?,
    })
    .into_response())
}

/// What the request form needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-documents/override_form.html")]
pub struct OverrideFormPage {
    tx: Texts,
    /// `(key, name)` of the kinds that can be overridden.
    kinds: Vec<(String, String)>,
    kind: String,
    guarded: String,
    record: String,
}

/// The query of the form: what a refusal's link prefills.
#[derive(Deserialize, Debug, Default)]
pub struct FormQuery {
    /// The kind's key.
    #[serde(default)]
    pub kind: String,
    /// The guarded transition.
    #[serde(default)]
    pub guarded: String,
    /// The guarded record's id.
    #[serde(default)]
    pub record: String,
}

/// `GET /documents/overrides/new?kind=..&guarded=..&record=..`: the request
/// form, prefilled from the query.
async fn new(State(db): State<Db>, lang: Lang, Query(query): Query<FormQuery>) -> Result<View> {
    let kinds = DocumentKind::query()
        .order_by(DocumentKind::KEY)
        .get(&db)
        .await?
        .into_iter()
        .filter(|kind| !kind.legal_block && kind.override_mode == OverrideMode::WithApproval)
        .map(|kind| {
            let name = kind.name(&lang.locale).to_owned();
            (kind.key, name)
        })
        .collect();
    Ok(View::typed(OverrideFormPage {
        tx: texts::all(&lang),
        kinds,
        kind: query.kind,
        guarded: query.guarded,
        record: query.record,
    }))
}

/// The request form.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct OverrideForm {
    /// The kind's key.
    #[validate(required, max = 128)]
    pub kind: String,
    /// The guarded transition, `<entity>.<transition>`.
    #[validate(required, max = 128)]
    pub guarded: String,
    /// The guarded record's id.
    pub record: i64,
    /// Why.
    #[validate(max = 2000)]
    #[serde(default)]
    pub reason: String,
}

/// A 422 under `field`.
fn invalid(field: &str, message: String, input: &impl Serialize) -> Error {
    let mut errors = Errors::new();
    errors.add(field, message);
    ValidationError::new(errors).with_input(input).into()
}

/// `POST /documents/overrides`: asks for the override and opens its page
/// (the list when several subjects fail). A refusal of the service is a 422
/// under the field it is about.
async fn store(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Valid(form): Valid<OverrideForm>,
) -> Result<Response> {
    if form.reason.trim().is_empty() {
        return Err(invalid(
            "reason",
            texts::t(&lang, "wl_documents.override.blank_reason"),
            &form,
        ));
    }
    let created = match Documents::request_override(
        &state,
        &user,
        form.kind.trim(),
        form.guarded.trim(),
        form.record,
        &form.reason,
    )
    .await
    {
        Ok(created) => created,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)) => {
            return Err(invalid("kind", message, &form));
        }
        Err(other) => return Err(other),
    };
    let toast = Toast::success(text(&lang, "overrides.requested"));
    let redirect = match created.as_slice() {
        [only] => Redirect::route("wl_documents.override", &[&only.id])?,
        _ => Redirect::route("wl_documents.overrides", &[])?,
    };
    Ok((toast, redirect).into_response())
}

/// What the details page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-documents/override.html")]
pub struct OverridePage {
    tx: Texts,
    title: String,
    kind: String,
    subject: String,
    guarded: String,
    record: i64,
    reason: String,
    requested_by: String,
    approved_by: Option<String>,
    approved_at: Option<String>,
    valid_until: Option<String>,
    used_at: Option<String>,
    status_label: String,
    status_kind: &'static str,
    /// The approval cards (`wl-kernel/approval_card.html`).
    approvals: Vec<ApprovalCard>,
    /// The machine's history merged with the approval request's.
    timeline: Vec<Value>,
}

/// Whether `user` may see `o`: the requester, a current approver, or a
/// holder of `documents.view` in the override's scope.
async fn may_see(db: &Db, user: &User, o: &BlockOverride) -> Result<bool> {
    Ok(o.requested_by == user.id
        || user.has_permission_in(permissions::VIEW, &o.scope())
        || is_step_approver(db, user, o).await?)
}

/// `GET /documents/overrides/{id}`: the override, the approval card and the
/// history; 403 for anyone who may not see it.
async fn show(
    State(db): State<Db>,
    user: AuthUser,
    lang: Lang,
    Found(o): Found<BlockOverride>,
) -> Result<View> {
    if !may_see(&db, &user, &o).await? {
        return Err(Error::Forbidden);
    }
    let at = renox::db::now();
    let mut names = Names::new(&db, &lang);
    let status = shown_status(&o, at);
    let approver = match o.approved_by {
        Some(id) => Some(names.user(id).await?),
        None => None,
    };
    let subject = Subject::new(super::SUBJECT_TYPE, o.id);
    let cards = approvals::cards_for(&db, &subject, &user, &lang).await?;
    let history = state_machine::history(&db, BlockOverride::TABLE, o.id).await?;
    let mut timeline =
        state_machine::timeline(&history, Some(&state_machine::info_of::<BlockOverride>()));
    timeline.extend(approvals::timeline_for(&db, &subject, &lang).await?);
    timeline.sort_by(|a, b| a["time"].as_str().cmp(&b["time"].as_str()));
    for item in &mut timeline {
        let shown = item["time"]
            .as_str()
            .and_then(|time| renox::chrono::DateTime::parse_from_rfc3339(time).ok())
            .map(|time| dubai(time.with_timezone(&renox::chrono::Utc)));
        if let Some(shown) = shown {
            item["time"] = Value::String(shown);
        }
    }
    let kind = names.kind(o.kind_id).await?;
    Ok(View::typed(OverridePage {
        tx: texts::all(&lang),
        title: format!("{} #{}", text(&lang, "overrides.details"), o.id),
        kind,
        subject: format!("{} #{}", o.subject_type, o.subject_id),
        guarded: o.guarded.clone(),
        record: o.record_id,
        reason: o.reason.clone(),
        requested_by: names.user(o.requested_by).await?,
        approved_by: approver,
        approved_at: o.approved_at.map(dubai),
        valid_until: o.valid_until.map(dubai),
        used_at: o.used_at.map(dubai),
        status_label: text(&lang, &format!("overrides.status.{status}")),
        status_kind: status_kind(status),
        approvals: cards,
        timeline,
    }))
}

#[cfg(test)]
mod tests {
    use renox::chrono::{Duration, TimeZone, Utc};

    use super::*;

    #[test]
    fn times_show_in_dubai() {
        let at = Utc.with_ymd_and_hms(2026, 12, 1, 6, 0, 0).unwrap();
        assert_eq!(dubai(at), "2026-12-01 10:00");
    }

    #[test]
    fn an_approved_override_past_its_valid_until_shows_as_expired() {
        let at = Utc.with_ymd_and_hms(2026, 12, 4, 6, 0, 0).unwrap();
        let approved = BlockOverride {
            status: BlockOverrideStatus::Approved,
            valid_until: Some(at),
            ..Default::default()
        };
        assert_eq!(shown_status(&approved, at), "approved");
        assert_eq!(
            shown_status(&approved, at + Duration::minutes(1)),
            "expired"
        );
        assert_eq!(shown_status(&BlockOverride::default(), at), "requested");
    }
}
