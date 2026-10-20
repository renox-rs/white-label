//! The payload of an e-invoice transmission (wl-invoicing §4 *E-invoice
//! transmission*, note on A-10): the structured data of an issued invoice or
//! credit note, built from the document with nothing typed again, and its
//! hash.
//!
//! Until the accountant answers question 4 (`⚠ open: accountant (A-10)`) the
//! payload carries the fields printed on the tax document: the issuer
//! snapshot (names, TRN, address), the recipient's name, TRN and recipient
//! identifier, the document kind, number, issue date and date of supply, the
//! currency and exchange rate, each line (description, quantity, unit price,
//! discount, VAT category, VAT amount), the totals per VAT category and the AED
//! VAT total, and for a credit note the invoice it credits. The adapter maps
//! these to the ASP's format.
//!
//! The party block is read from wl-parties **each time** the payload is built
//! (at `queue` and at `resubmit`), so a fixed recipient identifier reaches the
//! rebuilt payload while the issued document stays as it was; when the party
//! is not found the snapshot taken at `issue` is used. The recipient
//! identifier is returned beside the payload, to be stored on the
//! transmission, never on the document.
//!
//! The hash is the SHA-256 of the payload's JSON text as stored (object keys
//! in sorted order), so a recomputation over the stored payload must match
//! it ([`hash_of`]).

use renox::prelude::*;
use renox::serde_json::{Value, json};
use sha2::{Digest, Sha256};
use wl_kernel::parties::directory;
use wl_kernel::{IssuerSnapshot, Subject};

use crate::credit_note::CreditNote;
use crate::credit_note_line::CreditNoteLine;
use crate::einvoice::{CREDIT_NOTE, INVOICE};
use crate::invoice::{Invoice, PartySnapshot};
use crate::invoicer::lines_of;
use crate::line::InvoiceLine;
use crate::vat::{LineAmounts, VatTotals};

/// An issued document that can be sent.
#[derive(Debug, Clone)]
pub enum Document {
    /// An issued invoice.
    Invoice(Invoice),
    /// An issued credit note.
    CreditNote(CreditNote),
}

impl Document {
    /// The document as a [`Subject`].
    pub fn subject(&self) -> Subject {
        match self {
            Document::Invoice(invoice) => invoice.subject(),
            Document::CreditNote(note) => note.subject(),
        }
    }

    /// The subject type: invoice or credit note.
    pub fn kind(&self) -> &'static str {
        match self {
            Document::Invoice(_) => INVOICE,
            Document::CreditNote(_) => CREDIT_NOTE,
        }
    }

    /// The issuing legal entity.
    pub fn legal_entity_id(&self) -> Option<i64> {
        match self {
            Document::Invoice(invoice) => invoice.legal_entity_id,
            Document::CreditNote(note) => note.legal_entity_id,
        }
    }

    /// The party the document is addressed to.
    pub fn party(&self) -> wl_kernel::PartyId {
        match self {
            Document::Invoice(invoice) => invoice.party(),
            Document::CreditNote(note) => note.party(),
        }
    }

    /// The issue date (Asia/Dubai); `None` while a draft.
    pub fn issue_date(&self) -> Option<renox::chrono::NaiveDate> {
        match self {
            Document::Invoice(invoice) => invoice.issue_date,
            Document::CreditNote(note) => note.issue_date,
        }
    }

    /// The document `subject` names, if it is an invoice or a credit note.
    pub async fn load(db: &Db, subject: &Subject) -> Result<Option<Document>> {
        Ok(match subject.subject_type.as_str() {
            INVOICE => Invoice::find(db, subject.subject_id)
                .await?
                .map(Document::Invoice),
            CREDIT_NOTE => CreditNote::find(db, subject.subject_id)
                .await?
                .map(Document::CreditNote),
            _ => None,
        })
    }
}

/// A payload with its hash and the recipient identifier read from the party.
#[derive(Debug, Clone, PartialEq)]
pub struct Built {
    /// The structured data.
    pub payload: Value,
    /// Its SHA-256, lower-case hex.
    pub hash: String,
    /// The recipient identifier (Peppol ID or TRN), if the party has one.
    pub recipient_id: Option<String>,
}

/// The SHA-256 of `payload`'s JSON text, lower-case hex: what a recomputation
/// over a stored payload must give.
pub fn hash_of(payload: &Value) -> String {
    let text = renox::serde_json::to_string(payload).unwrap_or_default();
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn issuer_block(issuer: Option<&IssuerSnapshot>) -> Value {
    match issuer {
        Some(issuer) => json!({
            "name_en": issuer.name_en,
            "name_ar": issuer.name_ar,
            "trn": issuer.trn,
            "trade_licence_no": issuer.trade_licence_no,
            "address_en": issuer.address_en,
        }),
        None => Value::Null,
    }
}

fn recipient_block(snapshot: Option<&PartySnapshot>, identifier: Option<&str>) -> Value {
    json!({
        "name_en": snapshot.map(|s| s.legal_name_en.clone()),
        "name_ar": snapshot.and_then(|s| s.legal_name_ar.clone()),
        "trn": snapshot.and_then(|s| s.trn.clone()),
        "address": snapshot.and_then(|s| s.billing_address.clone()),
        "identifier": identifier,
    })
}

fn totals_block(amounts: &[LineAmounts]) -> Value {
    let totals = VatTotals::of(amounts.iter());
    json!({
        "net": totals.net,
        "vat": totals.vat,
        "gross": totals.gross,
        "vat_aed": totals.vat_aed,
        "per_category": totals
            .categories
            .iter()
            .map(|c| json!({
                "category": c.category.as_str(),
                "net": c.net,
                "vat": c.vat,
                "vat_aed": c.vat_aed,
            }))
            .collect::<Vec<_>>(),
    })
}

fn invoice_line(line: &InvoiceLine) -> Value {
    json!({
        "position": line.position,
        "description": line.description,
        "quantity_milli": line.quantity_milli,
        "unit_price": line.unit_price,
        "discount": line.discount,
        "net": line.net_amount,
        "vat_category": line.vat_category.as_str(),
        "vat_rate_bp": line.vat_rate_bp,
        "vat_amount": line.vat_amount,
    })
}

fn note_line(line: &CreditNoteLine) -> Value {
    json!({
        "position": line.position,
        "description": line.description,
        "net": line.net_amount,
        "vat_category": line.vat_category.as_str(),
        "vat_rate_bp": line.vat_rate_bp,
        "vat_amount": line.vat_amount,
    })
}

/// The party block of the document: the identity wl-parties has now, else the
/// snapshot taken at `issue`. Returns the block and the recipient identifier.
async fn recipient_of(
    state: &AppState,
    party: wl_kernel::PartyId,
    snapshot: Option<&PartySnapshot>,
) -> Result<(Value, Option<String>)> {
    let identity = directory(state).billing_identity(&state.db, party).await?;
    let Some(identity) = identity else {
        return Ok((recipient_block(snapshot, None), None));
    };
    let identifier = identity
        .einvoice_recipient_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned);
    let current = PartySnapshot::of(&identity);
    Ok((
        recipient_block(Some(&current), identifier.as_deref()),
        identifier,
    ))
}

/// Builds the payload of `document` from the stored document and the party
/// as wl-parties has it now.
///
/// # Errors
///
/// A database error.
pub async fn build(state: &AppState, document: &Document) -> Result<Built> {
    let (payload, recipient_id) = match document {
        Document::Invoice(invoice) => {
            let lines = lines_of(&state.db, invoice.id).await?;
            let amounts: Vec<LineAmounts> = lines.iter().map(InvoiceLine::amounts).collect();
            let (recipient, recipient_id) = recipient_of(
                state,
                invoice.party(),
                invoice.party_snapshot.as_ref().map(|s| &s.0),
            )
            .await?;
            let payload = json!({
                "document": {
                    "type": "invoice",
                    "kind": invoice.kind.as_str(),
                    "number": invoice.number,
                    "issue_date": invoice.issue_date.map(|d| d.to_string()),
                    "date_of_supply": invoice.date_of_supply.map(|d| d.to_string()),
                    "due_date": invoice.due_date.map(|d| d.to_string()),
                    "currency": invoice.currency,
                    "exchange_rate_micro": invoice.exchange_rate_micro,
                },
                "issuer": issuer_block(invoice.issuer_snapshot.as_ref().map(|s| &s.0)),
                "recipient": recipient,
                "lines": lines.iter().map(invoice_line).collect::<Vec<_>>(),
                "totals": totals_block(&amounts),
            });
            (payload, recipient_id)
        }
        Document::CreditNote(note) => {
            let lines = CreditNoteLine::query()
                .where_(CreditNoteLine::CREDIT_NOTE_ID.eq(note.id))
                .order_by(CreditNoteLine::POSITION)
                .get(&state.db)
                .await?;
            let amounts: Vec<LineAmounts> = lines.iter().map(CreditNoteLine::amounts).collect();
            let credited = Invoice::find(&state.db, note.invoice_id).await?;
            let (recipient, recipient_id) = recipient_of(
                state,
                note.party(),
                note.party_snapshot.as_ref().map(|s| &s.0),
            )
            .await?;
            let payload = json!({
                "document": {
                    "type": "credit_note",
                    "kind": "credit_note",
                    "number": note.number,
                    "issue_date": note.issue_date.map(|d| d.to_string()),
                    "date_of_supply": Value::Null,
                    "currency": note.currency,
                    "exchange_rate_micro": note.exchange_rate_micro,
                    "reason": note.reason,
                    "credits_invoice": credited.and_then(|i| i.number),
                },
                "issuer": issuer_block(note.issuer_snapshot.as_ref().map(|s| &s.0)),
                "recipient": recipient,
                "lines": lines.iter().map(note_line).collect::<Vec<_>>(),
                "totals": totals_block(&amounts),
            });
            (payload, recipient_id)
        }
    };
    let hash = hash_of(&payload);
    Ok(Built {
        payload,
        hash,
        recipient_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::invoicing::VatCategory;

    #[test]
    fn the_hash_is_the_sha256_of_the_text_in_lower_case_hex() {
        // SHA-256 of `{}`.
        assert_eq!(
            hash_of(&json!({})),
            "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
        );
        let payload = json!({"b": 1, "a": 2});
        assert_eq!(hash_of(&payload), hash_of(&payload.clone()));
        assert_ne!(hash_of(&payload), hash_of(&json!({"a": 2})));
        assert_eq!(hash_of(&payload).len(), 64);
    }

    #[test]
    fn the_totals_are_per_category_with_the_aed_vat() {
        let standard = LineAmounts::for_net(100_000, VatCategory::Standard, Some(1_000_000));
        let exempt = LineAmounts::for_net(50_000, VatCategory::Exempt, Some(1_000_000));
        let block = totals_block(&[standard, exempt]);
        assert_eq!(block["net"], 150_000);
        assert_eq!(block["vat"], 5_000);
        assert_eq!(block["gross"], 155_000);
        assert_eq!(block["vat_aed"], 5_000);
        let categories = block["per_category"].as_array().unwrap();
        assert_eq!(categories.len(), 2);
        assert_eq!(categories[0]["category"], "standard");
        assert_eq!(categories[0]["vat"], 5_000);
    }

    #[test]
    fn a_line_carries_its_vat_category_and_amount() {
        let line = InvoiceLine {
            position: 1,
            description: "Rent".into(),
            quantity_milli: 1_000,
            unit_price: 100_000,
            net_amount: 100_000,
            vat_category: VatCategory::Standard,
            vat_rate_bp: 500,
            vat_amount: 5_000,
            ..Default::default()
        };
        let value = invoice_line(&line);
        assert_eq!(value["vat_category"], "standard");
        assert_eq!(value["vat_amount"], 5_000);
        assert_eq!(value["description"], "Rent");
    }

    #[test]
    fn the_recipient_block_has_the_identifier_beside_the_identity() {
        let snapshot = PartySnapshot {
            legal_name_en: "Blue Dunes LLC".into(),
            legal_name_ar: None,
            trn: Some("100000000000003".into()),
            billing_address: None,
            billing_email: None,
            taken_at: renox::db::now(),
        };
        let block = recipient_block(Some(&snapshot), Some("0235:100000000000003"));
        assert_eq!(block["trn"], "100000000000003");
        assert_eq!(block["identifier"], "0235:100000000000003");
        assert_eq!(recipient_block(None, None)["identifier"], Value::Null);
    }
}
