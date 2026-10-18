//! The registry of voidable entities (wl-kernel §5.12), keyed by subject
//! type like [`crate::machines`].
//!
//! A module declares its entity once in its `register()`:
//!
//! ```ignore
//! Voidable::of::<Receipt>("payments.receipt")
//!     .permission("receipts.void")
//!     .scope(|receipt: &Receipt| Scope::of_id::<Building>(receipt.building_id))
//!     .number("receipt", |receipt| receipt.number.clone())
//!     .given_to_third_party(|receipt| receipt.issued_to_third_party)
//!     .reverse(EffectKind::Allocation, Reverse::new(pending_allocations, reverse_allocations))
//!     .register();
//! ```
//!
//! The record also needs a [`crate::SubjectMachine`] entry, which is how the
//! kernel runs `void_approved` by name.

use std::any::Any;
use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;
use std::sync::{Arc, PoisonError, RwLock};

use renox::auth::permissions::Scope;
use renox::db::Transaction;
use renox::state_machine::{MachineInfo, StateMachine, TransitionError, info_of};
use renox::{AppState, AuthUser, Result};

use super::record::{Effect, EffectKind, ReverseError, VoidRecord};
use super::run::VoidOutcome;
use super::{RecordVoided, VOID, VOID_APPROVED, VOIDED_STATE};
use crate::machines::{BoxFuture, load};
use crate::permissions::RECORDS_VOID;
use crate::subject::Subject;

type PendingFn = Arc<
    dyn for<'a> Fn(&'a mut Transaction, i64) -> BoxFuture<'a, Result<Vec<Effect>>> + Send + Sync,
>;
type ReverseFn = Arc<
    dyn for<'a> Fn(&'a mut Transaction, i64) -> BoxFuture<'a, Result<Vec<Effect>, ReverseError>>
        + Send
        + Sync,
>;
type ScopeFn =
    Arc<dyn for<'a> Fn(&'a mut Transaction, i64) -> BoxFuture<'a, Result<Scope>> + Send + Sync>;
type NumberFn = Arc<
    dyn for<'a> Fn(&'a mut Transaction, i64) -> BoxFuture<'a, Result<Option<String>>> + Send + Sync,
>;
type ThirdPartyFn =
    Arc<dyn for<'a> Fn(&'a mut Transaction, i64) -> BoxFuture<'a, Result<bool>> + Send + Sync>;

type RunFn = Arc<
    dyn for<'a> Fn(
            &'a AppState,
            &'a AuthUser,
            i64,
            &'a str,
        ) -> BoxFuture<'a, std::result::Result<VoidOutcome, TransitionError>>
        + Send
        + Sync,
>;

/// A function of a loaded record.
type RecordFn<M, T> = Arc<dyn Fn(&M) -> T + Send + Sync>;

/// The handler of one kind of effect: what is there to reverse, and the
/// reversal itself. Both run in the void's transaction.
#[derive(Clone)]
pub struct Reverse {
    pending: PendingFn,
    reverse: ReverseFn,
}

impl Reverse {
    /// A handler from its two parts. `pending(tx, id)` lists what there is
    /// to reverse on record `id` without writing anything; `reverse(tx, id)`
    /// undoes it and lists what it undid. An error from `reverse` fails the
    /// whole void.
    pub fn new<P, R>(pending: P, reverse: R) -> Self
    where
        P: for<'a> Fn(&'a mut Transaction, i64) -> BoxFuture<'a, Result<Vec<Effect>>>
            + Send
            + Sync
            + 'static,
        R: for<'a> Fn(&'a mut Transaction, i64) -> BoxFuture<'a, Result<Vec<Effect>, ReverseError>>
            + Send
            + Sync
            + 'static,
    {
        Reverse {
            pending: Arc::new(pending),
            reverse: Arc::new(reverse),
        }
    }
}

/// A voidable entity without its model type. Built with [`Voidable::of`].
pub struct Voidable {
    subject_type: String,
    table: &'static str,
    permission: String,
    series_key: Option<String>,
    has_scope: bool,
    scope: ScopeFn,
    number: Option<NumberFn>,
    third_party: Option<ThirdPartyFn>,
    /// The typed `Fn(&M) -> Scope`, for the transition's permission guard.
    record_scope: Option<Arc<dyn Any + Send + Sync>>,
    /// The typed `Fn(&M) -> bool`, for the transition's third-party guard.
    record_third_party: Option<Arc<dyn Any + Send + Sync>>,
    run: RunFn,
    handlers: Vec<(EffectKind, Reverse)>,
}

impl fmt::Debug for Voidable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Voidable")
            .field("subject_type", &self.subject_type)
            .field("table", &self.table)
            .field("permission", &self.permission)
            .field("series_key", &self.series_key)
            .field("kinds", &self.kinds())
            .finish()
    }
}

impl Voidable {
    /// Starts the entry of `M`, the model of records of type `subject_type`
    /// (`<module>.<entity>`).
    ///
    /// # Panics
    ///
    /// When `subject_type` is not `<module>.<entity>`, or when `M`'s machine
    /// breaks the void rules (see [`check_machine`]): both are programming
    /// errors, found at boot.
    pub fn of<M>(subject_type: &str) -> VoidableBuilder<M>
    where
        M: StateMachine<Key = i64>,
    {
        // Checks the type's form; panics like `Subject::new` on a bad one.
        let _ = Subject::new(subject_type, 0);
        let info = info_of::<M>();
        if let Err(problems) = check_machine(&info) {
            panic!(
                "{subject_type}: {} cannot be voided: {}",
                info.model,
                problems.join("; ")
            );
        }
        VoidableBuilder {
            subject_type: subject_type.to_owned(),
            permission: RECORDS_VOID.to_owned(),
            series_key: None,
            scope: None,
            number: None,
            third_party: None,
            handlers: Vec::new(),
            _model: PhantomData,
        }
    }

    /// The subject type, e.g. `payments.receipt`.
    pub fn subject_type(&self) -> &str {
        &self.subject_type
    }

    /// The model's table.
    pub fn table(&self) -> &'static str {
        self.table
    }

    /// The permission that voids the record (and approves a void).
    pub fn permission(&self) -> &str {
        &self.permission
    }

    /// The number series of the record's number, if it has one.
    pub fn series_key(&self) -> Option<&str> {
        self.series_key.as_deref()
    }

    /// Whether the entry gives a scope; without one the global scope is used.
    pub fn has_scope(&self) -> bool {
        self.has_scope
    }

    /// The scope of an already loaded `record` (the global scope when the
    /// entry gives none, or when `M` is not the entry's model).
    pub fn scope_of<M: 'static>(&self, record: &M) -> Scope {
        self.record_scope
            .as_ref()
            .and_then(|scope| scope.downcast_ref::<RecordFn<M, Scope>>())
            .map_or_else(Scope::global, |scope| scope(record))
    }

    /// Whether an already loaded `record` was given to a third party;
    /// `false` when the entry does not say.
    pub fn given_to_third_party_of<M: 'static>(&self, record: &M) -> bool {
        self.record_third_party
            .as_ref()
            .and_then(|given| given.downcast_ref::<RecordFn<M, bool>>())
            .is_some_and(|given| given(record))
    }

    /// Runs the typed `void` transition on record `id` as `user`, in its own
    /// transaction, with `reason`. See [`super::void`].
    pub(super) async fn run_as(
        &self,
        state: &AppState,
        user: &AuthUser,
        id: i64,
        reason: &str,
    ) -> std::result::Result<VoidOutcome, TransitionError> {
        (self.run)(state, user, id, reason).await
    }

    /// The kinds of effect the entry reverses, in registration order.
    pub fn kinds(&self) -> Vec<EffectKind> {
        self.handlers.iter().map(|(kind, _)| *kind).collect()
    }

    /// The scope of record `id` (the global scope when the entry gives none).
    pub async fn scope_in(&self, tx: &mut Transaction, id: i64) -> Result<Scope> {
        (self.scope)(tx, id).await
    }

    /// The number record `id` keeps; `None` when the entry has no number or
    /// the record has none yet.
    pub async fn number_in(&self, tx: &mut Transaction, id: i64) -> Result<Option<String>> {
        match &self.number {
            Some(number) => number(tx, id).await,
            None => Ok(None),
        }
    }

    /// Whether record `id` was given to a third party; `false` when the
    /// entry does not say.
    pub async fn given_to_third_party_in(&self, tx: &mut Transaction, id: i64) -> Result<bool> {
        match &self.third_party {
            Some(third_party) => third_party(tx, id).await,
            None => Ok(false),
        }
    }

    /// What there is to reverse on record `id`, from every handler in
    /// registration order. Writes nothing.
    pub async fn pending_in(&self, tx: &mut Transaction, id: i64) -> Result<Vec<Effect>> {
        let mut effects = Vec::new();
        for (_, handler) in &self.handlers {
            effects.extend((handler.pending)(tx, id).await?);
        }
        Ok(effects)
    }

    /// How many of record `id`'s pending effects are money or stock
    /// ([`Effect::is_posted`]); the kernel works this out itself.
    pub async fn posted_in(&self, tx: &mut Transaction, id: i64) -> Result<i64> {
        let effects = self.pending_in(tx, id).await?;
        Ok(i64::try_from(effects.iter().filter(|e| e.is_posted()).count()).unwrap_or(i64::MAX))
    }

    /// Reverses record `id`'s effects, every handler in registration order,
    /// and lists what was undone. The first error stops it; the caller's
    /// transaction then rolls back.
    pub async fn reverse_in(
        &self,
        tx: &mut Transaction,
        id: i64,
    ) -> Result<Vec<Effect>, ReverseError> {
        let mut undone = Vec::new();
        for (_, handler) in &self.handlers {
            undone.extend((handler.reverse)(tx, id).await?);
        }
        Ok(undone)
    }

    /// Adds this entry to the process-wide registry, replacing one of the
    /// same subject type.
    pub fn register(self) {
        register(self);
    }
}

/// The entry being declared, from [`Voidable::of`]; end with
/// [`VoidableBuilder::register`] (or [`VoidableBuilder::build`]).
#[must_use = "an entity is only voidable once registered"]
pub struct VoidableBuilder<M: StateMachine<Key = i64>> {
    subject_type: String,
    permission: String,
    series_key: Option<String>,
    scope: Option<RecordFn<M, Scope>>,
    number: Option<RecordFn<M, Option<String>>>,
    third_party: Option<RecordFn<M, bool>>,
    handlers: Vec<(EffectKind, Reverse)>,
    _model: PhantomData<fn() -> M>,
}

impl<M: StateMachine<Key = i64> + 'static> VoidableBuilder<M> {
    /// The module's alias of the void permission, e.g. `receipts.void`
    /// (default: `records.void`).
    pub fn permission(mut self, permission: impl Into<String>) -> Self {
        self.permission = permission.into();
        self
    }

    /// The scope of a record, where the permission is checked and approvers
    /// are looked for (default: the global scope).
    pub fn scope(mut self, scope: impl Fn(&M) -> Scope + Send + Sync + 'static) -> Self {
        self.scope = Some(Arc::new(scope));
        self
    }

    /// The number a voided record keeps and the series it came from.
    pub fn number(
        mut self,
        series_key: impl Into<String>,
        number: impl Fn(&M) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        self.series_key = Some(series_key.into());
        self.number = Some(Arc::new(number));
        self
    }

    /// Whether a record was given to a third party (issued, sent): such a
    /// record cannot be voided.
    pub fn given_to_third_party(
        mut self,
        given: impl Fn(&M) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.third_party = Some(Arc::new(given));
        self
    }

    /// The handler of one kind of effect.
    ///
    /// # Panics
    ///
    /// When a handler of `kind` was already given.
    pub fn reverse(mut self, kind: EffectKind, handler: Reverse) -> Self {
        assert!(
            self.handlers.iter().all(|(k, _)| *k != kind),
            "{}: there is already a reverse handler for `{kind}`",
            self.subject_type
        );
        self.handlers.push((kind, handler));
        self
    }

    /// The entry, without registering it.
    pub fn build(self) -> Voidable {
        let has_scope = self.scope.is_some();
        let record_scope = self
            .scope
            .clone()
            .map(|scope| -> Arc<dyn Any + Send + Sync> { Arc::new(scope) });
        let record_third_party = self
            .third_party
            .clone()
            .map(|given| -> Arc<dyn Any + Send + Sync> { Arc::new(given) });
        let scope = self.scope;
        let scope: ScopeFn = Arc::new(move |tx: &mut Transaction, id: i64| {
            let scope = scope.clone();
            Box::pin(async move {
                let Some(scope) = scope else {
                    return Ok(Scope::global());
                };
                Ok(scope(&load::<M>(tx, id).await?))
            })
        });
        let number = self.number.map(|number| -> NumberFn {
            Arc::new(move |tx: &mut Transaction, id: i64| {
                let number = number.clone();
                Box::pin(async move { Ok(number(&load::<M>(tx, id).await?)) })
            })
        });
        let third_party = self.third_party.map(|given| -> ThirdPartyFn {
            Arc::new(move |tx: &mut Transaction, id: i64| {
                let given = given.clone();
                Box::pin(async move { Ok(given(&load::<M>(tx, id).await?)) })
            })
        });
        let subject = self.subject_type.clone();
        let run: RunFn = Arc::new(
            move |state: &AppState, user: &AuthUser, id: i64, reason: &str| {
                let subject = crate::subject::Subject::new(&subject, id);
                Box::pin(run_as::<M>(state, user, subject, reason))
            },
        );
        Voidable {
            subject_type: self.subject_type,
            table: M::TABLE,
            permission: self.permission,
            series_key: self.series_key,
            has_scope,
            scope,
            number,
            third_party,
            record_scope,
            record_third_party,
            run,
            handlers: self.handlers,
        }
    }

    /// Builds the entry and adds it to the process-wide registry, replacing
    /// one of the same subject type.
    pub fn register(self) {
        self.build().register();
    }
}

/// Runs `void` on `subject` as `user`: one transaction, then `Transitioned`
/// and `RecordVoided` once it has committed.
async fn run_as<M: StateMachine<Key = i64>>(
    state: &AppState,
    user: &AuthUser,
    subject: Subject,
    reason: &str,
) -> std::result::Result<VoidOutcome, TransitionError> {
    use renox::state_machine::Outcome;

    let mut tx = state.db.begin().await?;
    let record = load::<M>(&mut tx, subject.subject_id).await?;
    let outcome = record
        .transition(VOID)
        .by(user)
        .reason(reason.trim())
        .run_in(&mut tx)
        .await?;
    match outcome {
        Outcome::Done(done) => {
            let void = VoidRecord::of_subject(&mut tx, &subject)
                .await?
                .ok_or_else(|| {
                    renox::Error::Internal(renox::anyhow::anyhow!(
                        "{subject}: voided without a Void record"
                    ))
                })?;
            tx.commit().await?;
            done.emit(state).await?;
            let event = RecordVoided {
                subject,
                state_before: void.state_before,
                reason: void.reason,
                user_id: void.voided_by,
                approval_request_id: void.approval_request_id,
            };
            state.emit(event.clone()).await?;
            Ok(VoidOutcome::Voided(event))
        }
        Outcome::Pending(pending) => {
            tx.commit().await?;
            Ok(VoidOutcome::Pending(pending.data))
        }
    }
}

/// What the void rules look at in a machine.
struct Shape<'a> {
    states: Vec<(&'a str, bool)>,
    transitions: Vec<(&'a str, &'a [String], &'a str, bool)>,
}

impl<'a> Shape<'a> {
    fn of(info: &'a MachineInfo) -> Self {
        Shape {
            states: info
                .states
                .iter()
                .map(|s| (s.name.as_str(), s.terminal))
                .collect(),
            transitions: info
                .transitions
                .iter()
                .map(|t| (t.name.as_str(), t.from.as_slice(), t.to.as_str(), t.system))
                .collect(),
        }
    }

    /// Everything wrong with the machine for voiding; empty when it is fine.
    fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        match self.states.iter().find(|(name, _)| *name == VOIDED_STATE) {
            None => problems.push(format!("it has no `{VOIDED_STATE}` state")),
            Some((_, terminal)) => {
                if !terminal {
                    problems.push(format!("`{VOIDED_STATE}` is not terminal"));
                }
                if self
                    .transitions
                    .iter()
                    .any(|(_, from, _, _)| from.iter().any(|s| s == VOIDED_STATE))
                {
                    problems.push(format!("a transition leaves `{VOIDED_STATE}`"));
                }
            }
        }
        for name in [VOID, VOID_APPROVED] {
            match self.transitions.iter().find(|(n, ..)| *n == name) {
                None => problems.push(format!("it has no `{name}` transition")),
                Some((_, _, to, system)) => {
                    if *to != VOIDED_STATE {
                        problems.push(format!("`{name}` leads to `{to}`, not `{VOIDED_STATE}`"));
                    }
                    if name == VOID_APPROVED && !system {
                        problems.push(format!("`{name}` must be `.system()`"));
                    }
                    if name == VOID && *system {
                        problems.push(format!("`{name}` must be run by a user, not `.system()`"));
                    }
                }
            }
        }
        problems
    }
}

/// Checks a machine against the void rules: it has a terminal `voided` state
/// with no transition out of it, and `void` (a user's) and `void_approved`
/// (`.system()`) both lead to it. Lists every problem found.
pub fn check_machine(info: &MachineInfo) -> std::result::Result<(), Vec<String>> {
    let problems = Shape::of(info).problems();
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

static REGISTRY: RwLock<BTreeMap<String, Arc<Voidable>>> = RwLock::new(BTreeMap::new());

/// Adds `voidable` to the process-wide registry; registering the same
/// subject type again replaces the entry.
pub fn register(voidable: Voidable) {
    REGISTRY
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(voidable.subject_type.clone(), Arc::new(voidable));
}

/// The entry of `subject_type`, if registered.
pub fn get(subject_type: &str) -> Option<Arc<Voidable>> {
    REGISTRY
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(subject_type)
        .cloned()
}

/// The registered voidable subject types, sorted.
pub fn subject_types() -> Vec<String> {
    REGISTRY
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .keys()
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use renox::prelude::*;
    use renox::state_machine::{Machine, Transition};
    use serde::{Deserialize, Serialize};

    use super::*;

    fn shape<'a>(
        states: &[(&'a str, bool)],
        transitions: &'a [(&'a str, Vec<String>, &'a str, bool)],
    ) -> Vec<String> {
        Shape {
            states: states.to_vec(),
            transitions: transitions
                .iter()
                .map(|(n, f, t, s)| (*n, f.as_slice(), *t, *s))
                .collect(),
        }
        .problems()
    }

    fn from(states: &[&str]) -> Vec<String> {
        states.iter().map(|s| (*s).to_owned()).collect()
    }

    fn good() -> Vec<(&'static str, Vec<String>, &'static str, bool)> {
        vec![
            ("void", from(&["issued"]), "voided", false),
            ("void_approved", from(&["issued"]), "voided", true),
        ]
    }

    #[test]
    fn a_machine_that_follows_the_rules_has_no_problem() {
        assert!(shape(&[("issued", false), ("voided", true)], &good()).is_empty());
    }

    #[test]
    fn a_missing_voided_state_is_a_problem() {
        let problems = shape(&[("issued", false)], &good());
        assert_eq!(problems, vec!["it has no `voided` state"]);
    }

    #[test]
    fn voided_must_be_terminal_and_have_no_way_out() {
        let problems = shape(&[("issued", false), ("voided", false)], &good());
        assert_eq!(problems, vec!["`voided` is not terminal"]);
        let mut transitions = good();
        transitions.push(("reopen", from(&["voided"]), "issued", false));
        let problems = shape(&[("issued", false), ("voided", true)], &transitions);
        assert_eq!(problems, vec!["a transition leaves `voided`"]);
    }

    #[test]
    fn void_and_void_approved_must_exist_and_lead_to_voided() {
        let states = [("issued", false), ("voided", true), ("open", false)];
        let transitions = vec![("void", from(&["issued"]), "open", false)];
        let problems = shape(&states, &transitions);
        assert_eq!(
            problems,
            vec![
                "`void` leads to `open`, not `voided`",
                "it has no `void_approved` transition"
            ]
        );
        let problems = shape(&states, &[]);
        assert_eq!(
            problems,
            vec![
                "it has no `void` transition",
                "it has no `void_approved` transition"
            ]
        );
    }

    #[test]
    fn void_approved_must_be_system_and_void_must_not() {
        let transitions = vec![
            ("void", from(&["issued"]), "voided", true),
            ("void_approved", from(&["issued"]), "voided", false),
        ];
        let problems = shape(&[("issued", false), ("voided", true)], &transitions);
        assert_eq!(
            problems,
            vec![
                "`void` must be run by a user, not `.system()`",
                "`void_approved` must be `.system()`"
            ]
        );
    }

    #[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    enum Status {
        #[default]
        Issued,
        Voided,
    }

    /// A voidable machine that follows the rules.
    #[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
    #[model(table = "test_void_good", state_machine)]
    struct Good {
        id: i64,
        status: Status,
    }

    impl StateMachine for Good {
        type State = Status;
        const COLUMN: renox::db::Col<Good, Status> = Good::STATUS;

        fn machine() -> Machine<Self> {
            Machine::new()
                .initial([Status::Issued])
                .state(Status::Issued)
                .state(Status::Voided)
                .tone("neutral")
                .terminal()
                .transition(
                    Transition::new("void")
                        .from([Status::Issued])
                        .to(Status::Voided)
                        .reason_required(),
                )
                .transition(
                    Transition::new("void_approved")
                        .from([Status::Issued])
                        .to(Status::Voided)
                        .system(),
                )
        }
    }

    /// `voided` is not terminal here.
    #[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
    #[model(table = "test_void_open", state_machine)]
    struct NotTerminal {
        id: i64,
        status: Status,
    }

    impl StateMachine for NotTerminal {
        type State = Status;
        const COLUMN: renox::db::Col<NotTerminal, Status> = NotTerminal::STATUS;

        fn machine() -> Machine<Self> {
            Machine::new()
                .initial([Status::Issued])
                .state(Status::Issued)
                .state(Status::Voided)
                .transition(
                    Transition::new("void")
                        .from([Status::Issued])
                        .to(Status::Voided),
                )
                .transition(
                    Transition::new("void_approved")
                        .from([Status::Issued])
                        .to(Status::Voided)
                        .system(),
                )
        }
    }

    /// `void_approved` can be run by a user here.
    #[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
    #[model(table = "test_void_user", state_machine)]
    struct UserApproved {
        id: i64,
        status: Status,
    }

    impl StateMachine for UserApproved {
        type State = Status;
        const COLUMN: renox::db::Col<UserApproved, Status> = UserApproved::STATUS;

        fn machine() -> Machine<Self> {
            Machine::new()
                .initial([Status::Issued])
                .state(Status::Issued)
                .state(Status::Voided)
                .terminal()
                .transition(
                    Transition::new("void")
                        .from([Status::Issued])
                        .to(Status::Voided),
                )
                .transition(
                    Transition::new("void_approved")
                        .from([Status::Issued])
                        .to(Status::Voided),
                )
        }
    }

    #[test]
    fn a_real_machine_is_checked_from_its_declaration() {
        assert!(check_machine(&info_of::<Good>()).is_ok());
        assert_eq!(
            check_machine(&info_of::<NotTerminal>()).unwrap_err(),
            vec!["`voided` is not terminal"]
        );
    }

    #[test]
    #[should_panic(expected = "`voided` is not terminal")]
    fn registering_a_machine_whose_voided_is_not_terminal_panics() {
        let _ = Voidable::of::<NotTerminal>("tests.not_terminal");
    }

    #[test]
    #[should_panic(expected = "`void_approved` must be `.system()`")]
    fn registering_a_void_approved_that_is_not_system_panics() {
        let _ = Voidable::of::<UserApproved>("tests.user_approved");
    }

    #[test]
    fn the_registry_finds_an_entry_by_subject_type_and_replaces_it() {
        Voidable::of::<Good>("tests.void_good").register();
        let entry = get("tests.void_good").expect("registered");
        assert_eq!(entry.permission(), RECORDS_VOID);
        assert_eq!(entry.table(), "test_void_good");
        assert_eq!(entry.series_key(), None);
        assert!(!entry.has_scope());
        assert!(entry.kinds().is_empty());

        Voidable::of::<Good>("tests.void_good")
            .permission("receipts.void")
            .number("receipt", |_| None)
            .scope(|_| Scope::global())
            .reverse(
                EffectKind::Task,
                Reverse::new(
                    |_tx, _id| Box::pin(async { Ok(Vec::new()) }),
                    |_tx, _id| Box::pin(async { Ok(Vec::new()) }),
                ),
            )
            .register();
        let entry = get("tests.void_good").expect("registered");
        assert_eq!(entry.permission(), "receipts.void");
        assert_eq!(entry.series_key(), Some("receipt"));
        assert!(entry.has_scope());
        assert_eq!(entry.kinds(), vec![EffectKind::Task]);
        assert!(subject_types().contains(&"tests.void_good".to_owned()));
        assert!(get("tests.unknown").is_none());
    }

    #[test]
    #[should_panic(expected = "already a reverse handler")]
    fn a_kind_of_effect_has_one_handler() {
        let handler = Reverse::new(
            |_tx, _id| Box::pin(async { Ok(Vec::new()) }),
            |_tx, _id| Box::pin(async { Ok(Vec::new()) }),
        );
        let _ = Voidable::of::<Good>("tests.void_twice")
            .reverse(EffectKind::Posting, handler.clone())
            .reverse(EffectKind::Posting, handler);
    }
}
