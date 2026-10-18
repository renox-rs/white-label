//! The registry of subjects' state machines by name (wl-kernel §4.1,
//! WORKFLOW §3.4).
//!
//! The Renox 1.2 engine runs a transition on a typed record and has no run by
//! name. The approval kernel (and later automation) picks the transition at
//! run time, from data: `back_to_<state>` after a request ends without a yes,
//! `<action>_approved` after the last approval. So each module declares its
//! machines here once, in its `register()`:
//!
//! ```ignore
//! SubjectMachine::of::<Refund>("payments.refund")
//!     .waiting(RefundStatus::PendingApproval)
//!     .scope(|refund: &Refund| Scope::of_id::<Building>(refund.building_id))
//!     .release(|tx, hold| Box::pin(async move { release_hold(tx, hold.request_id).await }))
//!     .register();
//! ```
//!
//! and the kernel finds the entry by subject type ([`get`]) or by table
//! ([`by_table`]) and runs a transition by name, as the system, inside the
//! caller's transaction ([`SubjectMachine::run_in`]).
//!
//! Transitions named `back_to_*` or `*_approved` must be `.system()`: they
//! are never offered to a user. Declaring one that is not panics at boot.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError, RwLock};

use renox::auth::permissions::Scope;
use renox::db::{Executor, Transaction, sql};
use renox::serde_json::Value;
use renox::state_machine::{Outcome, StateMachine, TransitionError, TransitionInfo, info_of};
use renox::{AppState, Result};
use serde::Serialize;

use crate::block_guard::ActionKind;
use crate::parties::PartyId;
use crate::subject::Subject;

/// A boxed future, as the handlers return.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The prefix of the transitions back to a stored state.
pub const BACK_TO_PREFIX: &str = "back_to_";
/// The suffix of the transitions the kernel runs once an action is approved.
pub const APPROVED_SUFFIX: &str = "_approved";

/// The name of the transition back to `state`: `back_to_draft`.
pub fn back_to(state: impl fmt::Display) -> String {
    format!("{BACK_TO_PREFIX}{state}")
}

/// The name of the transition the kernel runs when `action` is approved:
/// `refund_approved`.
pub fn approved(action: &str) -> String {
    format!("{action}{APPROVED_SUFFIX}")
}

/// The default action key of `transition` on subjects of `subject_type`:
/// `<module>.<transition>`.
fn default_action_key(subject_type: &str, transition: &str) -> String {
    let module = subject_type.split('.').next().unwrap_or_default();
    format!("{module}.{transition}")
}

/// Whether only the system may run a transition of this name, by convention.
pub fn is_system_name(name: &str) -> bool {
    name.starts_with(BACK_TO_PREFIX) || name.ends_with(APPROVED_SUFFIX)
}

/// The transitions (name, is it `system`) that break the naming rule: a
/// `back_to_*` or `*_approved` one a user could run.
fn not_system<'a>(transitions: impl IntoIterator<Item = (&'a str, bool)>) -> Vec<&'a str> {
    transitions
        .into_iter()
        .filter(|(name, system)| is_system_name(name) && !system)
        .map(|(name, _)| name)
        .collect()
}

/// What a reserve or release handler is told: the approval request it runs
/// for (its id is the idempotency key) and the subject.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct Hold {
    /// The approval request's id.
    pub request_id: i64,
    /// The record the request is about.
    pub subject: Subject,
    /// The action key, e.g. `payments.refund`.
    pub action: String,
    /// The transition's input at submit (the payload snapshot).
    pub payload: Value,
}

impl Hold {
    /// The hold of request `request_id` on `subject` for `action`.
    pub fn new(
        request_id: i64,
        subject: Subject,
        action: impl Into<String>,
        payload: Value,
    ) -> Self {
        Hold {
            request_id,
            subject,
            action: action.into(),
            payload,
        }
    }
}

/// A reserve or release handler: runs in the kernel's transaction and must be
/// idempotent, keyed by [`Hold::request_id`].
type HandlerFn =
    Arc<dyn for<'a> Fn(&'a mut Transaction, &'a Hold) -> BoxFuture<'a, Result> + Send + Sync>;
type ScopeFn =
    Arc<dyn for<'a> Fn(&'a mut Transaction, i64) -> BoxFuture<'a, Result<Scope>> + Send + Sync>;
type RunFn = Arc<
    dyn for<'a> Fn(
            &'a mut Transaction,
            i64,
            &'a str,
            Run,
        ) -> BoxFuture<'a, std::result::Result<Ran, TransitionError>>
        + Send
        + Sync,
>;
/// The scope of a loaded record.
type RecordScopeFn<M> = Arc<dyn Fn(&M) -> Scope + Send + Sync>;
/// The party a guarded transition acts on, read from the stored record.
type PartyFn = Arc<
    dyn for<'a> Fn(&'a mut Transaction, i64) -> BoxFuture<'a, Result<Option<PartyId>>>
        + Send
        + Sync,
>;
type EmitFn = Box<dyn for<'s> FnOnce(&'s AppState) -> BoxFuture<'s, Result> + Send>;

/// What a run by name carries: the history reason, the input for guards and
/// effects, and the history row's metadata (who acted, the request…).
#[derive(Debug, Clone, Default)]
pub struct Run {
    reason: Option<String>,
    input: Value,
    meta: Value,
}

impl Run {
    /// A run with no reason, input or metadata.
    pub fn new() -> Self {
        Run::default()
    }

    /// The history reason; a blank one counts as none.
    pub fn reason(mut self, reason: impl Into<String>) -> Self {
        let reason = reason.into();
        self.reason = (!reason.trim().is_empty()).then_some(reason);
        self
    }

    /// The input guards and effects read.
    pub fn input(mut self, input: Value) -> Self {
        self.input = input;
        self
    }

    /// The history row's metadata.
    pub fn meta(mut self, meta: Value) -> Self {
        self.meta = meta;
        self
    }
}

/// A transition run by name: the status moved (or an interceptor deferred
/// it). Emit it with [`Ran::emit`] once the transaction has committed.
#[must_use = "emit it after the transaction commits"]
pub struct Ran {
    /// The transition's name.
    pub transition: String,
    /// The state the record was in.
    pub from: String,
    /// The state it entered (or would have, when deferred).
    pub to: String,
    /// What the interceptor returned when it deferred the run; `None` when
    /// the status moved.
    pub deferred: Option<Value>,
    emit: Option<EmitFn>,
}

impl fmt::Debug for Ran {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ran")
            .field("transition", &self.transition)
            .field("from", &self.from)
            .field("to", &self.to)
            .field("deferred", &self.deferred)
            .finish()
    }
}

impl Ran {
    /// Whether the status moved.
    pub fn moved(&self) -> bool {
        self.deferred.is_none()
    }

    /// Emits the engine's `Transitioned` event to the app's listeners, if
    /// the status moved. Call it after the commit.
    pub async fn emit(self, state: &AppState) -> Result {
        match self.emit {
            Some(emit) => emit(state).await,
            None => Ok(()),
        }
    }
}

fn ran<M: StateMachine>(outcome: Outcome<M>) -> Ran {
    match outcome {
        Outcome::Done(done) => Ran {
            transition: done.transition.clone(),
            from: done.from.clone(),
            to: done.to.clone(),
            deferred: None,
            emit: Some(Box::new(move |state: &AppState| -> BoxFuture<'_, Result> {
                Box::pin(done.emit(state))
            })),
        },
        Outcome::Pending(pending) => Ran {
            transition: pending.transition,
            from: pending.from,
            to: pending.to,
            deferred: Some(pending.data),
            emit: None,
        },
    }
}

/// A transition that must pass the block guard: what it does and whose
/// party it acts on.
#[derive(Clone)]
struct Guarded {
    kind: ActionKind,
    party: PartyFn,
}

/// A subject's state machine without its model type: what the kernel needs
/// to run its transitions by name. Built with [`SubjectMachine::of`].
pub struct SubjectMachine {
    subject_type: String,
    table: &'static str,
    column: &'static str,
    waiting: Option<String>,
    actions: BTreeMap<String, String>,
    transitions: Vec<TransitionInfo>,
    has_scope: bool,
    scope: ScopeFn,
    reserve: Option<HandlerFn>,
    release: Option<HandlerFn>,
    guarded: BTreeMap<String, Guarded>,
    run: RunFn,
}

impl fmt::Debug for SubjectMachine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SubjectMachine")
            .field("subject_type", &self.subject_type)
            .field("table", &self.table)
            .field("waiting", &self.waiting)
            .field("reserve", &self.reserve.is_some())
            .field("release", &self.release.is_some())
            .finish()
    }
}

impl SubjectMachine {
    /// Starts the entry of `M`, the model of subjects of type `subject_type`
    /// (`<module>.<entity>`).
    ///
    /// # Panics
    ///
    /// When `subject_type` is not `<module>.<entity>`, or when a transition
    /// named `back_to_*` or `*_approved` is not `.system()`: both are
    /// programming errors, found at boot.
    pub fn of<M>(subject_type: &str) -> SubjectMachineBuilder<M>
    where
        M: StateMachine<Key = i64>,
    {
        // Checks the type's form; panics like `Subject::new` on a bad one.
        let _ = Subject::new(subject_type, 0);
        let info = info_of::<M>();
        let wrong = not_system(info.transitions.iter().map(|t| (t.name.as_str(), t.system)));
        assert!(
            wrong.is_empty(),
            "{}: the transitions {} must be `.system()`: only the kernel runs `back_to_*` and `*_approved`",
            info.model,
            wrong.join(", ")
        );
        SubjectMachineBuilder {
            subject_type: subject_type.to_owned(),
            transitions: info.transitions,
            states: info.states.iter().map(|s| s.name.clone()).collect(),
            waiting: None,
            actions: BTreeMap::new(),
            scope: None,
            reserve: None,
            release: None,
            guarded: BTreeMap::new(),
            _model: std::marker::PhantomData,
        }
    }

    /// The subject type, e.g. `payments.refund`.
    pub fn subject_type(&self) -> &str {
        &self.subject_type
    }

    /// The model's table.
    pub fn table(&self) -> &'static str {
        self.table
    }

    /// The state the module shows while a request is pending, if it has one.
    pub fn waiting_state(&self) -> Option<&str> {
        self.waiting.as_deref()
    }

    /// The approval policies' action key of the transition `transition`:
    /// the one given with [`SubjectMachineBuilder::action`], else
    /// `<module>.<transition>` (`deposits.refund` for the transition `refund`
    /// of `deposits.deposit`).
    pub fn action_key(&self, transition: &str) -> String {
        self.actions
            .get(transition)
            .cloned()
            .unwrap_or_else(|| default_action_key(&self.subject_type, transition))
    }

    /// The block-guard kind of `transition`, if it was declared with
    /// [`SubjectMachineBuilder::block_guarded`].
    pub fn guarded_kind(&self, transition: &str) -> Option<ActionKind> {
        self.guarded.get(transition).map(|g| g.kind)
    }

    /// The party record `id` acts on in the guarded transition `transition`,
    /// read in `tx`. `None` when the transition is not guarded, the record is
    /// missing, or the record has no party.
    pub async fn guarded_party_in(
        &self,
        tx: &mut Transaction,
        id: i64,
        transition: &str,
    ) -> Result<Option<PartyId>> {
        match self.guarded.get(transition) {
            Some(guarded) => (guarded.party)(tx, id).await,
            None => Ok(None),
        }
    }

    /// The transition called `name`, if the machine has one.
    pub fn transition(&self, name: &str) -> Option<&TransitionInfo> {
        self.transitions.iter().find(|t| t.name == name)
    }

    /// The machine's transitions.
    pub fn transitions(&self) -> &[TransitionInfo] {
        &self.transitions
    }

    /// Whether the entry gives a scope; without one, approvers are found
    /// globally.
    pub fn has_scope(&self) -> bool {
        self.has_scope
    }

    /// Whether the module reserves something at submit.
    pub fn has_reserve(&self) -> bool {
        self.reserve.is_some()
    }

    /// Whether the module releases something on an end that is not a yes.
    pub fn has_release(&self) -> bool {
        self.release.is_some()
    }

    /// The state record `id` is in now, or `None` when there is no such
    /// record.
    pub async fn current_state<'c>(
        &self,
        db: impl Executor<'c>,
        id: i64,
    ) -> Result<Option<String>> {
        let state = sql(format!(
            "SELECT {} FROM {} WHERE id = ?",
            self.column, self.table
        ))
        .bind(id)
        .scalar_optional::<String>(db)
        .await?;
        Ok(state)
    }

    /// Whether record `id` is in the waiting state; always `false` for a
    /// machine without one.
    pub async fn is_waiting(&self, tx: &mut Transaction, id: i64) -> Result<bool> {
        let Some(waiting) = &self.waiting else {
            return Ok(false);
        };
        Ok(self.current_state(&mut *tx, id).await?.as_deref() == Some(waiting.as_str()))
    }

    /// The scope of record `id` (the global scope when the entry gives none).
    pub async fn scope_in(&self, tx: &mut Transaction, id: i64) -> Result<Scope> {
        (self.scope)(tx, id).await
    }

    /// Runs the module's reserve handler, if any, in `tx`.
    pub async fn reserve_in(&self, tx: &mut Transaction, hold: &Hold) -> Result {
        match &self.reserve {
            Some(reserve) => reserve(tx, hold).await,
            None => Ok(()),
        }
    }

    /// Runs the module's release handler, if any, in `tx`.
    pub async fn release_in(&self, tx: &mut Transaction, hold: &Hold) -> Result {
        match &self.release {
            Some(release) => release(tx, hold).await,
            None => Ok(()),
        }
    }

    /// Runs the transition `name` on record `id` as the system, inside `tx`,
    /// without committing. The record is loaded in `tx` (locked on
    /// PostgreSQL). A missing record fails with `Failed(NotFound)`; the
    /// engine's own refusals (`Unknown`, `NotFrom`, `Guard`, `Conflict`,
    /// `ReasonRequired`, `Forbidden` for a user transition) come back as is.
    pub async fn run_in(
        &self,
        tx: &mut Transaction,
        id: i64,
        name: &str,
        run: Run,
    ) -> std::result::Result<Ran, TransitionError> {
        (self.run)(tx, id, name, run).await
    }

    /// Runs `back_to_<state>` on record `id` (see [`SubjectMachine::run_in`]).
    pub async fn run_back_to(
        &self,
        tx: &mut Transaction,
        id: i64,
        state: &str,
        run: Run,
    ) -> std::result::Result<Ran, TransitionError> {
        self.run_in(tx, id, &back_to(state), run).await
    }

    /// Adds this entry to the process-wide registry, replacing one of the
    /// same subject type.
    pub fn register(self) {
        register(self);
    }
}

/// The entry being declared, from [`SubjectMachine::of`]; end with
/// [`SubjectMachineBuilder::register`] (or [`SubjectMachineBuilder::build`]).
#[must_use = "a machine is only found once registered"]
pub struct SubjectMachineBuilder<M: StateMachine<Key = i64>> {
    subject_type: String,
    transitions: Vec<TransitionInfo>,
    states: Vec<String>,
    waiting: Option<String>,
    actions: BTreeMap<String, String>,
    scope: Option<RecordScopeFn<M>>,
    reserve: Option<HandlerFn>,
    release: Option<HandlerFn>,
    guarded: BTreeMap<String, Guarded>,
    _model: std::marker::PhantomData<fn() -> M>,
}

impl<M: StateMachine<Key = i64>> SubjectMachineBuilder<M> {
    /// The state the module shows while a request is pending, e.g.
    /// `pending_approval`.
    ///
    /// # Panics
    ///
    /// When the machine has no such state.
    pub fn waiting(mut self, state: M::State) -> Self {
        let state = state.to_string();
        assert!(
            self.states.contains(&state),
            "{}: the waiting state `{state}` is not a state of the machine",
            self.subject_type
        );
        self.waiting = Some(state);
        self
    }

    /// The approval policies' action key of the transition `transition`,
    /// instead of `<module>.<transition>`.
    ///
    /// # Panics
    ///
    /// When the machine has no such transition.
    pub fn action(mut self, transition: &str, key: impl Into<String>) -> Self {
        assert!(
            self.transitions.iter().any(|t| t.name == transition),
            "{}: `{transition}` is not a transition of the machine",
            self.subject_type
        );
        self.actions.insert(transition.to_owned(), key.into());
        self
    }

    /// The scope of a record (its building, project…), where approvers are
    /// looked for.
    pub fn scope(mut self, scope: impl Fn(&M) -> Scope + Send + Sync + 'static) -> Self {
        self.scope = Some(Arc::new(scope));
        self
    }

    /// What the module reserves at submit (a stock hold, a slot), in the
    /// transaction that creates the request. Must be idempotent, keyed by
    /// [`Hold::request_id`].
    pub fn reserve<F>(mut self, reserve: F) -> Self
    where
        F: for<'a> Fn(&'a mut Transaction, &'a Hold) -> BoxFuture<'a, Result>
            + Send
            + Sync
            + 'static,
    {
        self.reserve = Some(Arc::new(reserve));
        self
    }

    /// What undoes the reservation on every end that is not a yes. Must be
    /// idempotent, keyed by [`Hold::request_id`].
    pub fn release<F>(mut self, release: F) -> Self
    where
        F: for<'a> Fn(&'a mut Transaction, &'a Hold) -> BoxFuture<'a, Result>
            + Send
            + Sync
            + 'static,
    {
        self.release = Some(Arc::new(release));
        self
    }

    /// Declares that `transition` is a guarded action of kind `kind`
    /// (see [`crate::block_guard`]): [`crate::block_guard::run_guarded`]
    /// refuses it when the party `party_of` returns for the record is
    /// blocked, an exception can be requested for it, and the guard is
    /// checked again at the final approval of a request for it. A record
    /// without a party (`None`) is not guarded.
    ///
    /// Registering a machine with a guarded transition also registers the
    /// subject type's final re-check hook
    /// ([`crate::approvals::register_final_check`]), replacing one the module
    /// registered for the same type.
    ///
    /// # Panics
    ///
    /// When the machine has no such transition, when it is a system one, or
    /// when `kind` is [`ActionKind::MoneyIn`], which is never refused.
    pub fn block_guarded(
        mut self,
        transition: &str,
        kind: ActionKind,
        party_of: impl Fn(&M) -> Option<PartyId> + Send + Sync + 'static,
    ) -> Self {
        let info = self.transitions.iter().find(|t| t.name == transition);
        assert!(
            info.is_some_and(|t| !t.system),
            "{}: `{transition}` is not a user transition of the machine",
            self.subject_type
        );
        assert!(
            kind.is_guarded(),
            "{}: `{transition}` cannot be guarded as `{kind}`: money in is never refused",
            self.subject_type
        );
        let party_of = Arc::new(party_of);
        let party: PartyFn = Arc::new(move |tx: &mut Transaction, id: i64| {
            let party_of = Arc::clone(&party_of);
            Box::pin(async move {
                let record = M::query()
                    .where_eq("id", id)
                    .first(&mut *tx)
                    .await?
                    .ok_or(renox::Error::NotFound)?;
                Ok(party_of(&record))
            })
        });
        self.guarded
            .insert(transition.to_owned(), Guarded { kind, party });
        self
    }

    /// The entry, without registering it.
    pub fn build(self) -> SubjectMachine {
        let has_scope = self.scope.is_some();
        let scope = self.scope;
        let scope: ScopeFn = Arc::new(move |tx: &mut Transaction, id: i64| {
            let scope = scope.clone();
            Box::pin(async move {
                let Some(scope) = scope else {
                    return Ok(Scope::global());
                };
                let record = load::<M>(tx, id).await?;
                Ok(scope(&record))
            })
        });
        let run: RunFn = Arc::new(|tx: &mut Transaction, id: i64, name: &str, run: Run| {
            Box::pin(async move {
                let record = load::<M>(tx, id).await.map_err(TransitionError::Failed)?;
                let mut builder = record
                    .transition(name)
                    .by_system()
                    .input(&run.input)
                    .meta(run.meta);
                if let Some(reason) = run.reason {
                    builder = builder.reason(reason);
                }
                builder.run_in(tx).await.map(ran::<M>)
            })
        });
        SubjectMachine {
            subject_type: self.subject_type,
            table: M::TABLE,
            column: M::COLUMN.name(),
            waiting: self.waiting,
            actions: self.actions,
            transitions: self.transitions,
            has_scope,
            scope,
            reserve: self.reserve,
            release: self.release,
            guarded: self.guarded,
            run,
        }
    }

    /// Builds the entry and adds it to the process-wide registry, replacing
    /// one of the same subject type.
    pub fn register(self) {
        self.build().register();
    }
}

/// Record `id` of `M`, loaded in `tx` and locked (PostgreSQL) until it ends.
async fn load<M: StateMachine<Key = i64>>(tx: &mut Transaction, id: i64) -> Result<M> {
    M::query()
        .where_eq("id", id)
        .lock_for_update()
        .first(&mut *tx)
        .await?
        .ok_or(renox::Error::NotFound)
}

static REGISTRY: RwLock<BTreeMap<String, Arc<SubjectMachine>>> = RwLock::new(BTreeMap::new());

/// Adds `machine` to the process-wide registry; registering the same subject
/// type again replaces the entry.
pub fn register(machine: SubjectMachine) {
    if !machine.guarded.is_empty() {
        crate::approvals::register_final_check(machine.subject_type.clone(), |tx, request| {
            Box::pin(crate::block_guard::recheck(tx, request))
        });
    }
    REGISTRY
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(machine.subject_type.clone(), Arc::new(machine));
}

/// The entry of `subject_type`, if registered.
pub fn get(subject_type: &str) -> Option<Arc<SubjectMachine>> {
    REGISTRY
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(subject_type)
        .cloned()
}

/// The entry whose model's table is `table` (what an interceptor sees), if
/// registered.
pub fn by_table(table: &str) -> Option<Arc<SubjectMachine>> {
    REGISTRY
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .values()
        .find(|m| m.table == table)
        .cloned()
}

/// The registered subject types, sorted.
pub fn subject_types() -> Vec<String> {
    REGISTRY
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .keys()
        .cloned()
        .collect()
}

/// The routes `<rx-transition-actions>` posts to, like Renox's
/// [`renox::state_machine::routes`]: `POST /{id}/transitions/{name}` runs the
/// transition `name` on record `id` of `M` as the logged-in user, with the
/// form's optional `reason`, then goes back to the page the form was on.
/// After the commit it announces the approval request a run created
/// ([`crate::approvals::announce`]) and says "Sent for approval". Mount it
/// under the model's path:
///
/// ```ignore
/// Routes::new().group("/refunds", "refunds.", wl_kernel::machines::routes::<Refund>())
/// ```
///
/// Refusals answer like [`TransitionError::into_error`].
pub fn routes<M>() -> renox::Routes
where
    M: StateMachine<Key = i64> + Send + Sync + 'static,
{
    renox::Routes::new().post("/{id}/transitions/{name}", run_transition::<M>)
}

#[derive(serde::Deserialize)]
struct ReasonForm {
    reason: Option<String>,
}

async fn run_transition<M>(
    renox::prelude::State(state): renox::prelude::State<AppState>,
    user: renox::AuthUser,
    renox::prelude::Path((id, name)): renox::prelude::Path<(i64, String)>,
    lang: renox::Lang,
    back: renox::Back,
    renox::prelude::Form(form): renox::prelude::Form<ReasonForm>,
) -> Result<renox::prelude::Response>
where
    M: StateMachine<Key = i64> + Send + Sync + 'static,
{
    use renox::prelude::IntoResponse;

    let record = M::find(&state.db, id)
        .await?
        .ok_or(renox::Error::NotFound)?;
    let mut run = record.transition(&name).by(&user);
    let reason = form.reason.unwrap_or_default();
    let reason = reason.trim();
    if !reason.is_empty() {
        run = run.reason(reason);
    }
    // `run` commits and emits `Transitioned`; the request it created (if
    // any) is announced only now, after the commit.
    let outcome = run.run(&state).await.map_err(TransitionError::into_error)?;
    crate::approvals::announce(&state, &outcome).await?;
    let sent = outcome.is_pending()
        || matches!(&outcome, Outcome::Done(done)
            if by_table(M::TABLE).is_some_and(|m| m.waiting_state() == Some(done.to.as_str())));
    if sent {
        let toast = renox::Toast::info(crate::texts::t(&lang, "wl_kernel.approvals.sent"));
        return Ok((toast, back).into_response());
    }
    Ok(back.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_action_key_is_the_module_and_the_transition() {
        assert_eq!(
            default_action_key("deposits.deposit", "refund"),
            "deposits.refund"
        );
        assert_eq!(
            default_action_key("payments.refund", "refund"),
            "payments.refund"
        );
    }

    #[test]
    fn names_follow_the_convention() {
        assert_eq!(back_to("draft"), "back_to_draft");
        assert_eq!(approved("refund"), "refund_approved");
        assert!(is_system_name("back_to_requested"));
        assert!(is_system_name("refund_approved"));
        assert!(!is_system_name("approve"));
        assert!(!is_system_name("submit"));
    }

    #[test]
    fn back_and_approved_transitions_must_be_system() {
        let transitions = [
            ("submit", false),
            ("back_to_draft", true),
            ("back_to_active", false),
            ("refund_approved", false),
            ("cancel_approved", true),
        ];
        assert_eq!(
            not_system(transitions),
            vec!["back_to_active", "refund_approved"]
        );
        assert!(not_system([("submit", false), ("back_to_draft", true)]).is_empty());
    }

    #[test]
    fn a_blank_reason_counts_as_none() {
        assert_eq!(Run::new().reason("  ").reason, None);
        assert_eq!(
            Run::new().reason("duplicate").reason.as_deref(),
            Some("duplicate")
        );
    }
}
