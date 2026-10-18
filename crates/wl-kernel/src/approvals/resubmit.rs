//! Resubmission links (#26, `BL-KRN-02`): a new request on the same action
//! and subject points to the request before it, when that one was returned
//! (whatever the payload) or rejected (and the payload is unchanged). The
//! approver's card follows the links to list the earlier decisions.

use std::collections::HashSet;

use renox::db::Transaction;
use renox::prelude::*;
use renox::serde_json::{Map, Number, Value};

use super::request::{ApprovalDecision, ApprovalRequest, ApprovalStatus, DecisionKind};
use crate::subject::Subject;

/// One earlier return or rejection in a request's chain.
#[derive(Debug, Clone, PartialEq)]
pub struct EarlierDecision {
    /// `Returned` or `Rejected`.
    pub status: ApprovalStatus,
    /// When the ending decision was taken (an instant; show it in
    /// Asia/Dubai).
    pub at: Option<DateTime>,
    /// The approver who returned or rejected it.
    pub approver: i64,
    /// The approver's comment.
    pub comment: String,
}

/// Whether two payload snapshots are the same request: equal after the
/// top-level `reason` key is removed. Integers (fils) compare exactly; any
/// other number is rounded to 2 decimals (AED) first.
pub fn same_payload(a: &Value, b: &Value) -> bool {
    strip(a, true) == strip(b, true)
}

fn strip(value: &Value, top: bool) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| !(top && key.as_str() == "reason"))
                .map(|(key, value)| (key.clone(), strip(value, false)))
                .collect::<Map<_, _>>(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(|v| strip(v, false)).collect()),
        Value::Number(number) => Value::Number(round(number)),
        other => other.clone(),
    }
}

/// Integers stay as they are; other numbers are rounded to 2 decimals.
fn round(number: &Number) -> Number {
    if number.is_i64() || number.is_u64() {
        return number.clone();
    }
    number
        .as_f64()
        .and_then(|f| Number::from_f64((f * 100.0).round() / 100.0))
        .unwrap_or_else(|| number.clone())
}

/// Whether a finished request is the one a new submit resubmits: returned,
/// or rejected with the same payload as the new submit.
pub(super) fn is_resubmitted(previous: &ApprovalRequest, payload: &Value) -> bool {
    match previous.status {
        ApprovalStatus::Returned => true,
        ApprovalStatus::Rejected => same_payload(&previous.payload.0, payload),
        _ => false,
    }
}

/// The request a new submit of `action` on `subject` with `payload`
/// resubmits, if any: the most recent finished request of the same action
/// and subject, when [`is_resubmitted`]. Each request links only to the one
/// directly before it.
pub(super) async fn predecessor(
    tx: &mut Transaction,
    subject: &Subject,
    action: &str,
    payload: &Value,
) -> Result<Option<ApprovalRequest>> {
    let last = ApprovalRequest::query()
        .where_(ApprovalRequest::SUBJECT_TYPE.eq(subject.subject_type.as_str()))
        .where_(ApprovalRequest::SUBJECT_ID.eq(subject.subject_id))
        .where_(ApprovalRequest::ACTION.eq(action))
        .where_op("status", "!=", ApprovalStatus::Pending)
        .order_by_desc(ApprovalRequest::ID)
        .first(&mut *tx)
        .await?;
    Ok(last.filter(|previous| is_resubmitted(previous, payload)))
}

/// The returns and rejections before `request`, newest first, found by
/// following `resubmission_of`. It stops at a request that is neither
/// returned nor rejected and never visits a request twice.
pub async fn earlier_decisions(db: &Db, request: &ApprovalRequest) -> Result<Vec<EarlierDecision>> {
    let mut found = Vec::new();
    let mut seen = HashSet::from([request.id]);
    let mut next = request.resubmission_of;
    while let Some(id) = next {
        if !seen.insert(id) {
            break;
        }
        let Some(previous) = ApprovalRequest::find(db, id).await? else {
            break;
        };
        let Some(kind) = ending_kind(previous.status) else {
            break;
        };
        let decision = ApprovalDecision::query()
            .where_(ApprovalDecision::REQUEST_ID.eq(previous.id))
            .where_(ApprovalDecision::DECISION.eq(kind))
            .order_by_desc(ApprovalDecision::ID)
            .first(db)
            .await?;
        found.push(match decision {
            Some(decision) => EarlierDecision {
                status: previous.status,
                at: decision.created_at.or(previous.decided_at),
                approver: decision.approver_id,
                comment: decision.comment.unwrap_or_default(),
            },
            None => EarlierDecision {
                status: previous.status,
                at: previous.decided_at,
                approver: 0,
                comment: String::new(),
            },
        });
        next = previous.resubmission_of;
    }
    Ok(found)
}

/// The decision that ends a request in `status`, when it is a return or a
/// rejection.
fn ending_kind(status: ApprovalStatus) -> Option<DecisionKind> {
    match status {
        ApprovalStatus::Returned => Some(DecisionKind::Return),
        ApprovalStatus::Rejected => Some(DecisionKind::Reject),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use renox::db::Json;
    use renox::serde_json::json;

    use super::*;

    fn finished(status: ApprovalStatus, payload: Value) -> ApprovalRequest {
        ApprovalRequest {
            id: 5,
            status,
            payload: Json(payload),
            ..Default::default()
        }
    }

    #[test]
    fn the_reason_is_not_part_of_the_payload() {
        let a = json!({ "amount": 120_000, "reason": "damaged" });
        let b = json!({ "amount": 120_000, "reason": "a different reason" });
        assert!(same_payload(&a, &b));
        assert!(same_payload(&a, &json!({ "amount": 120_000 })));
    }

    #[test]
    fn a_different_amount_is_a_different_payload() {
        let a = json!({ "amount": 120_000, "reason": "x" });
        assert!(!same_payload(
            &a,
            &json!({ "amount": 125_000, "reason": "x" })
        ));
        assert!(!same_payload(&a, &json!({ "amount": 120_000, "other": 1 })));
    }

    #[test]
    fn integers_compare_exactly_and_other_numbers_to_two_decimals() {
        assert!(!same_payload(
            &json!({ "fils": 100 }),
            &json!({ "fils": 101 })
        ));
        assert!(same_payload(
            &json!({ "aed": 1200.004 }),
            &json!({ "aed": 1200.0 })
        ));
        assert!(!same_payload(
            &json!({ "aed": 1200.0 }),
            &json!({ "aed": 1200.01 })
        ));
    }

    #[test]
    fn only_the_top_level_reason_is_ignored() {
        let a = json!({ "line": { "reason": "a" } });
        let b = json!({ "line": { "reason": "b" } });
        assert!(!same_payload(&a, &b));
    }

    #[test]
    fn a_returned_request_is_resubmitted_whatever_the_payload() {
        let previous = finished(ApprovalStatus::Returned, json!({ "amount": 1 }));
        assert!(is_resubmitted(&previous, &json!({ "amount": 2 })));
    }

    #[test]
    fn a_rejected_request_is_resubmitted_only_with_an_equal_payload() {
        let previous = finished(
            ApprovalStatus::Rejected,
            json!({ "amount": 120_000, "reason": "a" }),
        );
        assert!(is_resubmitted(
            &previous,
            &json!({ "amount": 120_000, "reason": "b" })
        ));
        assert!(!is_resubmitted(
            &previous,
            &json!({ "amount": 125_000, "reason": "a" })
        ));
    }

    #[test]
    fn other_ends_are_never_linked() {
        for status in [
            ApprovalStatus::Approved,
            ApprovalStatus::Withdrawn,
            ApprovalStatus::Expired,
            ApprovalStatus::Failed,
            ApprovalStatus::Pending,
        ] {
            assert!(!is_resubmitted(&finished(status, json!({})), &json!({})));
        }
    }

    #[test]
    fn only_returns_and_rejections_have_an_ending_decision() {
        assert_eq!(
            ending_kind(ApprovalStatus::Returned),
            Some(DecisionKind::Return)
        );
        assert_eq!(
            ending_kind(ApprovalStatus::Rejected),
            Some(DecisionKind::Reject)
        );
        assert_eq!(ending_kind(ApprovalStatus::Approved), None);
    }
}
