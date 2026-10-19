//! The back-office pages (wl-parties §6, DESIGN-SYSTEM):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /parties` | `wl_parties.index` | `parties.view` (`?export=` also `parties.export`) |
//! | `GET /parties/new` | `wl_parties.create` | `parties.create` |
//! | `POST /parties` | `wl_parties.store` | `parties.create` |
//! | `GET /parties/{id}` | `wl_parties.show` | `parties.view` |
//! | `POST /parties/{id}/block` | `wl_parties.block` | `parties.block` (a restricted reason also `parties.block_reason.view`) |
//! | `POST /parties/{id}/unblock` | `wl_parties.unblock` | `parties.block` (multipart: note, basis and evidence files; the service checks the rest) |
//! | `GET /parties/{id}/block-records/{record}` | `wl_parties.block_record` | `parties.view` (a restricted record also `parties.block_reason.view`, else 404) |
//! | `GET /parties/{id}/block-records/{record}/evidence/{file}` | `wl_parties.block_evidence` | `parties.view` (same visibility as the record) |
//! | `POST /parties/{id}/contact-points/{point}` | `wl_parties.contact_update` | `parties.update` (a point of another party is 404; a bad value is a 422) |
//! | `POST /parties/{id}/contact-points/{point}/verify` | `wl_parties.contact_verify` | `parties.update` (the same 404) |
//! | `GET /parties/block-effects` | `wl_parties.block_effects` | `parties.block` (read-only: the vertical sets the values) |
//! | `GET /parties/review-tasks` | `wl_parties.review_tasks` | signed in; lists the open tasks whose decider role the user holds |
//! | `GET /parties/review-tasks/{id}` | `wl_parties.review_task` | the task's decider role in its scope, else 403 |
//! | `GET /parties/screening-tasks` | `wl_parties.screening_tasks` | `parties.view` and `parties.block_reason.view`, else 404 |
//! | `GET /parties/screening-tasks/{id}` | `wl_parties.screening_task` | the same |
//! | `POST /parties/screening-tasks/{id}/confirm` | `wl_parties.screening_confirm` | the same; the service needs `screening.review` (403); form field `note` |
//! | `GET /parties/compliance-tasks` | `wl_parties.compliance_tasks` | the same as the screening tasks (404 without `parties.block_reason.view`) |
//! | `GET /parties/compliance-tasks/{id}` | `wl_parties.compliance_task` | the same |
//! | `POST /parties/compliance-tasks/{id}/funds` | `wl_parties.compliance_fund_add` | the same; adding needs `screening.review` (403) |
//! | `POST /parties/compliance-tasks/{id}/funds/{fund}` | `wl_parties.compliance_fund_update` | the same |
//! | `POST /parties/compliance-tasks/{id}/complete` | `wl_parties.compliance_complete` | the same; records the freeze time, or the reference and date |
//! | `GET /parties/screening` | `wl_parties.screening` | any of `screening.run`, `screening.review`, `parties.block_reason.view`, else 403 |
//! | `POST /parties/review-tasks/{id}/items/{item}` | `wl_parties.review_decide` | the same; form field `decision` is `pause`, `continue` or `end` |
//!
//! Lists, search, filters and exports read the party's `hold_reason`, which
//! is only ever the display code, so they cannot show a restricted reason
//! (`PTY.ACL.02`). Every template is compiled in and can be replaced by the
//! app (`resources/views/wl-parties/…`).

use std::collections::BTreeMap;

use renox::Download;
use renox::chrono::{NaiveDate, NaiveDateTime};
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::state_machine::{self, TransitionError};
use renox::validation::{Errors, ValidationError};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{self, ApprovalCard};
use wl_kernel::block_effects::{self as kernel_effects, BlockEffect, Decision};
use wl_kernel::screening_guard::{ScreeningNoticeView, screening_guard, stale_notice_view};
use wl_kernel::{PartyKind, PartyStatus, ScreeningStatus, Subject};

use crate::block::block;
use crate::block_record::BlockRecord;
use crate::compliance_tasks::{
    self, CompleteInput, ComplianceTask, ComplianceTaskKind, ComplianceTaskStatus, FundInput,
    PartyStanding, SANCTIONS, amount_text, parse_amount,
};
use crate::confirm_match;
use crate::contacts::{ContactPoint, ContactStatus, ContactType, update_contact, verify_contact};
use crate::create::{NewParty, create};
use crate::details::{CompanyDetails, PersonDetails};
use crate::evidence::{UnblockBasis, UnblockEvidence};
use crate::list_parser::ListParsers;
use crate::list_upload::{
    UploadError, UploadRefusal, UploadRequest, UploadSettings, upload_list_file,
};
use crate::manual_check::{ManualCheckError, ManualRefusal, ManualResult, record_manual_check};
use crate::party::{Channel, Language, Party};
use crate::permissions;
use crate::reasons::BlockReasons;
use crate::review_tasks::{self, ReviewTask, ReviewTaskStatus};
use crate::roles::PartyRole;
use crate::sanctions_lists::{ListFormat, ListStatus, SanctionsList, UpdateSource};
use crate::screening::{Screening, ScreeningMethod, ScreeningSettings, ScreeningTrigger};
use crate::screening_task::{ScreeningTask, ScreeningTaskStatus, dubai_label};
use crate::texts;
use crate::unblock::{UnblockRequest, unblock};
use crate::visibility;

/// The hold reasons the list's filter offers: the configured reasons that
/// are not restricted, `on_hold` among them (wl-parties §7). Restricted
/// codes are never in `hold_reason`, so they are never offered, to anyone
/// (`PTY.ACL.02`).
pub fn filter_reasons(reasons: &BlockReasons) -> Vec<&str> {
    reasons.unrestricted().map(|r| r.code.as_str()).collect()
}

/// The texts a template reads as `tx['fields.code']`.
type Texts = BTreeMap<&'static str, String>;

/// The module's routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .get("/parties/new", new)
        .name("wl_parties.create")
        .post("/parties", store)
        .name("wl_parties.store")
        .require_permission(permissions::CREATE)
        .merge(
            Routes::new()
                .get("/parties", index)
                .name("wl_parties.index")
                .get("/parties/{id}", show)
                .name("wl_parties.show")
                .get("/parties/{id}/block-records/{record}", block_record)
                .name("wl_parties.block_record")
                .get("/parties/screening-tasks", screening_tasks_page)
                .name("wl_parties.screening_tasks")
                .get("/parties/screening-tasks/{id}", screening_task_page)
                .name("wl_parties.screening_task")
                .post("/parties/screening-tasks/{id}/confirm", confirm_screening)
                .name("wl_parties.screening_confirm")
                .get("/parties/compliance-tasks", compliance_tasks_page)
                .name("wl_parties.compliance_tasks")
                .get("/parties/compliance-tasks/{id}", compliance_task_page)
                .name("wl_parties.compliance_task")
                .post("/parties/screening/lists/{id}/upload", upload_list)
                .name("wl_parties.list_upload")
                .post("/parties/{id}/manual-check", store_manual_check)
                .name("wl_parties.manual_check")
                .post("/parties/compliance-tasks/{id}/funds", add_compliance_fund)
                .name("wl_parties.compliance_fund_add")
                .post(
                    "/parties/compliance-tasks/{id}/funds/{fund}",
                    update_compliance_fund,
                )
                .name("wl_parties.compliance_fund_update")
                .post(
                    "/parties/compliance-tasks/{id}/complete",
                    complete_compliance_task,
                )
                .name("wl_parties.compliance_complete")
                .get(
                    "/parties/{id}/block-records/{record}/evidence/{file}",
                    block_evidence,
                )
                .name("wl_parties.block_evidence")
                .require_permission(permissions::VIEW),
        )
        .merge(
            Routes::new()
                .get("/parties/block-effects", block_effects_page)
                .name("wl_parties.block_effects")
                .require_permission(permissions::BLOCK),
        )
        // The decider role, not a permission, gates these (no new permissions).
        .merge(
            Routes::new()
                .get("/parties/review-tasks", review_tasks_page)
                .name("wl_parties.review_tasks")
                .get("/parties/screening", screening_page)
                .name("wl_parties.screening")
                .get("/parties/review-tasks/{id}", review_task_page)
                .name("wl_parties.review_task")
                .post("/parties/review-tasks/{id}/items/{item}", decide_item)
                .name("wl_parties.review_decide")
                .post("/parties/review-tasks/{id}/neutral-message", send_neutral)
                .name("wl_parties.review_neutral")
                .require_auth(),
        )
        .merge(
            Routes::new()
                .post("/parties/{id}/contact-points/{point}", store_contact)
                .name("wl_parties.contact_update")
                .post(
                    "/parties/{id}/contact-points/{point}/verify",
                    verify_contact_point,
                )
                .name("wl_parties.contact_verify")
                .require_permission(permissions::UPDATE),
        )
        .merge(
            Routes::new()
                .post("/parties/{id}/block", store_block)
                .name("wl_parties.block")
                .post("/parties/{id}/unblock", store_unblock)
                .name("wl_parties.unblock")
                .require_permission(permissions::BLOCK),
        )
}

/// The package text `key` (without the `wl_parties.` prefix) in a language.
type Text<'a> = &'a dyn Fn(&str) -> String;

/// The package texts in `lang`.
fn texts_in(lang: &Lang) -> impl Fn(&str) -> String + '_ {
    move |key: &str| texts::t(lang, &format!("wl_parties.{key}"))
}

/// `(value, label)` pairs for a select, labelled with `prefix.value`.
fn options<'a>(
    t: Text<'_>,
    prefix: &str,
    values: impl IntoIterator<Item = &'a str>,
) -> Vec<(String, String)> {
    values
        .into_iter()
        .map(|value| (value.to_owned(), t(&format!("{prefix}.{value}"))))
        .collect()
}

/// The parties grid: search on the code and names, filters by kind,
/// status and hold reason, and the export menu for `can_export`. A blocked
/// party shows "On hold" and its hold reason as "On hold — Non-payment" (or
/// just "On hold"), in the list and in exports.
pub fn grid(lang: &Lang, reasons: &BlockReasons, can_export: bool) -> Grid {
    let holds = filter_reasons(reasons)
        .into_iter()
        .map(|code| (code.to_owned(), visibility::hold_label(lang, code)))
        .collect();
    grid_with(&texts_in(lang), holds, can_export)
}

/// [`grid`] with the texts of `t` and the hold reason options `holds`.
fn grid_with(t: Text<'_>, holds: Vec<(String, String)>, can_export: bool) -> Grid {
    let grid = Grid::new("wl-parties")
        .title(&t("title"))
        .column(
            Column::text("code", &t("fields.code"))
                .frozen()
                .mobile()
                .searchable()
                .copyable()
                .link("/parties/{id}"),
        )
        .column(
            Column::text("display_name", &t("fields.display_name"))
                .mobile()
                .searchable(),
        )
        .column(Column::text("legal_name_en", &t("fields.legal_name_en")).searchable())
        .column(
            Column::text("legal_name_ar", &t("fields.legal_name_ar"))
                .searchable()
                .hidden(),
        )
        .column(Column::select(
            "kind",
            &t("fields.kind"),
            options(t, "kind", PartyKind::ALL.iter().map(PartyKind::as_str)),
        ))
        .column(
            Column::select(
                "status",
                &t("fields.status"),
                options(
                    t,
                    "status",
                    PartyStatus::ALL.iter().map(PartyStatus::as_str),
                ),
            )
            .badges(&[
                ("active", "success"),
                ("blocked", "danger"),
                ("archived", "neutral"),
                ("merged", "neutral"),
            ])
            .mobile(),
        )
        .column(Column::select("hold_reason", &t("fields.hold_reason"), holds).mobile())
        .column(Column::datetime("created_at", &t("fields.created_at")).hidden())
        .sort_by("code")
        .row_url("/parties/{id}")
        .cards_on_mobile()
        .empty_state(&t("empty"), Some(&t("empty_hint")));
    if can_export { grid.exports() } else { grid }
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-parties/index.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    parties: GridPage<Party>,
}

/// `GET /parties`: the grid, or its export (`?export=csv|xlsx|print`,
/// with `parties.export`).
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    let can_export = user.has_permission(permissions::EXPORT);
    let grid = grid(&lang, &BlockReasons::of(&state), can_export);
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    if let Some(file) = grid.export(Party::query(), &request).await? {
        return Ok(file);
    }
    let parties = grid.page(Party::query(), &request).await?;
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        parties,
    })
    .into_response())
}

/// What the create form needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-parties/new.html")]
pub(crate) struct NewPage {
    tx: Texts,
    kinds: Vec<(String, String)>,
    languages: Vec<(String, String)>,
    channels: Vec<(String, String)>,
}

/// `GET /parties/new`: the create form.
async fn new(lang: Lang) -> Result<View> {
    let t = texts_in(&lang);
    Ok(View::typed(NewPage {
        tx: texts::all(&lang),
        kinds: options(&t, "kind", PartyKind::ALL.iter().map(PartyKind::as_str)),
        languages: options(&t, "language", Language::ALL.iter().map(Language::as_str)),
        channels: options(&t, "channel", Channel::ALL.iter().map(Channel::as_str)),
    }))
}

/// The create form. The service checks the rest (a 15-digit TRN, one
/// primary per contact type).
#[derive(Deserialize, Validate, Debug)]
pub struct PartyForm {
    /// Person or company.
    pub kind: PartyKind,
    /// The name to show; the English legal name when blank.
    #[validate(max = 200)]
    pub display_name: Option<String>,
    /// The legal name in English.
    #[validate(required, max = 200)]
    pub legal_name_en: String,
    /// The legal name in Arabic.
    #[validate(max = 200)]
    pub legal_name_ar: Option<String>,
    /// The language documents and messages use.
    #[serde(default)]
    pub default_language: Language,
    /// The preferred channel.
    #[serde(default)]
    pub preferred_channel: Channel,
    /// The primary e-mail address.
    #[validate(email, max = 200)]
    pub email: Option<String>,
    /// The primary mobile number.
    #[validate(max = 30)]
    pub mobile: Option<String>,
    /// A company's TRN (sent only for a company).
    #[validate(max = 15)]
    pub trn: Option<String>,
}

impl PartyForm {
    /// The service's input for this form.
    pub fn into_new_party(self) -> NewParty {
        let primary = |contact_type, value: Option<String>| {
            value
                .filter(|v| !v.trim().is_empty())
                .map(|value| ContactPoint {
                    contact_type,
                    value,
                    is_primary: true,
                    ..Default::default()
                })
        };
        let trn = self.trn.filter(|trn| !trn.trim().is_empty());
        NewParty {
            kind: self.kind,
            display_name: self.display_name.unwrap_or_default(),
            legal_name_en: self.legal_name_en,
            legal_name_ar: self.legal_name_ar,
            default_language: self.default_language,
            preferred_channel: self.preferred_channel,
            person: (self.kind == PartyKind::Person).then(PersonDetails::default),
            company: (self.kind == PartyKind::Company).then(|| CompanyDetails {
                vat_registered: trn.is_some(),
                trn,
                ..Default::default()
            }),
            roles: Vec::new(),
            contacts: [
                primary(ContactType::Email, self.email),
                primary(ContactType::Mobile, self.mobile),
            ]
            .into_iter()
            .flatten()
            .collect(),
            source: "back_office".to_owned(),
        }
    }
}

/// `POST /parties`: creates the party and opens its page.
async fn store(
    State(state): State<AppState>,
    lang: Lang,
    Valid(form): Valid<PartyForm>,
) -> Result<Response> {
    let party = create(&state, form.into_new_party()).await?;
    Ok((
        Toast::success(texts::t(&lang, "wl_parties.created")),
        Redirect::route("wl_parties.show", &[&party.id])?,
    )
        .into_response())
}

/// What the party page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-parties/show.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    party: Party,
    person: Option<PersonDetails>,
    company: Option<CompanyDetails>,
    contacts: Vec<ContactRow>,
    /// The contact points that need updating, for the banner.
    invalid_contacts: Vec<ContactRow>,
    /// Whether the edit and verify actions show (`parties.update`).
    can_update: bool,
    roles: Vec<PartyRole>,
    timeline: Vec<renox::serde_json::Value>,
    /// "On hold — Non-payment" / "On hold" while blocked.
    hold_label: Option<String>,
    /// Whether the block action is shown (`parties.block`, party `active`).
    can_block: bool,
    /// The reasons the block dialog offers this user, `(code, label)`.
    reasons: Vec<(String, String)>,
    /// Whether the unblock action is shown (`parties.block`, party `blocked`).
    can_unblock: bool,
    /// Whether the unblock sheet asks for a basis and files: the viewer
    /// holds `parties.block_reason.view` and an open record is `sanctions`.
    unblock_evidence: bool,
    /// The bases the unblock sheet offers, `(value, label)`.
    bases: Vec<(String, String)>,
    /// The stale sanctions list line, for `screening.run`, `screening.review`
    /// or `parties.block_reason.view` only (#41).
    notice: Option<ScreeningNoticeView>,
    /// The Block records this user may read, newest first.
    block_records: Vec<BlockRecordRow>,
    /// Whether *Record manual check* shows: `screening.review`, screening
    /// on, a PDF-only list exists and the party is not archived.
    can_manual_check: bool,
    /// The PDF-only lists the manual check offers, `(id, name)`.
    manual_lists: Vec<(i64, String)>,
    /// The results the manual check offers, `(value, label)`.
    manual_results: Vec<(String, String)>,
}

/// One contact point as the party page shows it.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ContactRow {
    id: i64,
    contact_type: ContactType,
    value: String,
    label: Option<String>,
    is_primary: bool,
    verified_at: Option<DateTime>,
    invalid: bool,
    /// "Valid" or "Invalid: Hard bounce".
    status_label: String,
    /// The reason's label while invalid.
    reason: Option<String>,
    invalid_since: Option<DateTime>,
}

impl ContactRow {
    fn of(point: ContactPoint, lang: &Lang) -> Self {
        let invalid = point.status == ContactStatus::Invalid;
        let reason = invalid.then(|| {
            let code = point.invalid_reason.as_deref().unwrap_or_default();
            let key = format!("wl_parties.invalid_reason.{code}");
            let label = texts::t(lang, &key);
            if label == key { code.to_owned() } else { label }
        });
        let status_label = match &reason {
            Some(reason) => format!(
                "{}: {reason}",
                texts::t(lang, "wl_parties.contact_status.invalid")
            ),
            None => texts::t(lang, "wl_parties.contact_status.valid"),
        };
        Self {
            id: point.id,
            contact_type: point.contact_type,
            value: point.value,
            label: point.label,
            is_primary: point.is_primary,
            verified_at: point.verified_at,
            invalid,
            status_label,
            reason,
            invalid_since: point.invalid_since,
        }
    }
}

/// One evidence file of a closed Block record.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct EvidenceRow {
    id: i64,
    file_name: String,
}

/// One Block record as the party page shows it.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct BlockRecordRow {
    id: i64,
    reason_code: String,
    reason: String,
    restricted: bool,
    note: String,
    blocked_by: String,
    blocked_at: DateTime,
    unblocked_at: Option<DateTime>,
    unblocked_by: Option<String>,
    unblock_note: Option<String>,
    unblock_basis: Option<String>,
    evidence: Vec<EvidenceRow>,
    is_open: bool,
}

/// The Block records a user may read, with their labels: restricted ones
/// only with `parties.block_reason.view` (`PTY.ACL.02`).
async fn block_rows(
    db: &Db,
    lang: &Lang,
    party_id: i64,
    sees_restricted: bool,
) -> Result<Vec<BlockRecordRow>> {
    let records =
        visibility::visible_records(BlockRecord::for_party(db, party_id).await?, sees_restricted);
    let mut rows = Vec::with_capacity(records.len());
    for record in records {
        let blocked_by = match record.blocked_by {
            Some(id) => User::find(db, id)
                .await?
                .map_or_else(|| format!("#{id}"), |user| user.name),
            None => texts::t(lang, "wl_parties.block.system"),
        };
        let unblocked_by = match record.unblocked_by {
            Some(id) => Some(
                User::find(db, id)
                    .await?
                    .map_or_else(|| format!("#{id}"), |user| user.name),
            ),
            None => None,
        };
        let evidence = UnblockEvidence::for_record(db, record.id)
            .await?
            .into_iter()
            .map(|file| EvidenceRow {
                id: file.id,
                file_name: file.file_name,
            })
            .collect();
        rows.push(BlockRecordRow {
            id: record.id,
            reason: visibility::reason_label(lang, &record.reason_code),
            is_open: record.is_open(),
            reason_code: record.reason_code,
            restricted: record.restricted,
            note: record.note,
            blocked_by,
            blocked_at: record.blocked_at,
            unblocked_at: record.unblocked_at,
            unblocked_by,
            unblock_note: record.unblock_note,
            unblock_basis: record
                .unblock_basis
                .map(|basis| texts::t(lang, &format!("wl_parties.unblock_basis.{basis}"))),
            evidence,
        });
    }
    Ok(rows)
}

/// `GET /parties/{id}`: the card, contact points, roles, Block records,
/// history and the block action.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(party): Found<Party>,
) -> Result<View> {
    let db = state.db.clone();
    let person = PersonDetails::query()
        .where_(PersonDetails::PARTY_ID.eq(party.id))
        .first(&db)
        .await?;
    let company = CompanyDetails::query()
        .where_(CompanyDetails::PARTY_ID.eq(party.id))
        .first(&db)
        .await?;
    let contacts = ContactPoint::query()
        .where_(ContactPoint::PARTY_ID.eq(party.id))
        .order_by(ContactPoint::CONTACT_TYPE)
        .order_by_desc(ContactPoint::IS_PRIMARY)
        .order_by(ContactPoint::ID)
        .get(&db)
        .await?;
    let contacts: Vec<ContactRow> = contacts
        .into_iter()
        .map(|point| ContactRow::of(point, &lang))
        .collect();
    let invalid_contacts = contacts.iter().filter(|c| c.invalid).cloned().collect();
    let can_update = user.has_permission(permissions::UPDATE);
    let roles = PartyRole::query()
        .where_(PartyRole::PARTY_ID.eq(party.id))
        .order_by(PartyRole::ROLE_KEY)
        .get(&db)
        .await?;
    let history = state_machine::history(&db, Party::TABLE, party.id).await?;
    let timeline = state_machine::timeline(&history, Some(&state_machine::info_of::<Party>()));
    let sees_restricted = user.has_permission(permissions::BLOCK_REASON_VIEW);
    let hold_label = (party.status == PartyStatus::Blocked)
        .then(|| visibility::hold_label(&lang, party.hold_reason.as_deref().unwrap_or_default()));
    let can_block = party.status == PartyStatus::Active && user.has_permission(permissions::BLOCK);
    let reasons = if can_block {
        visibility::offered_reasons(&BlockReasons::of(&state), &lang, sees_restricted)
    } else {
        Vec::new()
    };
    let can_unblock =
        party.status == PartyStatus::Blocked && user.has_permission(permissions::BLOCK);
    let unblock_evidence = can_unblock
        && sees_restricted
        && BlockRecord::open_for(&db, party.id)
            .await?
            .iter()
            .any(|record| record.reason_code == "sanctions");
    let bases = options(
        &texts_in(&lang),
        "unblock_basis",
        ["delisted", "false_match"],
    );
    let block_records = block_rows(&db, &lang, party.id, sees_restricted).await?;
    let notice = stale_notice(&state, &user, &lang, party.id).await?;
    let manual_lists = if user.has_permission(permissions::SCREENING_REVIEW)
        && ScreeningSettings::of(&state).on
        && party.status != PartyStatus::Archived
    {
        manual_check_lists(&db).await?
    } else {
        Vec::new()
    };
    let can_manual_check = !manual_lists.is_empty();
    let manual_results = options(
        &texts_in(&lang),
        "manual_check.result",
        ["no_match", "candidate"],
    );
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        party,
        person,
        company,
        contacts,
        invalid_contacts,
        can_update,
        roles,
        timeline,
        hold_label,
        can_block,
        reasons,
        can_unblock,
        unblock_evidence,
        bases,
        block_records,
        notice,
        can_manual_check,
        manual_lists,
        manual_results,
    }))
}

/// The PDF-only lists, `(id, name)`.
async fn manual_check_lists(db: &Db) -> Result<Vec<(i64, String)>> {
    Ok(SanctionsList::all(db)
        .await?
        .into_iter()
        .filter(|list| list.format == ListFormat::Pdf)
        .map(|list| (list.id, list.name))
        .collect())
}

/// The "screened on a stale list" line of the party page. Screening data never
/// reaches viewers without `screening.run`, `screening.review` or
/// `parties.block_reason.view` (`ACC.ACL.05`), so they get `None` without a
/// read. It is also `None` with screening off (the no-op guard).
async fn stale_notice(
    state: &AppState,
    user: &AuthUser,
    lang: &Lang,
    party: i64,
) -> Result<Option<ScreeningNoticeView>> {
    if ![
        permissions::SCREENING_RUN,
        permissions::SCREENING_REVIEW,
        permissions::BLOCK_REASON_VIEW,
    ]
    .into_iter()
    .any(|permission| user.has_permission(permission))
    {
        return Ok(None);
    }
    let mut tx = state.db.begin().await?;
    let lists = screening_guard(state)
        .latest_notice_in(&mut tx, wl_kernel::PartyId(party))
        .await?;
    Ok(stale_notice_view(lang, &lists))
}

/// The block dialog's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct BlockForm {
    /// The reason code.
    #[validate(required, max = 64)]
    pub reason_code: String,
    /// Why, in the blocker's words.
    #[validate(required, max = 2000)]
    pub note: String,
}

/// `POST /parties/{id}/block`: blocks the party and reloads its page. The
/// transition's guards refuse an unknown code, a blank note and a
/// restricted code without `parties.block_reason.view`: a 422 with the
/// message under the field, and nothing written.
async fn store_block(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(party): Found<Party>,
    Valid(form): Valid<BlockForm>,
) -> Result<Response> {
    match block(&state, &user, &party, &form.reason_code, &form.note).await {
        Ok(()) => Ok((
            Toast::success(texts::t(&lang, "wl_parties.block.done")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::Guard(message)) => {
            let field = if form.note.trim().is_empty() {
                "note"
            } else {
                "reason_code"
            };
            let mut errors = Errors::new();
            errors.add(field, message);
            Err(Error::Validation(
                ValidationError::new(errors).with_input(&form),
            ))
        }
        Err(other) => Err(other.into_error()),
    }
}

/// `GET /parties/{id}/block-records/{record}`: one Block record, as JSON.
/// A restricted one answers 404 without `parties.block_reason.view`, as
/// does a record of another party.
async fn block_record(
    State(db): State<Db>,
    user: AuthUser,
    Path((id, record)): Path<(i64, i64)>,
) -> Result<Json<BlockRecord>> {
    let sees_restricted = user.has_permission(permissions::BLOCK_REASON_VIEW);
    let record = BlockRecord::find(&db, record)
        .await?
        .filter(|r| r.party_id == id && visibility::can_read(r, sees_restricted))
        .ok_or(Error::NotFound)?;
    Ok(Json(record))
}

/// The unblock sheet's multipart form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct UnblockForm {
    /// Why the block is lifted.
    #[validate(required, max = 2000)]
    pub note: String,
    /// The statement behind lifting a `sanctions` block.
    pub basis: Option<UnblockBasis>,
    /// The evidence files.
    #[serde(default)]
    pub files: Vec<Upload>,
}

/// The sheet field a refusal belongs under.
fn refusal_field(message: &str) -> &'static str {
    let is = |key: &str| message == texts::english(&format!("wl_parties.errors.{key}"));
    if is("unblock_basis_required") {
        "basis"
    } else if is("unblock_evidence_required")
        || is("unblock_file_empty")
        || is("unblock_file_large")
        || is("unblock_file_type")
    {
        "files"
    } else {
        "note"
    }
}

/// `POST /parties/{id}/unblock`: lifts the hold and reloads the page. The
/// service refuses a blank note, a conflict of duties or a missing
/// permission (one neutral message), a missing basis or file, and a bad
/// file: a 422 with the message in the sheet, and nothing written.
async fn store_unblock(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<UnblockForm>,
) -> Result<Response> {
    let request = UnblockRequest {
        note: form.note.clone(),
        basis: form.basis,
        files: form.files.clone(),
    };
    match unblock(&state, &user, id, request).await {
        Ok(()) => Ok((
            Toast::success(texts::t(&lang, "wl_parties.unblock.done")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::Guard(message)) => {
            let mut errors = Errors::new();
            errors.add(refusal_field(&message), message);
            Err(Error::Validation(
                ValidationError::new(errors).with_input(&form),
            ))
        }
        Err(other) => Err(other.into_error()),
    }
}

/// The contact edit sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct ContactForm {
    /// The address or number.
    #[validate(required, max = 200)]
    pub value: String,
    /// A label, e.g. `work`.
    #[validate(max = 60)]
    pub label: Option<String>,
}

/// The contact point `point` of party `id`; 404 for one of another party.
async fn party_point(state: &AppState, id: i64, point: i64) -> Result<ContactPoint> {
    ContactPoint::find(&state.db, point)
        .await?
        .filter(|p| p.party_id == id)
        .ok_or(Error::NotFound)
}

/// `POST /parties/{id}/contact-points/{point}`: changes the value and label
/// and reloads the page. A new value makes an invalid point valid again. A
/// bad value is a 422 under the field; a point of another party is a 404.
async fn store_contact(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((id, point)): Path<(i64, i64)>,
    Valid(form): Valid<ContactForm>,
) -> Result<Response> {
    party_point(&state, id, point).await?;
    let value = form.value.clone();
    update_contact(&state, &user, point, &value, form.label.as_deref()).await?;
    Ok((
        Toast::success(texts::t(&lang, "wl_parties.contact.updated")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /parties/{id}/contact-points/{point}/verify`: marks the point
/// verified and valid and reloads the page; a point of another party is 404.
async fn verify_contact_point(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((id, point)): Path<(i64, i64)>,
) -> Result<Response> {
    party_point(&state, id, point).await?;
    verify_contact(&state, &user, point).await?;
    Ok((
        Toast::success(texts::t(&lang, "wl_parties.contact.verified")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `GET /parties/{id}/block-records/{record}/evidence/{file}`: an evidence
/// file of a closed Block record, as a download. It has the record's
/// visibility: a restricted record answers 404 without
/// `parties.block_reason.view`, as does a file of another record.
async fn block_evidence(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, record, file)): Path<(i64, i64, i64)>,
) -> Result<Download> {
    let sees_restricted = user.has_permission(permissions::BLOCK_REASON_VIEW);
    let record = BlockRecord::find(&state.db, record)
        .await?
        .filter(|r| r.party_id == id && visibility::can_read(r, sees_restricted))
        .ok_or(Error::NotFound)?;
    let file = UnblockEvidence::find(&state.db, file)
        .await?
        .filter(|f| f.block_record_id == record.id)
        .ok_or(Error::NotFound)?;
    Download::from_storage(&state.storage, &file.storage_key, file.file_name).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_shows_under_its_own_field() {
        let message = |key: &str| texts::english(&format!("wl_parties.errors.{key}"));
        assert_eq!(refusal_field(&message("unblock_basis_required")), "basis");
        assert_eq!(refusal_field(&message("unblock_file_type")), "files");
        assert_eq!(
            refusal_field(&message("unblock_evidence_required")),
            "files"
        );
        assert_eq!(refusal_field(&message("unblock_other_user")), "note");
        assert_eq!(refusal_field(&message("note_required")), "note");
    }

    fn form(kind: PartyKind) -> PartyForm {
        PartyForm {
            kind,
            display_name: None,
            legal_name_en: "Al Noor Trading LLC".to_owned(),
            legal_name_ar: None,
            default_language: Language::En,
            preferred_channel: Channel::Email,
            email: Some("accounts@alnoor.example".to_owned()),
            mobile: Some(" ".to_owned()),
            trn: Some("100123456700003".to_owned()),
        }
    }

    #[test]
    fn a_company_form_keeps_the_trn_and_one_primary_email() {
        let new = form(PartyKind::Company).into_new_party();
        let company = new.company.expect("company details");
        assert_eq!(company.trn.as_deref(), Some("100123456700003"));
        assert!(company.vat_registered);
        assert!(new.person.is_none());
        assert_eq!(new.contacts.len(), 1);
        assert_eq!(new.contacts[0].contact_type, ContactType::Email);
        assert!(new.contacts[0].is_primary);
        assert_eq!(new.source, "back_office");
    }

    #[test]
    fn a_person_form_has_no_company_details() {
        let new = form(PartyKind::Person).into_new_party();
        assert!(new.company.is_none());
        assert!(new.person.is_some());
    }

    #[test]
    fn the_grid_offers_no_restricted_reason_and_shows_blocked_as_on_hold() {
        let english = |key: &str| texts::english(&format!("wl_parties.{key}"));
        let mut reasons = BlockReasons::default();
        reasons
            .add("tax_evasion", true)
            .add("unpaid_deposit", false);
        let holds = filter_reasons(&reasons)
            .into_iter()
            .map(|code| (code.to_owned(), english(&format!("reason.{code}"))))
            .collect();
        let grid = grid_with(&english, holds, false);
        let reasons = grid
            .columns()
            .iter()
            .find(|c| c.key() == "hold_reason")
            .expect("hold reason column")
            .options()
            .iter()
            .map(|(value, _)| value.as_str())
            .collect::<Vec<_>>();
        assert!(reasons.contains(&"on_hold"));
        assert!(!reasons.contains(&"sanctions"));
        assert!(!reasons.contains(&"aml_concern"));
        assert!(!reasons.contains(&"tax_evasion"));
        assert!(reasons.contains(&"unpaid_deposit"));
        let status = grid
            .columns()
            .iter()
            .find(|c| c.key() == "status")
            .expect("status column");
        assert!(
            status
                .options()
                .iter()
                .any(|(value, label)| value == "blocked" && label == "On hold")
        );
    }
}

/// The label of an agreement kind: the app's `wl_kernel.block_kind.{kind}`
/// text, else the package's, else the key.
fn kind_label(lang: &Lang, kind: &str) -> String {
    let app_key = kernel_effects::label_key(kind);
    let found = lang.t(&app_key, &[]);
    if found != app_key {
        return found;
    }
    let key = format!("wl_parties.block_kind.{kind}");
    let label = texts::t(lang, &key);
    if label == key { kind.to_owned() } else { label }
}

/// The badge kind of an effect.
fn effect_tone(effect: BlockEffect) -> &'static str {
    match effect {
        BlockEffect::Pause => "warning",
        BlockEffect::Continue => "success",
        BlockEffect::Review => "info",
    }
}

/// One agreement kind as the settings list shows it.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct KindRow {
    key: String,
    label: String,
    owner: String,
    effect_label: String,
    effect_tone: &'static str,
    decider_role: Option<String>,
    source_label: String,
    has_source: bool,
}

/// What the settings list needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-parties/block_effects.html")]
pub(crate) struct BlockEffectsPage {
    tx: Texts,
    kinds: Vec<KindRow>,
}

/// `GET /parties/block-effects`: every agreement kind with its effect,
/// decider role and who set them. Read-only.
async fn block_effects_page(lang: Lang) -> Result<View> {
    let t = texts_in(&lang);
    let kinds = kernel_effects::kinds()
        .into_iter()
        .map(|info| KindRow {
            label: kind_label(&lang, &info.key),
            owner: info.owner,
            effect_label: t(&format!("effect.{}", info.effect.as_str())),
            effect_tone: effect_tone(info.effect),
            decider_role: info.decider_role,
            source_label: t(&format!("rule_source.{}", info.source.as_str())),
            has_source: info.has_source,
            key: info.key,
        })
        .collect();
    Ok(View::typed(BlockEffectsPage {
        tx: texts::all(&lang),
        kinds,
    }))
}

/// One open task in the list.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct TaskRow {
    id: i64,
    header: String,
    party: String,
    agreement: String,
    scope: String,
    pending: usize,
    opened: Option<DateTime>,
}

/// What the task list needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-parties/review_tasks.html")]
pub(crate) struct ReviewTasksPage {
    tx: Texts,
    tasks: Vec<TaskRow>,
}

/// The scope as the pages show it.
fn scope_label(lang: &Lang, task: &ReviewTask) -> String {
    match (&task.scope_kind, task.scope_id) {
        (Some(kind), Some(id)) => format!("{kind} #{id}"),
        _ => texts::t(lang, "wl_parties.review.global_scope"),
    }
}

/// The party's display name, or its id when it is gone.
async fn party_name(db: &Db, id: i64) -> Result<String> {
    Ok(Party::find(db, id)
        .await?
        .map_or_else(|| format!("#{id}"), |party| party.display_name))
}

/// `GET /parties/review-tasks`: the open tasks whose decider role the user
/// holds, oldest first.
async fn review_tasks_page(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
) -> Result<View> {
    let reasons = BlockReasons::of(&state);
    let mut tasks = Vec::new();
    for task in ReviewTask::open_for_user(&state.db, &user).await? {
        let pending = task
            .items(&state.db)
            .await?
            .iter()
            .filter(|item| item.is_pending())
            .count();
        tasks.push(TaskRow {
            id: task.id,
            header: crate::block_effects::header(&lang, &reasons, &task.display_reason),
            party: party_name(&state.db, task.party_id).await?,
            agreement: kind_label(&lang, &task.agreement_kind),
            scope: scope_label(&lang, &task),
            pending,
            opened: task.created_at,
        });
    }
    Ok(View::typed(ReviewTasksPage {
        tx: texts::all(&lang),
        tasks,
    }))
}

/// One agreement on the task page.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ItemRow {
    id: i64,
    label: String,
    decision_label: String,
    decision_tone: &'static str,
    pending: bool,
    decided_by: Option<String>,
    decided_at: Option<DateTime>,
}

/// One note on the task page.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct NoteRow {
    body: String,
    by: String,
    noted_at: DateTime,
}

/// What the task page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-parties/review_task.html")]
pub(crate) struct ReviewTaskPage {
    tx: Texts,
    task: ReviewTask,
    header: String,
    party: String,
    agreement: String,
    scope: String,
    status_label: String,
    status_tone: &'static str,
    open: bool,
    /// Whether a decision can be made: the task is open and its kind has a source.
    can_decide: bool,
    /// Whether "Send neutral message" is offered: the task is open and the
    /// `notify` module is on.
    can_send_neutral: bool,
    no_decider: bool,
    items: Vec<ItemRow>,
    notes: Vec<NoteRow>,
    decisions: Vec<(String, String)>,
}

/// The user's name, or `#id`.
async fn user_name(db: &Db, id: i64) -> Result<String> {
    Ok(User::find(db, id)
        .await?
        .map_or_else(|| format!("#{id}"), |user| user.name))
}

/// `GET /parties/review-tasks/{id}`: the header, the agreements with their
/// pause / continue / end action, and the notes. 403 for anyone without the
/// task's decider role.
async fn review_task_page(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(task): Found<ReviewTask>,
) -> Result<View> {
    if !task.can_decide(&user) {
        return Err(Error::Forbidden);
    }
    let db = &state.db;
    let t = texts_in(&lang);
    let mut items = Vec::new();
    for item in task.items(db).await? {
        let decided_by = match item.decided_by {
            Some(id) => Some(user_name(db, id).await?),
            None => None,
        };
        let key = item.decision.as_str();
        items.push(ItemRow {
            id: item.id,
            label: item.label.clone(),
            decision_label: t(&format!("review.decision.{key}")),
            decision_tone: match item.decision {
                review_tasks::ItemDecision::Pending => "warning",
                review_tasks::ItemDecision::Pause => "info",
                review_tasks::ItemDecision::Continue => "success",
                review_tasks::ItemDecision::End => "error",
            },
            pending: item.is_pending(),
            decided_by,
            decided_at: item.decided_at,
        });
    }
    let mut notes = Vec::new();
    for note in task.notes(db).await? {
        let by = match note.by_user {
            Some(id) => user_name(db, id).await?,
            None => t("review.system"),
        };
        notes.push(NoteRow {
            body: note.body,
            by,
            noted_at: note.noted_at,
        });
    }
    let open = task.status == ReviewTaskStatus::Open;
    let (status_label, status_tone) = if open {
        (t("review.status.open"), "warning")
    } else {
        (t("review.status.closed"), "success")
    };
    Ok(View::typed(ReviewTaskPage {
        tx: texts::all(&lang),
        header: crate::block_effects::header(
            &lang,
            &BlockReasons::of(&state),
            &task.display_reason,
        ),
        party: party_name(db, task.party_id).await?,
        agreement: kind_label(&lang, &task.agreement_kind),
        scope: scope_label(&lang, &task),
        status_label,
        status_tone,
        open,
        can_decide: open && kernel_effects::source_for(&task.agreement_kind).is_some(),
        can_send_neutral: open && crate::notify::available(&state),
        no_decider: task.decider_role.is_none(),
        items,
        notes,
        decisions: Decision::ALL
            .iter()
            .map(|d| {
                (
                    d.as_str().to_owned(),
                    t(&format!("review.decision.{}", d.as_str())),
                )
            })
            .collect(),
        task,
    }))
}

/// One open screening task in the list.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ScreeningTaskRow {
    id: i64,
    party: String,
    code: String,
    due: String,
    candidates: usize,
}

/// What the screening task list needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-parties/screening_tasks.html")]
pub(crate) struct ScreeningTasksPage {
    tx: Texts,
    tasks: Vec<ScreeningTaskRow>,
}

/// The party's code and display name, or `#id` when it is gone.
async fn party_code_and_name(db: &Db, id: i64) -> Result<(String, String)> {
    Ok(Party::find(db, id).await?.map_or_else(
        || (format!("#{id}"), format!("#{id}")),
        |party| (party.code, party.display_name),
    ))
}

/// `GET /parties/screening-tasks`: the open possible-match tasks, earliest
/// due first. Compliance only (`parties.block_reason.view`): 404 for anyone
/// else, so the page is not even known to exist (`PTY.ACL.02`).
async fn screening_tasks_page(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
) -> Result<View> {
    if !user.has_permission(permissions::BLOCK_REASON_VIEW) {
        return Err(Error::NotFound);
    }
    let mut tasks = Vec::new();
    for task in ScreeningTask::open_tasks(&state.db).await? {
        let (code, party) = party_code_and_name(&state.db, task.party_id).await?;
        tasks.push(ScreeningTaskRow {
            id: task.id,
            party,
            code,
            due: dubai_label(task.due_at),
            candidates: task.candidates(&state.db).await?.len(),
        });
    }
    Ok(View::typed(ScreeningTasksPage {
        tx: texts::all(&lang),
        tasks,
    }))
}

/// One screening run on the task page.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ScreeningRunRow {
    run_at: String,
    trigger: String,
    method: String,
    stale: String,
}

/// One candidate on the task page.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct CandidateRow {
    name: String,
    list: String,
    score: String,
    entry: String,
}

/// What the screening task page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-parties/screening_task.html")]
pub(crate) struct ScreeningTaskPage {
    tx: Texts,
    task: ScreeningTask,
    party_id: i64,
    party: String,
    code: String,
    due: String,
    flagged: String,
    closed: Option<String>,
    status_label: String,
    status_tone: &'static str,
    runs: Vec<ScreeningRunRow>,
    candidates: Vec<CandidateRow>,
    /// The confirm button is offered (open task, a possible match, no request
    /// pending, and the viewer may review).
    can_confirm: bool,
    /// A second reviewer's approval is pending.
    waiting: bool,
    /// The approval cards of the pending request.
    approvals: Vec<ApprovalCard>,
    /// The freeze and report tasks a confirmation opened.
    follow_up: Vec<FollowUpRow>,
}

/// A freeze or report task linked from the screening task page.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct FollowUpRow {
    id: i64,
    kind: String,
    status: String,
}

/// `GET /parties/screening-tasks/{id}`: the party code, each screening run
/// and the candidates. 404 without `parties.block_reason.view`, whether or
/// not the task exists.
async fn screening_task_page(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    if !user.has_permission(permissions::BLOCK_REASON_VIEW) {
        return Err(Error::NotFound);
    }
    let db = &state.db;
    let task = ScreeningTask::find(db, id).await?.ok_or(Error::NotFound)?;
    let t = texts_in(&lang);
    let yes_no = |flag: bool| {
        t(if flag {
            "screening.yes"
        } else {
            "screening.no"
        })
    };
    let runs = task
        .screenings(db)
        .await?
        .iter()
        .map(|run: &Screening| ScreeningRunRow {
            run_at: dubai_label(run.run_at),
            trigger: t(&format!(
                "screening.trigger.{}",
                match run.run_trigger {
                    ScreeningTrigger::Onboarding => "onboarding",
                    ScreeningTrigger::ListUpdate => "list_update",
                    ScreeningTrigger::Review => "review",
                    ScreeningTrigger::Manual => "manual",
                }
            )),
            method: t(&format!(
                "screening.method.{}",
                match run.method {
                    ScreeningMethod::Automatic => "automatic",
                    ScreeningMethod::Manual => "manual",
                }
            )),
            stale: yes_no(run.lists_stale),
        })
        .collect();
    let candidates = task
        .candidates(db)
        .await?
        .into_iter()
        .map(|c| CandidateRow {
            name: c.name,
            list: c.list_name,
            score: format!("{:.2}", c.score),
            entry: c.list_entry.unwrap_or_else(|| "\u{2014}".to_owned()),
        })
        .collect();
    let open = task.status == ScreeningTaskStatus::Open;
    let (code, party) = party_code_and_name(db, task.party_id).await?;
    let screenings = task.screenings(db).await?;
    let mut approval_cards = Vec::new();
    let mut waiting = false;
    let mut follow_up = Vec::new();
    for run in &screenings {
        let subject = Subject::new(confirm_match::SUBJECT_TYPE, run.id);
        waiting |= !approvals::pending_for(db, &subject).await?.is_empty();
        approval_cards.extend(approvals::cards_for(db, &subject, &user, &lang).await?);
        for follow in ComplianceTask::for_screening(db, run.id).await? {
            follow_up.push(FollowUpRow {
                id: follow.id,
                kind: t(&format!("tasks.kind.{}", kind_key(follow.kind))),
                status: t(&format!("tasks.status.{}", status_key(follow.status))),
            });
        }
    }
    let can_confirm = open
        && !waiting
        && user.has_permission(permissions::SCREENING_REVIEW)
        && screenings
            .iter()
            .any(|run| run.status == ScreeningStatus::PossibleMatch);
    Ok(View::typed(ScreeningTaskPage {
        can_confirm,
        waiting,
        approvals: approval_cards,
        follow_up,
        tx: texts::all(&lang),
        party_id: task.party_id,
        party,
        code,
        due: dubai_label(task.due_at),
        flagged: dubai_label(task.flagged_at),
        closed: task.closed_at.map(dubai_label),
        status_label: t(if open {
            "screening.status.open"
        } else {
            "screening.status.closed"
        }),
        status_tone: if open { "warning" } else { "success" },
        runs,
        candidates,
        task,
    }))
}

/// The decision form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct DecisionForm {
    /// `pause`, `continue` or `end`.
    #[validate(required, max = 16)]
    pub decision: String,
}

/// `POST /parties/review-tasks/{id}/items/{item}`: carries the decision out
/// through the agreement's source and records it only when that succeeds.
/// 403 without the decider role (nothing is recorded); a refusal from the
/// source leaves the item pending and is shown; the task closes by itself
/// when the last item is decided.
async fn decide_item(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((id, item_id)): Path<(i64, i64)>,
    Valid(form): Valid<DecisionForm>,
) -> Result<Response> {
    let task = ReviewTask::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    if !task.can_decide(&user) {
        return Err(Error::Forbidden);
    }
    let Some(decision) = Decision::ALL
        .into_iter()
        .find(|d| d.as_str() == form.decision)
    else {
        let mut errors = Errors::new();
        errors.add("decision", texts::t(&lang, "wl_parties.review.decide"));
        return Err(Error::Validation(
            ValidationError::new(errors).with_input(&form),
        ));
    };
    let item = task
        .items(&state.db)
        .await?
        .into_iter()
        .find(|item| item.id == item_id)
        .ok_or(Error::NotFound)?;
    let refresh = |toast: Toast| Ok((toast, renox::HxRefresh).into_response());
    if task.status != ReviewTaskStatus::Open {
        return refresh(Toast::warning(texts::t(
            &lang,
            "wl_parties.review.task_closed",
        )));
    }
    if !item.is_pending() {
        return refresh(Toast::warning(texts::t(
            &lang,
            "wl_parties.review.already_decided",
        )));
    }
    let Some(source) = kernel_effects::source_for(&task.agreement_kind) else {
        return refresh(Toast::error(texts::t(&lang, "wl_parties.review.no_source")));
    };
    let subject = Subject::new(&item.subject_type, item.subject_id);
    if let Err(refusal) = source.decide(&state, user.id, &subject, decision).await {
        let reason = refusal.to_string();
        return refresh(Toast::error(texts::t_with(
            &lang,
            "wl_parties.review.refused",
            &[("reason", &reason)],
        )));
    }
    review_tasks::record_decision(&state, &task, item.id, user.id, decision).await?;
    refresh(Toast::success(texts::t(&lang, "wl_parties.review.decided")))
}

/// `POST /parties/review-tasks/{id}/neutral-message`: queues the neutral
/// on-hold message to the party and adds a task note. 403 without the
/// decider role; nothing happens (and the action is hidden) without `notify`.
async fn send_neutral(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    let task = ReviewTask::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    if !task.can_decide(&user) {
        return Err(Error::Forbidden);
    }
    let refresh = |toast: Toast| Ok((toast, renox::HxRefresh).into_response());
    if task.status != ReviewTaskStatus::Open {
        return refresh(Toast::warning(texts::t(
            &lang,
            "wl_parties.review.task_closed",
        )));
    }
    if !crate::notify::send_neutral(&state, user.id, &task).await? {
        return refresh(Toast::warning(texts::t(
            &lang,
            "wl_parties.review.neutral_unavailable",
        )));
    }
    refresh(Toast::success(texts::t(
        &lang,
        "wl_parties.review.neutral_sent",
    )))
}

/// The text key part of a task kind.
fn kind_key(kind: ComplianceTaskKind) -> &'static str {
    match kind {
        ComplianceTaskKind::Freeze => "freeze",
        ComplianceTaskKind::Report => "report",
    }
}

/// The text key part of a task status.
fn status_key(status: ComplianceTaskStatus) -> &'static str {
    match status {
        ComplianceTaskStatus::Open => "open",
        ComplianceTaskStatus::Done => "done",
    }
}

/// A 422 with `message` under `field`.
fn invalid_field<T: Serialize>(field: &str, message: String, form: &T) -> Error {
    let mut errors = Errors::new();
    errors.add(field, message);
    Error::Validation(ValidationError::new(errors).with_input(form))
}

/// 404 for anyone without `parties.block_reason.view`: the compliance pages
/// are not even known to exist (`PTY.ACL.02`).
fn holders_only(user: &AuthUser) -> Result<()> {
    if user.has_permission(permissions::BLOCK_REASON_VIEW) {
        Ok(())
    } else {
        Err(Error::NotFound)
    }
}

/// The confirm sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct ConfirmForm {
    /// The reviewer's note on the decision.
    #[validate(required, max = 2000)]
    pub note: String,
}

/// `POST /parties/screening-tasks/{id}/confirm`: confirms the possible match
/// of the task. With the second-reviewer policy the screening stays a
/// possible match and "Waiting for a second reviewer" shows. 404 without
/// `parties.block_reason.view`, 403 without `screening.review`, 422 for a
/// blank note.
async fn confirm_screening(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<ConfirmForm>,
) -> Result<Response> {
    holders_only(&user)?;
    let task = ScreeningTask::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    let refresh = |toast: Toast| Ok((toast, renox::HxRefresh).into_response());
    let Some(screening) = task
        .screenings(&state.db)
        .await?
        .into_iter()
        .rev()
        .find(|run| run.status == ScreeningStatus::PossibleMatch)
    else {
        return refresh(Toast::warning(texts::t(
            &lang,
            "wl_parties.confirm.not_possible",
        )));
    };
    match confirm_match::confirm_match(&state, &user, &screening, &form.note).await {
        Ok(outcome) if outcome.is_pending() => refresh(Toast::success(texts::t(
            &lang,
            "wl_parties.confirm.requested",
        ))),
        Ok(_) => refresh(Toast::success(texts::t(&lang, "wl_parties.confirm.done"))),
        Err(TransitionError::Guard(message)) => Err(invalid_field("note", message, &form)),
        Err(TransitionError::NotFrom(_)) => refresh(Toast::warning(texts::t(
            &lang,
            "wl_parties.confirm.not_possible",
        ))),
        Err(other) => Err(other.into_error()),
    }
}

/// One task in the compliance task list.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ComplianceTaskRow {
    id: i64,
    kind: String,
    parties: usize,
    due: String,
    overdue: bool,
    status: String,
    status_tone: &'static str,
}

/// What the compliance task list needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-parties/compliance_tasks.html")]
pub(crate) struct ComplianceTasksPage {
    tx: Texts,
    tasks: Vec<ComplianceTaskRow>,
}

/// `GET /parties/compliance-tasks`: the open freeze and report tasks,
/// earliest due first, then the latest done ones. Holders only (404).
async fn compliance_tasks_page(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
) -> Result<View> {
    holders_only(&user)?;
    let db = &state.db;
    let t = texts_in(&lang);
    let mut all = ComplianceTask::open_tasks(db).await?;
    all.extend(
        ComplianceTask::query()
            .where_(ComplianceTask::STATUS.eq(ComplianceTaskStatus::Done))
            .order_by_desc(ComplianceTask::COMPLETED_AT)
            .limit(50)
            .get(db)
            .await?,
    );
    let now = renox::db::now();
    let mut tasks = Vec::new();
    for task in all {
        tasks.push(ComplianceTaskRow {
            id: task.id,
            kind: t(&format!("tasks.kind.{}", kind_key(task.kind))),
            parties: task.parties(db).await?.len(),
            due: dubai_label(task.due_at),
            overdue: task.is_overdue(now),
            status: t(&format!("tasks.status.{}", status_key(task.status))),
            status_tone: if task.status == ComplianceTaskStatus::Open {
                "warning"
            } else {
                "success"
            },
        });
    }
    Ok(View::typed(ComplianceTasksPage {
        tx: texts::all(&lang),
        tasks,
    }))
}

/// One named party on a freeze or report task.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct NamedPartyRow {
    id: i64,
    code: String,
    name: String,
    standing: String,
    state: String,
    record: bool,
}

/// One funds line on a freeze task.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct FundRow {
    id: i64,
    party: String,
    label: String,
    kind: String,
    amount: String,
    amount_input: String,
    currency: String,
    confirmed: bool,
    confirmed_label: String,
    note: String,
}

/// What a freeze or report task page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-parties/compliance_task.html")]
pub(crate) struct ComplianceTaskPage {
    tx: Texts,
    task: ComplianceTask,
    header: String,
    is_freeze: bool,
    open: bool,
    overdue: bool,
    status_label: String,
    status_tone: &'static str,
    due: String,
    completed: Option<String>,
    frozen_at: Option<String>,
    reference: Option<String>,
    reported_on: Option<String>,
    screening_task: Option<i64>,
    parties: Vec<NamedPartyRow>,
    funds: Vec<FundRow>,
    /// A named party is not yet on hold with an open sanctions record.
    not_ready: bool,
    can_complete: bool,
    states: Vec<(String, String)>,
}

/// `GET /parties/compliance-tasks/{id}`: the parties and their hold state,
/// the funds list of a freeze task and the completing action. Holders only
/// (404); the actions need `screening.review` as well.
async fn compliance_task_page(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    holders_only(&user)?;
    let db = &state.db;
    let task = ComplianceTask::find(db, id).await?.ok_or(Error::NotFound)?;
    let t = texts_in(&lang);
    let mut parties = Vec::new();
    let mut not_ready = false;
    let mut names = BTreeMap::new();
    for named in task.parties(db).await? {
        let party = Party::find(db, named.party_id).await?;
        let record = BlockRecord::open_for(db, named.party_id)
            .await?
            .iter()
            .any(|r| r.reason_code == SANCTIONS);
        let held = party
            .as_ref()
            .is_some_and(|p| matches!(p.status, PartyStatus::Blocked | PartyStatus::Archived));
        not_ready |= !(held && record);
        let (code, name) = party.as_ref().map_or_else(
            || {
                (
                    format!("#{}", named.party_id),
                    format!("#{}", named.party_id),
                )
            },
            |p| (p.code.clone(), p.display_name.clone()),
        );
        names.insert(named.party_id, name.clone());
        parties.push(NamedPartyRow {
            id: named.party_id,
            code,
            name,
            standing: t(&format!(
                "tasks.standing.{}",
                match named.standing {
                    PartyStanding::Blocked => "blocked",
                    PartyStanding::Archived => "archived",
                    PartyStanding::BlockedAlready => "blocked_already",
                }
            )),
            state: t(match party.map(|p| p.status) {
                Some(PartyStatus::Blocked) => "tasks.state.held",
                Some(PartyStatus::Archived) => "tasks.state.archived",
                _ => "tasks.state.open",
            }),
            record,
        });
    }
    let is_freeze = task.kind == ComplianceTaskKind::Freeze;
    let funds = if is_freeze {
        task.funds(db)
            .await?
            .into_iter()
            .map(|f| FundRow {
                id: f.id,
                party: f
                    .party_id
                    .and_then(|p| names.get(&p).cloned())
                    .unwrap_or_else(|| "\u{2014}".to_owned()),
                label: f.label,
                kind: f.kind,
                amount: amount_text(f.amount_fils),
                amount_input: format!("{}.{:02}", f.amount_fils / 100, f.amount_fils % 100),
                currency: f.currency,
                confirmed: f.confirmed,
                confirmed_label: t(if f.confirmed { "tasks.yes" } else { "tasks.no" }),
                note: f.note.unwrap_or_default(),
            })
            .collect()
    } else {
        Vec::new()
    };
    let open = task.status == ComplianceTaskStatus::Open;
    let screening_task = Screening::find(db, task.screening_id)
        .await?
        .and_then(|screening| screening.task_id);
    Ok(View::typed(ComplianceTaskPage {
        tx: texts::all(&lang),
        header: t(&format!("tasks.kind.{}", kind_key(task.kind))),
        is_freeze,
        open,
        overdue: task.is_overdue(renox::db::now()),
        status_label: t(&format!("tasks.status.{}", status_key(task.status))),
        status_tone: if open { "warning" } else { "success" },
        due: dubai_label(task.due_at),
        completed: task.completed_at.map(dubai_label),
        frozen_at: task.frozen_at.map(dubai_label),
        reference: task.reference.clone(),
        reported_on: task
            .reported_on
            .map(|d| dubai_label(d).chars().take(10).collect()),
        screening_task,
        parties,
        funds,
        not_ready: is_freeze && open && not_ready,
        can_complete: open && user.has_permission(permissions::SCREENING_REVIEW),
        states: vec![
            ("confirmed".to_owned(), t("tasks.yes")),
            ("unconfirmed".to_owned(), t("tasks.no")),
        ],
        task,
    }))
}

/// The funds line form (add and edit).
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct FundForm {
    /// What the money is (a new line).
    #[validate(max = 200)]
    pub label: Option<String>,
    /// The kind of money (a new line).
    #[validate(max = 64)]
    pub kind: Option<String>,
    /// The amount, as typed.
    #[validate(required, max = 32)]
    pub amount: String,
    /// `confirmed` or `unconfirmed`.
    #[validate(max = 16)]
    pub state: Option<String>,
    /// A note on the line.
    #[validate(max = 1000)]
    pub note: Option<String>,
}

impl FundForm {
    /// The service's input; the amount must parse.
    fn input(&self, lang: &Lang) -> Result<FundInput> {
        let amount_fils = parse_amount(&self.amount).ok_or_else(|| {
            invalid_field(
                "amount",
                texts::t(lang, "wl_parties.errors.amount_invalid"),
                self,
            )
        })?;
        Ok(FundInput {
            label: self.label.clone().unwrap_or_default(),
            kind: self.kind.clone().unwrap_or_default(),
            amount_fils,
            confirmed: self.state.as_deref() == Some("confirmed"),
            note: self.note.clone(),
        })
    }
}

/// `POST /parties/compliance-tasks/{id}/funds`: adds a line by hand.
async fn add_compliance_fund(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<FundForm>,
) -> Result<Response> {
    holders_only(&user)?;
    let input = form.input(&lang)?;
    match compliance_tasks::add_fund(&state, &user, id, &input).await {
        Ok(()) => Ok((
            Toast::success(texts::t(&lang, "wl_parties.tasks.line_added")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::Guard(message)) => {
            let field = if input.label.trim().is_empty() {
                "label"
            } else {
                "amount"
            };
            Err(invalid_field(field, message, &form))
        }
        Err(other) => Err(other.into_error()),
    }
}

/// `POST /parties/compliance-tasks/{id}/funds/{fund}`: confirms or edits a
/// line.
async fn update_compliance_fund(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((id, fund)): Path<(i64, i64)>,
    Valid(form): Valid<FundForm>,
) -> Result<Response> {
    holders_only(&user)?;
    let input = form.input(&lang)?;
    match compliance_tasks::update_fund(&state, &user, id, fund, &input).await {
        Ok(()) => Ok((
            Toast::success(texts::t(&lang, "wl_parties.tasks.line_saved")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::Guard(message)) => Err(invalid_field("amount", message, &form)),
        Err(other) => Err(other.into_error()),
    }
}

/// The record freeze / record report form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct CompleteForm {
    /// Freeze: when the funds were frozen, Asia/Dubai wall-clock time; empty
    /// means now.
    pub frozen_at: Option<NaiveDateTime>,
    /// Report: the filing reference.
    #[validate(max = 120)]
    pub reference: Option<String>,
    /// Report: the date reported.
    pub reported_on: Option<NaiveDate>,
}

/// `POST /parties/compliance-tasks/{id}/complete`: records the freeze (time
/// given or now) or the report (reference and date) and completes the task.
/// 404 without `parties.block_reason.view`, 403 without `screening.review`,
/// 422 when the details are missing or a named party is not held.
async fn complete_compliance_task(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<CompleteForm>,
) -> Result<Response> {
    holders_only(&user)?;
    let task = ComplianceTask::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    let calendar = wl_kernel::calendar::calendar(&state);
    let input = CompleteInput {
        frozen_at: (task.kind == ComplianceTaskKind::Freeze).then(|| {
            form.frozen_at
                .map_or_else(renox::db::now, |at| calendar.from_local(at))
        }),
        reference: form.reference.clone(),
        reported_on: form
            .reported_on
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .map(|at| calendar.from_local(at)),
    };
    let refresh = |toast: Toast| Ok((toast, renox::HxRefresh).into_response());
    match compliance_tasks::complete(&state, &user, id, &input).await {
        Ok(()) => refresh(Toast::success(texts::t(
            &lang,
            "wl_parties.tasks.completed",
        ))),
        Err(TransitionError::NotFrom(_)) => refresh(Toast::warning(texts::t(
            &lang,
            "wl_parties.tasks.already_done",
        ))),
        Err(TransitionError::Guard(message)) => {
            let blank_reference = form
                .reference
                .as_deref()
                .is_none_or(|r| r.trim().is_empty());
            let field = match task.kind {
                ComplianceTaskKind::Freeze => "frozen_at",
                ComplianceTaskKind::Report if blank_reference => "reference",
                ComplianceTaskKind::Report => "reported_on",
            };
            Err(invalid_field(field, message, &form))
        }
        Err(other) => Err(other.into_error()),
    }
}
/// How many of the latest screenings the screening page shows.
const SCREENING_PAGE_LIMIT: u64 = 50;

/// One sanctions list on the screening page.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct SanctionsListRow {
    id: i64,
    /// Whether *Upload official file* shows on this row.
    can_upload: bool,
    /// Who uploaded the current version (a manual upload only).
    uploaded_by: String,
    /// The page the current version was taken from.
    source_url: String,
    name: String,
    source: String,
    version: String,
    updated: String,
    status_label: String,
    status_tone: &'static str,
    error: String,
}

/// One screening on the screening page.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ScreeningListRow {
    id: i64,
    party: String,
    code: String,
    run_at: String,
    trigger: String,
    status: String,
    lists: String,
    stale: bool,
}

/// What the screening page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-parties/screening.html")]
pub(crate) struct ScreeningPage {
    tx: Texts,
    lists: Vec<SanctionsListRow>,
    screenings: Vec<ScreeningListRow>,
}

/// `GET /parties/screening`: each sanctions list with its version, last
/// successful update, status and last error, and the latest screenings with
/// the lists they used and the stale badge. 403 unless the user holds
/// `screening.run`, `screening.review` or `parties.block_reason.view`.
async fn screening_page(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    if ![
        permissions::SCREENING_RUN,
        permissions::SCREENING_REVIEW,
        permissions::BLOCK_REASON_VIEW,
    ]
    .into_iter()
    .any(|permission| user.has_permission(permission))
    {
        return Err(Error::Forbidden);
    }
    let db = &state.db;
    let t = texts_in(&lang);
    let never = t("freshness.never");
    let dash = "\u{2014}".to_owned();
    let reviewer =
        user.has_permission(permissions::SCREENING_REVIEW) && UploadSettings::of(&state).enabled;
    let parsers = state
        .provided::<ListParsers>()
        .map(|p| (*p).clone())
        .unwrap_or_default();
    let mut lists = Vec::new();
    for list in SanctionsList::all(db).await? {
        {
            let stale = list.status == ListStatus::Stale;
            let manual = list.update_source == Some(UpdateSource::Manual);
            let uploaded_by = match list.updated_by {
                Some(id) if manual => user_name(db, id).await?,
                _ => dash.clone(),
            };
            lists.push(SanctionsListRow {
                id: list.id,
                can_upload: reviewer && stale && parsers.get(list.format).is_some(),
                uploaded_by,
                source_url: list.source_url.clone().unwrap_or_else(|| dash.clone()),
                name: list.name,
                source: if manual {
                    t("screening.source_manual")
                } else {
                    list.source
                },
                version: list.version.unwrap_or_else(|| dash.clone()),
                updated: list
                    .last_updated_at
                    .map_or_else(|| never.clone(), dubai_label),
                status_label: t(if stale {
                    "screening.list_status.stale"
                } else {
                    "screening.list_status.current"
                }),
                status_tone: if stale { "warning" } else { "success" },
                error: list.last_error.unwrap_or_else(|| dash.clone()),
            });
        }
    }
    let latest = Screening::query()
        .order_by_desc(Screening::ID)
        .limit(SCREENING_PAGE_LIMIT)
        .get(db)
        .await?;
    let mut screenings = Vec::new();
    for run in latest {
        let (code, party) = party_code_and_name(db, run.party_id).await?;
        let used = run
            .lists_used
            .0
            .iter()
            .map(|list| {
                format!(
                    "{} {} ({})",
                    list.name,
                    list.version.as_deref().unwrap_or("\u{2014}"),
                    list.last_updated_at
                        .map_or_else(|| never.clone(), dubai_label),
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        screenings.push(ScreeningListRow {
            id: run.id,
            party,
            code,
            run_at: dubai_label(run.run_at),
            trigger: t(&format!(
                "screening.trigger.{}",
                match run.run_trigger {
                    ScreeningTrigger::Onboarding => "onboarding",
                    ScreeningTrigger::ListUpdate => "list_update",
                    ScreeningTrigger::Review => "review",
                    ScreeningTrigger::Manual => "manual",
                }
            )),
            status: t(&format!(
                "screening.state.{}",
                match run.status {
                    ScreeningStatus::Pending => "pending",
                    ScreeningStatus::Clear => "clear",
                    ScreeningStatus::PossibleMatch => "possible_match",
                    ScreeningStatus::FalsePositive => "false_positive",
                    ScreeningStatus::ConfirmedMatch => "confirmed_match",
                }
            )),
            lists: if used.is_empty() { dash.clone() } else { used },
            stale: run.lists_stale,
        });
    }
    Ok(View::typed(ScreeningPage {
        tx: texts::all(&lang),
        lists,
        screenings,
    }))
}

/// The upload sheet's multipart form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct ListUploadForm {
    /// The official file.
    #[serde(default)]
    pub file: Option<Upload>,
    /// The page of the authority the file was taken from.
    #[serde(default)]
    pub source_url: Option<String>,
    /// The date of the list, `YYYY-MM-DD`.
    #[serde(default)]
    pub list_date: Option<String>,
}

/// The sheet field an upload refusal belongs under.
fn upload_field(error: &UploadError) -> &'static str {
    match error {
        UploadError::Refused(UploadRefusal::SourceUrlMissing | UploadRefusal::SourceUrlInvalid) => {
            "source_url"
        }
        UploadError::Refused(
            UploadRefusal::ListDateMissing
            | UploadRefusal::ListDateInvalid
            | UploadRefusal::ListDateInFuture
            | UploadRefusal::OlderThanCurrent,
        ) => "list_date",
        _ => "file",
    }
}

/// `POST /parties/screening/lists/{id}/upload`: takes the official file of a
/// stale list. 403 without `screening.review`; a refusal is a 422 with the
/// message in the sheet and nothing changed.
async fn upload_list(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<ListUploadForm>,
) -> Result<Response> {
    if !user.has_permission(permissions::SCREENING_REVIEW) {
        return Err(Error::Forbidden);
    }
    let Some(file) = form.file.clone() else {
        return Err(invalid_field(
            "file",
            texts::t(&lang, "wl_parties.errors.upload_file_required"),
            &form,
        ));
    };
    let request = UploadRequest {
        file,
        source_url: form.source_url.clone().unwrap_or_default(),
        list_date: form.list_date.clone().unwrap_or_default(),
    };
    match upload_list_file(&state, &user, id, request).await {
        Ok(_) => Ok((
            Toast::success(texts::t(&lang, "wl_parties.screening.upload_done")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(UploadError::Forbidden) => Err(Error::Forbidden),
        Err(UploadError::NotFound) => Err(Error::NotFound),
        Err(UploadError::Failed(e)) => Err(e),
        Err(other) => Err(invalid_field(
            upload_field(&other),
            other.to_string(),
            &form,
        )),
    }
}

/// The manual check sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct ManualCheckForm {
    /// The list that was checked.
    #[serde(default)]
    pub list_id: Option<i64>,
    /// The date of the list file, `YYYY-MM-DD`.
    #[serde(default)]
    pub list_date: Option<String>,
    /// `no_match` or `candidate`.
    #[serde(default)]
    pub result: Option<String>,
    /// The candidate's name as written on the list.
    #[serde(default)]
    pub name: Option<String>,
    /// The candidate's entry on the list.
    #[serde(default)]
    pub list_entry: Option<String>,
}

/// The sheet field a manual check refusal belongs under.
fn manual_field(why: &ManualRefusal) -> &'static str {
    match why {
        ManualRefusal::ListDateMissing
        | ManualRefusal::ListDateInvalid
        | ManualRefusal::ListDateInFuture => "list_date",
        ManualRefusal::CandidateNameMissing => "name",
        ManualRefusal::ListEntryMissing => "list_entry",
        _ => "list_id",
    }
}

/// `POST /parties/{id}/manual-check`: records a manual sanctions check.
/// 403 without `screening.review`; a refusal is a 422 in the sheet.
async fn store_manual_check(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(party): Found<Party>,
    Valid(form): Valid<ManualCheckForm>,
) -> Result<Response> {
    if !user.has_permission(permissions::SCREENING_REVIEW) {
        return Err(Error::Forbidden);
    }
    let Some(list_id) = form.list_id else {
        return Err(invalid_field(
            "list_id",
            texts::t(&lang, "wl_parties.errors.manual_list_required"),
            &form,
        ));
    };
    let result = match form.result.as_deref() {
        Some("no_match") => ManualResult::NoMatch,
        Some("candidate") => ManualResult::Candidate {
            name: form.name.clone().unwrap_or_default(),
            list_entry: form.list_entry.clone().unwrap_or_default(),
        },
        _ => {
            return Err(invalid_field(
                "result",
                texts::t(&lang, "wl_parties.errors.manual_result_required"),
                &form,
            ));
        }
    };
    let list_date = form.list_date.clone().unwrap_or_default();
    match record_manual_check(&state, &user, &party, list_id, &list_date, result).await {
        Ok(_) => Ok((
            Toast::success(texts::t(&lang, "wl_parties.manual_check.done")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(ManualCheckError::Forbidden) => Err(Error::Forbidden),
        Err(ManualCheckError::NotFound) => Err(Error::NotFound),
        Err(ManualCheckError::Refused(why)) => {
            Err(invalid_field(manual_field(&why), why.to_string(), &form))
        }
        Err(ManualCheckError::Run(e)) => Err(e.into_error()),
    }
}
