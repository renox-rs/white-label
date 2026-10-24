//! Work a transaction-aware contract method leaves for after the caller's
//! commit (wl-kernel §6 *Capabilities*, BL-KRN-11).
//!
//! The money contracts (`Invoicer`, `Cheques`, `Deposits`) have variants that
//! run inside the caller's open transaction (`*_in(&mut tx, &mut after, …)`)
//! so a transition that changes several modules' records (01 lease
//! `terminate`) commits or rolls back as a whole. Such a variant never
//! commits and never emits: it queues its events (`CreditNoteIssued`,
//! `ChequeReturned`, the transitions' own events, and through their
//! listeners any message) on an [`AfterCommit`]. The caller runs it once the
//! transaction has committed; on a rollback it simply drops it, so nothing is
//! emitted and nobody is told about work that did not happen.
//!
//! ```ignore
//! let mut tx = state.db.begin().await?;
//! let mut after = AfterCommit::new();
//! invoicer(state).credit_partly_in(state, &mut tx, &mut after, invoice, lines, reason, None).await?;
//! cheques(state).return_undeposited_in(state, &mut tx, &mut after, request, None).await?;
//! tx.commit().await?;
//! after.run(state).await?;
//! ```

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use renox::prelude::*;

/// The future of one step.
type StepFuture = Pin<Box<dyn Future<Output = Result> + Send>>;

/// One step to run after the commit.
type Step = Box<dyn FnOnce(AppState) -> StepFuture + Send>;

/// The events (and other after-commit work) of the contract calls made in
/// one caller transaction, in the order they were queued.
#[derive(Default)]
#[must_use = "run it after the transaction commits, or drop it on a rollback"]
pub struct AfterCommit {
    steps: Vec<Step>,
}

impl fmt::Debug for AfterCommit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AfterCommit")
            .field("steps", &self.steps.len())
            .finish()
    }
}

impl AfterCommit {
    /// Nothing queued yet.
    pub fn new() -> Self {
        AfterCommit::default()
    }

    /// Queues `step`, run with the app's state after the commit.
    pub fn push<F, Fut>(&mut self, step: F)
    where
        F: FnOnce(AppState) -> Fut + Send + 'static,
        Fut: Future<Output = Result> + Send + 'static,
    {
        let step: Step = Box::new(move |state: AppState| -> StepFuture { Box::pin(step(state)) });
        self.steps.push(step);
    }

    /// Queues the emission of `event`.
    pub fn emit<E: Event>(&mut self, event: E) {
        self.push(move |state: AppState| async move { state.emit(event).await });
    }

    /// Moves the steps of `other` to the end of this one.
    pub fn append(&mut self, other: AfterCommit) {
        self.steps.extend(other.steps);
    }

    /// How many steps are queued.
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// Whether nothing is queued.
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Runs the steps in order; call it only after the commit. The first
    /// error stops the rest and is returned (the data is committed then).
    pub async fn run(self, state: &AppState) -> Result {
        for step in self.steps {
            step(state.clone()).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_counted_and_appended_in_order() {
        let mut after = AfterCommit::new();
        assert!(after.is_empty());
        after.push(|_state| async { Ok(()) });
        let mut more = AfterCommit::new();
        more.push(|_state| async { Ok(()) });
        more.push(|_state| async { Ok(()) });
        after.append(more);
        assert_eq!(after.len(), 3);
        assert_eq!(format!("{after:?}"), "AfterCommit { steps: 3 }");
    }
}
