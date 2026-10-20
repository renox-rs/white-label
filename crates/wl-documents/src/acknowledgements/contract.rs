//! The kernel contract [`SopAcknowledgements`](wl_kernel::documents::SopAcknowledgements)
//! that wl-hr's `sop_acknowledgement` onboarding step asks (wl-documents §6,
//! issue #53): which SOPs an employee still has to acknowledge, and recording
//! them in person.

use renox::prelude::*;
use wl_kernel::documents::{SopAcknowledgements, SopFuture, SopId, SopStaff};

use super::model::{AckStatus, SopAcknowledgement};
use super::service::{Acknowledgements, entity_fits, lists};
use crate::sops::{Sop, SopStatus, Sops};

/// What `wl-documents` answers to the onboarding.
#[derive(Debug, Clone, Copy, Default)]
pub struct SopDesk;

impl SopDesk {
    /// The active SOPs whose current version `who` has not acknowledged and
    /// whose required readers list the role and scope of `who`, with the
    /// current version of each.
    async fn owed(state: &AppState, who: &SopStaff) -> Result<Vec<(Sop, i64)>> {
        let employee = who.employee.subject_id;
        let entity = Acknowledgements::profile(state, employee)
            .await?
            .and_then(|p| p.legal_entity_id);
        let mut owed = Vec::new();
        for sop in Sops::list(&state.db, Some(SopStatus::Active)).await? {
            let readers = Sops::readers(&state.db, sop.id).await?;
            if !lists(&readers, &who.role, &who.scopes) || !entity_fits(&sop, entity) {
                continue;
            }
            let Some(current) = sop.current_document_id else {
                continue;
            };
            let done = SopAcknowledgement::query()
                .where_(SopAcknowledgement::SOP_ID.eq(sop.id))
                .where_(SopAcknowledgement::DOCUMENT_ID.eq(current))
                .where_(SopAcknowledgement::EMPLOYEE_ID.eq(employee))
                .where_(SopAcknowledgement::STATUS.eq(AckStatus::Acknowledged))
                .first(&state.db)
                .await?
                .is_some();
            if !done {
                owed.push((sop, current));
            }
        }
        Ok(owed)
    }
}

impl SopAcknowledgements for SopDesk {
    fn pending_for<'a>(
        &'a self,
        state: &'a AppState,
        who: &'a SopStaff,
    ) -> SopFuture<'a, Vec<SopId>> {
        Box::pin(async move {
            Ok(Self::owed(state, who)
                .await?
                .into_iter()
                .map(|(sop, _)| SopId(sop.id))
                .collect())
        })
    }

    fn acknowledge_in_person<'a>(
        &'a self,
        state: &'a AppState,
        who: &'a SopStaff,
        signature: &'a str,
        witness: i64,
    ) -> SopFuture<'a, Vec<SopId>> {
        Box::pin(async move {
            let employee = who.employee.subject_id;
            let day = wl_kernel::documents::today();
            let Some(profile) = Acknowledgements::profile(state, employee).await? else {
                return Ok(Vec::new());
            };
            let mut recorded = Vec::new();
            for (sop, current) in Self::owed(state, who).await? {
                let Some((version, number)) = Acknowledgements::current(state, &sop).await? else {
                    continue;
                };
                // The request exists already; make it for a reader added a
                // moment ago, without telling them.
                Acknowledgements::request(state, &sop, &version, number, &profile, day, false)
                    .await?;
                let Some(ack) = SopAcknowledgement::query()
                    .where_(SopAcknowledgement::SOP_ID.eq(sop.id))
                    .where_(SopAcknowledgement::DOCUMENT_ID.eq(current))
                    .where_(SopAcknowledgement::EMPLOYEE_ID.eq(employee))
                    .where_(SopAcknowledgement::STATUS.eq(AckStatus::Pending))
                    .first(&state.db)
                    .await?
                else {
                    continue;
                };
                if Acknowledgements::record_in_person(state, ack.id, witness, signature)
                    .await?
                    .is_ok()
                {
                    recorded.push(SopId(sop.id));
                }
            }
            Ok(recorded)
        })
    }
}
