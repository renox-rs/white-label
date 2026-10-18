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
//! | `GET /parties/block-effects` | `wl_parties.block_effects` | `parties.block` (read-only: the vertical sets the values) |
//! | `GET /parties/review-tasks` | `wl_parties.review_tasks` | signed in; lists the open tasks whose decider role the user holds |
//! | `GET /parties/review-tasks/{id}` | `wl_parties.review_task` | the task's decider role in its scope, else 403 |
//! | `POST /parties/review-tasks/{id}/items/{item}` | `wl_parties.review_decide` | the same; form field `decision` is `pause`, `continue` or `end` |
//!
//! Lists, search, filters and exports read the party's `hold_reason`, which
//! is only ever the display code, so they cannot show a restricted reason
//! (`PTY.ACL.02`). Every template is compiled in and can be replaced by the
//! app (`resources/views/wl-parties/…`).

use std::collections::BTreeMap;

use renox::Download;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::state_machine::{self, TransitionError};
use renox::validation::{Errors, ValidationError};
use serde::{Deserialize, Serialize};
use wl_kernel::block_effects::{self as kernel_effects, BlockEffect, Decision};
use wl_kernel::{PartyKind, PartyStatus, Subject};

use crate::block::block;
use crate::block_record::BlockRecord;
use crate::contacts::{ContactPoint, ContactType};
use crate::create::{NewParty, create};
use crate::details::{CompanyDetails, PersonDetails};
use crate::evidence::{UnblockBasis, UnblockEvidence};
use crate::party::{Channel, Language, Party};
use crate::permissions;
use crate::reasons::BlockReasons;
use crate::review_tasks::{self, ReviewTask, ReviewTaskStatus};
use crate::roles::PartyRole;
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
                .get("/parties/review-tasks/{id}", review_task_page)
                .name("wl_parties.review_task")
                .post("/parties/review-tasks/{id}/items/{item}", decide_item)
                .name("wl_parties.review_decide")
                .require_auth(),
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
    contacts: Vec<ContactPoint>,
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
    /// The Block records this user may read, newest first.
    block_records: Vec<BlockRecordRow>,
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
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        party,
        person,
        company,
        contacts,
        roles,
        timeline,
        hold_label,
        can_block,
        reasons,
        can_unblock,
        unblock_evidence,
        bases,
        block_records,
    }))
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
