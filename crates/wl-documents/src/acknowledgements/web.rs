//! The acknowledgement pages (wl-documents §4.3, §5.7, `HRM.ACL.10`, issue #53):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /documents/acknowledgements` | `wl_documents.acknowledgements` | a signed-in user |
//! | `GET /documents/acknowledgements/{id}/files/{document}` | `wl_documents.acknowledgements.open` | the reader |
//! | `POST /documents/acknowledgements/{id}/acknowledge` | `wl_documents.acknowledgements.acknowledge` | the reader |
//! | `POST /documents/acknowledgements/{id}/in-person` | `wl_documents.acknowledgements.in_person` | the reader's manager, or `sops.manage` |
//!
//! The staff inbox lists the signed-in reader's pending acknowledgements, each
//! with the version and its accepted translations to open, and the list of
//! readers without an account whom the signed-in user manages, to acknowledge
//! in person. The routes need a signed-in user and carry no permission guard
//! of their own: the reader is any member of staff. The service checks who
//! acts. The SOP page shows [`report`]: who acknowledged (with method,
//! language and witness), who is pending and who is overdue, per version, and
//! the in-person records per witness.

use std::collections::BTreeMap;

use renox::Download;
use renox::axum::http::HeaderMap;
use renox::chrono::NaiveDate;
use renox::grid::{Action, Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use serde::{Deserialize, Serialize};
use wl_kernel::workforce::workforce;

use super::model::{AckStatus, SopAcknowledgement};
use super::service::{AckRefused, Acknowledgements, OwnSession};
use crate::overrides::web::dubai;
use crate::sops::{Sop, Sops};
use crate::{Document, permissions, texts};

/// The texts a template reads as `tx['acks.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The routes of the acknowledgement pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/documents/acknowledgements", index)
        .name("wl_documents.acknowledgements")
        .get("/documents/acknowledgements/{id}/files/{document}", open)
        .name("wl_documents.acknowledgements.open")
        .post("/documents/acknowledgements/{id}/acknowledge", acknowledge)
        .name("wl_documents.acknowledgements.acknowledge")
        .post("/documents/acknowledgements/{id}/in-person", in_person)
        .name("wl_documents.acknowledgements.in_person")
        .require_auth()
}

/// The package text `key` (without the `wl_documents.` prefix) in `lang`.
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_documents.{key}"))
}

/// The person's name in the page's language, else `#id`.
async fn name_of(state: &AppState, lang: &Lang, party: wl_kernel::PartyId, id: i64) -> String {
    wl_kernel::parties::directory(state)
        .display_name(&state.db, party, &lang.locale)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("#{id}"))
}

/// The name of user `id`, else `#id`.
async fn user_name(db: &Db, id: i64) -> String {
    User::find(db, id)
        .await
        .ok()
        .flatten()
        .map_or_else(|| format!("#{id}"), |u| u.name)
}

/// A file the reader can open.
#[derive(Serialize, Debug, Clone)]
pub struct FileLink {
    /// `English` or the language of a translation.
    pub label: String,
    /// Where it opens.
    pub url: String,
    /// The reader opened it already.
    pub opened: bool,
}

/// The inbox grid: the signed-in reader's own acknowledgements (`mine`) and
/// the in-person ones they witness (`in_person`), told apart by the "list"
/// filter. Search on SOP code and title, filters by list and due date. The
/// row menu confirms an own acknowledgement or records one in person (the
/// sheet `in-person`); the service refuses the one that does not apply.
pub fn grid(lang: &Lang, mine: &[i64], has_in_person: bool) -> Grid {
    let lists = vec![
        ("mine".to_owned(), text(lang, "acks.mine")),
        ("in_person".to_owned(), text(lang, "acks.in_person_list")),
    ];
    let title_sql = if lang.locale == "ar" {
        "(SELECT COALESCE(NULLIF(\"title_ar\", ''), \"title_en\") FROM \"wl_documents_sops\" WHERE \"id\" = {T}.\"sop_id\")"
    } else {
        "(SELECT \"title_en\" FROM \"wl_documents_sops\" WHERE \"id\" = {T}.\"sop_id\")"
    };
    // The ids are the signed-in reader's own rows, numbers from the database.
    let list_sql = if mine.is_empty() {
        "'in_person'".to_owned()
    } else {
        let ids: Vec<String> = mine.iter().map(i64::to_string).collect();
        format!(
            "CASE WHEN {{T}}.\"id\" IN ({}) THEN 'mine' ELSE 'in_person' END",
            ids.join(",")
        )
    };
    let mut grid = Grid::new("wl-documents-acknowledgements")
        .title(&text(lang, "acks.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::custom("sop", &text(lang, "sops.fields.code"))
                .frozen()
                .mobile(),
        )
        .column(
            Column::text("code", &text(lang, "sops.fields.code"))
                .computed(
                    "(SELECT \"code\" FROM \"wl_documents_sops\" WHERE \"id\" = {T}.\"sop_id\")",
                )
                .searchable()
                .hidden(),
        )
        .column(
            Column::text("title", &text(lang, "sops.fields.title"))
                .computed(title_sql)
                .searchable()
                .mobile(),
        )
        .column(
            Column::text("version", &text(lang, "sops.fields.version"))
                .computed("'v' || CAST({T}.\"version_no\" AS TEXT)"),
        )
        .column(Column::custom("reader", &text(lang, "acks.reader")).mobile())
        .column(Column::date("due_date", &text(lang, "acks.due")).mobile())
        .column(Column::custom("overdue", &text(lang, "acks.overdue")))
        .column(Column::custom("files", &text(lang, "acks.open")))
        .column(
            Column::bool("opened", &text(lang, "acks.opened"))
                .computed("{T}.\"opened_document_id\" IS NOT NULL")
                .hidden(),
        )
        .column(
            Column::select("list", &text(lang, "acks.list"), lists)
                .computed(&list_sql)
                .hidden(),
        )
        .sort_by("due_date,id")
        .cards_on_mobile()
        .empty_state(
            &text(lang, "acks.empty"),
            Some(&text(lang, "acks.empty_hint")),
        );
    if !mine.is_empty() {
        grid = grid.row_action(
            Action::new(
                &text(lang, "acks.acknowledge"),
                "/documents/acknowledgements/{id}/acknowledge",
            )
            .confirm(&text(lang, "acks.acknowledge_title")),
        );
    }
    if has_in_person {
        grid = grid.row_action(
            Action::new(
                &text(lang, "acks.in_person"),
                "/documents/acknowledgements/{id}/in-person",
            )
            .sheet("in-person"),
        );
    }
    grid
}

/// What the staff inbox needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-documents/acknowledgements.html")]
pub struct AcknowledgementsPage {
    tx: Texts,
    acks: GridPage<SopAcknowledgement>,
    in_person: bool,
}

/// `GET /documents/acknowledgements`: the signed-in reader's pending
/// acknowledgements, and the readers without an account they manage, as one
/// grid.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<View> {
    let day = wl_kernel::documents::today();
    let staff = workforce(&state);
    let own = staff
        .employee_of_user(&state.db, user.id)
        .await?
        .map(|e| e.0);
    let mut mine: Vec<i64> = Vec::new();
    let mut readers: BTreeMap<i64, String> = BTreeMap::new();
    if let Some(employee) = own {
        for ack in Acknowledgements::pending_of(&state.db, employee).await? {
            let sop = Sop::find_or_404(&state.db, ack.sop_id).await?;
            if sop.current_document_id != Some(ack.document_id) {
                continue;
            }
            mine.push(ack.id);
        }
    }
    let may_all = user.has_permission(permissions::SOPS_MANAGE);
    let mut in_person: Vec<i64> = Vec::new();
    for ack in SopAcknowledgement::query()
        .where_(SopAcknowledgement::STATUS.eq(AckStatus::Pending))
        .order_by(SopAcknowledgement::DUE_DATE)
        .order_by(SopAcknowledgement::ID)
        .get(&state.db)
        .await?
    {
        let Some(profile) = Acknowledgements::profile(&state, ack.employee_id).await? else {
            continue;
        };
        if profile.user_id.is_some() || !profile.listed {
            continue;
        }
        if !may_all && !profile.managers.contains(&user.id) {
            continue;
        }
        let sop = Sop::find_or_404(&state.db, ack.sop_id).await?;
        if sop.current_document_id != Some(ack.document_id) {
            continue;
        }
        readers.insert(
            ack.id,
            name_of(&state, &lang, profile.party, ack.employee_id).await,
        );
        in_person.push(ack.id);
    }
    let grid = grid(&lang, &mine, !in_person.is_empty());
    let ids: Vec<i64> = mine.iter().chain(&in_person).copied().collect();
    let visible = SopAcknowledgement::query()
        .where_(SopAcknowledgement::ID.is_in(if ids.is_empty() { vec![-1] } else { ids }));
    let page = grid.page(visible, &request).await?;
    // Per row: the SOP's page, the reader, the overdue mark and the files.
    let mut extra: BTreeMap<i64, Value> = BTreeMap::new();
    for ack in page.items() {
        let sop = Sop::find_or_404(&state.db, ack.sop_id).await?;
        let mut files = Vec::new();
        if mine.contains(&ack.id) {
            files.push(FileLink {
                label: text(&lang, "acks.version_file"),
                url: format!(
                    "/documents/acknowledgements/{}/files/{}",
                    ack.id, ack.document_id
                ),
                opened: ack.opened_document_id == Some(ack.document_id),
            });
            for link in Sops::translations(&state.db, ack.document_id).await? {
                let accepted = Document::find(&state.db, link.document_id)
                    .await?
                    .is_some_and(|d| d.status == crate::DocumentStatus::Accepted);
                if accepted {
                    files.push(FileLink {
                        label: link.language.clone(),
                        url: format!(
                            "/documents/acknowledgements/{}/files/{}",
                            ack.id, link.document_id
                        ),
                        opened: ack.opened_document_id == Some(link.document_id),
                    });
                }
            }
        }
        extra.insert(
            ack.id,
            json!({
                "sop_url": format!("/documents/sops/{}", sop.id),
                "sop_code": sop.code,
                "reader": readers.get(&ack.id).cloned().unwrap_or_default(),
                "overdue": ack.overdue_on(day),
                "files": files,
            }),
        );
    }
    let page = page.extend(|ack| extra.get(&ack.id).cloned().unwrap_or(Value::Null));
    Ok(View::typed(AcknowledgementsPage {
        tx: texts::all(&lang),
        acks: page,
        in_person: !in_person.is_empty(),
    }))
}

/// `GET /documents/acknowledgements/{id}/files/{document}`: the version or an
/// accepted translation of it, shown inline to the reader; the opening is
/// kept (`DOC.RUL.04`: the reader opened it).
async fn open(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, document)): Path<(i64, i64)>,
) -> Result<Download> {
    match Acknowledgements::open(&state, id, user.id, document).await? {
        Ok(opened) => {
            let bytes = opened.file.read(&state).await?.ok_or(Error::NotFound)?;
            Ok(
                Download::bytes(opened.file.file_name, opened.file.mime_type, bytes.to_vec())
                    .inline(),
            )
        }
        Err(refused) => Err(refused.into_error()),
    }
}

/// `POST /documents/acknowledgements/{id}/acknowledge`: the reader confirms
/// in their own session. A version that is no longer current is refused and
/// the reader is told to open the current one.
async fn acknowledge(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    renox::ClientIp(ip): renox::ClientIp,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response> {
    let session = OwnSession {
        ip: ip.map(|ip| ip.to_string()),
        user_agent: headers
            .get("user-agent")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
    };
    match Acknowledgements::acknowledge(&state, id, user.id, session).await? {
        Ok(_) => Ok((
            Toast::success(text(&lang, "acks.acknowledged_ok")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(refused) => Err(Error::Status(
            if matches!(refused, AckRefused::NotReader) {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            },
            refused.message_in(&lang),
        )),
    }
}

/// The in-person form.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct InPersonForm {
    /// The reader's drawn signature.
    #[validate(max = 100000)]
    #[serde(default)]
    pub signature: String,
}

/// `POST /documents/acknowledgements/{id}/in-person`: the signed-in user, the
/// reader's manager or a holder of `sops.manage`, records the reader's
/// acknowledgement with the drawn signature and is the witness.
async fn in_person(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<InPersonForm>,
) -> Result<Response> {
    let ack = SopAcknowledgement::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    let profile = Acknowledgements::profile(&state, ack.employee_id)
        .await?
        .ok_or(Error::NotFound)?;
    if !Acknowledgements::may_witness(&user, &profile) {
        return Err(Error::Forbidden);
    }
    match Acknowledgements::record_in_person(&state, id, user.id, &form.signature).await? {
        Ok(_) => Ok((
            Toast::success(text(&lang, "acks.in_person_ok")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(refused) => Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            refused.message_in(&lang),
        )),
    }
}

/// One line of the SOP page's report.
#[derive(Serialize, Debug, Clone)]
pub struct ReportRow {
    /// The reader.
    pub reader: String,
    /// When acknowledged, or the due date of a pending one (Asia/Dubai).
    pub when: String,
    /// The method of an acknowledgement, in words.
    pub method: String,
    /// The language of the file the reader acknowledged.
    pub language: String,
    /// The witness of an in-person record.
    pub witness: String,
    /// Pending and past its due date.
    pub overdue: bool,
    /// How many reminders were sent.
    pub reminders: i64,
}

/// The in-person records of one witness.
#[derive(Serialize, Debug, Clone)]
pub struct WitnessGroup {
    /// The witness.
    pub witness: String,
    /// The readers they recorded.
    pub readers: String,
}

/// The acknowledgements of one version.
#[derive(Serialize, Debug, Clone)]
pub struct VersionReport {
    /// `v2`.
    pub version: String,
    /// Whether it is the SOP's current version.
    pub current: bool,
    /// Who acknowledged.
    pub acknowledged: Vec<ReportRow>,
    /// Who is pending, overdue ones first.
    pub pending: Vec<ReportRow>,
    /// How many of the pending are overdue.
    pub overdue: usize,
    /// The in-person records per witness.
    pub in_person: Vec<WitnessGroup>,
}

/// The report of the SOP's page: its versions that have acknowledgements,
/// newest first (wl-documents §5.7 step 8).
pub async fn report(
    state: &AppState,
    lang: &Lang,
    sop: &Sop,
    day: NaiveDate,
) -> Result<Vec<VersionReport>> {
    let all = SopAcknowledgement::query()
        .where_(SopAcknowledgement::SOP_ID.eq(sop.id))
        .order_by(SopAcknowledgement::ID)
        .get(&state.db)
        .await?;
    let mut documents: Vec<i64> = all.iter().map(|a| a.document_id).collect();
    documents.sort_unstable_by(|a, b| b.cmp(a));
    documents.dedup();
    let mut names: BTreeMap<i64, String> = BTreeMap::new();
    let mut out = Vec::new();
    for document in documents {
        let mut report = VersionReport {
            version: String::new(),
            current: sop.current_document_id == Some(document),
            acknowledged: Vec::new(),
            pending: Vec::new(),
            overdue: 0,
            in_person: Vec::new(),
        };
        let mut witnesses: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for ack in all.iter().filter(|a| a.document_id == document) {
            report.version = format!("v{}", ack.version_no);
            if let std::collections::btree_map::Entry::Vacant(slot) = names.entry(ack.employee_id) {
                let name = match Acknowledgements::profile(state, ack.employee_id).await? {
                    Some(profile) => name_of(state, lang, profile.party, ack.employee_id).await,
                    None => format!("#{}", ack.employee_id),
                };
                slot.insert(name);
            }
            let reader = names[&ack.employee_id].clone();
            match ack.status {
                AckStatus::Acknowledged => {
                    let witness = match ack.witness_id {
                        Some(id) => user_name(&state.db, id).await,
                        None => String::new(),
                    };
                    if !witness.is_empty() {
                        witnesses
                            .entry(witness.clone())
                            .or_default()
                            .push(reader.clone());
                    }
                    report.acknowledged.push(ReportRow {
                        reader,
                        when: ack.acknowledged_at.map(dubai).unwrap_or_default(),
                        method: if ack.in_person() {
                            text(lang, "acks.method.drawn")
                        } else {
                            text(lang, "acks.method.click")
                        },
                        language: ack.language.clone(),
                        witness,
                        overdue: false,
                        reminders: ack.reminders_sent,
                    });
                }
                AckStatus::Pending => {
                    let overdue = ack.overdue_on(day);
                    if overdue {
                        report.overdue += 1;
                    }
                    report.pending.push(ReportRow {
                        reader,
                        when: ack.due_date.to_string(),
                        method: String::new(),
                        language: String::new(),
                        witness: String::new(),
                        overdue,
                        reminders: ack.reminders_sent,
                    });
                }
                AckStatus::Replaced | AckStatus::Cancelled => {}
            }
        }
        report.pending.sort_by_key(|r| (!r.overdue, r.when.clone()));
        report.in_person = witnesses
            .into_iter()
            .map(|(witness, readers)| WitnessGroup {
                witness,
                readers: readers.join(", "),
            })
            .collect();
        out.push(report);
    }
    Ok(out)
}
