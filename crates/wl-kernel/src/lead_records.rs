//! The lead-capture contract (wl-sales §5.8 point 5, BL-SAL-27): a vertical
//! that keeps its own lead records (01's enquiries, `PRP.LEA.01`) reports them
//! here, and `wl-sales` counts them in the acquisition report as leads,
//! without copying them into its lead rows and without the vertical
//! depending on `wl-sales`.
//!
//! A vertical implements [`LeadRecords`] and registers it in its module's
//! `register` with [`register_lead_records`]; `wl-sales` reads every
//! registered provider ([`lead_providers`]) when it computes the report. Each
//! record says where it came from (the lead source code and the campaign
//! code, as stored on the record at capture), the day it was created, the day
//! it was won (01: the enquiry converted and its tenant's first lease
//! activated) and whether it is excluded (closed as duplicate, spam or
//! invalid), which leaves it out of the leads as a wl-sales lead would be.
//! A record that is won is still one lead: the provider reports it once.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::subject::Subject;

/// The future a [`LeadRecords`] method returns.
pub type LeadRecordsFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// One lead record of a vertical, as the acquisition report reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LeadRecord {
    /// The record, e.g. `property.enquiry:12`.
    pub subject: Option<Subject>,
    /// The lead source code (a `wl-sales` lead source, e.g. `paid_ads`).
    pub source: Option<String>,
    /// The campaign code, as given at capture.
    pub campaign: Option<String>,
    /// The day it was created (Asia/Dubai).
    pub created: NaiveDate,
    /// The day it was won, if it was.
    pub won_on: Option<NaiveDate>,
    /// Closed as duplicate, spam or invalid: left out of the leads.
    pub excluded: bool,
    /// The branch it belongs to, if the vertical has one.
    pub branch: Option<Subject>,
    /// The legal entity, if known.
    pub legal_entity_id: Option<i64>,
}

/// What a vertical implements to have its own lead records counted as leads.
pub trait LeadRecords: Send + Sync + 'static {
    /// The records created or won from `from` to `to` (both included; an
    /// unset bound is open). A record created before `from` and won inside
    /// the period is returned too, so its win counts.
    fn records<'a>(
        &'a self,
        db: &'a Db,
        from: Option<NaiveDate>,
        to: Option<NaiveDate>,
    ) -> LeadRecordsFuture<'a, Vec<LeadRecord>>;
}

/// A registered provider: its name, the label the vertical gives its leads
/// (*Enquiries* in 01) and the implementation.
#[derive(Clone)]
pub struct LeadProvider {
    /// The provider's name, e.g. `property.enquiries`.
    pub name: &'static str,
    /// The vertical's name for its leads, in English, e.g. `Enquiries`.
    pub label: &'static str,
    /// The implementation.
    pub records: Arc<dyn LeadRecords>,
}

impl std::fmt::Debug for LeadProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LeadProvider")
            .field("name", &self.name)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

/// Registers `records` as a lead provider named `name` whose leads the
/// vertical calls `label`; the vertical calls it in its module's `register`.
/// The registration belongs to the app being built (a contribution), so an
/// app without the vertical has no provider.
pub fn register_lead_records(
    app: &mut Registry,
    name: &'static str,
    label: &'static str,
    records: impl LeadRecords,
) {
    app.contribute(LeadProvider {
        name,
        label,
        records: Arc::new(records),
    });
}

/// Every lead provider the app's modules registered, in module order.
pub fn lead_providers(state: &AppState) -> Vec<LeadProvider> {
    state.contributions::<LeadProvider>().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_is_counted_unless_excluded() {
        let record = LeadRecord {
            source: Some("paid_ads".into()),
            created: NaiveDate::from_ymd_opt(2026, 10, 3).unwrap(),
            ..LeadRecord::default()
        };
        assert!(!record.excluded);
        assert!(record.won_on.is_none());
    }
}
