//! The extension points of e-invoicing (wl-invoicing §8): the
//! [`EInvoiceAdapter`], one per accredited service provider (ASP), and the
//! [`EInvoiceScope`] that decides which documents and recipients are in the
//! mandate.
//!
//! An adapter sends the stored payload of a transmission and, later, tells
//! what the ASP made of it. It maps the payload to the ASP's format and the
//! ASP's error codes to [`AspAnswer::Rejected`]; it never changes a document.
//! `submit` is idempotent per transmission attempt: the same
//! [`Submission::key`] sent twice is one document at the ASP.
//!
//! The module's entities choose their adapter by name
//! ([`WlInvoicing::einvoicing`](crate::WlInvoicing::einvoicing)); an adapter
//! is installed under that name
//! ([`WlInvoicing::asp_adapter`](crate::WlInvoicing::asp_adapter)).
//! [`FakeAsp`] is a scripted adapter for tests and demos.
//!
//! The default scope ([`DefaultScope`]) puts a document in the mandate when
//! its recipient is a company (B2B and B2G); a person (B2C) is out. A vertical
//! replaces it, e.g. 03 for the 24-month exclusion of international goods
//! transport with an air waybill.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use renox::prelude::*;
use renox::serde_json::Value;
use wl_kernel::einvoice::{Answer, LookupFuture, RecipientLookup};
use wl_kernel::parties::PartyKind;
use wl_kernel::{LegalEntity, Subject};

use crate::config::InvoicingConfig;
use crate::einvoice_send::connection_key;

/// The future an adapter or scope method returns.
pub type AspFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// What the adapter is asked to send.
#[derive(Debug, Clone, PartialEq)]
pub struct Submission {
    /// The idempotency key: `<transmission id>-<attempt>`.
    pub key: String,
    /// The transmission.
    pub transmission: Subject,
    /// The invoice or credit note.
    pub document: Subject,
    /// The recipient identifier (Peppol ID or TRN) stored on the
    /// transmission.
    pub recipient_id: String,
    /// The structured payload, as stored.
    pub payload: Value,
    /// The SHA-256 of the stored payload text.
    pub hash: String,
}

/// What the ASP says about a submitted document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AspAnswer {
    /// Still being validated.
    Pending,
    /// Validated and passed on.
    Accepted,
    /// Refused, with the error codes and messages.
    Rejected(Vec<AspError>),
}

/// One error of a rejection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AspError {
    /// The code, e.g. `BR-CO-15`.
    pub code: String,
    /// The ASP's message.
    pub message: String,
}

impl AspError {
    /// An error `code` with `message`.
    pub fn new(code: &str, message: &str) -> Self {
        AspError {
            code: code.to_owned(),
            message: message.to_owned(),
        }
    }
}

/// One ASP: sends a document and reports what became of it.
pub trait EInvoiceAdapter: Send + Sync + 'static {
    /// Sends `submission`; the ASP's reference. An `Err` is an ASP that
    /// could not be reached (or took nothing): the transmission stays
    /// queued and is tried again.
    fn submit<'a>(
        &'a self,
        state: &'a AppState,
        submission: &'a Submission,
    ) -> AspFuture<'a, String>;

    /// Asks what the ASP made of the document with `reference`. An `Err` is
    /// an ASP that could not be reached.
    fn poll<'a>(&'a self, state: &'a AppState, reference: &'a str) -> AspFuture<'a, AspAnswer>;

    /// Asks the ASP for the recipient identifier of the company with the
    /// 15-digit `trn` (wl-parties §5, the *Look up* of a company's
    /// e-invoicing identifier). An `Err` is an ASP that could not be reached.
    /// The default, for an ASP without a directory, knows none.
    fn lookup<'a>(&'a self, _state: &'a AppState, _trn: &'a str) -> AspFuture<'a, Answer> {
        Box::pin(async { Ok(Answer::NotFound) })
    }
}

/// The kernel `RecipientLookup` of the app: the ASP adapter set for each
/// entity (§7 e-invoicing settings) answers the lookups of that entity,
/// through the entity's connection `asp.<code>`, whose health the kernel
/// reports and which must be `up` for a lookup to be made.
#[derive(Clone)]
pub(crate) struct AdapterLookup {
    /// The entity codes with the name of their adapter.
    pub adapters: Vec<(String, String)>,
}

impl AdapterLookup {
    /// The adapters of `config`, by entity code.
    pub fn of(config: &InvoicingConfig) -> Self {
        AdapterLookup {
            adapters: config
                .einvoice
                .iter()
                .filter_map(|e| {
                    e.adapter
                        .as_ref()
                        .map(|a| (e.entity_code.trim().to_lowercase(), a.clone()))
                })
                .collect(),
        }
    }

    fn adapter_name(&self, entity: &LegalEntity) -> Option<&str> {
        let code = entity.code.trim().to_lowercase();
        self.adapters
            .iter()
            .find(|(c, _)| *c == code)
            .map(|(_, a)| a.as_str())
    }
}

impl RecipientLookup for AdapterLookup {
    fn connection(&self, entity: &LegalEntity) -> Option<String> {
        self.adapter_name(entity)
            .map(|_| connection_key(&entity.code))
    }

    fn lookup<'a>(
        &'a self,
        state: &'a AppState,
        entity: &'a LegalEntity,
        trn: &'a str,
    ) -> LookupFuture<'a, Answer> {
        Box::pin(async move {
            let adapter = self.adapter_name(entity).and_then(|n| adapter_of(state, n));
            match adapter {
                Some(adapter) => adapter.lookup(state, trn).await,
                None => Ok(Answer::NotFound),
            }
        })
    }
}

/// Which documents are in the e-invoicing mandate.
pub trait EInvoiceScope: Send + Sync + 'static {
    /// Whether a document of `entity` to a recipient of `kind` is in scope.
    /// `document` is the invoice or credit note.
    fn in_scope<'a>(
        &'a self,
        state: &'a AppState,
        entity: &'a LegalEntity,
        document: &'a Subject,
        recipient: Option<PartyKind>,
    ) -> AspFuture<'a, bool>;
}

/// B2B and B2G are in scope, B2C is not: a company recipient is, a person
/// (or an unknown party) is not.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultScope;

impl EInvoiceScope for DefaultScope {
    fn in_scope<'a>(
        &'a self,
        _state: &'a AppState,
        _entity: &'a LegalEntity,
        _document: &'a Subject,
        recipient: Option<PartyKind>,
    ) -> AspFuture<'a, bool> {
        Box::pin(async move { Ok(recipient == Some(PartyKind::Company)) })
    }
}

/// The adapters and the scope the app installed, as `register` provides
/// them.
#[derive(Clone)]
pub(crate) struct Installed {
    /// The adapters by name.
    pub adapters: Vec<(String, Arc<dyn EInvoiceAdapter>)>,
    /// The scope.
    pub scope: Arc<dyn EInvoiceScope>,
}

/// The adapter named `name` in `state`, if installed.
pub(crate) fn adapter_of(state: &AppState, name: &str) -> Option<Arc<dyn EInvoiceAdapter>> {
    state.provided::<Installed>().and_then(|installed| {
        installed
            .adapters
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, adapter)| Arc::clone(adapter))
    })
}

/// The scope in `state`: the one installed, else [`DefaultScope`].
pub(crate) fn scope_of(state: &AppState) -> Arc<dyn EInvoiceScope> {
    state.provided::<Installed>().map_or_else(
        || Arc::new(DefaultScope) as Arc<dyn EInvoiceScope>,
        |installed| Arc::clone(&installed.scope),
    )
}

/// A scripted ASP for tests and demos. Cloning shares the script. It is
/// reachable and says [`AspAnswer::Pending`] until told otherwise.
#[derive(Clone, Default)]
pub struct FakeAsp(Arc<Mutex<FakeState>>);

#[derive(Default)]
struct FakeState {
    unreachable: bool,
    answer: Option<AspAnswer>,
    directory: Vec<(String, String)>,
    submitted: Vec<Submission>,
    polled: Vec<String>,
}

impl FakeAsp {
    /// A reachable ASP.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the ASP cannot be reached: `submit` and `poll` fail.
    pub fn unreachable(&self, down: bool) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).unreachable = down;
    }

    /// The ASP's directory knows `id` as the identifier of the company with
    /// `trn`.
    pub fn knows(&self, trn: &str, id: &str) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .directory
            .push((trn.to_owned(), id.to_owned()));
    }

    /// What `poll` answers from now on.
    pub fn answers(&self, answer: AspAnswer) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).answer = Some(answer);
    }

    /// The submissions the ASP took, oldest first (a repeated key once).
    pub fn submitted(&self) -> Vec<Submission> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .submitted
            .clone()
    }

    /// The references polled so far.
    pub fn polled(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .polled
            .clone()
    }
}

impl EInvoiceAdapter for FakeAsp {
    fn submit<'a>(
        &'a self,
        _state: &'a AppState,
        submission: &'a Submission,
    ) -> AspFuture<'a, String> {
        Box::pin(async move {
            let mut fake = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if fake.unreachable {
                return Err(Error::Status(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "The ASP does not answer.".to_owned(),
                ));
            }
            if !fake.submitted.iter().any(|s| s.key == submission.key) {
                fake.submitted.push(submission.clone());
            }
            Ok(format!("ASP-{}", submission.key))
        })
    }

    fn poll<'a>(&'a self, _state: &'a AppState, reference: &'a str) -> AspFuture<'a, AspAnswer> {
        Box::pin(async move {
            let mut fake = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if fake.unreachable {
                return Err(Error::Status(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "The ASP does not answer.".to_owned(),
                ));
            }
            fake.polled.push(reference.to_owned());
            Ok(fake.answer.clone().unwrap_or(AspAnswer::Pending))
        })
    }

    fn lookup<'a>(&'a self, _state: &'a AppState, trn: &'a str) -> AspFuture<'a, Answer> {
        Box::pin(async move {
            let fake = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if fake.unreachable {
                return Err(Error::Status(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "The ASP does not answer.".to_owned(),
                ));
            }
            Ok(fake
                .directory
                .iter()
                .find(|(known, _)| known == trn)
                .map_or(Answer::NotFound, |(_, id)| Answer::Found(id.clone())))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn submission(key: &str) -> Submission {
        Submission {
            key: key.to_owned(),
            transmission: Subject::new("invoicing.einvoice_transmission", 1),
            document: Subject::new("invoicing.invoice", 2),
            recipient_id: "0235:100000000000003".into(),
            payload: renox::serde_json::json!({}),
            hash: String::new(),
        }
    }

    #[test]
    fn the_error_of_a_rejection_keeps_its_code() {
        let error = AspError::new("BR-CO-15", "totals differ");
        assert_eq!(
            (error.code.as_str(), error.message.as_str()),
            ("BR-CO-15", "totals differ")
        );
    }

    #[renox::test]
    async fn a_repeated_key_is_one_submission_at_the_fake_asp() {
        let asp = FakeAsp::new();
        let app = renox::testing::TestApp::new(App::new()).await;
        let sub = submission("1-1");
        assert_eq!(asp.submit(app.state(), &sub).await.unwrap(), "ASP-1-1");
        asp.submit(app.state(), &sub).await.unwrap();
        assert_eq!(asp.submitted().len(), 1);
        asp.unreachable(true);
        assert!(asp.submit(app.state(), &submission("2-1")).await.is_err());
        assert!(asp.poll(app.state(), "ASP-1-1").await.is_err());
        asp.unreachable(false);
        assert_eq!(
            asp.poll(app.state(), "ASP-1-1").await.unwrap(),
            AspAnswer::Pending
        );
        asp.answers(AspAnswer::Accepted);
        assert_eq!(
            asp.poll(app.state(), "ASP-1-1").await.unwrap(),
            AspAnswer::Accepted
        );
    }

    #[renox::test]
    async fn the_default_scope_is_a_company_recipient() {
        let app = renox::testing::TestApp::new(App::new()).await;
        let entity = LegalEntity::default();
        let document = Subject::new("invoicing.invoice", 1);
        let scope = DefaultScope;
        assert!(
            scope
                .in_scope(app.state(), &entity, &document, Some(PartyKind::Company))
                .await
                .unwrap()
        );
        assert!(
            !scope
                .in_scope(app.state(), &entity, &document, Some(PartyKind::Person))
                .await
                .unwrap()
        );
        assert!(
            !scope
                .in_scope(app.state(), &entity, &document, None)
                .await
                .unwrap()
        );
    }
}
