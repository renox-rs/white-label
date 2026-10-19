//! The screening guard contract (wl-parties §7): how a module warns about, or
//! refuses, onboarding a party while a sanctions list is stale.
//!
//! `wl-parties` provides a [`ScreeningGuard`] with [`provide_screening_guard`]
//! when screening is on. Other modules ask it through [`screening_guard`],
//! which falls back to [`NoScreeningGuard`] (always [`ScreeningOutcome::Allowed`],
//! no notice) when `parties` or screening is off. The guard is provided per app
//! with `Registry::provide`, never process-wide.
//!
//! # Outcomes
//!
//! - [`StaleListPolicy::Warn`] (the default) never refuses: a stale list gives
//!   [`ScreeningOutcome::Warn`] with the lists, and the transition runs.
//! - [`StaleListPolicy::Block`] refuses with a [`ScreeningRefusal`]: a 422 with
//!   code `stale_list` whose text names each stale list and its last successful
//!   update date. It names no candidate, match or block reason, so it is shown
//!   to everyone who can run the transition.
//!
//! The check runs in the caller's transaction. [`run_screened`] wraps one
//! transition: check, then transition, then commit. A module with no
//! transition calls [`ScreeningGuard::check_in`] directly.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::AuthUser;
use renox::chrono::NaiveDate;
use renox::db::Transaction;
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{Outcome, StateMachine, TransitionError};
use serde::{Deserialize, Serialize};

use crate::parties::PartyId;
use crate::texts;

/// The partial that shows the notice or the refusal.
pub const NOTICE_TEMPLATE: &str = "wl-kernel/screening_notice.html";

/// The `code` of a [`ScreeningRefusal`].
pub const STALE_LIST_CODE: &str = "stale_list";

/// What a guarded onboarding does while a sanctions list is stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StaleListPolicy {
    /// The transition runs and the record shows a notice.
    #[default]
    Warn,
    /// The transition is refused until the list is current and re-screened.
    Block,
}

/// A stale sanctions list with its last successful update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaleList {
    /// The list's name, such as "UN Consolidated List".
    pub name: String,
    /// The last successful update in Asia/Dubai; `None` when never updated.
    pub list_date: Option<NaiveDate>,
}

impl StaleList {
    /// A stale list. `list_date` is `None` for a list never updated.
    pub fn new(name: impl Into<String>, list_date: Option<NaiveDate>) -> Self {
        StaleList {
            name: name.into(),
            list_date,
        }
    }

    /// "UN Consolidated List of 2026-11-02", or "... (no update yet)".
    pub fn label(&self, lang: &Lang) -> String {
        match self.list_date {
            Some(date) => texts::t_with(
                lang,
                "wl_kernel.screening_guard.list",
                &[("name", &self.name), ("date", &date)],
            ),
            None => texts::t_with(
                lang,
                "wl_kernel.screening_guard.never_updated",
                &[("name", &self.name)],
            ),
        }
    }

    /// [`label`](Self::label) in English.
    pub fn label_en(&self) -> String {
        match self.list_date {
            Some(date) => texts::en_with(
                "wl_kernel.screening_guard.list",
                &[("name", &self.name), ("date", &date)],
            ),
            None => texts::en_with(
                "wl_kernel.screening_guard.never_updated",
                &[("name", &self.name)],
            ),
        }
    }
}

/// What the guard says about one onboarding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreeningOutcome {
    /// No list is stale: go ahead.
    Allowed,
    /// Go ahead, and tell the user these lists were stale.
    Warn(Vec<StaleList>),
    /// Do not go ahead.
    Blocked(ScreeningRefusal),
}

impl ScreeningOutcome {
    /// True for [`Allowed`](Self::Allowed).
    pub fn is_allowed(&self) -> bool {
        matches!(self, ScreeningOutcome::Allowed)
    }

    /// True for [`Blocked`](Self::Blocked).
    pub fn is_blocked(&self) -> bool {
        matches!(self, ScreeningOutcome::Blocked(_))
    }

    /// The stale lists of a [`Warn`](Self::Warn); empty otherwise.
    pub fn warned(&self) -> &[StaleList] {
        match self {
            ScreeningOutcome::Warn(lists) => lists,
            _ => &[],
        }
    }

    /// `Ok` with the outcome unless it is blocked.
    pub fn into_result(self) -> std::result::Result<ScreeningOutcome, ScreeningRefusal> {
        match self {
            ScreeningOutcome::Blocked(refusal) => Err(refusal),
            other => Ok(other),
        }
    }
}

/// The refusal of block mode: 422, code `stale_list`, naming the stale lists
/// and their dates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreeningRefusal {
    /// The party the transition acts on.
    pub party: PartyId,
    /// The stale lists, each once.
    pub lists: Vec<StaleList>,
}

impl ScreeningRefusal {
    /// A refusal naming `lists`.
    pub fn new(party: PartyId, lists: Vec<StaleList>) -> Self {
        ScreeningRefusal { party, lists }
    }

    /// Always `stale_list`.
    pub fn code(&self) -> &'static str {
        STALE_LIST_CODE
    }

    /// "Screening list is out of date: UN Consolidated List of 2026-11-02".
    pub fn message(&self, lang: &Lang) -> String {
        let lists = self
            .lists
            .iter()
            .map(|l| l.label(lang))
            .collect::<Vec<_>>()
            .join(", ");
        texts::t_with(
            lang,
            "wl_kernel.screening_guard.refused",
            &[("lists", &lists)],
        )
    }

    /// [`message`](Self::message) in English.
    pub fn message_en(&self) -> String {
        texts::en_with(
            "wl_kernel.screening_guard.refused",
            &[("lists", &join_en(&self.lists))],
        )
    }

    /// The view for `wl-kernel/screening_notice.html`.
    pub fn view(&self, lang: &Lang) -> ScreeningNoticeView {
        ScreeningNoticeView {
            code: Some(self.code().to_owned()),
            message: self.message(lang),
            lists: self.lists.iter().map(|l| l.label(lang)).collect(),
        }
    }

    /// A 422 with the English text.
    pub fn into_error(self) -> Error {
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, self.message_en())
    }
}

impl fmt::Display for ScreeningRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message_en())
    }
}

impl std::error::Error for ScreeningRefusal {}

fn join_en(lists: &[StaleList]) -> String {
    lists
        .iter()
        .map(StaleList::label_en)
        .collect::<Vec<_>>()
        .join(", ")
}

/// What `wl-kernel/screening_notice.html` shows as `notice`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScreeningNoticeView {
    /// `stale_list` for a refusal, `None` for the record's notice.
    pub code: Option<String>,
    /// The line to show.
    pub message: String,
    /// The lists, one label each.
    pub lists: Vec<String>,
}

/// The notice line "screened on a stale list (UN Consolidated List of
/// 2026-11-02)" for the record, or `None` when `lists` is empty.
pub fn stale_notice_view(lang: &Lang, lists: &[StaleList]) -> Option<ScreeningNoticeView> {
    if lists.is_empty() {
        return None;
    }
    let labels: Vec<String> = lists.iter().map(|l| l.label(lang)).collect();
    let message = texts::t_with(
        lang,
        "wl_kernel.screening_guard.notice",
        &[("lists", &labels.join(", "))],
    );
    Some(ScreeningNoticeView {
        code: None,
        message,
        lists: labels,
    })
}

/// A boxed future, so the trait can be used as `dyn`.
pub type ScreeningFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// What `wl-parties` provides.
pub trait ScreeningGuard: Send + Sync + 'static {
    /// Checks `party` in the caller's transaction. In warn mode it also keeps
    /// the notice on the party's latest screening. It never runs a screening.
    fn check_in<'a>(
        &'a self,
        tx: &'a mut Transaction,
        party: PartyId,
    ) -> ScreeningFuture<'a, ScreeningOutcome>;

    /// The lists the party's latest screening is flagged stale on; empty when
    /// there is no notice to show.
    fn latest_notice_in<'a>(
        &'a self,
        tx: &'a mut Transaction,
        party: PartyId,
    ) -> ScreeningFuture<'a, Vec<StaleList>>;
}

/// The guard when `parties` or screening is off: always allowed, no notice.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoScreeningGuard;

impl ScreeningGuard for NoScreeningGuard {
    fn check_in<'a>(
        &'a self,
        _tx: &'a mut Transaction,
        _party: PartyId,
    ) -> ScreeningFuture<'a, ScreeningOutcome> {
        Box::pin(async { Ok(ScreeningOutcome::Allowed) })
    }

    fn latest_notice_in<'a>(
        &'a self,
        _tx: &'a mut Transaction,
        _party: PartyId,
    ) -> ScreeningFuture<'a, Vec<StaleList>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// Provides `guard` to this app. `wl-parties` calls it in `register`.
pub fn provide_screening_guard(app: &mut Registry, guard: impl ScreeningGuard) {
    let guard: Arc<dyn ScreeningGuard> = Arc::new(guard);
    app.provide(guard);
}

/// The app's screening guard, or [`NoScreeningGuard`].
pub fn screening_guard(state: &AppState) -> Arc<dyn ScreeningGuard> {
    match state.provided::<Arc<dyn ScreeningGuard>>() {
        Some(guard) => Arc::clone(&guard),
        None => Arc::new(NoScreeningGuard),
    }
}

/// Why [`run_screened`] did not run the transition to the end.
#[derive(Debug)]
pub enum ScreenedError {
    /// Block mode refused; nothing ran.
    Refused(ScreeningRefusal),
    /// The transition failed.
    Transition(TransitionError),
}

impl ScreenedError {
    /// The refusal, if that is what happened.
    pub fn refusal(&self) -> Option<&ScreeningRefusal> {
        match self {
            ScreenedError::Refused(refusal) => Some(refusal),
            ScreenedError::Transition(_) => None,
        }
    }

    /// A 422 for a refusal, else the transition's own error.
    pub fn into_error(self) -> Error {
        match self {
            ScreenedError::Refused(refusal) => refusal.into_error(),
            ScreenedError::Transition(error) => error.into_error(),
        }
    }
}

impl From<TransitionError> for ScreenedError {
    fn from(error: TransitionError) -> Self {
        ScreenedError::Transition(error)
    }
}

impl From<renox::db::DbError> for ScreenedError {
    fn from(error: renox::db::DbError) -> Self {
        ScreenedError::Transition(error.into())
    }
}

impl From<Error> for ScreenedError {
    fn from(error: Error) -> Self {
        ScreenedError::Transition(TransitionError::Failed(error))
    }
}

/// A transition that ran, with what the guard said.
pub struct Screened<M: StateMachine> {
    /// The transition's outcome.
    pub outcome: Outcome<M>,
    /// [`ScreeningOutcome::Allowed`] or [`ScreeningOutcome::Warn`], for the
    /// caller's flash message.
    pub screening: ScreeningOutcome,
}

/// Runs `transition` on `record` for `party` in one transaction: the screening
/// check first, then the transition (with the same events and approvals as
/// [`run_guarded`](crate::block_guard::run_guarded), including its machine
/// hook). A refusal runs nothing and writes no history row.
pub async fn run_screened<M>(
    state: &AppState,
    user: &AuthUser,
    record: &M,
    transition: &str,
    party: PartyId,
    input: Value,
) -> std::result::Result<Screened<M>, ScreenedError>
where
    M: StateMachine<Key = i64> + Send + Sync,
{
    let mut tx = state.db.begin().await?;
    let screening = screening_guard(state).check_in(&mut tx, party).await?;
    if let ScreeningOutcome::Blocked(refusal) = screening {
        return Err(ScreenedError::Refused(refusal));
    }
    let outcome = record
        .transition(transition)
        .by(user)
        .input(&input)
        .run_in(&mut tx)
        .await?;
    tx.commit().await?;
    if let Outcome::Done(done) = &outcome {
        state.emit(done.clone()).await?;
    }
    crate::approvals::announce(state, &outcome).await?;
    Ok(Screened { outcome, screening })
}

#[cfg(test)]
mod tests {
    use renox::testing::TestApp;

    use super::*;

    fn un() -> StaleList {
        StaleList::new("UN Consolidated List", NaiveDate::from_ymd_opt(2026, 11, 2))
    }

    #[test]
    fn warn_is_the_default_policy() {
        assert_eq!(StaleListPolicy::default(), StaleListPolicy::Warn);
        let json = renox::serde_json::to_string(&StaleListPolicy::Block).unwrap();
        assert_eq!(json, r#""block""#);
    }

    #[test]
    fn the_refusal_names_the_list_and_its_date() {
        let refusal = ScreeningRefusal::new(PartyId(7), vec![un()]);
        assert_eq!(refusal.code(), "stale_list");
        assert_eq!(
            refusal.message_en(),
            "Screening list is out of date: UN Consolidated List of 2026-11-02"
        );
        assert_eq!(refusal.to_string(), refusal.message_en());
    }

    #[test]
    fn the_refusal_names_each_list_and_a_never_updated_one() {
        let refusal = ScreeningRefusal::new(
            PartyId(7),
            vec![un(), StaleList::new("OFAC SDN List", None)],
        );
        assert_eq!(
            refusal.message_en(),
            "Screening list is out of date: UN Consolidated List of 2026-11-02, \
             OFAC SDN List (no update yet)"
        );
    }

    #[test]
    fn the_refusal_is_a_422() {
        let error = ScreeningRefusal::new(PartyId(7), vec![un()]).into_error();
        match error {
            Error::Status(status, text) => {
                assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
                assert!(text.contains("UN Consolidated List of 2026-11-02"));
            }
            other => panic!("expected a 422, got {other:?}"),
        }
    }

    #[test]
    fn the_outcome_helpers_tell_the_cases_apart() {
        let refusal = ScreeningRefusal::new(PartyId(1), vec![un()]);
        assert!(ScreeningOutcome::Allowed.is_allowed());
        assert!(ScreeningOutcome::Allowed.warned().is_empty());
        let warn = ScreeningOutcome::Warn(vec![un()]);
        assert!(!warn.is_allowed() && !warn.is_blocked());
        assert_eq!(warn.warned().len(), 1);
        assert!(warn.into_result().is_ok());
        let blocked = ScreeningOutcome::Blocked(refusal.clone());
        assert!(blocked.is_blocked());
        assert_eq!(blocked.into_result().unwrap_err(), refusal);
    }

    #[renox::test]
    async fn the_notice_line_names_the_lists_and_is_none_without_any() {
        let app = TestApp::new(App::new()).await;
        let lang = app.state().lang("en");
        assert!(stale_notice_view(&lang, &[]).is_none());
        let view = stale_notice_view(&lang, &[un()]).unwrap();
        assert_eq!(
            view.message,
            "screened on a stale list (UN Consolidated List of 2026-11-02)"
        );
        assert_eq!(view.code, None);
        let refused = ScreeningRefusal::new(PartyId(1), vec![un()]).view(&lang);
        assert_eq!(refused.code.as_deref(), Some("stale_list"));
        assert_eq!(refused.lists, ["UN Consolidated List of 2026-11-02"]);
    }

    #[renox::test]
    async fn without_a_provider_the_guard_allows_and_shows_no_notice() {
        let app = TestApp::new(App::new()).await;
        let state = app.state();
        let guard = screening_guard(state);
        let mut tx = state.db.begin().await.unwrap();
        let outcome = guard.check_in(&mut tx, PartyId(1)).await.unwrap();
        assert_eq!(outcome, ScreeningOutcome::Allowed);
        let notice = guard.latest_notice_in(&mut tx, PartyId(1)).await.unwrap();
        assert!(notice.is_empty());
    }
}
