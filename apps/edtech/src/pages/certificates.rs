//! The certificate pages (10 `EDU.CRT.01`, §7.1 *Certificate*) and the public
//! page that verifies one.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /edtech/certificates` | `edtech.certificates` | `certificates.view` |
//! | `GET /edtech/certificates/{id}` | `edtech.certificate` | `certificates.view` |
//! | `POST /edtech/certificates/{id}/issue` | `edtech.certificate_issue` | `certificates.issue` |
//! | `POST /edtech/certificates/{id}/revoke` | `edtech.certificate_revoke` | `certificates.revoke` |
//! | `POST /edtech/certificates/{id}/reissue` | `edtech.certificate_reissue` | `certificates.issue` |
//! | `POST /edtech/certificates/{id}/copy` | `edtech.certificate_copy` | `certificates.issue` |
//! | `GET /edtech/certificates/{id}/print` | `edtech.certificate_print` | `certificates.view` |
//! | `GET /verify/certificate/{code}` | `edtech.verify_certificate` | nobody (public) |
//!
//! The public page needs no login: the verification code printed on the
//! certificate is the credential. It says whether the certificate is valid,
//! superseded (and by which number, or that the replacement is pending) or
//! revoked, and shows no more than the number, the name, the course and the
//! dates. The coordinator reissues an issued certificate with a reason and the
//! corrected data (never an edit of the issued one) and prints a lost copy again
//! (`print_copy`: the same document, the same number and code, nothing changes);
//! neither is offered for a draft, a revoked or a superseded certificate.

use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use super::{Texts, require, text};
use crate::certificate::{Certificate, CertificateStatus, verification_path};
use crate::certificates::{self, Verification};
use crate::permissions;
use crate::texts;

/// The routes of the back-office pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/edtech/certificates", index)
        .name("edtech.certificates")
        .get("/edtech/certificates/{id}", show)
        .name("edtech.certificate")
        .post("/edtech/certificates/{id}/issue", issue)
        .name("edtech.certificate_issue")
        .post("/edtech/certificates/{id}/revoke", revoke)
        .name("edtech.certificate_revoke")
        .post("/edtech/certificates/{id}/reissue", reissue)
        .name("edtech.certificate_reissue")
        .post("/edtech/certificates/{id}/copy", copy)
        .name("edtech.certificate_copy")
        .get("/edtech/certificates/{id}/print", print)
        .name("edtech.certificate_print")
        .require_auth()
}

/// The public route: anyone with the code may verify a certificate.
pub fn public_routes() -> Routes {
    Routes::new()
        .get("/verify/certificate", lookup)
        .name("edtech.verify_lookup")
        .get("/verify/certificate/{code}", verify)
        .name("edtech.verify_certificate")
}

/// The status as staff read it.
pub(crate) fn status_text(lang: &Lang, status: CertificateStatus) -> String {
    text(lang, &format!("certificates.status.{}", status.as_str()))
}

/// The badge kind of a status.
pub(crate) fn status_kind(status: CertificateStatus) -> &'static str {
    match status {
        CertificateStatus::Draft | CertificateStatus::Superseded => "info",
        CertificateStatus::Issued => "success",
        CertificateStatus::Revoked => "error",
    }
}

/// The certificates grid: search on the number, the name and the course,
/// filters by status and completion day, and the export for who issues them.
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    let statuses: Vec<(String, String)> = [
        CertificateStatus::Draft,
        CertificateStatus::Issued,
        CertificateStatus::Superseded,
        CertificateStatus::Revoked,
    ]
    .into_iter()
    .map(|status| (status.as_str().to_owned(), status_text(lang, status)))
    .collect();
    let grid = Grid::new("edtech-certificates")
        .title(&text(lang, "certificates.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        // The number once issued, else the draft label (`Certificate::label`).
        .column(
            Column::text("number", &text(lang, "certificates.number"))
                .computed(
                    "COALESCE({T}.\"number\", 'CRT-draft-' || CAST({T}.\"id\" AS TEXT))",
                )
                .frozen()
                .mobile()
                .searchable()
                .link("/edtech/certificates/{id}"),
        )
        .column(
            Column::text("student_name", &text(lang, "certificates.student"))
                .mobile()
                .searchable(),
        )
        .column(
            Column::text("course_title", &text(lang, "certificates.course"))
                .mobile()
                .searchable(),
        )
        .column(Column::date(
            "completion_date",
            &text(lang, "certificates.completed"),
        ))
        .column(Column::date("issued_on", &text(lang, "certificates.issued")).hidden())
        .column(Column::text("approval_no", &text(lang, "certificates.approval")).hidden())
        .column(
            Column::select("status", &text(lang, "fields.status"), statuses)
                .badges(&[
                    ("draft", "info"),
                    ("issued", "success"),
                    ("superseded", "info"),
                    ("revoked", "danger"),
                ])
                .mobile(),
        )
        .sort_by("-id")
        .row_url("/edtech/certificates/{id}")
        .cards_on_mobile()
        .empty_state(&text(lang, "certificates.empty"), None);
    if can_export { grid.exports() } else { grid }
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "edtech/certificates.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    certificates: GridPage<Certificate>,
}

/// `GET /edtech/certificates`, or its export (`?export=`, with
/// `certificates.issue`).
async fn index(
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    require(&user, permissions::CERTIFICATES_VIEW)?;
    let can_export = user.has_permission(permissions::CERTIFICATES_ISSUE);
    let grid = grid(&lang, can_export);
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    let query = Certificate::query();
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        certificates: grid.page(query, &request).await?,
    })
    .into_response())
}

/// What the page of one certificate needs.
#[derive(Serialize, ViewData)]
#[view(template = "edtech/certificate.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    id: i64,
    title: String,
    status: String,
    status_kind: &'static str,
    enrolment_id: i64,
    enrolment: String,
    student: String,
    course: String,
    approval: String,
    completed: String,
    issued: String,
    code: String,
    verify_path: String,
    replaces: String,
    replaced_by: String,
    revoke_reason: String,
    can_issue: bool,
    can_revoke: bool,
    can_reissue: bool,
    can_copy: bool,
    copies: i64,
    attestation: String,
    has_attestation: bool,
    reissue_reason: String,
    completed_value: String,
}

/// `GET /edtech/certificates/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    require(&user, permissions::CERTIFICATES_VIEW)?;
    let c = Certificate::find_or_404(&state.db, id).await?;
    let none = text(&lang, "none");
    let label_of =
        |other: Option<Certificate>| other.map_or_else(|| none.clone(), |found| found.label());
    let replaces = match c.supersedes_id {
        Some(old) => label_of(Certificate::find(&state.db, old).await?),
        None => none.clone(),
    };
    let replaced_by = match c.superseded_by_id {
        Some(new) => label_of(Certificate::find(&state.db, new).await?),
        None => none.clone(),
    };
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        id: c.id,
        title: c.label(),
        status: status_text(&lang, c.status),
        status_kind: status_kind(c.status),
        enrolment_id: c.enrolment_id,
        enrolment: format!("ENR-{:04}", c.enrolment_id),
        student: c.student_name.clone(),
        course: c.course_title.clone(),
        approval: c.approval_no.clone().unwrap_or_else(|| none.clone()),
        completed: c.completion_date.to_string(),
        issued: c.issued_on.map_or_else(|| none.clone(), |d| d.to_string()),
        code: c.verification_code.clone(),
        verify_path: c.verification_path(),
        replaces,
        replaced_by,
        revoke_reason: c.revoke_reason.clone().unwrap_or_else(|| none.clone()),
        can_issue: c.status == CertificateStatus::Draft && certificates::may_issue(&user),
        can_revoke: c.status == CertificateStatus::Issued
            && user.has_permission(permissions::CERTIFICATES_REVOKE),
        can_reissue: c.status == CertificateStatus::Issued && certificates::may_issue(&user),
        can_copy: c.status == CertificateStatus::Issued && certificates::may_issue(&user),
        copies: c.copies_printed,
        attestation: c.attestation_ref.clone().unwrap_or_else(|| none.clone()),
        has_attestation: c.attestation_ref.is_some(),
        reissue_reason: c.reissue_reason.clone().unwrap_or_else(|| none.clone()),
        completed_value: c.completion_date.to_string(),
    }))
}

/// The toast and refresh that answer a transition.
fn done(lang: &Lang, key: &str) -> Result<Response> {
    Ok((Toast::success(text(lang, key)), renox::HxRefresh).into_response())
}

/// `POST /edtech/certificates/{id}/issue`.
async fn issue(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    certificates::issue(&state, &user, id).await?;
    done(&lang, "certificates.issued_done")
}

/// The form of `revoke`.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct RevokeForm {
    /// Why the certificate is withdrawn.
    #[serde(default)]
    pub reason: String,
}

/// `POST /edtech/certificates/{id}/revoke`.
async fn revoke(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<RevokeForm>,
) -> Result<Response> {
    certificates::revoke(&state, &user, id, &form.reason).await?;
    done(&lang, "certificates.revoked_done")
}

/// What the public page needs.
#[derive(Serialize, ViewData)]
#[view(template = "edtech/verify.html")]
pub(crate) struct VerifyPage {
    tx: Texts,
    found: bool,
    state_text: String,
    state_kind: &'static str,
    number: String,
    student: String,
    course: String,
    completed: String,
    issued: String,
    revoked: String,
    has_replacement: bool,
    replacement_pending: bool,
    replacement_number: String,
    replacement_path: String,
    /// The result card is shown (not only the form that asks for the code).
    show_result: bool,
}

/// The code typed in the form of the verification page.
#[derive(Deserialize, Debug, Default)]
struct LookupQuery {
    #[serde(default)]
    code: String,
}

/// The verification page with nothing found.
fn blank_page(lang: &Lang) -> VerifyPage {
    VerifyPage {
        tx: texts::all(lang),
        found: false,
        state_text: text(lang, "verify.unknown"),
        state_kind: "warning",
        number: String::new(),
        student: String::new(),
        course: String::new(),
        completed: String::new(),
        issued: String::new(),
        revoked: String::new(),
        has_replacement: false,
        replacement_pending: false,
        replacement_number: String::new(),
        replacement_path: String::new(),
        show_result: true,
    }
}

/// `GET /verify/certificate?code=...`: the form that asks for the code; with a
/// code it goes to the page of that certificate. No login.
async fn lookup(
    lang: Lang,
    renox::axum::extract::Query(query): renox::axum::extract::Query<LookupQuery>,
) -> Result<Response> {
    let code: String = query
        .code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    if !code.is_empty() {
        return Ok(renox::axum::response::Redirect::to(&format!(
            "/verify/certificate/{}",
            code.to_ascii_uppercase()
        ))
        .into_response());
    }
    let mut page = blank_page(&lang);
    page.show_result = false;
    Ok(View::typed(page).into_response())
}

/// `GET /verify/certificate/{code}`: no login.
async fn verify(
    State(state): State<AppState>,
    lang: Lang,
    Path(code): Path<String>,
) -> Result<View> {
    let none = text(&lang, "none");
    let mut page = blank_page(&lang);
    let (shown, kind, label) = match certificates::verify(&state.db, &code).await? {
        Verification::Unknown => return Ok(View::typed(page)),
        Verification::Valid(shown) => (shown, "success", "verify.valid"),
        Verification::Revoked(shown) => (shown, "error", "verify.revoked"),
        Verification::Superseded { shown, replacement } => {
            match replacement {
                Some((number, new_code)) => {
                    page.has_replacement = true;
                    page.replacement_number = number;
                    page.replacement_path = verification_path(&new_code);
                }
                None => page.replacement_pending = true,
            }
            (shown, "info", "verify.superseded")
        }
    };
    page.found = true;
    page.state_text = text(&lang, label);
    page.state_kind = kind;
    page.number = shown.number;
    page.student = shown.student_name;
    page.course = shown.course_title;
    page.completed = shown.completion_date.to_string();
    page.issued = shown
        .issued_on
        .map_or_else(|| none.clone(), |d| d.to_string());
    page.revoked = shown.revoked_on.map_or(none, |d| d.to_string());
    Ok(View::typed(page))
}

/// The form of `reissue`.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ReissueForm {
    /// Why the certificate is reissued (a name or date correction, a changed
    /// result that still meets the criteria).
    #[serde(default)]
    pub reason: String,
    /// The corrected name; blank keeps the printed one.
    #[serde(default)]
    pub student_name: String,
    /// The corrected completion date; blank keeps the printed one.
    #[serde(default)]
    pub completion_date: String,
    /// Ticked when the coordinator confirms that the recorded KHDA attestation
    /// is copied to the new draft.
    #[serde(default)]
    pub copy_attestation: String,
}

/// `POST /edtech/certificates/{id}/reissue`: the certificate is superseded and a
/// linked draft carries the corrections; the page of the draft opens.
async fn reissue(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ReissueForm>,
) -> Result<Response> {
    let corrections = certificates::Corrections {
        student_name: Some(form.student_name.trim().to_owned()).filter(|name| !name.is_empty()),
        completion_date: if form.completion_date.trim().is_empty() {
            None
        } else {
            Some(super::parse_date(&form.completion_date)?)
        },
        copy_attestation: super::ticked(&form.copy_attestation),
    };
    let draft = certificates::reissue(&state, &user, id, &form.reason, corrections).await?;
    Ok((
        Toast::success(text(&lang, "certificates.reissued_done")),
        renox::HxRedirect(format!("/edtech/certificates/{}", draft.id)),
    )
        .into_response())
}

/// `POST /edtech/certificates/{id}/copy`: the lost copy is printed again; the
/// printable page opens.
async fn copy(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    certificates::print_copy(&state, &user, id).await?;
    Ok((
        Toast::success(text(&lang, "certificates.copy_done")),
        renox::HxRedirect(format!("/edtech/certificates/{id}/print")),
    )
        .into_response())
}

/// What the printable page of a certificate needs.
#[derive(Serialize, ViewData)]
#[view(template = "edtech/certificate_print.html")]
pub(crate) struct PrintPage {
    tx: Texts,
    id: i64,
    number: String,
    student: String,
    course: String,
    approval: String,
    has_approval: bool,
    completed: String,
    issued: String,
    code: String,
    verify_path: String,
    copies: i64,
}

/// `GET /edtech/certificates/{id}/print`: the certificate as it is printed: the
/// same wording, number and verification code for the original and every copy.
async fn print(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    require(&user, permissions::CERTIFICATES_VIEW)?;
    let c = Certificate::find_or_404(&state.db, id).await?;
    if c.status != CertificateStatus::Issued {
        return Err(super::invalid(&text(
            &lang,
            "certificates.problem.not_printable",
        )));
    }
    let none = text(&lang, "none");
    Ok(View::typed(PrintPage {
        tx: texts::all(&lang),
        id: c.id,
        number: c.label(),
        student: c.student_name.clone(),
        course: c.course_title.clone(),
        approval: c.approval_no.clone().unwrap_or_else(|| none.clone()),
        has_approval: c.states_approval && c.approval_no.is_some(),
        completed: c.completion_date.to_string(),
        issued: c.issued_on.map_or(none, |d| d.to_string()),
        code: c.verification_code.clone(),
        verify_path: c.verification_path(),
        copies: c.copies_printed,
    }))
}
