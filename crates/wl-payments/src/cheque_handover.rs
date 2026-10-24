//! The cheque handover receipt (wl-payments §4 *Cheque* `return`, §5.3,
//! `DOC.GEN.01`; BL-PAY-30).
//!
//! Each return call (`return_by_system` for a batch, or a user's `return` of
//! one cheque) records one [`ChequeHandover`]: the cheques handed back, the
//! party who receives them, the reason and the day, under the receipt
//! reference the cheques' history rows carry. After the commit the receipt
//! PDF is generated through the kernel's document generator
//! ([`wl_kernel::generation`], wl-documents, BL-DOC-11) from this module's
//! data provider, template [`TEMPLATE`], numbered from the kernel series
//! [`SERIES`] (`CHR-2026-0042`, per entity, yearly), bilingual, and stored as
//! an accepted generated document of the handover; its number and document
//! are kept on the handover, which each cheque's page shows. The party is
//! e-mailed the receipt with the PDF attached ([`HANDED_OVER`], BL-NTF-10);
//! without an e-mail address the message gives up as any message does.
//!
//! A receipt is generated once: an issued template never regenerates, and a
//! cheque that changes later does not touch it. When documents are off or the
//! client has no template [`TEMPLATE`] registered with wl-documents, nothing is
//! generated and the return goes on as before.
//!
//! Not built here: the receiver of a handover to a new manager (agreement
//! `end`) is the cheques' party until the kernel's `ReturnCheques` names a
//! receiver ([`ChequeHandover::receiver_party_id`] is ready for it); the
//! *Signed by the receiver* upload slot (a document of kind
//! [`SIGNED_KIND`] on the handover, uploaded in wl-documents).

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Json};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::Transitioned;
use serde::{Deserialize, Serialize};
use wl_kernel::document_store::WithAttachments;
use wl_kernel::documents::{DocumentId, today};
use wl_kernel::generation::{
    self, DataProvider, GenerateRequest, GenerationData, GenerationFuture, LanguageMode,
};
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::numbering::{Reset, Series};
use wl_kernel::{PartyId, Subject};

use crate::cheque::Cheque;
use crate::cheque_return::{RETURN, RETURN_BY_SYSTEM, receipt_reference};
use crate::followup::money_text;
use crate::texts::english;

/// The subject type of a handover: `payments.cheque_handover:7`.
pub const SUBJECT_TYPE: &str = "payments.cheque_handover";

/// The template key of the receipt (the client registers its text with
/// wl-documents).
pub const TEMPLATE: &str = "payments.cheque_handover_receipt";

/// The document kind of the receipt.
pub const KIND: &str = "cheque_handover_receipt";

/// The document kind of the copy signed by the receiver.
pub const SIGNED_KIND: &str = "cheque_handover_signed";

/// The receipt's numbers: `CHR-2026-0042`, per legal entity, from 1 each year.
pub const SERIES: Series = Series {
    key: "cheque_handover_receipt",
    pattern: "{prefix}-{yyyy}-{0000}",
    prefix: "CHR",
    reset: Reset::Yearly,
    gapless: true,
    per_entity: true,
};

/// The notification that sends the receipt to the party.
pub const HANDED_OVER: &str = "payments.cheques_handed_over";

/// One handover of cheques against one receipt.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_payments_cheque_handovers", unique(receipt))]
pub struct ChequeHandover {
    /// The primary key.
    pub id: i64,
    /// The receipt reference in the cheques' history rows (`HOR-20261104-12`).
    pub receipt: String,
    /// The legal entity that held the cheques.
    pub legal_entity_id: i64,
    /// The party the cheques came from.
    pub party_id: i64,
    /// Who receives them: the party, or the new holder of the agreement.
    pub receiver_party_id: i64,
    /// The cheques handed back.
    pub cheques: Json<Vec<i64>>,
    /// What they add up to, in fils.
    pub total: i64,
    /// The currency.
    pub currency: String,
    /// Why they are handed back.
    pub reason: String,
    /// The day (Asia/Dubai).
    pub returned_on: NaiveDate,
    /// The receipt's number once generated (`CHR-2026-0042`).
    pub number: Option<String>,
    /// The generated receipt document.
    pub document_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ChequeHandover {
    /// The handover as a subject.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The handover of receipt `receipt`, if any.
    pub async fn by_receipt<'c>(
        db: impl renox::db::Executor<'c>,
        receipt: &str,
    ) -> Result<Option<ChequeHandover>> {
        ChequeHandover::query()
            .where_(ChequeHandover::RECEIPT.eq(receipt))
            .first(db)
            .await
    }

    /// The latest handover that lists cheque `cheque`, if any.
    pub async fn of_cheque(db: &Db, cheque: i64) -> Result<Option<ChequeHandover>> {
        let rows = ChequeHandover::query()
            .order_by_desc(ChequeHandover::ID)
            .get(db)
            .await?;
        Ok(rows.into_iter().find(|h| h.cheques.0.contains(&cheque)))
    }

    /// The receipt's title: `Cheque handover receipt CHR-2026-0042`.
    pub fn title(&self) -> String {
        match &self.number {
            Some(number) => format!("Cheque handover receipt {number}"),
            None => format!("Cheque handover receipt {}", self.receipt),
        }
    }
}

/// Records the handover of `cheques` (already returned in `tx`) under
/// `receipt`, received by `receiver` (the cheques' party when `None`).
///
/// # Errors
///
/// The database's errors.
pub async fn record_in(
    tx: &mut renox::db::Transaction,
    receipt: &str,
    cheques: &[Cheque],
    reason: &str,
    receiver: Option<PartyId>,
) -> Result<Option<ChequeHandover>> {
    let Some(first) = cheques.first() else {
        return Ok(None);
    };
    if let Some(found) = ChequeHandover::by_receipt(&mut *tx, receipt).await? {
        return Ok(Some(found));
    }
    let mut row = ChequeHandover {
        receipt: receipt.to_owned(),
        legal_entity_id: first.legal_entity_id,
        party_id: first.party_id,
        receiver_party_id: receiver.map_or(first.party_id, |p| p.0),
        cheques: Json(cheques.iter().map(|c| c.id).collect()),
        total: cheques.iter().map(|c| c.amount).sum(),
        currency: first.currency.clone(),
        reason: reason.trim().to_owned(),
        returned_on: today(),
        ..ChequeHandover::default()
    };
    row.insert(&mut *tx).await?;
    Ok(Some(row))
}

/// The display name of `party`.
async fn name_of(state: &AppState, party: PartyId, locale: &str) -> Result<String> {
    Ok(wl_kernel::parties::directory(state)
        .display_name(&state.db, party, locale)
        .await?
        .unwrap_or_else(|| format!("#{}", party.0)))
}

/// The data the receipt template reads: `data.receipt`, `data.date`,
/// `data.party`, `data.party_ar`, `data.receiver`, `data.receiver_ar`,
/// `data.reason`, `data.cheques` (each `number`, `bank`, `date`, `amount`),
/// `data.count` and `data.total`.
pub async fn data_of(state: &AppState, handover: &ChequeHandover) -> Result<Value> {
    let mut cheques = Vec::new();
    for id in &handover.cheques.0 {
        if let Some(cheque) = Cheque::find(&state.db, *id).await? {
            cheques.push(json!({
                "number": cheque.number,
                "bank": cheque.bank,
                "date": cheque.cheque_date.format("%d %b %Y").to_string(),
                "amount": money_text(&cheque.currency, cheque.amount),
            }));
        }
    }
    let party = PartyId(handover.party_id);
    let receiver = PartyId(handover.receiver_party_id);
    Ok(json!({
        "receipt": handover.receipt,
        "date": handover.returned_on.format("%d %b %Y").to_string(),
        "party": name_of(state, party, "en").await?,
        "party_ar": name_of(state, party, "ar").await?,
        "receiver": name_of(state, receiver, "en").await?,
        "receiver_ar": name_of(state, receiver, "ar").await?,
        "reason": handover.reason,
        "count": cheques.len(),
        "cheques": cheques,
        "total": money_text(&handover.currency, handover.total),
    }))
}

/// The data provider of the handovers.
pub struct Handovers;

impl DataProvider for Handovers {
    fn data<'a>(
        &'a self,
        state: &'a AppState,
        _template: &'a str,
        subject: &'a Subject,
    ) -> GenerationFuture<'a, Option<GenerationData>> {
        Box::pin(async move {
            let Some(handover) = ChequeHandover::find(&state.db, subject.subject_id).await? else {
                return Ok(None);
            };
            let values = data_of(state, &handover).await?;
            Ok(Some(
                GenerationData::new(values)
                    .entity(handover.legal_entity_id)
                    .title("Cheque handover receipt"),
            ))
        })
    }
}

/// The notification type [`HANDED_OVER`].
pub fn handed_over_type() -> NotificationTypeDef {
    NotificationTypeDef::new(HANDED_OVER, "payments")
        .group("payments")
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.cheques_handed_over.subject")),
            english("wl_payments.cheques_handed_over.body"),
        )
}

/// Generates the receipt of `handover` once and sends it to the receiver.
/// Nothing happens when the receipt was generated before, or when documents
/// or the template are not there.
///
/// # Errors
///
/// The database's and the notifier's errors.
pub async fn issue(state: &AppState, handover: &ChequeHandover) -> Result<Option<ChequeHandover>> {
    let mut handover = ChequeHandover::find_or_404(&state.db, handover.id).await?;
    if handover.document_id.is_some() {
        return Ok(Some(handover));
    }
    let request =
        GenerateRequest::new(TEMPLATE, handover.subject()).language(LanguageMode::Bilingual);
    let generated = match generation::generator(state).generate(state, request).await? {
        Ok(generated) => generated,
        // Documents off, or no template of the client: the return stands
        // without a receipt document.
        Err(_refusal) => return Ok(None),
    };
    handover.number = generated.number.clone();
    handover.document_id = Some(generated.document.0);
    handover.save(&state.db).await?;
    let title = handover.title();
    let request = NotifyRequest::new(HANDED_OVER, handover.subject())
        .data(json!({
            "number": handover.number.clone().unwrap_or_else(|| handover.receipt.clone()),
            "count": handover.cheques.0.len(),
            "total": money_text(&handover.currency, handover.total),
            "date": handover.returned_on.format("%d %b %Y").to_string(),
        }))
        .to_party(PartyId(handover.receiver_party_id))
        .attach_document(DocumentId(generated.document.0), &format!("{title}.pdf"));
    notify::notifier(state).notify(state, request).await?;
    Ok(Some(handover))
}

/// After a return commits: the handover of its receipt (a user's `return`
/// of one cheque records its own) gets its receipt generated and sent.
async fn after_return(state: &AppState, event: &Transitioned<Cheque>) -> Result {
    let cheque = Cheque::find_or_404(&state.db, event.key).await?;
    let receipt = event
        .meta
        .get("receipt")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let handover = match receipt {
        Some(receipt) => ChequeHandover::by_receipt(&state.db, &receipt).await?,
        None => {
            // A user's `return`: one cheque, one receipt.
            let receipt = receipt_reference(today(), cheque.id);
            let mut tx = state.db.begin().await?;
            let reason = event.reason.clone().unwrap_or_default();
            let row = record_in(&mut tx, &receipt, std::slice::from_ref(&cheque), &reason, None)
                .await?;
            tx.commit().await?;
            row
        }
    };
    if let Some(handover) = handover {
        issue(state, &handover).await?;
    }
    Ok(())
}

/// Registers the data provider, the notification type, the subject's label
/// and the listener.
pub(crate) fn register(app: &mut Registry) {
    generation::register_data_provider(SUBJECT_TYPE, Handovers);
    notify::register_type(handed_over_type());
    wl_kernel::documents::register_subject_label(SUBJECT_TYPE, |db, id| {
        Box::pin(async move { Ok(ChequeHandover::find(db, id).await?.map(|h| h.title())) })
    });
    app.listen(|event: Transitioned<Cheque>, state: AppState| async move {
        let name = event.transition.as_str();
        if name != RETURN && name != RETURN_BY_SYSTEM {
            return Ok(());
        }
        after_return(&state, &event).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_series_numbers_per_entity_each_year() {
        assert_eq!(SERIES.prefix, "CHR");
        assert!(SERIES.per_entity && SERIES.gapless);
        assert_eq!(SERIES.reset, Reset::Yearly);
    }

    #[test]
    fn the_title_uses_the_number_once_there_is_one() {
        let mut handover = ChequeHandover {
            receipt: "HOR-20261104-12".into(),
            ..ChequeHandover::default()
        };
        assert_eq!(handover.title(), "Cheque handover receipt HOR-20261104-12");
        handover.number = Some("CHR-2026-0042".into());
        assert_eq!(handover.title(), "Cheque handover receipt CHR-2026-0042");
    }
}
