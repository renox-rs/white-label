//! The e-invoice board (wl-invoicing §5.12 step 5, `FIN.INV.01`):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /einvoices` | `wl_invoicing.einvoices` | `einvoices.view` |
//! | `POST /einvoices/{id}/resubmit` | `wl_invoicing.einvoice_resubmit` | `einvoices.manage` |
//! | `POST /invoices/{id}/replaces` | `wl_invoicing.invoice_replaces` | `invoices.create` |
//!
//! The board is a `renox::grid` of the transmissions with filters by legal
//! entity and state (`?in.legal_entity_id=`, `?in.status=`; `?export=` needs
//! `einvoices.manage`), with the document, the recipient identifier, the attempt, the
//! ASP's reference, the error codes of a rejection and the last error of a
//! queued one, and a banner with the count while transmissions wait for the
//! ASP. A rejected transmission offers *Resubmit* to a holder of
//! `einvoices.manage`, who confirms that the cause is outside the document
//! and has been fixed. The third route lets the user name, on a draft invoice,
//! the invoice it replaces after an *e-invoice correction*. The template is
//! compiled in and can be replaced by the app
//! (`resources/views/wl-invoicing/einvoices.html`).

use std::collections::BTreeMap;

use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
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

/// What the board needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/einvoices.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    rows: GridPage<EInvoiceTransmission>,
    /// The rejected transmissions on the page the user may resubmit: one
    /// sheet each, after the grid.
    resubmit: Vec<i64>,
    banner: Option<String>,
}

/// The transmissions grid: search on the document, recipient and ASP
/// reference, filters by legal entity and state. The problem and the actions
/// are drawn by the template (`custom`).
pub fn grid(lang: &Lang, entities: Vec<(String, String)>, can_export: bool) -> Grid {
    let t = |key: &str| texts::t(lang, &format!("wl_invoicing.{key}"));
    let statuses: Vec<(String, String)> = TransmissionStatus::ALL
        .iter()
        .map(|s| {
            (
                s.as_str().to_owned(),
                t(&format!("einvoice.status.{}", s.as_str())),
            )
        })
        .collect();
    // The number of the invoice or credit note sent, else its reference.
    let document = format!(
        "COALESCE(CASE {{T}}.\"document_type\" \
         WHEN '{}' THEN (SELECT \"number\" FROM \"wl_invoicing_invoices\" WHERE \"id\" = {{T}}.\"document_id\") \
         WHEN '{}' THEN (SELECT \"number\" FROM \"wl_invoicing_credit_notes\" WHERE \"id\" = {{T}}.\"document_id\") END, \
         {{T}}.\"document_type\" || ':' || {{T}}.\"document_id\")",
        crate::einvoice::INVOICE,
        crate::einvoice::CREDIT_NOTE
    );
    let grid = Grid::new("wl-invoicing-einvoices")
        .title(&t("einvoice.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("document", &t("einvoice.document"))
                .computed(&document)
                .frozen()
                .mobile()
                .searchable()
                .copyable(),
        )
        .column(Column::select("legal_entity_id", &t("einvoice.entity"), entities).mobile())
        .column(Column::text("recipient_id", &t("einvoice.recipient")).searchable())
        .column(Column::number("attempt", &t("einvoice.attempt")))
        .column(
            Column::select("status", &t("einvoice.status"), statuses)
                .badges(&[
                    ("queued", "info"),
                    ("submitted", "warning"),
                    ("accepted", "success"),
                    ("rejected", "danger"),
                    ("superseded", "info"),
                ])
                .mobile(),
        )
        .column(Column::text("asp_reference", &t("einvoice.reference")).searchable())
        .column(Column::custom("problem", &t("einvoice.problem")).mobile())
        .column(Column::custom("actions", ""))
        .sort_by("-id")
        .cards_on_mobile()
        .empty_state(&t("einvoice.empty"), Some(&t("einvoice.empty_hint")));
    if can_export { grid.exports() } else { grid }
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
            .zip(
                t.error_messages
                    .0
                    .iter()
                    .chain(std::iter::repeat(&String::new())),
            )
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

/// `GET /einvoices`: the transmissions, or their export (`?export=csv|xlsx|print`,
/// with `einvoices.manage`).
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    let t = |key: &str| texts::t(&lang, &format!("wl_invoicing.{key}"));
    let entities: Vec<(String, String)> = LegalEntity::query()
        .order_by(LegalEntity::CODE)
        .get(&state.db)
        .await?
        .iter()
        .map(|e| (e.id.to_string(), format!("{} - {}", e.code, e.name_en)))
        .collect();
    let can_manage = user.has_permission_anywhere(permissions::EINVOICES_MANAGE);
    let grid = grid(&lang, entities, can_manage);
    if request.param("export").is_some() && !can_manage {
        return Err(Error::Forbidden);
    }
    if let Some(file) = grid.export(EInvoiceTransmission::query(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(EInvoiceTransmission::query(), &request).await?;
    let resubmit: Vec<i64> = page
        .items()
        .iter()
        .filter(|item| can_manage && item.status == TransmissionStatus::Rejected)
        .map(|item| item.id)
        .collect();
    let rows = page.extend(|item| {
        json!({
            "problem": problem_of(item),
            "link": document_link(item),
            "can_resubmit": resubmit.contains(&item.id),
        })
    });
    let queued = queued_count_all(&state.db).await?;
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        rows,
        resubmit: resubmit.clone(),
        banner: queue_banner(&t("einvoice.banner"), queued),
    })
    .into_response())
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
    fn every_status_has_a_text() {
        for status in TransmissionStatus::ALL {
            let key = format!("wl_invoicing.einvoice.status.{}", status.as_str());
            assert_ne!(texts::english(&key), key);
        }
    }
}
