//! The Payment (wl-payments §3): money received from or paid to a party,
//! with its receipt or voucher number. Its status moves only through the
//! machine in [`crate::machine`]. Allocations are separate rows (they come
//! with part 4 of the module).

use renox::chrono::NaiveDate;
use renox::db::Json;
use renox::prelude::*;
use renox::serde_json::{self, Value};
use serde::{Deserialize, Serialize};
use wl_kernel::payments::{
    Direction, PaymentId, PaymentMethod, PaymentSource, PaymentStatus, PaymentSummary,
    ReversalReason,
};
use wl_kernel::{PartyId, Subject};

/// A payment. Money is `i64` fils of [`Payment::currency`].
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_payments_payments",
    state_machine,
    unique(number),
    index(status),
    index(party_id),
    index(prepaid_type, prepaid_id)
)]
pub struct Payment {
    /// The primary key.
    pub id: i64,
    /// The receipt or voucher number, set by `record`; `None` while
    /// `pending`.
    pub number: Option<String>,
    /// Money in (receipt) or out (voucher).
    pub direction: Direction,
    /// The legal entity whose books it belongs to.
    pub legal_entity_id: i64,
    /// The bank account label, free text for now.
    pub bank_account: Option<String>,
    /// Who paid, or is paid.
    pub party_id: i64,
    /// How the money moved.
    pub method: PaymentMethod,
    /// The method's details: cheque number, bank and date; card slip;
    /// transfer reference.
    pub method_details: Json<Value>,
    /// The amount, in fils; always > 0.
    pub amount: i64,
    /// The ISO currency code, e.g. `AED`.
    pub currency: String,
    /// The day the money moved.
    pub payment_date: NaiveDate,
    /// The payer's or bank's reference.
    pub reference: Option<String>,
    /// Where it came from.
    pub source: PaymentSource,
    /// The bank statement line it was matched from.
    pub statement_line_id: Option<i64>,
    /// The gateway provider of an online payment.
    pub gateway: Option<String>,
    /// The gateway's transaction id.
    pub gateway_transaction: Option<String>,
    /// The user who collected the money.
    pub collected_by: Option<i64>,
    /// The type of the record paid in advance for.
    pub prepaid_type: Option<String>,
    /// The id of the record paid in advance for.
    pub prepaid_id: Option<i64>,
    /// What the money is for: `none`, or `owner_drawings` (an owner drawing
    /// out, a repayment by the owner in; `FIN.ACL.21`).
    pub purpose: String,
    /// The code of the drawings category of an owner drawings payment.
    pub drawing_category: Option<String>,
    /// The drawing request it was recorded from (`FIN.ACL.21`).
    pub drawing_request_id: Option<i64>,
    /// Whether the sole owner approved the drawing alone (`FIN.ACL.23`).
    pub self_approved: bool,
    /// The user who recorded it; `None` for the system.
    pub recorded_by: Option<i64>,
    /// The gateway's dispute reference (chargeback flow).
    pub dispute_reference: Option<String>,
    /// The dispute's status, e.g. `open`.
    pub dispute_status: Option<String>,
    /// When the dispute's evidence is due.
    pub dispute_evidence_due: Option<NaiveDate>,
    /// Why it was reversed.
    pub reversal_reason: Option<ReversalReason>,
    /// The note given with the reversal.
    pub reversal_note: Option<String>,
    /// The user who reversed it; `None` for the system or while not
    /// reversed.
    pub reversed_by: Option<i64>,
    /// The day it was reversed (Asia/Dubai).
    pub reversed_on: Option<NaiveDate>,
    /// The lifecycle state; changed only by a transition.
    pub status: PaymentStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Payment {
    /// The payment's id in the kernel contract.
    pub fn payment_id(&self) -> PaymentId {
        PaymentId(self.id)
    }

    /// The payment as a [`Subject`]: `payments.payment:42`.
    pub fn subject(&self) -> Subject {
        self.payment_id().subject()
    }

    /// Who paid, or is paid.
    pub fn party(&self) -> PartyId {
        PartyId(self.party_id)
    }

    /// The record paid in advance for, if any.
    pub fn prepaid(&self) -> Option<Subject> {
        match (&self.prepaid_type, self.prepaid_id) {
            (Some(subject_type), Some(subject_id)) => Some(Subject {
                subject_type: subject_type.clone(),
                subject_id,
            }),
            _ => None,
        }
    }

    /// The text of the method detail `key`, trimmed, when present and not
    /// empty.
    pub fn detail(&self, key: &str) -> Option<String> {
        let text = match self.method_details.0.get(key)? {
            Value::String(text) => text.trim().to_owned(),
            Value::Number(number) => number.to_string(),
            _ => return None,
        };
        (!text.is_empty()).then_some(text)
    }

    /// The reference: the `reference` column, else the detail of that name.
    pub fn effective_reference(&self) -> Option<String> {
        self.reference
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(str::to_owned)
            .or_else(|| self.detail("reference"))
    }

    /// What other modules see of it. `allocated` is the sum of its active
    /// allocations; the unallocated amount is the rest, and 0 unless the
    /// payment is `received`.
    pub fn summary(&self, allocated: i64) -> PaymentSummary {
        let unallocated = if self.status.is_received() {
            (self.amount - allocated).max(0)
        } else {
            0
        };
        PaymentSummary {
            id: self.payment_id(),
            number: self.number.clone(),
            party: self.party(),
            legal_entity_id: self.legal_entity_id,
            direction: self.direction,
            method: self.method,
            amount: self.amount,
            allocated,
            unallocated,
            currency: self.currency.clone(),
            status: self.status,
            date: self.payment_date,
        }
    }
}

/// The method details as stored: an object, `{}` for anything else.
pub(crate) fn details_json(details: Value) -> Json<Value> {
    Json(if details.is_object() {
        details
    } else {
        serde_json::json!({})
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn payment() -> Payment {
        Payment {
            id: 5,
            party_id: 7,
            legal_entity_id: 2,
            amount: 300_000,
            currency: "AED".into(),
            status: PaymentStatus::Received,
            method_details: details_json(json!({"number": 1042, "bank": " ENBD ", "blank": " "})),
            ..Default::default()
        }
    }

    #[test]
    fn details_are_read_as_trimmed_text() {
        let p = payment();
        assert_eq!(p.detail("number").as_deref(), Some("1042"));
        assert_eq!(p.detail("bank").as_deref(), Some("ENBD"));
        assert_eq!(p.detail("blank"), None);
        assert_eq!(p.detail("missing"), None);
    }

    #[test]
    fn the_reference_falls_back_to_the_details() {
        let mut p = payment();
        assert_eq!(p.effective_reference(), None);
        p.method_details = details_json(json!({"reference": "TRX-9"}));
        assert_eq!(p.effective_reference().as_deref(), Some("TRX-9"));
        p.reference = Some(" REF-1 ".into());
        assert_eq!(p.effective_reference().as_deref(), Some("REF-1"));
        p.reference = Some("  ".into());
        assert_eq!(p.effective_reference().as_deref(), Some("TRX-9"));
    }

    #[test]
    fn details_that_are_not_an_object_become_an_empty_one() {
        assert_eq!(details_json(json!("x")).0, json!({}));
        assert_eq!(details_json(json!({"a": 1})).0, json!({"a": 1}));
    }

    #[test]
    fn the_summary_counts_the_unallocated_amount_only_while_received() {
        let mut p = payment();
        let summary = p.summary(100_000);
        assert_eq!(summary.id, PaymentId(5));
        assert_eq!((summary.allocated, summary.unallocated), (100_000, 200_000));
        assert_eq!(summary.party, PartyId(7));
        assert_eq!(p.subject().to_string(), "payments.payment:5");
        p.status = PaymentStatus::Reversed;
        assert_eq!(p.summary(100_000).unallocated, 0);
        assert_eq!(p.prepaid(), None);
        p.prepaid_type = Some("property.deposit".into());
        p.prepaid_id = Some(3);
        assert_eq!(p.prepaid(), Some(Subject::new("property.deposit", 3)));
    }
}
