//! The documents 01 generates: the tenancy contract at lease `submit` and the
//! owner statement PDF at `publish` (01 Property §4.1 step 6, §4.4 step 2,
//! §7.1 *Lease* `submit` and *Owner statement* `publish`, `DOC.GEN.01`;
//! BL-PRP-31).
//!
//! **What is generated, and when** (this module):
//!
//! - lease `submit` ([`crate::leases::submit`]): the bilingual (English and
//!   Arabic) tenancy contract of the version, *Tenancy contract L-0042 v1*,
//!   from [`contract_data`] (parties, unit, term, rent, payment plan, deposit,
//!   clauses, signatory block), template [`TENANCY_CONTRACT_TEMPLATE`];
//! - owner statement `publish` ([`publish`]): the bilingual statement PDF from
//!   [`statement_data`] (collected, expenses, fee, adjustments, balance and the
//!   lines), template [`OWNER_STATEMENT_TEMPLATE`].
//!
//! Each is generated before the transition runs: when the generation fails the
//! transition is refused with the error and nothing is submitted or published.
//! A record has one document per template ([`GeneratedDocument`]): the
//! contract of a submitted version and the PDF of a published statement are
//! never generated again; a renewal or amendment version, and a revision of a
//! statement, are other records and get their own, while the earlier stay.
//! The notification that follows carries the document's id
//! (`lease.contract_for_signature`, `owner_statement.published`).
//!
//! **Who renders and stores it:** a [`PdfDocuments`] generator, set for the
//! app with [`set_generator`]. Rendering templates into a PDF stored as a
//! document of the record is wl-documents' (BL-DOC-11) and attaching the file
//! to the e-mail is wl-notify's (BL-NTF-10); neither has a kernel contract
//! yet, so no generator is set by default and, without one, nothing is
//! generated and the transitions behave as before.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError, RwLock};

use renox::db::DateTime;
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{StateMachine, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use crate::building::Building;
use crate::lease::Lease;
use crate::money_text;
use crate::statement::{OwnerStatement, STATEMENT_SUBJECT_TYPE};
use crate::statement_machine::PUBLISH;
use crate::unit::Unit;

/// The template of the tenancy contract.
pub const TENANCY_CONTRACT_TEMPLATE: &str = "property.tenancy_contract";

/// The template of the owner statement PDF.
pub const OWNER_STATEMENT_TEMPLATE: &str = "property.owner_statement";

/// The document kind a generated tenancy contract is stored as.
pub const TENANCY_CONTRACT_KIND: &str = "tenancy_contract";

/// The document kind a generated owner statement is stored as.
pub const OWNER_STATEMENT_KIND: &str = "owner_statement";

/// What to generate.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PdfRequest {
    /// The template.
    pub template: &'static str,
    /// The document kind to store it as.
    pub kind: &'static str,
    /// The record it belongs to.
    pub subject: Subject,
    /// The title, e.g. *Tenancy contract L-000042 v1*.
    pub title: String,
    /// The languages, in order: English then Arabic.
    pub languages: Vec<&'static str>,
    /// The template's data.
    pub data: Value,
}

/// The future a generator answers with.
pub type PdfFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Renders a [`PdfRequest`] and stores the file as a document of its subject,
/// returning the document's id. wl-documents provides it once BL-DOC-11 is
/// built; the host sets it with [`set_generator`].
pub trait PdfDocuments: Send + Sync + 'static {
    /// Generates and stores `request`; an error refuses the transition.
    fn generate<'a>(&'a self, state: &'a AppState, request: PdfRequest) -> PdfFuture<'a, i64>;
}

/// Where the app keeps its generator.
#[derive(Default)]
pub(crate) struct GeneratorSlot(RwLock<Option<Arc<dyn PdfDocuments>>>);

/// Sets the app's generator (and replaces one set before).
pub fn set_generator(state: &AppState, generator: Arc<dyn PdfDocuments>) {
    if let Some(slot) = state.provided::<GeneratorSlot>() {
        *slot.0.write().unwrap_or_else(PoisonError::into_inner) = Some(generator);
    }
}

/// The app's generator, if one is set.
fn generator(state: &AppState) -> Option<Arc<dyn PdfDocuments>> {
    state
        .provided::<GeneratorSlot>()
        .and_then(|slot| slot.0.read().unwrap_or_else(PoisonError::into_inner).clone())
}

/// A document 01 generated for a record.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "property_generated_documents",
    unique(subject_type, subject_id, template)
)]
pub struct GeneratedDocument {
    /// The primary key.
    pub id: i64,
    /// The record: its type.
    pub subject_type: String,
    /// The record: its id.
    pub subject_id: i64,
    /// The template.
    pub template: String,
    /// The title.
    pub title: String,
    /// The stored document (wl-documents).
    pub document_id: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl GeneratedDocument {
    /// The document of `template` generated for `subject`, if any.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of(
        db: &Db,
        subject: &Subject,
        template: &str,
    ) -> Result<Option<GeneratedDocument>> {
        GeneratedDocument::query()
            .where_(GeneratedDocument::SUBJECT_TYPE.eq(subject.subject_type.as_str()))
            .where_(GeneratedDocument::SUBJECT_ID.eq(subject.subject_id))
            .where_(GeneratedDocument::TEMPLATE.eq(template))
            .first(db)
            .await
    }
}

/// The document of `request`: the one generated before, or a new one from the
/// generator. `None` without a generator.
///
/// # Errors
///
/// The database's errors and the generator's.
pub async fn ensure(state: &AppState, request: PdfRequest) -> Result<Option<GeneratedDocument>> {
    if let Some(found) = GeneratedDocument::of(&state.db, &request.subject, request.template).await?
    {
        return Ok(Some(found));
    }
    let Some(generator) = generator(state) else {
        return Ok(None);
    };
    let (subject, template, title) = (
        request.subject.clone(),
        request.template,
        request.title.clone(),
    );
    let document_id = generator.generate(state, request).await?;
    GeneratedDocument::create(
        &state.db,
        GeneratedDocument {
            subject_type: subject.subject_type,
            subject_id: subject.subject_id,
            template: template.to_owned(),
            title,
            document_id,
            ..Default::default()
        },
    )
    .await
    .map(Some)
}

/// The title of the contract of `lease`: *Tenancy contract L-000042 v1*.
pub fn contract_title(lease: &Lease) -> String {
    format!("Tenancy contract {}", lease.number_with_version())
}

/// The data of the tenancy contract of `lease`, in English and Arabic.
///
/// # Errors
///
/// The database's errors.
pub async fn contract_data(state: &AppState, lease: &Lease) -> Result<Value> {
    let unit = Unit::find(&state.db, lease.unit_id).await?;
    let building = Building::find(&state.db, lease.building_id).await?;
    let tenant_en = wl_kernel::parties::directory(state)
        .display_name(&state.db, lease.tenant(), "en")
        .await?
        .unwrap_or_default();
    let tenant_ar = wl_kernel::parties::directory(state)
        .display_name(&state.db, lease.tenant(), "ar")
        .await?
        .unwrap_or_else(|| tenant_en.clone());
    let plan: Vec<Value> = lease
        .charges(&state.db)
        .await?
        .iter()
        .map(|c| {
            json!({
                "description": c.description,
                "due": c.due_date.to_string(),
                "amount": money_text(c.amount),
            })
        })
        .collect();
    Ok(json!({
        "title": {"en": contract_title(lease), "ar": format!("عقد إيجار {}", lease.number_with_version())},
        "lease": lease.number_with_version(),
        "tenant": {"en": tenant_en, "ar": tenant_ar},
        "unit": unit.map(|u| u.number).unwrap_or_default(),
        "building": building.map(|b| b.name_en).unwrap_or_default(),
        "start": lease.start_date.to_string(),
        "end": lease.end_date.to_string(),
        "annual_rent": money_text(lease.annual_rent),
        "payment_plan": plan,
        "deposit": money_text(if lease.deposit_waived { 0 } else { lease.deposit_amount }),
        "clauses": lease.terms.clone().unwrap_or_default(),
        "signatories": {"en": ["Landlord", "Tenant"], "ar": ["المؤجر", "المستأجر"]},
    }))
}

/// The contract of the draft `lease`, generated before `submit` (or the one
/// generated before). `None` without a generator.
///
/// # Errors
///
/// The database's errors and the generator's: `submit` is refused then.
pub async fn contract_for(state: &AppState, lease: &Lease) -> Result<Option<GeneratedDocument>> {
    if generator(state).is_none() {
        return Ok(None);
    }
    let request = PdfRequest {
        template: TENANCY_CONTRACT_TEMPLATE,
        kind: TENANCY_CONTRACT_KIND,
        subject: lease.subject(),
        title: contract_title(lease),
        languages: vec!["en", "ar"],
        data: contract_data(state, lease).await?,
    };
    ensure(state, request).await
}

/// The title of the PDF of `statement`.
pub fn statement_title(statement: &OwnerStatement) -> String {
    format!("Owner statement {}", statement.number())
}

/// The data of the PDF of `statement`, in English and Arabic.
///
/// # Errors
///
/// The database's errors.
pub async fn statement_data(db: &Db, statement: &OwnerStatement) -> Result<Value> {
    let lines: Vec<Value> = statement
        .lines(db)
        .await?
        .iter()
        .map(|line| {
            json!({
                "kind": line.kind.as_str(),
                "description": line.description,
                "amount": money_text(line.amount),
            })
        })
        .collect();
    Ok(json!({
        "title": {"en": statement_title(statement), "ar": format!("كشف حساب المالك {}", statement.number())},
        "statement": statement.number(),
        "period_start": statement.period_start.to_string(),
        "period_end": statement.period_end.to_string(),
        "brought_forward": money_text(statement.brought_forward),
        "collected": money_text(statement.collected),
        "expenses": money_text(statement.expenses),
        "fee": money_text(statement.fee),
        "adjustments": money_text(statement.adjustments),
        "balance": money_text(statement.balance()),
        "lines": lines,
    }))
}

/// The PDF of `statement`, generated before `publish` (or the one generated
/// before). `None` without a generator.
///
/// # Errors
///
/// The database's errors and the generator's: `publish` is refused then.
pub async fn statement_pdf(
    state: &AppState,
    statement: &OwnerStatement,
) -> Result<Option<GeneratedDocument>> {
    if generator(state).is_none() {
        return Ok(None);
    }
    let request = PdfRequest {
        template: OWNER_STATEMENT_TEMPLATE,
        kind: OWNER_STATEMENT_KIND,
        subject: Subject::new(STATEMENT_SUBJECT_TYPE, statement.id),
        title: statement_title(statement),
        languages: vec!["en", "ar"],
        data: statement_data(&state.db, statement).await?,
    };
    ensure(state, request).await
}

/// `user` publishes the approved statement `id`: its PDF is generated first
/// (when a generator is set), then `publish` runs and the owner is sent the
/// statement with it. When the PDF cannot be generated, `publish` is refused
/// with the error and the statement stays `approved`.
///
/// # Errors
///
/// `Forbidden` without `owner_statements.prepare`, `NotFrom` unless
/// `approved`, `Guard` (422) with the generator's error.
pub async fn publish(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<OwnerStatement, TransitionError> {
    let statement = OwnerStatement::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    if !crate::statement::may_prepare(user) {
        return Err(TransitionError::Forbidden);
    }
    if statement.status == crate::statement::StatementStatus::Approved
        && let Err(error) = statement_pdf(state, &statement).await
    {
        return Err(TransitionError::Guard(error.to_string()));
    }
    let _outcome = statement.transition(PUBLISH).by(user).run(state).await?;
    OwnerStatement::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))
}

/// Registers the slot of the generator.
pub(crate) fn register(app: &mut Registry) {
    app.provide(GeneratorSlot::default());
}
