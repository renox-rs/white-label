//! The names of the records the Manpower approvals are about, for the
//! approvals inbox (BL-MPW-29): a client contract by its reference and client
//! (`CC-DEMO-001, Gulf Tower Facilities LLC`), a request by its positions and
//! client, a deployment by its worker. Never an id.

use renox::prelude::*;
use wl_kernel::Subject;

use crate::contract::ClientContract;
use crate::deployment::{self, Deployment};
use crate::deployments::worker_name;
use crate::request::{self, ManpowerRequest};
use crate::screens::{party_text, role_text};

/// The name of `subject` when it is a record of this app; none for another
/// record.
pub async fn subject_name(state: &AppState, subject: &Subject, locale: &str) -> Option<String> {
    let lang = state.lang(locale);
    match subject.subject_type.as_str() {
        crate::contract::SUBJECT_TYPE => {
            let contract = ClientContract::find(&state.db, subject.subject_id)
                .await
                .ok()??;
            let client = party_text(state, &lang, Some(contract.client_party_id), "").await;
            Some(format!("{}, {client}", contract.reference))
        }
        request::SUBJECT_TYPE => {
            let request = ManpowerRequest::find(&state.db, subject.subject_id)
                .await
                .ok()??;
            let client = party_text(state, &lang, Some(request.client_party_id), "").await;
            Some(format!(
                "{} x {}, {client}",
                request.headcount,
                role_text(&request.role)
            ))
        }
        deployment::SUBJECT_TYPE => {
            let deployment = Deployment::find(&state.db, subject.subject_id)
                .await
                .ok()??;
            Some(format!(
                "{}, {}",
                worker_name(state, locale, deployment.employee()).await,
                role_text(&deployment.role)
            ))
        }
        _ => None,
    }
}

/// Why a request of this app was made, when its payload does not say: a
/// release of a held contract is asked because of the reasons that hold it
/// (`Invoice overdue (INV-DEMO)`, `Bounced cheque (CHQ-9)`).
pub async fn request_reason(
    state: &AppState,
    request: &wl_kernel::approvals::ApprovalRequest,
    locale: &str,
) -> Option<String> {
    if request.action != crate::release::ACTION {
        return None;
    }
    let lang = state.lang(locale);
    let holds = crate::hold::ContractHold::of_contract(&state.db, request.subject_id)
        .await
        .ok()?;
    let lines: Vec<String> = holds
        .iter()
        .filter(|hold| hold.is_open())
        .map(|hold| {
            let word = crate::texts::word(&lang, "hold_reason", hold.reason.as_str());
            match hold.reason_text.as_deref().or(hold.label.as_deref()) {
                Some(detail) if !detail.trim().is_empty() => format!("{word} ({detail})"),
                _ => word,
            }
        })
        .collect();
    (!lines.is_empty()).then(|| lines.join(", "))
}
