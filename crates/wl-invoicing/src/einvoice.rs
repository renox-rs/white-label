//! The e-invoice transmission (wl-invoicing §3, §4 *E-invoice transmission*,
//! §5.12, `FIN.INV.01`): one sending of an invoice or credit note to the
//! entity's accredited service provider (ASP). It has a life of its own and
//! never changes the issued document.
//!
//! ```text
//! [*] --queue--> queued --submit--> submitted --accept--> accepted
//!                queued --submit--> queued   (the ASP cannot be reached)
//!                submitted --reject--> rejected --resubmit--> queued
//!                rejected --supersede--> superseded
//! ```
//!
//! - **queue** is the creation of the row in [`crate::einvoice_send`], after
//!   the document's `issue` commits: e-invoicing applies to the entity on the
//!   document's own issue date and the recipient is in scope. The payload is
//!   built from the issued document ([`crate::einvoice_payload`]) and the
//!   recipient identifier is read from the party and kept here.
//! - **submit** moves `queued` to `submitted` once the ASP took the payload and
//!   gave its reference. A try the ASP does not answer is not a transition:
//!   the row stays `queued` with its error and its next try, which is
//!   measured from the attempt's start (5 min, 30 min, 2 h, 6 h, then every
//!   6 h), and a queued transmission is never dropped.
//! - **accept** and **reject** are the ASP's answers; a rejection stores the
//!   error codes and messages and raises a task ([`crate::einvoice_tasks`]).
//! - **resubmit** (`einvoices.manage`) sends a rejected transmission again
//!   when its cause is outside the document and has been fixed; the payload
//!   is rebuilt, so the recipient identifier and party data are read again,
//!   and the attempt count rises.
//! - **supersede** (system) closes a rejected transmission whose cause is in
//!   the document, when a credit note with reason *e-invoice correction* is
//!   issued against it; it is linked to the credit note, and to the new
//!   invoice when that is issued.
//!
//! The machine is `#[model(state_machine)]`: the status moves only by a
//! transition.

use renox::db::Json;
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use crate::permissions;

/// The subject type of a transmission.
pub const SUBJECT_TYPE: &str = "invoicing.einvoice_transmission";

/// The subject type of an invoice.
pub const INVOICE: &str = "invoicing.invoice";

/// The subject type of a credit note.
pub const CREDIT_NOTE: &str = "invoicing.credit_note";

/// The reason of a credit note that corrects an invoice the ASP rejected for
/// its content (compared without case and surrounding spaces).
pub const CORRECTION_REASON: &str = "e-invoice correction";

/// The `submit` transition.
pub const SUBMIT: &str = "submit";
/// The `accept` transition.
pub const ACCEPT: &str = "accept";
/// The `reject` transition.
pub const REJECT: &str = "reject";
/// The `resubmit` transition.
pub const RESUBMIT: &str = "resubmit";
/// The `supersede` transition.
pub const SUPERSEDE: &str = "supersede";

/// Where a transmission stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TransmissionStatus {
    /// Waiting to be sent, or for a retry while the ASP is unreachable.
    #[default]
    Queued,
    /// The ASP has it and is validating it.
    Submitted,
    /// Validated and passed on.
    Accepted,
    /// Refused, with error codes.
    Rejected,
    /// Not sent again: the document was corrected by a credit note and a new
    /// invoice.
    Superseded,
}

/// One sending of a document to the ASP.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_einvoice_transmissions",
    state_machine,
    index(document_type, document_id),
    index(status, next_try_at),
    index(legal_entity_id, status)
)]
pub struct EInvoiceTransmission {
    /// The primary key.
    pub id: i64,
    /// The issuing legal entity.
    pub legal_entity_id: i64,
    /// `invoicing.invoice` or `invoicing.credit_note`.
    pub document_type: String,
    /// The document's id.
    pub document_id: i64,
    /// The ASP adapter of the entity when it was queued.
    pub adapter: Option<String>,
    /// The recipient identifier read from the party; never on the document.
    pub recipient_id: Option<String>,
    /// The structured payload built from the issued document.
    pub payload: Json<Value>,
    /// The SHA-256 of the stored payload text.
    pub payload_hash: String,
    /// 1 for the first sending, plus one for each `resubmit`.
    pub attempt: i64,
    /// How many tries of this attempt failed because the ASP could not be
    /// reached.
    pub tries: i64,
    /// The error of the last failed try.
    pub last_error: Option<String>,
    /// When the attempt began: the retry times are measured from it.
    pub queued_at: DateTime,
    /// When it may be tried again.
    pub next_try_at: Option<DateTime>,
    /// When the ASP took it.
    pub submitted_at: Option<DateTime>,
    /// When the ASP answered.
    pub response_at: Option<DateTime>,
    /// The ASP's reference.
    pub asp_reference: Option<String>,
    /// The error codes of a rejection.
    pub error_codes: Json<Vec<String>>,
    /// The error messages of a rejection.
    pub error_messages: Json<Vec<String>>,
    /// The credit note that corrected the document.
    pub replaced_by_credit_note_id: Option<i64>,
    /// The new invoice that replaces the document.
    pub replaced_by_invoice_id: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: TransmissionStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl EInvoiceTransmission {
    /// The transmission as a [`Subject`].
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The invoice or credit note it sends.
    pub fn document(&self) -> Subject {
        Subject::new(&self.document_type, self.document_id)
    }

    /// The transmissions of the document `document`, oldest first.
    pub async fn of_document(
        db: impl renox::db::Executor<'_>,
        document: &Subject,
    ) -> Result<Vec<EInvoiceTransmission>> {
        EInvoiceTransmission::query()
            .where_(EInvoiceTransmission::DOCUMENT_TYPE.eq(document.subject_type.as_str()))
            .where_(EInvoiceTransmission::DOCUMENT_ID.eq(document.subject_id))
            .order_by(EInvoiceTransmission::ID)
            .get(db)
            .await
    }

    /// The e-invoice status of the document `document`: the status of its
    /// latest transmission, `None` when it has none (out of scope, or before
    /// go-live).
    pub async fn status_of(db: &Db, document: &Subject) -> Result<Option<TransmissionStatus>> {
        Ok(EInvoiceTransmission::of_document(db, document)
            .await?
            .last()
            .map(|t| t.status))
    }

    /// Whether the document is called `reason`-corrected: the reason of a
    /// credit note is *e-invoice correction*.
    pub fn is_correction(reason: &str) -> bool {
        reason.trim().eq_ignore_ascii_case(CORRECTION_REASON)
    }
}

/// What a `submit` run carries: whether the entity has its adapter, and the
/// ASP's reference.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct SubmitInput {
    /// Whether the ASP adapter of the entity is configured.
    pub adapter_configured: bool,
    /// The ASP's reference.
    pub reference: String,
}

/// What a `reject` run carries: the ASP's errors.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct RejectInput {
    /// `(code, message)` of each error.
    pub errors: Vec<(String, String)>,
}

/// What a `resubmit` run carries: the user's confirmation and the payload
/// rebuilt from the same document with the party data read again.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ResubmitInput {
    /// The user says the cause is outside the document and has been fixed.
    pub cause_fixed: bool,
    /// The rebuilt payload.
    pub payload: Value,
    /// Its hash.
    pub hash: String,
    /// The recipient identifier read again.
    pub recipient_id: Option<String>,
    /// The adapter of the entity now.
    pub adapter: Option<String>,
}

/// What a `supersede` run carries: the credit note that corrects the
/// document.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct SupersedeInput {
    /// The credit note's id.
    pub credit_note_id: i64,
    /// The credit note's reason.
    pub reason: String,
}

fn run<T: serde::de::DeserializeOwned>(ctx: &TransitionCtx) -> Option<T> {
    ctx.input_as::<T>().ok()
}

/// Writes columns other than the status, in the transition's transaction.
async fn record(
    tx: &mut renox::db::Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    EInvoiceTransmission::query()
        .where_(EInvoiceTransmission::ID.eq(id))
        .update(&mut *tx, columns)
        .await?;
    Ok(())
}

fn submit_transition() -> Transition<EInvoiceTransmission> {
    Transition::new(SUBMIT)
        .label("Submit")
        .system()
        .from([TransmissionStatus::Queued])
        .to(TransmissionStatus::Submitted)
        .guard("adapter_configured", |_t: &EInvoiceTransmission, ctx| {
            let configured = run::<SubmitInput>(ctx).is_some_and(|i| i.adapter_configured);
            Box::pin(async move {
                if configured {
                    Ok(())
                } else {
                    Err("The entity has no ASP adapter configured.".to_owned())
                }
            })
        })
        .effect(|t: &EInvoiceTransmission, ctx, tx| {
            let id = t.id;
            let reference = run::<SubmitInput>(ctx).map(|i| i.reference);
            Box::pin(async move {
                let at = renox::db::now();
                record(
                    tx,
                    id,
                    &[
                        ("asp_reference", &reference),
                        ("submitted_at", &Some(at)),
                        ("next_try_at", &None::<DateTime>),
                        ("last_error", &None::<String>),
                        ("updated_at", &Some(at)),
                    ],
                )
                .await
            })
        })
}

fn accept_transition() -> Transition<EInvoiceTransmission> {
    Transition::new(ACCEPT)
        .label("Accept")
        .system()
        .from([TransmissionStatus::Submitted])
        .to(TransmissionStatus::Accepted)
        .effect(|t: &EInvoiceTransmission, _ctx, tx| {
            let id = t.id;
            Box::pin(async move {
                let at = renox::db::now();
                record(
                    tx,
                    id,
                    &[("response_at", &Some(at)), ("updated_at", &Some(at))],
                )
                .await
            })
        })
}

fn reject_transition() -> Transition<EInvoiceTransmission> {
    Transition::new(REJECT)
        .label("Reject")
        .system()
        .from([TransmissionStatus::Submitted])
        .to(TransmissionStatus::Rejected)
        .effect(|t: &EInvoiceTransmission, ctx, tx| {
            let id = t.id;
            let errors = run::<RejectInput>(ctx)
                .map(|i| i.errors)
                .unwrap_or_default();
            Box::pin(async move {
                let at = renox::db::now();
                let codes = Json(errors.iter().map(|(c, _)| c.clone()).collect::<Vec<_>>());
                let messages = Json(errors.iter().map(|(_, m)| m.clone()).collect::<Vec<_>>());
                record(
                    tx,
                    id,
                    &[
                        ("error_codes", &codes),
                        ("error_messages", &messages),
                        ("response_at", &Some(at)),
                        ("updated_at", &Some(at)),
                    ],
                )
                .await
            })
        })
}

fn resubmit_transition() -> Transition<EInvoiceTransmission> {
    Transition::new(RESUBMIT)
        .label("Resubmit")
        .tone("primary")
        .permission(permissions::EINVOICES_MANAGE)
        .from([TransmissionStatus::Rejected])
        .to(TransmissionStatus::Queued)
        .guard("cause_fixed", |_t: &EInvoiceTransmission, ctx| {
            let fixed = run::<ResubmitInput>(ctx).is_some_and(|i| i.cause_fixed);
            Box::pin(async move {
                if fixed {
                    Ok(())
                } else {
                    Err(
                        "Confirm that the cause is outside the issued document and has been \
                         fixed."
                            .to_owned(),
                    )
                }
            })
        })
        .guard("recipient_identifier", |_t: &EInvoiceTransmission, ctx| {
            let found = run::<ResubmitInput>(ctx)
                .and_then(|i| i.recipient_id)
                .is_some_and(|id| !id.trim().is_empty());
            Box::pin(async move {
                if found {
                    Ok(())
                } else {
                    Err("The party has no e-invoicing recipient identifier.".to_owned())
                }
            })
        })
        .effect(|t: &EInvoiceTransmission, ctx, tx| {
            let (id, attempt) = (t.id, t.attempt + 1);
            let input = run::<ResubmitInput>(ctx).unwrap_or_default();
            Box::pin(async move {
                let at = renox::db::now();
                let payload = Json(input.payload);
                let empty = Json(Vec::<String>::new());
                record(
                    tx,
                    id,
                    &[
                        ("attempt", &attempt),
                        ("tries", &0_i64),
                        ("payload", &payload),
                        ("payload_hash", &input.hash),
                        ("recipient_id", &input.recipient_id),
                        ("adapter", &input.adapter),
                        ("queued_at", &at),
                        ("next_try_at", &Some(at)),
                        ("last_error", &None::<String>),
                        ("asp_reference", &None::<String>),
                        ("submitted_at", &None::<DateTime>),
                        ("response_at", &None::<DateTime>),
                        ("error_codes", &empty),
                        ("error_messages", &empty),
                        ("updated_at", &Some(at)),
                    ],
                )
                .await
            })
        })
}

fn supersede_transition() -> Transition<EInvoiceTransmission> {
    Transition::new(SUPERSEDE)
        .label("Supersede")
        .system()
        .from([TransmissionStatus::Rejected])
        .to(TransmissionStatus::Superseded)
        .guard("cause_in_document", |_t: &EInvoiceTransmission, ctx| {
            let correction = run::<SupersedeInput>(ctx)
                .is_some_and(|i| EInvoiceTransmission::is_correction(&i.reason));
            Box::pin(async move {
                if correction {
                    Ok(())
                } else {
                    Err(
                        "Only a credit note with the reason e-invoice correction supersedes \
                         a rejected transmission."
                            .to_owned(),
                    )
                }
            })
        })
        .effect(|t: &EInvoiceTransmission, ctx, tx| {
            let id = t.id;
            let credit_note = run::<SupersedeInput>(ctx).map(|i| i.credit_note_id);
            Box::pin(async move {
                record(
                    tx,
                    id,
                    &[
                        ("replaced_by_credit_note_id", &credit_note),
                        ("updated_at", &Some(renox::db::now())),
                    ],
                )
                .await
            })
        })
}

impl StateMachine for EInvoiceTransmission {
    type State = TransmissionStatus;
    const COLUMN: renox::db::Col<EInvoiceTransmission, TransmissionStatus> =
        EInvoiceTransmission::STATUS;

    fn machine() -> Machine<Self> {
        use TransmissionStatus::{Accepted, Queued, Rejected, Submitted, Superseded};
        Machine::new()
            .initial([Queued])
            .state(Queued)
            .tone("neutral")
            .state(Submitted)
            .tone("info")
            .state(Accepted)
            .tone("success")
            .terminal()
            .state(Rejected)
            .tone("error")
            .state(Superseded)
            .tone("neutral")
            .terminal()
            .transition(submit_transition())
            .transition(accept_transition())
            .transition(reject_transition())
            .transition(resubmit_transition())
            .transition(supersede_transition())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<T: Serialize>(input: &T) -> TransitionCtx {
        TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).unwrap())
    }

    #[test]
    fn the_machine_has_the_five_states_and_transitions() {
        let machine = EInvoiceTransmission::machine();
        assert_eq!(machine.initial_states(), ["queued"]);
        let find = |name: &str| machine.find(name).expect("transition");
        assert_eq!(find(SUBMIT).from_states(), ["queued"]);
        assert_eq!(find(SUBMIT).to_state(), Some("submitted"));
        assert_eq!(find(ACCEPT).to_state(), Some("accepted"));
        assert_eq!(find(REJECT).from_states(), ["submitted"]);
        assert_eq!(find(REJECT).to_state(), Some("rejected"));
        assert_eq!(find(RESUBMIT).from_states(), ["rejected"]);
        assert_eq!(find(RESUBMIT).to_state(), Some("queued"));
        assert!(!find(RESUBMIT).is_system());
        assert_eq!(find(SUPERSEDE).to_state(), Some("superseded"));
        for name in [SUBMIT, ACCEPT, REJECT, SUPERSEDE] {
            assert!(find(name).is_system(), "{name}");
        }
        let terminal: Vec<_> = machine
            .states()
            .iter()
            .filter(|s| s.terminal)
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(terminal, ["accepted", "superseded"]);
    }

    #[renox::test]
    async fn submit_needs_the_adapter() {
        let machine = EInvoiceTransmission::machine();
        let submit = machine.find(SUBMIT).expect("submit");
        let t = EInvoiceTransmission::default();
        let with = SubmitInput {
            adapter_configured: true,
            reference: "ASP-1".into(),
        };
        assert!(submit.check_guards(&t, &ctx(&with)).await.is_ok());
        let without = SubmitInput::default();
        assert!(submit.check_guards(&t, &ctx(&without)).await.is_err());
    }

    #[renox::test]
    async fn resubmit_needs_the_confirmation_and_a_recipient_identifier() {
        let machine = EInvoiceTransmission::machine();
        let resubmit = machine.find(RESUBMIT).expect("resubmit");
        let t = EInvoiceTransmission::default();
        let ready = ResubmitInput {
            cause_fixed: true,
            recipient_id: Some("0235:100000000000003".into()),
            ..Default::default()
        };
        assert!(resubmit.check_guards(&t, &ctx(&ready)).await.is_ok());
        let unconfirmed = ResubmitInput {
            cause_fixed: false,
            ..ready.clone()
        };
        assert!(resubmit.check_guards(&t, &ctx(&unconfirmed)).await.is_err());
        let anonymous = ResubmitInput {
            recipient_id: Some("  ".into()),
            ..ready
        };
        assert!(resubmit.check_guards(&t, &ctx(&anonymous)).await.is_err());
    }

    #[renox::test]
    async fn only_a_correction_credit_note_supersedes() {
        let machine = EInvoiceTransmission::machine();
        let supersede = machine.find(SUPERSEDE).expect("supersede");
        let t = EInvoiceTransmission::default();
        let correction = SupersedeInput {
            credit_note_id: 3,
            reason: " E-Invoice Correction ".into(),
        };
        assert!(supersede.check_guards(&t, &ctx(&correction)).await.is_ok());
        let other = SupersedeInput {
            credit_note_id: 3,
            reason: "price error".into(),
        };
        assert!(supersede.check_guards(&t, &ctx(&other)).await.is_err());
    }

    #[test]
    fn a_transmission_names_itself_and_its_document() {
        let t = EInvoiceTransmission {
            id: 4,
            document_type: INVOICE.into(),
            document_id: 9,
            ..Default::default()
        };
        assert_eq!(
            t.subject(),
            Subject::new("invoicing.einvoice_transmission", 4)
        );
        assert_eq!(t.document(), Subject::new("invoicing.invoice", 9));
        assert!(EInvoiceTransmission::is_correction("e-invoice correction"));
        assert!(!EInvoiceTransmission::is_correction("wrong rate"));
    }

    #[test]
    fn statuses_are_stored_in_snake_case() {
        let stored: Vec<_> = TransmissionStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            stored,
            ["queued", "submitted", "accepted", "rejected", "superseded"]
        );
    }
}
