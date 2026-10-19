//! The e-invoice board (wl-invoicing §5.12 step 5, `FIN.INV.01`):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /einvoices` | `wl_invoicing.einvoices` | `einvoices.view` |
//! | `POST /einvoices/{id}/resubmit` | `wl_invoicing.einvoice_resubmit` | `einvoices.manage` |
//! | `POST /invoices/{id}/replaces` | `wl_invoicing.invoice_replaces` | `invoices.create` |
//!
//! The board lists the transmissions per legal entity and state (`?entity=`,
//! `?status=`), with the document, the recipient identifier, the attempt, the
//! ASP's reference, the error codes of a rejection and the last error of a
//! queued one, and a banner with the count while transmissions wait for the
//! ASP. A rejected transmission offers *Resubmit* to a holder of
//! `einvoices.manage`, who confirms that the cause is outside the document
//! and has been fixed. The third route lets the user name, on a draft invoice,
//! the invoice it replaces after an *e-invoice correction*. The template is
//! compiled in and can be replaced by the app
//! (`resources/views/wl-invoicing/einvoices.html`).

use std::collections::BTreeMap;

use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};
use wl_kernel::LegalEntity;

use crate::einvoice::{EInvoiceTransmission, TransmissionStatus};
use crate::einvoice_send::{self, queue_banner, queued_count_all};
use crate::invoice::Invoice;
use crate::permissions;
use crate::texts;

/// The texts a template reads as `tx['einvoice.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The module's e-invoice routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/einvoices", index)
                .name("wl_invoicing.einvoices")
                .require_permission(permissions::EINVOICES_VIEW),
        )
        .merge(
            Routes::new()
                .post("/einvoices/{id}/resubmit", resubmit)
                .name("wl_invoicing.einvoice_resubmit")
                .require_permission(permissions::EINVOICES_MANAGE),
        )
        .merge(
            Routes::new()
                .post("/invoices/{id}/replaces", replaces)
                .name("wl_invoicing.invoice_replaces")
                .require_permission(permissions::CREATE),
        )
}

/// The query of the board.
#[derive(Deserialize, Default, Debug)]
pub(crate) struct Filter {
    /// The legal entity id; empty or missing for all.
    entity: Option<String>,
    /// A status code; empty or missing for all.
    status: Option<String>,
}

/// One transmission as the board shows it.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    id: i64,
    entity: String,
    document: String,
    link: String,
    recipient: String,
    attempt: i64,
    status: String,
    status_kind: &'static str,
    reference: String,
    problem: String,
    can_resubmit: bool,
}

/// What the board needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/einvoices.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    rows: Vec<Row>,
    entities: Vec<(String, String)>,
    entity: String,
    statuses: Vec<(String, String)>,
    status: String,
    banner: Option<String>,
    can_manage: bool,
}

/// The badge colour of a status.
fn status_kind(status: TransmissionStatus) -> &'static str {
    match status {
        TransmissionStatus::Queued | TransmissionStatus::Superseded => "info",
        TransmissionStatus::Submitted => "warning",
        TransmissionStatus::Accepted => "success",
        TransmissionStatus::Rejected => "error",
    }
}

/// The page of the document a transmission sends.
fn document_link(t: &EInvoiceTransmission) -> String {
    match t.document_type.as_str() {
        crate::einvoice::INVOICE => format!("/invoices/{}", t.document_id),
        crate::einvoice::CREDIT_NOTE => format!("/credit-notes/{}", t.document_id),
        _ => String::new(),
    }
}

/// What is wrong with a transmission, in words: a rejection's codes and
/// messages, or a queued one's last error.
fn problem_of(t: &EInvoiceTransmission) -> String {
    match t.status {
        TransmissionStatus::Rejected => t
            .error_codes
            .0
            .iter()
            .zip(t.error_messages.0.iter().chain(std::iter::repeat(&String::new())))
            .map(|(code, message)| {
                if message.is_empty() {
                    code.clone()
                } else {
                    format!("{code}: {message}")
                }
            })
            .collect::<Vec<_>>()
            .join("; "),
        TransmissionStatus::Queued => t.last_error.clone().unwrap_or_default(),
        _ => String::new(),
    }
}

/// `GET /einvoices`: the transmissions per entity and state.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(filter): Query<Filter>,
) -> Result<View> {
    let t = |key: &str| texts::t(&lang, &format!("wl_invoicing.{key}"));
    let entities_all = LegalEntity::query()
        .order_by(LegalEntity::CODE)
        .get(&state.db)
        .await?;
    let asked: Option<i64> = filter.entity.as_deref().and_then(|e| e.trim().parse().ok());
    let status: Option<TransmissionStatus> = filter
        .status
        .as_deref()
        .and_then(|s| TransmissionStatus::ALL.iter().copied().find(|c| c.as_str() == s.trim()));
    let mut query = EInvoiceTransmission::query()
        .order_by_desc(EInvoiceTransmission::ID)
        .limit(200);
    if let Some(id) = asked {
        query = query.where_(EInvoiceTransmission::LEGAL_ENTITY_ID.eq(id));
    }
    if let Some(status) = status {
        query = query.where_(EInvoiceTransmission::STATUS.eq(status));
    }
    let found = query.get(&state.db).await?;
    let can_manage = user.has_permission_anywhere(permissions::EINVOICES_MANAGE);
    let mut rows = Vec::new();
    for item in &found {
        let code = entities_all
            .iter()
            .find(|e| e.id == item.legal_entity_id)
            .map(|e| e.code.clone())
            .unwrap_or_default();
        let number = match item.document_type.as_str() {
            crate::einvoice::INVOICE => Invoice::find(&state.db, item.document_id)
                .await?
                .and_then(|i| i.number),
            crate::einvoice::CREDIT_NOTE => {
                crate::credit_note::CreditNote::find(&state.db, item.document_id)
                    .await?
                    .and_then(|n| n.number)
            }
            _ => None,
        };
        rows.push(Row {
            id: item.id,
            entity: code,
            document: number.unwrap_or_else(|| item.document().to_string()),
            link: document_link(item),
            recipient: item.recipient_id.clone().unwrap_or_default(),
            attempt: item.attempt,
            status: t(&format!("einvoice.status.{}", item.status.as_str())),
            status_kind: status_kind(item.status),
            reference: item.asp_reference.clone().unwrap_or_default(),
            problem: problem_of(item),
            can_resubmit: can_manage && item.status == TransmissionStatus::Rejected,
        });
    }
    let mut entities = vec![(String::new(), t("einvoice.all_entities"))];
    entities.extend(
        entities_all
            .iter()
            .map(|e| (e.id.to_string(), format!("{} - {}", e.code, e.name_en))),
    );
    let mut statuses = vec![(String::new(), t("einvoice.all_statuses"))];
    statuses.extend(
        TransmissionStatus::ALL
            .iter()
            .map(|s| (s.as_str().to_owned(), t(&format!("einvoice.status.{}", s.as_str())))),
    );
    let queued = queued_count_all(&state.db).await?;
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        rows,
        entities,
        entity: asked.map(|e| e.to_string()).unwrap_or_default(),
        statuses,
        status: status.map(|s| s.as_str().to_owned()).unwrap_or_default(),
        banner: queue_banner(&t("einvoice.banner"), queued),
        can_manage,
    }))
}

/// The form that resubmits a rejected transmission.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct ResubmitForm {
    /// The user's confirmation that the cause is outside the issued document
    /// and has been fixed.
    #[serde(default)]
    pub confirm: bool,
}

/// `POST /einvoices/{id}/resubmit`: sends the rejected transmission again.
async fn resubmit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<ResubmitForm>,
) -> Result<Response> {
    let t = |key: &str| texts::t(&lang, &format!("wl_invoicing.{key}"));
    match einvoice_send::resubmit(&state, &user, id, form.confirm).await {
        Ok(_) => Ok((
            Toast::success(t("einvoice.resubmitted")),
            Redirect::route("wl_invoicing.einvoices", &[])?,
        )
            .into_response()),
        Err(TransitionError::Guard(message)) => Ok((
            Toast::error(message),
            Redirect::route("wl_invoicing.einvoices", &[])?,
        )
            .into_response()),
        Err(error) => Err(error.into_error()),
    }
}

/// The form that names the invoice a draft replaces.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct ReplacesForm {
    /// The number of the invoice it replaces.
    #[validate(required, max = 64)]
    pub number: Option<String>,
}

/// `POST /invoices/{id}/replaces`: names, on the draft invoice, the invoice
/// it replaces after an e-invoice correction.
async fn replaces(
    State(state): State<AppState>,
    lang: Lang,
    Found(draft): Found<Invoice>,
    Valid(form): Valid<ReplacesForm>,
) -> Result<Response> {
    let number = form.number.clone().unwrap_or_default();
    let old = Invoice::query()
        .where_(Invoice::NUMBER.eq(number.trim()))
        .first(&state.db)
        .await?;
    let toast = match old {
        None => Toast::error(texts::t(&lang, "wl_invoicing.einvoice.replaces_unknown")),
        Some(old) => match einvoice_send::set_replaces(&state.db, &draft, old.id).await {
            Ok(()) => Toast::success(texts::t(&lang, "wl_invoicing.einvoice.replaces_saved")),
            Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)) => Toast::error(message),
            Err(other) => return Err(other),
        },
    };
    Ok((toast, Redirect::route("wl_invoicing.show", &[&draft.id])?).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::db::Json;

    #[test]
    fn a_rejection_lists_its_codes_with_their_messages() {
        let rejected = EInvoiceTransmission {
            status: TransmissionStatus::Rejected,
            error_codes: Json(vec!["BR-CO-15".into(), "BR-S-08".into()]),
            error_messages: Json(vec!["Totals differ".into()]),
            ..Default::default()
        };
        assert_eq!(problem_of(&rejected), "BR-CO-15: Totals differ; BR-S-08");
        let queued = EInvoiceTransmission {
            status: TransmissionStatus::Queued,
            last_error: Some("The ASP does not answer.".into()),
            ..Default::default()
        };
        assert_eq!(problem_of(&queued), "The ASP does not answer.");
        assert_eq!(problem_of(&EInvoiceTransmission::default()), "");
    }

    #[test]
    fn a_document_opens_on_its_own_page() {
        let t = EInvoiceTransmission {
            document_type: "invoicing.invoice".into(),
            document_id: 4,
            ..Default::default()
        };
        assert_eq!(document_link(&t), "/invoices/4");
        let note = EInvoiceTransmission {
            document_type: "invoicing.credit_note".into(),
            document_id: 2,
            ..Default::default()
        };
        assert_eq!(document_link(&note), "/credit-notes/2");
    }

    #[test]
    fn every_status_has_a_colour_and_a_text() {
        for status in TransmissionStatus::ALL {
            assert!(!status_kind(*status).is_empty());
            let key = format!("wl_invoicing.einvoice.status.{}", status.as_str());
            assert_ne!(texts::english(&key), key);
        }
    }
}
