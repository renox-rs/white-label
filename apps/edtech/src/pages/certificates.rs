//! The certificate pages (10 `EDU.CRT.01`, §7.1 *Certificate*) and the public
//! page that verifies one.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /edtech/certificates` | `edtech.certificates` | `certificates.view` |
//! | `GET /edtech/certificates/{id}` | `edtech.certificate` | `certificates.view` |
//! | `POST /edtech/certificates/{id}/issue` | `edtech.certificate_issue` | `certificates.issue` |
//! | `POST /edtech/certificates/{id}/revoke` | `edtech.certificate_revoke` | `certificates.revoke` |
//! | `GET /verify/certificate/{code}` | `edtech.verify_certificate` | nobody (public) |
//!
//! The public page needs no login: the verification code printed on the
//! certificate is the credential. It says whether the certificate is valid,
//! superseded (and by which number, or that the replacement is pending) or
//! revoked, and shows no more than the number, the name, the course and the
//! dates. The reissue form and the printed copy are BL-EDU-16's.

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
        .require_auth()
}

/// The public route: anyone with the code may verify a certificate.
pub fn public_routes() -> Routes {
    Routes::new()
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

/// One certificate of the list.
#[derive(Serialize, Debug, Clone)]
pub struct CertificateRow {
    /// The primary key.
    pub id: i64,
    /// The number, or a draft label.
    pub number: String,
    /// The name on it.
    pub student: String,
    /// The course on it.
    pub course: String,
    /// The day the course was completed.
    pub completed: String,
    /// The status.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
}

fn row_of(lang: &Lang, c: &Certificate) -> CertificateRow {
    CertificateRow {
        id: c.id,
        number: c.label(),
        student: c.student_name.clone(),
        course: c.course_title.clone(),
        completed: c.completion_date.to_string(),
        status: status_text(lang, c.status),
        status_kind: status_kind(c.status),
    }
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "edtech/certificates.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    certificates: Vec<CertificateRow>,
}

/// `GET /edtech/certificates`.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    require(&user, permissions::CERTIFICATES_VIEW)?;
    let all = Certificate::query()
        .order_by_desc(Certificate::ID)
        .get(&state.db)
        .await?;
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        certificates: all.iter().map(|c| row_of(&lang, c)).collect(),
    }))
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
    let label_of = |other: Option<Certificate>| {
        other.map_or_else(|| none.clone(), |found| found.label())
    };
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
}

/// `GET /verify/certificate/{code}`: no login.
async fn verify(
    State(state): State<AppState>,
    lang: Lang,
    Path(code): Path<String>,
) -> Result<View> {
    let none = text(&lang, "none");
    let mut page = VerifyPage {
        tx: texts::all(&lang),
        found: false,
        state_text: text(&lang, "verify.unknown"),
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
    };
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
    page.issued = shown.issued_on.map_or_else(|| none.clone(), |d| d.to_string());
    page.revoked = shown.revoked_on.map_or(none, |d| d.to_string());
    Ok(View::typed(page))
}
