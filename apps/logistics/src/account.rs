//! A change of a client's remittance bank account (03 Logistics §3.3
//! `LOG.ACL.13`, §4.1 row *Bank returns the remittance transfer*; BL-LOG-17).
//!
//! A fraudulent change must not divert the client's money, so a change takes
//! effect only when **both** a second user approved it and the client
//! confirmed it from its known contact, in either order:
//!
//! - **Request** ([`request`]): an accountant (`remittances.prepare` and
//!   `remittances.request_account`) or a client user of the company in the
//!   portal enters the new account. It is `pending`. One change per client is
//!   open at a time; a second request is refused until the first is
//!   withdrawn, rejected, lapsed or active.
//! - **Known contact.** The confirmation link goes to the client's primary
//!   e-mail on record when the request was made ([`request`] reads it from
//!   the party's contacts and keeps it on the change), never to an address
//!   given in the request.
//! - **Approval** ([`approve`]): a user with `remittances.approve` other than
//!   the requester. The decision is a guard of the change's own machine, as
//!   for the adjustment lines, not a kernel approval request.
//! - **Confirmation** ([`confirm`]): the signed link
//!   `GET /remittances/accounts/{id}/confirm` (public: the signature is the
//!   credential; opening it confirms, as the portal's signed links do).
//! - **Active.** The second of the two makes the change `active`: the
//!   account becomes the client's remittance account
//!   ([`crate::reweigh::ClientTerms::bank_account`]). Until then remittances
//!   use the old account.
//! - **Lapse** ([`lapse_due`], daily): a change not both approved and
//!   confirmed within 14 days of the request lapses; the link stops working
//!   and the requester is told. The old account stays.
//! - **Rejection** and **withdrawal** discard it; the requester is told of a
//!   rejection.

use std::time::Duration;

use renox::chrono::TimeDelta;
use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use renox::signed::ValidSignature;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::notify::{self, NotifyChannel, NotifyRecipient, NotifyRequest};
use wl_kernel::parties;
use wl_kernel::{PartyId, Subject};

use crate::booking;
use crate::notices::{ACCOUNT_CONFIRM, tell_user};
use crate::permissions;
use crate::reweigh::{self, ClientTerms};
use crate::texts::{english, t};

/// The subject type of an account change.
pub const SUBJECT_TYPE: &str = "logistics.account_change";

/// Days a change may wait for its approval and confirmation.
pub const LAPSE_DAYS: i64 = 14;

/// The scheduled task that lapses the changes.
pub const TASK: &str = "logistics-account-changes";

/// The time it runs, Asia/Dubai.
pub const TASK_AT: &str = "03:00";

/// The name of the confirmation route.
pub const CONFIRM_ROUTE: &str = "logistics.account_confirm";

/// `approve`: pending → approved.
pub const APPROVE: &str = "approve";
/// `approve_confirmed`: confirmed → active (the approval completes it).
pub const APPROVE_CONFIRMED: &str = "approve_confirmed";
/// `confirm`: pending → confirmed, by the client's link.
pub const CONFIRM: &str = "confirm";
/// `confirm_approved`: approved → active (the confirmation completes it).
pub const CONFIRM_APPROVED: &str = "confirm_approved";
/// `reject`: an open change → rejected.
pub const REJECT: &str = "reject";
/// `withdraw`: the requester takes an open change back.
pub const WITHDRAW: &str = "withdraw";
/// `lapse`: the system, after [`LAPSE_DAYS`].
pub const LAPSE: &str = "lapse";

/// Where a change is.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AccountChangeStatus {
    /// Waiting for the approval and the confirmation.
    #[default]
    Pending,
    /// Approved; waiting for the client's confirmation.
    Approved,
    /// Confirmed by the client; waiting for the approval.
    Confirmed,
    /// Both exist: the account is the client's remittance account.
    Active,
    /// A second user rejected it; discarded.
    Rejected,
    /// Not approved and confirmed in time; discarded.
    Lapsed,
    /// The requester took it back; discarded.
    Withdrawn,
}

impl AccountChangeStatus {
    /// The states in which a change waits.
    pub const OPEN: [AccountChangeStatus; 3] = [
        AccountChangeStatus::Pending,
        AccountChangeStatus::Approved,
        AccountChangeStatus::Confirmed,
    ];
}

/// A request to change the client's remittance bank account.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "logistics_account_changes",
    state_machine,
    index(client_party_id),
    index(status)
)]
pub struct AccountChange {
    /// The primary key.
    pub id: i64,
    /// The client.
    pub client_party_id: i64,
    /// The new account (an IBAN, without spaces).
    pub account: String,
    /// The user who asked.
    pub requested_by: i64,
    /// When.
    pub requested_at: DateTime,
    /// `staff` or `portal`.
    pub via: String,
    /// The client's primary e-mail on record when it was asked: where the
    /// confirmation link went.
    pub contact: String,
    /// The user who approved it.
    pub approved_by: Option<i64>,
    /// When.
    pub approved_at: Option<DateTime>,
    /// When the client confirmed it.
    pub confirmed_at: Option<DateTime>,
    /// When it lapses if it is not both approved and confirmed.
    pub expires_at: DateTime,
    /// The user who rejected or withdrew it.
    pub closed_by: Option<i64>,
    /// When it was rejected, withdrawn or lapsed.
    pub closed_at: Option<DateTime>,
    /// The note of the rejection.
    pub closed_note: Option<String>,
    /// The state; changed only by a transition.
    pub status: AccountChangeStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl AccountChange {
    /// The client.
    pub fn client(&self) -> PartyId {
        PartyId(self.client_party_id)
    }

    /// The change as a [`Subject`].
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// Whether it still waits.
    pub fn is_open(&self) -> bool {
        AccountChangeStatus::OPEN.contains(&self.status)
    }
}

/// An IBAN as stored: spaces dropped, upper case.
pub fn normalized(account: &str) -> String {
    account
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_uppercase()
}

/// Whether `account` (normalised) looks like an IBAN: 15 to 34 letters and
/// digits, starting with a country code and two check digits.
pub fn plausible(account: &str) -> bool {
    let chars: Vec<char> = account.chars().collect();
    (15..=34).contains(&chars.len())
        && chars.iter().all(char::is_ascii_alphanumeric)
        && chars[..2].iter().all(char::is_ascii_alphabetic)
        && chars[2..4].iter().all(char::is_ascii_digit)
}

/// Makes the change's account the client's remittance account, inside `tx`.
async fn activate(tx: &mut Transaction, client: i64, account: &str) -> Result {
    let account = Some(account.to_owned());
    let found = ClientTerms::query()
        .where_(ClientTerms::CLIENT_PARTY_ID.eq(client))
        .first(&mut *tx)
        .await?;
    match found {
        Some(_) => {
            ClientTerms::query()
                .where_(ClientTerms::CLIENT_PARTY_ID.eq(client))
                .update(&mut *tx, &[("bank_account", &account)])
                .await?;
        }
        None => {
            let mut terms = ClientTerms {
                client_party_id: client,
                bank_account: account,
                ..ClientTerms::default()
            };
            terms.insert(&mut *tx).await?;
        }
    }
    Ok(())
}

/// The guard that keeps the requester from deciding.
fn not_the_requester(change: &AccountChange, ctx: &renox::state_machine::TransitionCtx) -> bool {
    ctx.actor()
        .is_some_and(|user| user.id == change.requested_by)
}

/// `approve` and `approve_confirmed` (`LOG.ACL.13`): a user with
/// `remittances.approve` other than the requester; the second of the two
/// conditions activates the account.
fn approve(name: &'static str, from: AccountChangeStatus) -> Transition<AccountChange> {
    let activates = from == AccountChangeStatus::Confirmed;
    Transition::new(name)
        .label("Approve")
        .tone("primary")
        .permission(permissions::REMITTANCES_APPROVE)
        .from([from])
        .to(if activates {
            AccountChangeStatus::Active
        } else {
            AccountChangeStatus::Approved
        })
        .guard("not_the_requester", |change: &AccountChange, ctx| {
            let same = not_the_requester(change, ctx);
            Box::pin(async move {
                if same {
                    Err(english("logistics.account.own_request", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .effect(move |change: &AccountChange, ctx, tx| {
            let id = change.id;
            let client = change.client_party_id;
            let account = change.account.clone();
            let actor = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                let at = Some(renox::db::now());
                AccountChange::query()
                    .where_(AccountChange::ID.eq(id))
                    .update(&mut *tx, &[("approved_by", &actor), ("approved_at", &at)])
                    .await?;
                if activates {
                    activate(&mut *tx, client, &account).await?;
                }
                Ok(())
            })
        })
}

/// `confirm` and `confirm_approved`: the client's link, run by the system;
/// the second of the two conditions activates the account.
fn confirm_transition(name: &'static str, from: AccountChangeStatus) -> Transition<AccountChange> {
    let activates = from == AccountChangeStatus::Approved;
    Transition::new(name)
        .label("Confirm")
        .tone("primary")
        .system()
        .from([from])
        .to(if activates {
            AccountChangeStatus::Active
        } else {
            AccountChangeStatus::Confirmed
        })
        .effect(move |change: &AccountChange, _ctx, tx| {
            let id = change.id;
            let client = change.client_party_id;
            let account = change.account.clone();
            Box::pin(async move {
                let at = Some(renox::db::now());
                AccountChange::query()
                    .where_(AccountChange::ID.eq(id))
                    .update(&mut *tx, &[("confirmed_at", &at)])
                    .await?;
                if activates {
                    activate(&mut *tx, client, &account).await?;
                }
                Ok(())
            })
        })
}

/// Records who closed the change, when and with which note.
async fn closed(tx: &mut Transaction, id: i64, actor: Option<i64>, note: Option<String>) -> Result {
    let at = Some(renox::db::now());
    AccountChange::query()
        .where_(AccountChange::ID.eq(id))
        .update(
            &mut *tx,
            &[
                ("closed_by", &actor),
                ("closed_at", &at),
                ("closed_note", &note),
            ],
        )
        .await?;
    Ok(())
}

/// `reject`: a user with `remittances.approve` other than the requester
/// discards the change.
fn reject() -> Transition<AccountChange> {
    Transition::new(REJECT)
        .label("Reject")
        .tone("danger")
        .permission(permissions::REMITTANCES_APPROVE)
        .from(AccountChangeStatus::OPEN)
        .to(AccountChangeStatus::Rejected)
        .guard("not_the_requester", |change: &AccountChange, ctx| {
            let same = not_the_requester(change, ctx);
            Box::pin(async move {
                if same {
                    Err(english("logistics.account.own_request", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .effect(|change: &AccountChange, ctx, tx| {
            let id = change.id;
            let actor = ctx.actor().map(|user| user.id);
            let note = ctx
                .reason()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned);
            Box::pin(async move { closed(&mut *tx, id, actor, note).await })
        })
}

/// `withdraw`: only the requester takes the open change back.
fn withdraw() -> Transition<AccountChange> {
    Transition::new(WITHDRAW)
        .label("Withdraw")
        .tone("warning")
        .permission(permissions::REMITTANCES_REQUEST_ACCOUNT)
        .from(AccountChangeStatus::OPEN)
        .to(AccountChangeStatus::Withdrawn)
        .guard("the_requester", |change: &AccountChange, ctx| {
            let other = ctx
                .actor()
                .is_some_and(|user| user.id != change.requested_by);
            Box::pin(async move {
                if other {
                    Err(english("logistics.account.not_your_request", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .effect(|change: &AccountChange, ctx, tx| {
            let id = change.id;
            let actor = ctx.actor().map(|user| user.id);
            Box::pin(async move { closed(&mut *tx, id, actor, None).await })
        })
}

/// `lapse`: the system discards a change that waited [`LAPSE_DAYS`].
fn lapse() -> Transition<AccountChange> {
    Transition::new(LAPSE)
        .label("Lapse")
        .tone("warning")
        .system()
        .from(AccountChangeStatus::OPEN)
        .to(AccountChangeStatus::Lapsed)
        .effect(|change: &AccountChange, _ctx, tx| {
            let id = change.id;
            Box::pin(async move { closed(&mut *tx, id, None, None).await })
        })
}

impl StateMachine for AccountChange {
    type State = AccountChangeStatus;
    const COLUMN: renox::db::Col<AccountChange, AccountChangeStatus> = AccountChange::STATUS;

    fn machine() -> Machine<Self> {
        use AccountChangeStatus::{Active, Approved, Confirmed, Lapsed, Pending, Rejected, Withdrawn};
        Machine::new()
            .initial([Pending])
            .state(Pending)
            .tone("neutral")
            .state(Approved)
            .tone("info")
            .state(Confirmed)
            .tone("info")
            .state(Active)
            .tone("success")
            .state(Rejected)
            .tone("error")
            .state(Lapsed)
            .tone("warning")
            .state(Withdrawn)
            .tone("neutral")
            .transition(approve(APPROVE, Pending))
            .transition(approve(APPROVE_CONFIRMED, Confirmed))
            .transition(confirm_transition(CONFIRM, Pending))
            .transition(confirm_transition(CONFIRM_APPROVED, Approved))
            .transition(reject())
            .transition(withdraw())
            .transition(lapse())
    }
}

// ---------------------------------------------------------------------------
// The services

fn guard(message: String) -> TransitionError {
    TransitionError::Guard(message)
}

async fn load(state: &AppState, id: i64) -> std::result::Result<AccountChange, TransitionError> {
    AccountChange::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))
}

/// The open change of `client`, if there is one.
///
/// # Errors
///
/// Database errors.
pub async fn open_change_of(db: &Db, client: PartyId) -> Result<Option<AccountChange>> {
    AccountChange::query()
        .where_(AccountChange::CLIENT_PARTY_ID.eq(client.0))
        .where_(AccountChange::STATUS.is_in(AccountChangeStatus::OPEN))
        .order_by(AccountChange::ID)
        .first(db)
        .await
}

/// Every change of `client`, newest first.
///
/// # Errors
///
/// Database errors.
pub async fn changes_of(db: &Db, client: PartyId) -> Result<Vec<AccountChange>> {
    AccountChange::query()
        .where_(AccountChange::CLIENT_PARTY_ID.eq(client.0))
        .order_by_desc(AccountChange::ID)
        .get(db)
        .await
}

/// `user` asks to change the remittance bank account of `client` to
/// `account`, now. See [`request_at`].
///
/// # Errors
///
/// As [`request_at`].
pub async fn request(
    state: &AppState,
    user: &AuthUser,
    client: PartyId,
    account: &str,
) -> std::result::Result<AccountChange, TransitionError> {
    request_at(state, user, client, account, renox::db::now()).await
}

/// `user` asks to change the remittance bank account of `client` to
/// `account` at the instant `at`: the change is `pending`, the link to
/// confirm it goes to the client's primary e-mail on record, and it lapses
/// after [`LAPSE_DAYS`].
///
/// # Errors
///
/// `Forbidden` unless `user` is an accountant (`remittances.prepare` and
/// `remittances.request_account`) or a client user of the company with
/// `remittances.request_account`; `Guard` (422) for an account that is not an
/// IBAN, the account already on file, another change still open ("a bank
/// account change is already pending") and a client with no e-mail on record.
pub async fn request_at(
    state: &AppState,
    user: &AuthUser,
    client: PartyId,
    account: &str,
    at: DateTime,
) -> std::result::Result<AccountChange, TransitionError> {
    let may = user.has_permission(permissions::REMITTANCES_REQUEST_ACCOUNT);
    let staff = may && user.has_permission(permissions::REMITTANCES_PREPARE);
    let portal = may && !staff && booking::acts_for(state, user, client).await?;
    if !staff && !portal {
        return Err(TransitionError::Forbidden);
    }
    lapse_due(state, at).await?;
    let account = normalized(account);
    if !plausible(&account) {
        return Err(guard(english("logistics.account.invalid", &[])));
    }
    let terms = reweigh::terms(&state.db, client).await?;
    if terms.bank_account.as_deref().map(normalized).as_deref() == Some(account.as_str()) {
        return Err(guard(english("logistics.account.same", &[])));
    }
    if open_change_of(&state.db, client).await?.is_some() {
        return Err(guard(english("logistics.account.already_pending", &[])));
    }
    let contact = parties::directory(state)
        .contacts(&state.db, client)
        .await?
        .and_then(|contacts| {
            contacts
                .point_for(NotifyChannel::Email)
                .map(|point| point.address.clone())
        })
        .ok_or_else(|| guard(english("logistics.account.no_contact", &[])))?;
    let mut change = AccountChange {
        client_party_id: client.0,
        account,
        requested_by: user.id,
        requested_at: at,
        via: if staff { "staff" } else { "portal" }.to_owned(),
        contact: contact.clone(),
        expires_at: at + TimeDelta::days(LAPSE_DAYS),
        status: AccountChangeStatus::Pending,
        ..AccountChange::default()
    };
    change.insert(&state.db).await?;
    let link = state.signed_url(
        CONFIRM_ROUTE,
        &[&change.id],
        Duration::from_secs(u64::try_from(LAPSE_DAYS).unwrap_or(14) * 24 * 3600),
    )?;
    let last = change
        .account
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    let request = NotifyRequest::new(ACCOUNT_CONFIRM, change.subject())
        .subject_label(&format!("#{}", change.id))
        .to(NotifyRecipient::Address {
            channel: NotifyChannel::Email,
            address: contact,
        })
        .data(renox::serde_json::json!({ "link": link, "last4": last }));
    notify::notifier(state).notify(state, request).await?;
    load(state, change.id).await
}

/// Lapses every open change whose time is up at `now`; the requesters are
/// told. Answers the changes that lapsed.
///
/// # Errors
///
/// Database errors and the notifier's.
pub async fn lapse_due(
    state: &AppState,
    now: DateTime,
) -> std::result::Result<Vec<AccountChange>, TransitionError> {
    let due = AccountChange::query()
        .where_(AccountChange::STATUS.is_in(AccountChangeStatus::OPEN))
        .where_(AccountChange::EXPIRES_AT.lte(now))
        .order_by(AccountChange::ID)
        .get(&state.db)
        .await?;
    let mut lapsed = Vec::new();
    for change in due {
        let _outcome = change.transition(LAPSE).by_system().run(state).await?;
        let change = load(state, change.id).await?;
        let title = english("logistics.account.lapsed_title", &[]);
        let body = english(
            "logistics.account.lapsed_body",
            &[("account", &change.account)],
        );
        tell_user(
            state,
            change.subject(),
            &format!("#{}", change.id),
            change.requested_by,
            &title,
            &body,
        )
        .await?;
        lapsed.push(change);
    }
    Ok(lapsed)
}

/// `user` (`remittances.approve`, not the requester) approves change `id`.
/// With the client's confirmation already there, the account becomes active.
///
/// # Errors
///
/// `Forbidden` without the permission; `NotFrom` unless the change waits for
/// the approval; `Guard` (422) for the requester and for a change whose time
/// is up.
pub async fn approve_change(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<AccountChange, TransitionError> {
    lapse_due(state, renox::db::now()).await?;
    let change = load(state, id).await?;
    let name = if change.status == AccountChangeStatus::Confirmed {
        APPROVE_CONFIRMED
    } else {
        APPROVE
    };
    let _outcome = change.transition(name).by(user).run(state).await?;
    load(state, id).await
}

/// `user` (`remittances.approve`, not the requester) rejects change `id`
/// with `note`; the pending account is discarded and the requester is told.
///
/// # Errors
///
/// `Forbidden` without the permission; `NotFrom` unless the change is open;
/// `Guard` (422) for the requester.
pub async fn reject_change(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    note: &str,
) -> std::result::Result<AccountChange, TransitionError> {
    let change = load(state, id).await?;
    let _outcome = change
        .transition(REJECT)
        .by(user)
        .reason(note.trim())
        .run(state)
        .await?;
    let change = load(state, id).await?;
    let title = english("logistics.account.rejected_title", &[]);
    let body = english(
        "logistics.account.rejected_body",
        &[("account", &change.account), ("note", &note.trim())],
    );
    tell_user(
        state,
        change.subject(),
        &format!("#{}", change.id),
        change.requested_by,
        &title,
        &body,
    )
    .await?;
    Ok(change)
}

/// The requester `user` takes change `id` back.
///
/// # Errors
///
/// `Forbidden` without `remittances.request_account`; `NotFrom` unless the
/// change is open; `Guard` (422) for another user than the requester.
pub async fn withdraw_change(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<AccountChange, TransitionError> {
    let change = load(state, id).await?;
    let _outcome = change.transition(WITHDRAW).by(user).run(state).await?;
    load(state, id).await
}

/// The client confirms change `id` from the link sent to its known contact,
/// at `now`. With the approval already there, the account becomes active. A
/// second opening of the link changes nothing.
///
/// # Errors
///
/// `Guard` (422) "this link no longer works" for a change that was rejected,
/// withdrawn, lapsed or whose time is up.
pub async fn confirm(
    state: &AppState,
    id: i64,
    now: DateTime,
) -> std::result::Result<AccountChange, TransitionError> {
    lapse_due(state, now).await?;
    let change = load(state, id).await?;
    let name = match change.status {
        AccountChangeStatus::Pending => CONFIRM,
        AccountChangeStatus::Approved => CONFIRM_APPROVED,
        // Opened again: it already counts.
        AccountChangeStatus::Confirmed | AccountChangeStatus::Active => return Ok(change),
        _ => return Err(guard(english("logistics.account.link_dead", &[]))),
    };
    let _outcome = change.transition(name).by_system().run(state).await?;
    load(state, id).await
}

/// Registers the daily lapse of the changes that waited too long.
pub(crate) fn register(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            lapse_due(&state, renox::db::now())
                .await
                .map(|_| ())
                .map_err(TransitionError::into_error)
        })
        .timezone("Asia/Dubai");
}

/// The routes: the client's confirmation link (public, signed) and the
/// requests of the staff and of a client user.
pub fn routes() -> Routes {
    Routes::new()
        .get("/remittances/accounts/{id}/confirm", confirm_link)
        .name(CONFIRM_ROUTE)
}

/// `GET /remittances/accounts/{id}/confirm`: the signed link; opening it
/// confirms the change. An altered or expired signature answers 403.
async fn confirm_link(
    _signature: ValidSignature,
    State(state): State<AppState>,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    confirm(&state, id, renox::db::now())
        .await
        .map_err(TransitionError::into_error)?;
    Ok(renox::axum::response::Html(t(&lang, "logistics.account.confirmed_page")).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_iban_is_normalised_and_checked_for_its_shape() {
        assert_eq!(normalized(" ae07 0331 2345 6789 0123 456 "), "AE070331234567890123456");
        assert!(plausible("AE070331234567890123456"));
        assert!(!plausible("AE07"));
        assert!(!plausible("1207033123456789012345X"));
        assert!(!plausible("AE0703312345678901234-6"));
    }

    #[test]
    fn the_machine_has_seven_states_and_its_transitions() {
        let machine = AccountChange::machine();
        assert_eq!(machine.states().len(), 7);
        for name in [
            APPROVE,
            APPROVE_CONFIRMED,
            CONFIRM,
            CONFIRM_APPROVED,
            REJECT,
            WITHDRAW,
            LAPSE,
        ] {
            assert!(machine.find(name).is_ok(), "{name}");
        }
        assert!(machine.find(CONFIRM).unwrap().is_system());
        assert!(machine.find(LAPSE).unwrap().is_system());
        assert!(!machine.find(APPROVE).unwrap().is_system());
        assert_eq!(machine.find(CONFIRM_APPROVED).unwrap().to_state(), Some("active"));
    }
}
