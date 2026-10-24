//! Agent actions (BL-AGT-08, decision 0006 point 5, `PLT.AGT.02`): the only
//! writes an assistant can make, each a named action a module registers.
//!
//! A module registers a state machine transition as an action:
//!
//! ```ignore
//! AgentAction::transition::<Invoice>("invoicing.invoice", "issue")
//!     .label("Issue the invoice")
//!     .input(|state, invoice, _args| Box::pin(async move { gather(&state, &invoice).await }))
//!     .describe(|_state, invoice, _args| Box::pin(async move { Ok(Change::new("…")) }))
//!     .register();
//! ```
//!
//! or a create or a message with [`AgentAction::custom`], giving its own
//! preview and confirm.
//!
//! **Two steps.** [`preview`] writes nothing: it runs the same checks as the
//! confirm (the user's permission, the record's state, the guards, the
//! approval policy) and answers the change in words, the amounts, whether an
//! approval will be requested, and a **confirmation token** valid
//! [`TOKEN_MINUTES`] minutes. The token is sealed with `APP_KEY` (the action,
//! the record, the arguments, the user, the assistant and a fingerprint of
//! the record), so nothing is stored at preview. [`confirm`] opens it and
//! runs the action **once**: the first confirm claims the token's nonce in
//! `wl_agent_runs` and keeps its result, and a second confirm with the same
//! token returns that result without running anything.
//!
//! **Edge cases decided.** The record moved between preview and confirm (its
//! fingerprint differs): confirm is refused and the assistant is told to
//! preview again. An action over the user's limit: the approval interceptor
//! defers the transition, confirm answers [`Outcome::ApprovalRequested`] and
//! the request is announced to its approvers as on the screen.
//!
//! **Must not.** `approve`, `approve_step`, `reject` and `void` are never
//! agent actions, nor a system transition (`back_to_*`, `*_approved`):
//! registering one panics at boot.

use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::{Arc, PoisonError, RwLock};

use renox::db::{DateTime, Json as DbJson, now, sql};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{
    self, Outcome as MachineOutcome, Preview as MachinePreview, StateMachine, TransitionCtx,
    TransitionError, TransitionRequest,
};
use serde::{Deserialize, Serialize};

use super::access::{self, Use};
use super::{AgentCall, Money};
use crate::machines::{self, BoxFuture};
use crate::subject::Subject;

/// How long a confirmation token is valid.
pub const TOKEN_MINUTES: i64 = 10;

/// Names that are never agent actions: deciding an approval stays with a
/// person (decision 0006 point 4), and voids are not offered.
pub const NEVER_ACTIONS: &[&str] = &["approve", "approve_step", "reject", "void"];

/// The refusal when the record changed after the preview.
pub const MOVED: &str =
    "The record changed since the preview: preview the action again before confirming.";

/// The refusal for an expired token.
pub const EXPIRED: &str = "The confirmation expired: preview the action again.";

/// The refusal for a token that does not open.
pub const BAD_TOKEN: &str = "The confirmation token is not valid: preview the action again.";

/// What kind of write an action makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    /// A state machine transition of an existing record.
    Transition,
    /// A new record (a draft).
    Create,
    /// A message from a template (wl-notify).
    Message,
}

/// One line of a change: `Total` → `AED 1,050.00`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChangeLine {
    /// What it is.
    pub label: String,
    /// Its value, in words.
    pub value: String,
}

/// An amount of a change, in minor units with its currency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Amount {
    /// What it is, e.g. `VAT`.
    pub label: String,
    /// The amount.
    #[serde(flatten)]
    pub money: Money,
}

/// What an action will do, in words a person reads before confirming.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Change {
    /// One sentence: "Issue invoice draft #42 to Al Ali Trading LLC".
    pub summary: String,
    /// The details.
    pub lines: Vec<ChangeLine>,
    /// The amounts.
    pub amounts: Vec<Amount>,
    /// What the user should know (a consent missing, a limit…).
    pub warnings: Vec<String>,
}

impl Change {
    /// A change summed up in `summary`.
    pub fn new(summary: impl Into<String>) -> Self {
        Change {
            summary: summary.into(),
            ..Default::default()
        }
    }

    /// Adds a line.
    pub fn line(mut self, label: impl Into<String>, value: impl Display) -> Self {
        self.lines.push(ChangeLine {
            label: label.into(),
            value: value.to_string(),
        });
        self
    }

    /// Adds an amount of `minor` units of `currency`.
    pub fn amount(mut self, label: impl Into<String>, minor: i64, currency: &str) -> Self {
        self.amounts.push(Amount {
            label: label.into(),
            money: Money::new(minor, currency),
        });
        self
    }

    /// Adds a warning.
    pub fn warning(mut self, text: impl Into<String>) -> Self {
        self.warnings.push(text.into());
        self
    }

    /// The value of the line labelled `label`, if any.
    pub fn value_of(&self, label: &str) -> Option<&str> {
        self.lines
            .iter()
            .find(|l| l.label == label)
            .map(|l| l.value.as_str())
    }
}

/// What an action's preview found: the change, whether an approval will be
/// requested, and a fingerprint of the record the confirm compares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    /// The change in words.
    pub change: Change,
    /// Whether the confirm will submit an approval request instead.
    pub requires_approval: bool,
    /// Any text that changes when the record changes ("" for a create).
    pub fingerprint: String,
}

impl Planned {
    /// A plan with no approval and no record fingerprint.
    pub fn new(change: Change) -> Self {
        Planned {
            change,
            requires_approval: false,
            fingerprint: String::new(),
        }
    }

    /// The fingerprint of the record the action reads.
    pub fn fingerprint(mut self, fingerprint: impl Into<String>) -> Self {
        self.fingerprint = fingerprint.into();
        self
    }
}

/// The answer of [`preview`], as the tool returns it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Preview {
    /// The action key, e.g. `invoicing.invoice.issue`.
    pub action: String,
    /// The record, e.g. `invoicing.invoice:42` (none for a create).
    pub record: Option<String>,
    /// What will change.
    pub change: Change,
    /// Whether confirming submits an approval request instead of acting.
    pub requires_approval: bool,
    /// Pass this to `confirm_action` to do it.
    pub token: String,
    /// Until when the token is valid (Asia/Dubai).
    pub expires_at: String,
    /// Nothing has been written yet.
    pub note: String,
}

/// How a confirmed action ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// It was done.
    Done,
    /// It is over the user's limit: an approval request was submitted.
    ApprovalRequested,
    /// The message was queued to go out.
    Sent,
}

/// The answer of [`confirm`]; the same for a repeated confirm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Confirmed {
    /// How it ended.
    pub outcome: Outcome,
    /// The action key.
    pub action: String,
    /// The record acted on or created, e.g. `invoicing.invoice:42`.
    pub record: Option<String>,
    /// What happened, in words.
    pub message: String,
    /// The record's page.
    pub url: Option<String>,
    /// More facts (the number given, the approval request…).
    pub data: Value,
}

/// A preview handler: `(call, record id, arguments)`.
pub type PreviewFn =
    Arc<dyn Fn(AgentCall, Option<i64>, Value) -> BoxFuture<'static, Result<Planned>> + Send + Sync>;

/// A confirm handler: `(call, record id, arguments, fingerprint at
/// preview)`. It refuses with [`MOVED`] when the fingerprint changed.
pub type ConfirmFn = Arc<
    dyn Fn(AgentCall, Option<i64>, Value, String) -> BoxFuture<'static, Result<Confirmed>>
        + Send
        + Sync,
>;

/// One registered agent action.
#[derive(Clone)]
pub struct AgentAction {
    key: String,
    subject_type: String,
    name: String,
    kind: ActionKind,
    label: String,
    description: String,
    arguments: Value,
    needs_record: bool,
    preview: PreviewFn,
    confirm: ConfirmFn,
}

/// An action as the `list_actions` tool describes it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActionInfo {
    /// The key to pass to `preview_action`.
    pub key: String,
    /// The record type it acts on.
    pub subject_type: String,
    /// What it does.
    pub label: String,
    /// When to use it.
    pub description: String,
    /// Transition, create or message.
    pub kind: ActionKind,
    /// Whether a record id is needed.
    pub needs_record: bool,
    /// The JSON schema of its arguments.
    pub arguments: Value,
}

impl AgentAction {
    /// A transition of `M` (records of `subject_type`) as an action.
    pub fn transition<M>(subject_type: &str, name: &str) -> TransitionAction<M>
    where
        M: StateMachine + Model<Key = i64> + Serialize + Clone + Send + Sync + 'static,
    {
        TransitionAction {
            subject_type: subject_type.to_owned(),
            name: name.to_owned(),
            label: None,
            description: String::new(),
            arguments: json!({ "type": "object", "properties": {} }),
            input: None,
            describe: None,
            done: None,
        }
    }

    /// An action with its own preview and confirm (a create, a message).
    pub fn custom(subject_type: &str, name: &str, kind: ActionKind) -> CustomAction {
        CustomAction {
            subject_type: subject_type.to_owned(),
            name: name.to_owned(),
            kind,
            label: name.replace('_', " "),
            description: String::new(),
            arguments: json!({ "type": "object", "properties": {} }),
            needs_record: kind != ActionKind::Create,
            preview: None,
            confirm: None,
        }
    }

    /// The key, `<subject type>.<name>`.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The record type.
    pub fn subject_type(&self) -> &str {
        &self.subject_type
    }

    /// The name (the transition's, for a transition).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The kind.
    pub fn kind(&self) -> ActionKind {
        self.kind
    }

    /// The label.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// As `list_actions` describes it.
    pub fn info(&self) -> ActionInfo {
        ActionInfo {
            key: self.key.clone(),
            subject_type: self.subject_type.clone(),
            label: self.label.clone(),
            description: self.description.clone(),
            kind: self.kind,
            needs_record: self.needs_record,
            arguments: self.arguments.clone(),
        }
    }
}

/// Whether `name` may never be an agent action.
pub fn is_forbidden(name: &str) -> bool {
    NEVER_ACTIONS.contains(&name) || machines::is_system_name(name)
}

static REGISTRY: RwLock<BTreeMap<String, Arc<AgentAction>>> = RwLock::new(BTreeMap::new());

/// Adds `action` to the registry, replacing one of the same key.
///
/// # Panics
///
/// When the action is a forbidden name ([`NEVER_ACTIONS`], a system
/// transition): a programming error, found at boot.
pub fn register(action: AgentAction) {
    assert!(
        !is_forbidden(&action.name),
        "`{}` may never be an agent action (decision 0006 point 4)",
        action.key
    );
    REGISTRY
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(action.key.clone(), Arc::new(action));
}

/// The action `key`, if registered.
pub fn get(key: &str) -> Option<Arc<AgentAction>> {
    REGISTRY
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(key)
        .cloned()
}

/// Every registered action, by key.
pub fn registered() -> Vec<Arc<AgentAction>> {
    REGISTRY
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .values()
        .cloned()
        .collect()
}

/// The actions on records of `subject_type`.
pub fn for_type(subject_type: &str) -> Vec<Arc<AgentAction>> {
    registered()
        .into_iter()
        .filter(|a| a.subject_type == subject_type)
        .collect()
}

/// What a token carries, sealed with `APP_KEY`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Ticket {
    nonce: String,
    action: String,
    record: Option<i64>,
    args: Value,
    user_id: i64,
    client: String,
    fingerprint: String,
    expires_at: i64,
}

fn status(code: StatusCode, message: impl Into<String>) -> Error {
    Error::Status(code, message.into())
}

fn action_of(key: &str) -> Result<Arc<AgentAction>> {
    get(key).ok_or_else(|| {
        status(
            StatusCode::NOT_FOUND,
            format!("There is no agent action `{key}`; list_actions names them."),
        )
    })
}

fn record_of(action: &AgentAction, record: Option<i64>) -> Result<Option<i64>> {
    if action.needs_record && record.is_none() {
        return Err(status(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("`{}` needs the record's id.", action.key),
        ));
    }
    Ok(record)
}

/// Previews action `key` on `record` with `args` for `call`. Writes nothing.
///
/// # Errors
///
/// 403 when the policy or the user's rights refuse it, 404 for an unknown
/// action or record, 422 with the reason when the record's state or a guard
/// refuses it.
pub async fn preview(
    call: &AgentCall,
    key: &str,
    record: Option<i64>,
    args: Value,
) -> Result<Preview> {
    let action = action_of(key)?;
    access::check(call, Some(&action.subject_type), Use::Act).await?;
    let record = record_of(&action, record)?;
    let planned = (action.preview)(call.clone(), record, args.clone()).await?;
    let expires = now() + renox::chrono::Duration::minutes(TOKEN_MINUTES);
    let ticket = Ticket {
        nonce: renox::random_token(),
        action: action.key.clone(),
        record,
        args,
        user_id: call.user.id,
        client: call.client.clone(),
        fingerprint: planned.fingerprint,
        expires_at: expires.timestamp(),
    };
    let sealed = renox::serde_json::to_string(&ticket).map_err(|e| Error::from(renox::anyhow::anyhow!(e)))?;
    let note = if planned.requires_approval {
        "Nothing has been written. This is over your limit: confirming submits an approval \
         request, which a person decides."
    } else {
        "Nothing has been written. Show this to the user and confirm only when they agree."
    };
    Ok(Preview {
        action: action.key.clone(),
        record: record.map(|id| Subject::new(&action.subject_type, id).to_string()),
        change: planned.change,
        requires_approval: planned.requires_approval,
        token: call.state.encrypt(&sealed),
        expires_at: crate::documents::dubai_text(expires),
        note: note.to_owned(),
    })
}

/// Confirms the action a [`preview`] gave `token` for. Runs it once; a
/// second confirm with the same token returns the first result.
///
/// # Errors
///
/// 422 [`BAD_TOKEN`] or [`EXPIRED`], 403 for a token given to another user
/// or assistant, 409 [`MOVED`] when the record changed since the preview,
/// and the action's own refusals.
pub async fn confirm(call: &AgentCall, token: &str) -> Result<Confirmed> {
    let ticket: Ticket = call
        .state
        .decrypt(token.trim())
        .ok()
        .and_then(|plain| renox::serde_json::from_str(&plain).ok())
        .ok_or_else(|| status(StatusCode::UNPROCESSABLE_ENTITY, BAD_TOKEN))?;
    if ticket.user_id != call.user.id || ticket.client != call.client {
        return Err(status(
            StatusCode::FORBIDDEN,
            "This confirmation was given to another user or assistant.",
        ));
    }
    let action = action_of(&ticket.action)?;
    access::check(call, Some(&action.subject_type), Use::Act).await?;
    let db = &call.state.db;
    if let Some(run) = AgentRun::by_nonce(db, &ticket.nonce).await? {
        return run.replay();
    }
    if now().timestamp() > ticket.expires_at {
        return Err(status(StatusCode::UNPROCESSABLE_ENTITY, EXPIRED));
    }
    // Claim the token: the unique nonce lets one confirm through.
    let mut claim = AgentRun {
        nonce: ticket.nonce.clone(),
        action: action.key.clone(),
        subject_type: action.subject_type.clone(),
        subject_id: ticket.record,
        user_id: call.user.id,
        client: call.client.clone(),
        outcome: RUNNING.to_owned(),
        result: DbJson(Value::Null),
        ..Default::default()
    };
    if claim.insert(db).await.is_err() {
        return match AgentRun::by_nonce(db, &ticket.nonce).await? {
            Some(run) => run.replay(),
            None => Err(status(StatusCode::CONFLICT, "Try confirming again.")),
        };
    }
    let ran = (action.confirm)(
        call.clone(),
        ticket.record,
        ticket.args.clone(),
        ticket.fingerprint.clone(),
    )
    .await;
    match ran {
        Ok(confirmed) => {
            let subject_id = confirmed
                .record
                .as_deref()
                .and_then(|r| r.parse::<Subject>().ok())
                .map(|s| s.subject_id)
                .or(ticket.record);
            let outcome = renox::serde_json::to_value(confirmed.outcome)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default();
            let result = renox::serde_json::to_value(&confirmed)
                .map_err(|e| Error::from(renox::anyhow::anyhow!(e)))?;
            AgentRun::query()
                .where_(AgentRun::ID.eq(claim.id))
                .update(
                    db,
                    &[
                        ("outcome", &outcome),
                        ("result", &DbJson(result)),
                        ("subject_id", &subject_id),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
            Ok(confirmed)
        }
        Err(error) => {
            // Nothing ran: the token may be confirmed again (it will be
            // refused the same way while the reason stands).
            AgentRun::query()
                .where_(AgentRun::ID.eq(claim.id))
                .delete(db)
                .await?;
            Err(error)
        }
    }
}

/// The outcome of a run row whose action is still running.
pub const RUNNING: &str = "running";

/// One confirmed agent action (`wl_agent_runs`): the token's nonce, who ran
/// what through which assistant, and the result a repeated confirm returns.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_agent_runs", unique(nonce), index(user_id))]
pub struct AgentRun {
    /// The primary key.
    pub id: i64,
    /// The token's nonce: one run per token.
    pub nonce: String,
    /// The action key.
    pub action: String,
    /// The record type.
    pub subject_type: String,
    /// The record acted on or created.
    pub subject_id: Option<i64>,
    /// The user the agent acted for.
    pub user_id: i64,
    /// The assistant, e.g. `Claude`.
    pub client: String,
    /// `running`, `done`, `approval_requested` or `sent`.
    pub outcome: String,
    /// The [`Confirmed`] answer.
    pub result: DbJson<Value>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl AgentRun {
    /// The run of the token with `nonce`, if confirmed already.
    pub async fn by_nonce(db: &Db, nonce: &str) -> Result<Option<AgentRun>> {
        AgentRun::query()
            .where_(AgentRun::NONCE.eq(nonce))
            .first(db)
            .await
    }

    /// The runs about `subject`, oldest first.
    pub async fn about(db: &Db, subject: &Subject) -> Result<Vec<AgentRun>> {
        AgentRun::query()
            .where_(AgentRun::SUBJECT_TYPE.eq(subject.subject_type.as_str()))
            .where_(AgentRun::SUBJECT_ID.eq(subject.subject_id))
            .order_by(AgentRun::ID)
            .get(db)
            .await
    }

    /// The stored answer of a confirmed run, for a repeated confirm.
    fn replay(&self) -> Result<Confirmed> {
        if self.outcome == RUNNING {
            return Err(status(
                StatusCode::CONFLICT,
                "This action is being confirmed right now.",
            ));
        }
        renox::serde_json::from_value(self.result.0.clone())
            .map_err(|e| Error::from(renox::anyhow::anyhow!(e)))
    }
}

/// An action with its own handlers, from [`AgentAction::custom`].
#[must_use = "an agent action is only offered once registered"]
pub struct CustomAction {
    subject_type: String,
    name: String,
    kind: ActionKind,
    label: String,
    description: String,
    arguments: Value,
    needs_record: bool,
    preview: Option<PreviewFn>,
    confirm: Option<ConfirmFn>,
}

impl CustomAction {
    /// What it does, e.g. `Send a message from a template`.
    pub fn label(mut self, label: &str) -> Self {
        self.label = label.to_owned();
        self
    }

    /// When an assistant should use it.
    pub fn description(mut self, description: &str) -> Self {
        self.description = description.to_owned();
        self
    }

    /// The JSON schema of its arguments.
    pub fn arguments(mut self, schema: Value) -> Self {
        self.arguments = schema;
        self
    }

    /// Whether a record id is needed (a create needs none by default).
    pub fn needs_record(mut self, needs: bool) -> Self {
        self.needs_record = needs;
        self
    }

    /// The preview: writes nothing.
    pub fn preview<F>(mut self, preview: F) -> Self
    where
        F: Fn(AgentCall, Option<i64>, Value) -> BoxFuture<'static, Result<Planned>>
            + Send
            + Sync
            + 'static,
    {
        self.preview = Some(Arc::new(preview));
        self
    }

    /// The confirm: does it, once.
    pub fn confirm<F>(mut self, confirm: F) -> Self
    where
        F: Fn(AgentCall, Option<i64>, Value, String) -> BoxFuture<'static, Result<Confirmed>>
            + Send
            + Sync
            + 'static,
    {
        self.confirm = Some(Arc::new(confirm));
        self
    }

    /// The action.
    ///
    /// # Panics
    ///
    /// Without a preview or a confirm.
    pub fn build(self) -> AgentAction {
        AgentAction {
            key: format!("{}.{}", self.subject_type, self.name),
            subject_type: self.subject_type,
            name: self.name,
            kind: self.kind,
            label: self.label,
            description: self.description,
            arguments: self.arguments,
            needs_record: self.needs_record,
            preview: self.preview.expect("an agent action needs a preview"),
            confirm: self.confirm.expect("an agent action needs a confirm"),
        }
    }

    /// Builds and registers it.
    pub fn register(self) {
        register(self.build());
    }
}

/// Gathers a transition's input from the stored record and the arguments.
pub type InputFn<M> = Arc<dyn Fn(AppState, M, Value) -> BoxFuture<'static, Result<Value>> + Send + Sync>;

/// Says what a transition will change, for the preview.
pub type DescribeFn<M> =
    Arc<dyn Fn(AppState, M, Value) -> BoxFuture<'static, Result<Change>> + Send + Sync>;

/// Says what was done, from the record as stored after the run.
pub type DoneFn<M> =
    Arc<dyn Fn(AppState, M) -> BoxFuture<'static, Result<(String, Value)>> + Send + Sync>;

/// A transition as an action, from [`AgentAction::transition`].
#[must_use = "an agent action is only offered once registered"]
pub struct TransitionAction<M> {
    subject_type: String,
    name: String,
    label: Option<String>,
    description: String,
    arguments: Value,
    input: Option<InputFn<M>>,
    describe: Option<DescribeFn<M>>,
    done: Option<DoneFn<M>>,
}

/// What a transition action keeps for its handlers.
struct Spec<M> {
    subject_type: String,
    name: String,
    label: String,
    input: Option<InputFn<M>>,
    describe: Option<DescribeFn<M>>,
    done: Option<DoneFn<M>>,
}

impl<M> TransitionAction<M>
where
    M: StateMachine + Model<Key = i64> + Serialize + Clone + Send + Sync + 'static,
{
    /// What it does (default: the transition's label).
    pub fn label(mut self, label: &str) -> Self {
        self.label = Some(label.to_owned());
        self
    }

    /// When an assistant should use it.
    pub fn description(mut self, description: &str) -> Self {
        self.description = description.to_owned();
        self
    }

    /// The JSON schema of its arguments (`reason` is read for transitions
    /// that need one).
    pub fn arguments(mut self, schema: Value) -> Self {
        self.arguments = schema;
        self
    }

    /// The transition's input, as the screen gathers it.
    pub fn input<F>(mut self, input: F) -> Self
    where
        F: Fn(AppState, M, Value) -> BoxFuture<'static, Result<Value>> + Send + Sync + 'static,
    {
        self.input = Some(Arc::new(input));
        self
    }

    /// The change in words, for the preview.
    pub fn describe<F>(mut self, describe: F) -> Self
    where
        F: Fn(AppState, M, Value) -> BoxFuture<'static, Result<Change>> + Send + Sync + 'static,
    {
        self.describe = Some(Arc::new(describe));
        self
    }

    /// What was done, in words, and facts, from the stored record after the
    /// run.
    pub fn done<F>(mut self, done: F) -> Self
    where
        F: Fn(AppState, M) -> BoxFuture<'static, Result<(String, Value)>> + Send + Sync + 'static,
    {
        self.done = Some(Arc::new(done));
        self
    }

    /// The action.
    ///
    /// # Panics
    ///
    /// When the machine has no such transition, or it is a system one.
    pub fn build(self) -> AgentAction {
        let machine = M::machine();
        let transition = machine.find(&self.name).unwrap_or_else(|_| {
            panic!(
                "`{}` is not a transition of the machine of {}",
                self.name, self.subject_type
            )
        });
        assert!(
            !transition.is_system(),
            "the system transition `{}` of {} may never be an agent action",
            self.name,
            self.subject_type
        );
        let label = self.label.unwrap_or_else(|| transition.label_text());
        let spec = Arc::new(Spec {
            subject_type: self.subject_type.clone(),
            name: self.name.clone(),
            label: label.clone(),
            input: self.input,
            describe: self.describe,
            done: self.done,
        });
        let for_preview = Arc::clone(&spec);
        let for_confirm = spec;
        AgentAction {
            key: format!("{}.{}", self.subject_type, self.name),
            subject_type: self.subject_type,
            name: self.name,
            kind: ActionKind::Transition,
            label,
            description: self.description,
            arguments: self.arguments,
            needs_record: true,
            preview: Arc::new(
                move |call: AgentCall,
                      record: Option<i64>,
                      args: Value|
                      -> BoxFuture<'static, Result<Planned>> {
                let spec = Arc::clone(&for_preview);
                Box::pin(async move {
                    let id = record.unwrap_or_default();
                    plan(&spec, &call, id, &args).await.map(|(_, _, planned)| planned)
                })
            }),
            confirm: Arc::new(
                move |call: AgentCall,
                      record: Option<i64>,
                      args: Value,
                      fingerprint: String|
                      -> BoxFuture<'static, Result<Confirmed>> {
                let spec = Arc::clone(&for_confirm);
                Box::pin(async move {
                    run(&spec, &call, record.unwrap_or_default(), &args, &fingerprint).await
                })
            }),
        }
    }

    /// Builds and registers it.
    pub fn register(self) {
        register(self.build());
    }
}

/// The text a refusal of the engine answers with.
fn refusal(error: TransitionError, label: &str) -> Error {
    match error {
        TransitionError::Forbidden | TransitionError::Unauthenticated => status(
            StatusCode::FORBIDDEN,
            format!("You may not do this: {label}."),
        ),
        TransitionError::Conflict => status(StatusCode::CONFLICT, MOVED),
        other => other.into_error(),
    }
}

/// The stored state of record `id` of `M`, as text.
async fn current_state<M: StateMachine>(db: &Db, id: i64) -> Result<Option<String>> {
    sql(format!(
        "SELECT {} FROM {} WHERE id = ?",
        M::COLUMN.name(),
        M::TABLE
    ))
    .bind(id)
    .scalar_optional::<String>(db)
    .await
    .map_err(Into::into)
}

/// A fingerprint of `record` in `state`: any change of a column changes it.
fn fingerprint_of<M: Serialize>(record: &M, state: &str) -> String {
    let text = renox::serde_json::to_string(record).unwrap_or_default();
    renox::webhook::sha256_hex(format!("{state}|{text}"))
}

/// The page of record `id` of `subject_type`, when its type is searchable.
pub fn url_of(subject_type: &str, id: i64) -> Option<String> {
    crate::search::searchable_type(subject_type).map(|s| s.url_of(id))
}

/// The reason given in the arguments, if any.
fn reason_of(args: &Value) -> Option<String> {
    args.get("reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_owned)
}

/// The checks of a transition, writing nothing: the record, the user's
/// right, its state, the guards, the reason and the approval policy.
async fn plan<M>(
    spec: &Spec<M>,
    call: &AgentCall,
    id: i64,
    args: &Value,
) -> Result<(M, Value, Planned)>
where
    M: StateMachine + Model<Key = i64> + Serialize + Clone + Send + Sync + 'static,
{
    let db = &call.state.db;
    let Some(record) = M::find(db, id).await? else {
        return Err(Error::NotFound);
    };
    let Some(current) = current_state::<M>(db, id).await? else {
        return Err(Error::NotFound);
    };
    let machine = M::machine();
    let transition = machine
        .find(&spec.name)
        .map_err(|e| refusal(e, &spec.label))?;
    transition
        .check_access(&record, Some(&call.user))
        .map_err(|e| refusal(e, &spec.label))?;
    transition
        .check_from(&current)
        .map_err(|e| refusal(e, &spec.label))?;
    let input = match &spec.input {
        Some(gather) => gather(call.state.clone(), record.clone(), args.clone()).await?,
        None => Value::Null,
    };
    let reason = reason_of(args);
    let mut ctx = TransitionCtx::new(Some(call.user.clone()))
        .with_input(input.clone())
        .with_meta(call.meta());
    if let Some(reason) = &reason {
        ctx = ctx.with_reason(reason.clone());
    }
    transition
        .check_guards(&record, &ctx)
        .await
        .map_err(|e| refusal(e, &spec.label))?;
    if transition.needs_reason() && reason.is_none() {
        return Err(status(
            StatusCode::UNPROCESSABLE_ENTITY,
            "This action needs a reason: ask the user for one and pass it as `reason`.",
        ));
    }
    let to = transition.to_state().unwrap_or_default().to_owned();
    let mut request = TransitionRequest::new(M::TABLE, id, spec.name.clone(), current.clone(), to.clone())
        .with_input(input.clone());
    request.actor = Some(call.user.clone());
    request.reason = reason;
    let verdict = state_machine::preview(&request, db).await?;
    if verdict == MachinePreview::Pending {
        return Err(status(
            StatusCode::UNPROCESSABLE_ENTITY,
            "An approval request for this is already waiting; a person decides it.",
        ));
    }
    let change = match &spec.describe {
        Some(describe) => describe(call.state.clone(), record.clone(), args.clone()).await?,
        None => Change::new(format!("{}: {}", spec.label, Subject::new(&spec.subject_type, id).words())),
    }
    .line("Status", format!("{current} → {to}"));
    let requires_approval = verdict == MachinePreview::RequiresApproval;
    let change = if requires_approval {
        change.warning("This is over your limit: it will be sent for approval, not done now.")
    } else {
        change
    };
    let planned = Planned {
        change,
        requires_approval,
        fingerprint: fingerprint_of(&record, &current),
    };
    Ok((record, input, planned))
}

/// Runs a transition for a confirm: the same checks again, the fingerprint,
/// then the run as the user, with the agent in the history's metadata.
async fn run<M>(
    spec: &Spec<M>,
    call: &AgentCall,
    id: i64,
    args: &Value,
    fingerprint: &str,
) -> Result<Confirmed>
where
    M: StateMachine + Model<Key = i64> + Serialize + Clone + Send + Sync + 'static,
{
    // The record as stored now must be the one the user saw in the preview.
    let db = &call.state.db;
    let stored = M::find(db, id).await?;
    let state = current_state::<M>(db, id).await?;
    match (&stored, &state) {
        (Some(record), Some(state)) if fingerprint_of(record, state) == fingerprint => {}
        _ => return Err(status(StatusCode::CONFLICT, MOVED)),
    }
    let (record, input, _planned) = plan(spec, call, id, args).await?;
    let subject = Subject::new(&spec.subject_type, id);
    let mut tx = call.state.db.begin().await?;
    let mut builder = record
        .transition(&spec.name)
        .by(&call.user)
        .input(&input)
        .meta(call.meta());
    if let Some(reason) = reason_of(args) {
        builder = builder.reason(reason);
    }
    let outcome = builder
        .run_in(&mut tx)
        .await
        .map_err(|e| refusal(e, &spec.label))?;
    crate::search::index_in(&mut tx, &subject).await?;
    tx.commit().await?;
    let url = url_of(&spec.subject_type, id);
    let confirmed = match &outcome {
        MachineOutcome::Pending(pending) => {
            let request = pending
                .data
                .get(crate::approvals::submit::REQUEST_KEY)
                .cloned()
                .unwrap_or(Value::Null);
            Confirmed {
                outcome: Outcome::ApprovalRequested,
                action: format!("{}.{}", spec.subject_type, spec.name),
                record: Some(subject.to_string()),
                message: format!(
                    "This is over your limit: an approval request was submitted ({}). \
                     It is done once a person approves it.",
                    spec.label
                ),
                url: url.clone(),
                data: json!({ "approval_request": request }),
            }
        }
        MachineOutcome::Done(done) => {
            let (message, data) = match &spec.done {
                Some(describe) => match M::find(&call.state.db, id).await? {
                    Some(stored) => describe(call.state.clone(), stored).await?,
                    None => (String::new(), Value::Null),
                },
                None => (String::new(), Value::Null),
            };
            let message = if message.is_empty() {
                format!("Done: {} ({} → {}).", spec.label, done.from, done.to)
            } else {
                message
            };
            Confirmed {
                outcome: Outcome::Done,
                action: format!("{}.{}", spec.subject_type, spec.name),
                record: Some(subject.to_string()),
                message,
                url: url.clone(),
                data,
            }
        }
    };
    crate::approvals::announce(&call.state, &outcome).await?;
    outcome.emit(&call.state).await?;
    Ok(confirmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approvals_and_voids_are_never_actions() {
        for name in ["approve", "approve_step", "reject", "void"] {
            assert!(is_forbidden(name), "{name}");
        }
        assert!(is_forbidden("issue_approved"));
        assert!(is_forbidden("back_to_draft"));
        assert!(!is_forbidden("issue"));
    }

    #[test]
    #[should_panic(expected = "may never be an agent action")]
    fn registering_an_approval_decision_panics() {
        AgentAction::custom("kernel.approval_request", "approve", ActionKind::Transition)
            .preview(|_, _, _| Box::pin(async { Ok(Planned::new(Change::new("no"))) }))
            .confirm(|_, _, _, _| Box::pin(async { Err(Error::NotFound) }))
            .register();
    }

    #[test]
    fn a_change_keeps_its_lines_and_amounts() {
        let change = Change::new("Issue invoice")
            .line("Number", "Assigned when issued")
            .amount("Total", 105_000, "AED")
            .amount("VAT", 5_000, "AED");
        assert_eq!(change.value_of("Number"), Some("Assigned when issued"));
        assert_eq!(change.amounts[0].money.text, "AED 1,050.00");
        assert_eq!(change.amounts[1].money.minor, 5_000);
    }
}
