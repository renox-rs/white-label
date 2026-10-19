//! What follows a lead transition, a void, an invoice and a party merge
//! (issue #139), all after the commit:
//!
//! - `win`: `LeadWon`, and the party's acquisition takes the lead's source and
//!   campaign when it has none ([`crate::acquisition::attribute_from_lead`]).
//! - `lose`: `LeadLost`.
//! - `revert_win`: `LeadWinReverted`; an acquisition set from the lead with
//!   no other first sale is cleared with `CustomerAcquisitionChanged`; the
//!   owner is told.
//! - `RecordVoided` of a record that is a reported sale: the sale is marked
//!   gone, and a won lead converted by that record runs `revert_win`. A
//!   conversion record cancelled for a business reason is not voided, so the
//!   lead stays won.
//! - `InvoiceIssued`: the invoice is reported as a sale of its party;
//!   `CreditNoteIssued`: the net of the credit note comes off it.
//! - `PartiesMerged`: [`crate::acquisition::merge_parties`].

use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::Transitioned;
use wl_kernel::invoicing::{CreditNoteIssued, InvoiceIssued};
use wl_kernel::parties::PartiesMerged;
use wl_kernel::sales::{LeadLost, LeadWinReverted, LeadWon};
use wl_kernel::voids::RecordVoided;
use wl_kernel::{FirstSale, Subject};

use crate::acquisition;
use crate::lead::{Lead, LeadStatus};
use crate::lead_machine::{LOSE, REVERT_WIN, WIN};
use crate::lead_source::LeadSource;

/// The code of the lead's source, empty if it is gone.
async fn source_code(db: &Db, lead: &Lead) -> Result<String> {
    Ok(LeadSource::find(db, lead.source_id)
        .await?
        .map(|s| s.code)
        .unwrap_or_default())
}

/// Runs `revert_win` on every won lead converted by `voided`, in its own
/// transaction each.
pub async fn revert_wins(state: &AppState, voided: &Subject) -> Result {
    let leads = Lead::query()
        .where_(Lead::CONVERTED_TYPE.eq(voided.subject_type.clone()))
        .where_(Lead::CONVERTED_ID.eq(voided.subject_id))
        .where_(Lead::STATUS.eq(LeadStatus::Won))
        .order_by(Lead::ID)
        .get(&state.db)
        .await?;
    for lead in leads {
        lead.transition(REVERT_WIN)
            .by_system()
            .meta(json!({ "voided": voided.to_string() }))
            .run(state)
            .await
            .map_err(|e| e.into_error())?;
    }
    Ok(())
}

/// Registers the listeners.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: Transitioned<Lead>, state: AppState| async move {
        let name = event.transition.as_str();
        if ![WIN, LOSE, REVERT_WIN].contains(&name) {
            return Ok(());
        }
        let Some(lead) = Lead::find(&state.db, event.key).await? else {
            return Ok(());
        };
        let source = source_code(&state.db, &lead).await?;
        match name {
            WIN => {
                let Some(converted) = lead.converted() else {
                    return Ok(());
                };
                acquisition::attribute_from_lead(&state, &lead).await?;
                state
                    .emit(LeadWon {
                        lead: lead.lead_id(),
                        party: lead.party(),
                        source,
                        owner: lead.owner_id,
                        converted,
                    })
                    .await
            }
            LOSE => {
                state
                    .emit(LeadLost {
                        lead: lead.lead_id(),
                        party: lead.party(),
                        source,
                        owner: lead.owner_id,
                        reason: lead.lost_reason.clone().unwrap_or_default(),
                    })
                    .await
            }
            _ => {
                let voided = event
                    .meta
                    .get("voided")
                    .and_then(Value::as_str)
                    .and_then(|text| text.parse::<Subject>().ok());
                let Some(voided) = voided else {
                    return Ok(());
                };
                acquisition::clear_for_reverted_lead(&state, lead.id, &voided).await?;
                state
                    .emit(LeadWinReverted {
                        lead: lead.lead_id(),
                        party: lead.party(),
                        voided: voided.clone(),
                    })
                    .await?;
                crate::notices::lead_win_reverted(&state, &lead, &voided).await
            }
        }
    });
    app.listen(|event: RecordVoided, state: AppState| async move {
        acquisition::sale_gone(&state, &event.subject).await?;
        revert_wins(&state, &event.subject).await
    });
    app.listen(|event: InvoiceIssued, state: AppState| async move {
        let sale = FirstSale::new(
            event.party,
            event.invoice.subject(),
            event.net_total,
            event.issue_date,
        );
        acquisition::record_sale(&state, sale).await.map(|_| ())
    });
    app.listen(|event: CreditNoteIssued, state: AppState| async move {
        acquisition::credit_sale(&state, &event.invoice.subject(), event.net_amount).await
    });
    app.listen(|event: PartiesMerged, state: AppState| async move {
        acquisition::merge_parties(&state, event.survivor, event.merged).await
    });
}
