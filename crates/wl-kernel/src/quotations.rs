//! The terms of an accepted quotation that a record built from it needs
//! (`FIN.QUO.01`; 03 Logistics §4.2 row *Client cancels before departure*).
//!
//! The quotation belongs to a module that is not built yet (wl-invoicing
//! quotations). A vertical asks [`Quotations::terms`] for the terms of the
//! quotation a record was opened from, by the record's [`Subject`]; the
//! owner of quotations provides the answer with [`provide_quotations`] and
//! [`quotations`] gives `None` until then, so the vertical falls back to
//! terms entered by hand.
//!
//! Only the terms a vertical acts on are here: the cancellation charges (one
//! fixed amount, or one amount per stage) and whether the costs already
//! incurred are recharged.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::prelude::*;

use crate::subject::Subject;

/// The terms of an accepted quotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotationTerms {
    /// The quotation's reference.
    pub reference: String,
    /// The cancellation charge when it is one amount for any stage, in fils.
    pub cancellation_fixed: Option<i64>,
    /// The cancellation charge per stage, in fils: `(stage, amount)`. When any
    /// is given, only the amount of the stage counts.
    pub cancellation_by_stage: Vec<(String, i64)>,
    /// Whether the costs already incurred are recharged to the client.
    pub recharge_incurred: bool,
}

impl QuotationTerms {
    /// Terms with no cancellation charge, recharging the incurred costs.
    pub fn new(reference: &str) -> Self {
        QuotationTerms {
            reference: reference.to_owned(),
            cancellation_fixed: None,
            cancellation_by_stage: Vec::new(),
            recharge_incurred: true,
        }
    }

    /// The cancellation charge at `stage`, in fils: the stage's amount when
    /// the quotation has amounts per stage (none for a stage it does not
    /// list: no cancellation terms), else the fixed amount. An amount of 0
    /// or less is no charge.
    pub fn cancellation_charge(&self, stage: &str) -> Option<i64> {
        let amount = if self.cancellation_by_stage.is_empty() {
            self.cancellation_fixed
        } else {
            self.cancellation_by_stage
                .iter()
                .find(|(name, _)| name == stage)
                .map(|(_, amount)| *amount)
        };
        amount.filter(|amount| *amount > 0)
    }
}

/// The future a [`Quotations`] method returns.
pub type QuotationFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Reads the terms of the quotation a record was opened from.
pub trait Quotations: Send + Sync + 'static {
    /// The terms of the quotation `subject` (a freight job...) was opened
    /// from; `None` when it has none.
    fn terms<'a>(
        &'a self,
        db: &'a Db,
        subject: &'a Subject,
    ) -> QuotationFuture<'a, Option<QuotationTerms>>;
}

/// Makes `quotations` the app's [`Quotations`]; the owner of quotations
/// calls it in `register`.
pub fn provide_quotations(app: &mut Registry, quotations: impl Quotations) {
    let quotations: Arc<dyn Quotations> = Arc::new(quotations);
    app.provide(quotations);
}

/// The app's [`Quotations`], or `None` when no module provides them.
pub fn quotations(state: &AppState) -> Option<Arc<dyn Quotations>> {
    state
        .provided::<Arc<dyn Quotations>>()
        .map(|found| Arc::clone(&*found))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fixed_amount_counts_for_any_stage() {
        let mut terms = QuotationTerms::new("QUO-1");
        terms.cancellation_fixed = Some(50_000);
        assert_eq!(terms.cancellation_charge("booked"), Some(50_000));
        assert_eq!(terms.cancellation_charge("documents_pending"), Some(50_000));
    }

    #[test]
    fn amounts_per_stage_count_only_for_their_stage() {
        let mut terms = QuotationTerms::new("QUO-1");
        terms.cancellation_by_stage = vec![("booked".to_owned(), 30_000)];
        assert_eq!(terms.cancellation_charge("booked"), Some(30_000));
        assert_eq!(terms.cancellation_charge("documents_pending"), None);
    }

    #[test]
    fn terms_without_an_amount_have_no_charge() {
        let terms = QuotationTerms::new("QUO-1");
        assert_eq!(terms.cancellation_charge("booked"), None);
    }
}
