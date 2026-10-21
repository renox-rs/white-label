//! The Payment link (wl-payments §3, §4): a request to pay online with an
//! expiry. The party opens it on a public page behind a sealed token
//! ([`token`]) and pays on the gateway's hosted page; the link stays `open`
//! when a payment on it fails, so the party can try again, until it is paid,
//! expires or staff cancel it.
//!
//! ```text
//! [*] --> open --pay--> paid (terminal)
//! open --expire--> expired (terminal)
//! open --cancel--> cancelled (terminal)
//! open --withdraw--> cancelled (terminal)
//! ```
//!
//! `pay` is the system's, run when the online payment on it is confirmed.
//! `expire` is the system's too, run by the hourly task [`TASK`] once the
//! expiry passed; the listener then emits `PaymentLinkExpired`, so
//! wl-scheduling can release an unpaid booking hold. `cancel` is for staff
//! with `payments.links`; `withdraw` is the system's, run through the kernel's
//! `PaymentLinks::cancel` when a module releases what the link was for.

use renox::chrono::TimeDelta;
use renox::db::Json;
use renox::prelude::*;
use renox::state_machine::Transitioned;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::payments::{
    PAYMENT_LINK_SUBJECT_TYPE, PaymentFuture, PaymentLinkExpired, PaymentLinkRef,
    PaymentLinkRequest, PaymentLinks,
};
use wl_kernel::{LegalEntity, PartyId, Subject};

use crate::config_of;
use crate::gateway::installed_gateway;
use crate::permissions;

/// The `pay` transition's name.
pub const PAY: &str = "pay";

/// The `expire` transition's name.
pub const EXPIRE: &str = "expire";

/// The `cancel` transition's name.
pub const CANCEL: &str = "cancel";

/// The `withdraw` transition's name.
pub const WITHDRAW: &str = "withdraw";

/// The scheduled task and command that expire the links.
pub const TASK: &str = "payments:expire_links";

/// The lifecycle state of a payment link (wl-payments §4); the state of the
/// Payment link machine.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LinkStatus {
    /// Sent; can be paid.
    #[default]
    Open,
    /// Paid in full (terminal).
    Paid,
    /// The expiry passed (terminal).
    Expired,
    /// Withdrawn by staff (terminal).
    Cancelled,
}

impl LinkStatus {
    /// Whether the link can still be paid.
    pub fn is_open(self) -> bool {
        self == LinkStatus::Open
    }
}

/// A document a link pays, with the part of the link's amount that goes to it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct LinkTarget {
    /// The type of the target, e.g. `invoicing.invoice`.
    pub target_type: String,
    /// The id of the target.
    pub target_id: i64,
    /// The amount to allocate to it, in fils.
    pub amount: i64,
}

impl LinkTarget {
    /// A target `subject` that takes `amount` fils.
    pub fn new(subject: &Subject, amount: i64) -> Self {
        LinkTarget {
            target_type: subject.subject_type.clone(),
            target_id: subject.subject_id,
            amount,
        }
    }

    /// The target as a [`Subject`].
    pub fn subject(&self) -> Subject {
        Subject {
            subject_type: self.target_type.clone(),
            subject_id: self.target_id,
        }
    }
}

/// A payment link. Money is `i64` fils of [`PaymentLink::currency`].
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_payments_payment_links",
    state_machine,
    index(status),
    index(prepaid_type, prepaid_id)
)]
pub struct PaymentLink {
    /// The primary key.
    pub id: i64,
    /// Who is asked to pay.
    pub party_id: i64,
    /// The legal entity whose books it belongs to.
    pub legal_entity_id: i64,
    /// The amount, in fils; always > 0.
    pub amount: i64,
    /// The ISO currency code, e.g. `AED`.
    pub currency: String,
    /// The documents it pays; empty for a link for an amount.
    pub targets: Json<Vec<LinkTarget>>,
    /// The type of the record paid in advance for.
    pub prepaid_type: Option<String>,
    /// The id of the record paid in advance for.
    pub prepaid_id: Option<i64>,
    /// The subscription it is a charge for.
    pub subscription_id: Option<i64>,
    /// The account key of the gateway the party pays through.
    pub gateway: String,
    /// When it expires.
    pub expires_at: DateTime,
    /// The payment that paid it.
    pub paid_payment_id: Option<i64>,
    /// The staff user who sent it: the one who took the payment.
    pub created_by: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: LinkStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PaymentLink {
    /// The link as a [`Subject`]: `payments.payment_link:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(PAYMENT_LINK_SUBJECT_TYPE, self.id)
    }

    /// Who is asked to pay.
    pub fn party(&self) -> PartyId {
        PartyId(self.party_id)
    }

    /// The record paid in advance for, if any.
    pub fn prepaid(&self) -> Option<Subject> {
        match (&self.prepaid_type, self.prepaid_id) {
            (Some(subject_type), Some(subject_id)) => Some(Subject {
                subject_type: subject_type.clone(),
                subject_id,
            }),
            _ => None,
        }
    }
}

/// What a run carries to its guards and effects.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct LinkInput {
    /// The payment that paid the link (`pay`).
    pub payment_id: Option<i64>,
    /// The moment the rule runs at (`expire`).
    pub at: Option<DateTime>,
}

fn input(ctx: &TransitionCtx) -> Option<LinkInput> {
    ctx.input_as::<LinkInput>().ok()
}

impl StateMachine for PaymentLink {
    type State = LinkStatus;
    const COLUMN: renox::db::Col<PaymentLink, LinkStatus> = PaymentLink::STATUS;

    fn machine() -> Machine<Self> {
        use LinkStatus::{Cancelled, Expired, Open, Paid};
        Machine::new()
            .initial([Open])
            .state(Open)
            .tone("info")
            .state(Paid)
            .tone("success")
            .terminal()
            .state(Expired)
            .tone("neutral")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(pay())
            .transition(expire())
            .transition(cancel())
            .transition(withdraw())
    }
}

/// `pay`: the online payment on the link was confirmed.
fn pay() -> Transition<PaymentLink> {
    Transition::new(PAY)
        .label("Paid")
        .tone("success")
        .system()
        .from([LinkStatus::Open])
        .to(LinkStatus::Paid)
        .effect(|link: &PaymentLink, ctx, tx| {
            let id = link.id;
            let payment = input(ctx).and_then(|i| i.payment_id);
            Box::pin(async move {
                PaymentLink::query()
                    .where_(PaymentLink::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("paid_payment_id", &payment),
                            ("updated_at", &renox::db::now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// `expire`: the expiry passed.
fn expire() -> Transition<PaymentLink> {
    Transition::new(EXPIRE)
        .label("Expire")
        .tone("neutral")
        .system()
        .from([LinkStatus::Open])
        .to(LinkStatus::Expired)
        .guard("expiry_passed", |link: &PaymentLink, ctx| {
            let expires_at = link.expires_at;
            let at = input(ctx).and_then(|i| i.at);
            Box::pin(async move {
                match at {
                    Some(at) if at >= expires_at => Ok(()),
                    _ => Err("The link has not expired yet.".to_owned()),
                }
            })
        })
}

/// `cancel`: staff withdraw the link.
fn cancel() -> Transition<PaymentLink> {
    Transition::new(CANCEL)
        .label("Cancel")
        .tone("danger")
        .permission(permissions::LINKS)
        .from([LinkStatus::Open])
        .to(LinkStatus::Cancelled)
}

/// `withdraw`: the system closes the link because the thing it asked for is
/// gone (wl-scheduling released an unpaid booking hold, `SCH.RUL.04`).
/// Decision: a separate system transition, as `cancel` is staff's and a
/// system run may only use `system` transitions; both end `cancelled`.
fn withdraw() -> Transition<PaymentLink> {
    Transition::new(WITHDRAW)
        .label("Withdraw")
        .tone("neutral")
        .system()
        .from([LinkStatus::Open])
        .to(LinkStatus::Cancelled)
}

/// A link to make.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NewLink {
    /// Who is asked to pay.
    pub party: PartyId,
    /// The legal entity whose books it belongs to.
    pub legal_entity_id: i64,
    /// The amount, in fils; above 0.
    pub amount: i64,
    /// The ISO currency code, e.g. `AED`.
    pub currency: String,
    /// The documents it pays; their amounts must not exceed the link's.
    pub targets: Vec<LinkTarget>,
    /// The record paid in advance for, if any.
    pub prepaid: Option<Subject>,
    /// The subscription it is a charge for, if any.
    pub subscription_id: Option<i64>,
    /// When it expires; `None` for the module's setting (7 days) from now.
    pub expires_at: Option<DateTime>,
    /// The staff user who sends it.
    pub created_by: Option<i64>,
}

impl NewLink {
    /// A link asking `party` of `legal_entity_id` for `amount` fils of
    /// `currency`.
    pub fn new(party: PartyId, legal_entity_id: i64, amount: i64, currency: &str) -> Self {
        NewLink {
            party,
            legal_entity_id,
            amount,
            currency: currency.trim().to_ascii_uppercase(),
            targets: Vec::new(),
            prepaid: None,
            subscription_id: None,
            expires_at: None,
            created_by: None,
        }
    }

    /// Paying `target` with `amount` fils of it.
    pub fn target(mut self, target: &Subject, amount: i64) -> Self {
        self.targets.push(LinkTarget::new(target, amount));
        self
    }

    /// Paid in advance for `prepaid`.
    pub fn prepaid(mut self, prepaid: Subject) -> Self {
        self.prepaid = Some(prepaid);
        self
    }

    /// Expiring at `at`.
    pub fn expires_at(mut self, at: DateTime) -> Self {
        self.expires_at = Some(at);
        self
    }

    /// Sent by the staff user `user`.
    pub fn created_by(mut self, user: i64) -> Self {
        self.created_by = Some(user);
        self
    }
}

/// Why `new` cannot be made, if it cannot (what needs no database).
pub(crate) fn link_problem(new: &NewLink) -> Option<String> {
    if new.amount <= 0 {
        return Some("The amount must be above 0.".to_owned());
    }
    if new.targets.iter().any(|t| t.amount <= 0) {
        return Some("A document takes an amount above 0.".to_owned());
    }
    let targeted: i64 = new.targets.iter().map(|t| t.amount).sum();
    if targeted > new.amount {
        return Some("The documents take more than the link asks.".to_owned());
    }
    None
}

/// Makes the link `new` `open`, in one transaction.
///
/// # Errors
///
/// `Guard` (422) for an amount of 0 or less, documents that take more than
/// the link or an amount of 0 or less, an unknown party, an inactive legal
/// entity, an expiry in the past, and while no gateway is installed (online
/// payment is off).
pub async fn create(
    state: &AppState,
    new: NewLink,
) -> std::result::Result<PaymentLink, TransitionError> {
    if let Some(problem) = link_problem(&new) {
        return Err(TransitionError::Guard(problem));
    }
    let Some(gateway) = installed_gateway(state) else {
        return Err(TransitionError::Guard(
            "Online payment is not available: no gateway is installed.".to_owned(),
        ));
    };
    let entity = LegalEntity::find(&state.db, new.legal_entity_id).await?;
    if !entity.as_ref().is_some_and(LegalEntity::is_active) {
        return Err(TransitionError::Guard(
            "The legal entity is not active.".to_owned(),
        ));
    }
    let party = wl_kernel::parties::directory(state)
        .find(&state.db, new.party)
        .await?;
    if party.is_none() {
        return Err(TransitionError::Guard(
            "The party was not found.".to_owned(),
        ));
    }
    let now = renox::db::now();
    let expires_at = new
        .expires_at
        .unwrap_or_else(|| now + TimeDelta::days(config_of(state).link_expiry_days));
    if expires_at <= now {
        return Err(TransitionError::Guard(
            "The expiry must be in the future.".to_owned(),
        ));
    }
    let mut link = PaymentLink {
        party_id: new.party.0,
        legal_entity_id: new.legal_entity_id,
        amount: new.amount,
        currency: new.currency.clone(),
        targets: Json(new.targets.clone()),
        prepaid_type: new.prepaid.as_ref().map(|p| p.subject_type.clone()),
        prepaid_id: new.prepaid.as_ref().map(|p| p.subject_id),
        subscription_id: new.subscription_id,
        gateway: gateway.account().to_owned(),
        expires_at,
        created_by: new.created_by,
        status: LinkStatus::Open,
        ..Default::default()
    };
    let mut tx = state.db.begin().await?;
    link.insert_in_state(&mut tx, LinkStatus::Open, "created")
        .await?;
    tx.commit().await?;
    Ok(link)
}

/// Expires every `open` link whose expiry is at or before `at`; the
/// listener then emits `PaymentLinkExpired` for each. Returns how many
/// expired. A link that was paid or cancelled in the meantime is skipped.
///
/// # Errors
///
/// The database's errors.
pub async fn expire_due(state: &AppState, at: DateTime) -> Result<usize> {
    let due = PaymentLink::query()
        .where_(PaymentLink::STATUS.eq(LinkStatus::Open))
        .where_(PaymentLink::EXPIRES_AT.lte(at))
        .order_by(PaymentLink::ID)
        .get(&state.db)
        .await?;
    let mut expired = 0;
    for link in due {
        let run = LinkInput {
            payment_id: None,
            at: Some(at),
        };
        match link
            .transition(EXPIRE)
            .by_system()
            .input(&run)
            .run(state)
            .await
        {
            Ok(_) => expired += 1,
            Err(TransitionError::NotFrom(_) | TransitionError::Guard(_)) => {}
            Err(other) => return Err(other.into_error()),
        }
    }
    Ok(expired)
}

/// Marks `link` paid by `payment_id`. A link that is no longer open (it
/// expired or was cancelled while the payment was on its way) stays as it is.
///
/// # Errors
///
/// A failure other than the link not being open.
pub(crate) async fn mark_paid(state: &AppState, link_id: i64, payment_id: i64) -> Result {
    let Some(link) = PaymentLink::find(&state.db, link_id).await? else {
        return Ok(());
    };
    let run = LinkInput {
        payment_id: Some(payment_id),
        at: None,
    };
    match link
        .transition(PAY)
        .by_system()
        .input(&run)
        .run(state)
        .await
    {
        Ok(_) | Err(TransitionError::NotFrom(_)) => Ok(()),
        Err(other) => Err(other.into_error()),
    }
}

/// The prefix sealed into a link's token.
const PREFIX: &str = "pay:";

/// The route name of the public page.
pub const ROUTE: &str = "wl_payments.pay";

/// The token in the URL of link `id`: URL safe, sealed with the app key, so
/// a changed token opens nothing ([`id_of`]).
pub fn token(state: &AppState, id: i64) -> String {
    state
        .encrypt(&format!("{PREFIX}{id}"))
        .bytes()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The link id `token` was made for; `None` for anything else.
pub fn id_of(state: &AppState, token: &str) -> Option<i64> {
    if token.is_empty() || !token.len().is_multiple_of(2) || !token.is_ascii() {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..token.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&token[i..i + 2], 16).ok())
        .collect();
    let sealed = String::from_utf8(bytes?).ok()?;
    let plain = state.decrypt(&sealed).ok()?;
    plain.strip_prefix(PREFIX)?.parse().ok()
}

/// The path of the public page of link `id`, e.g. `/pay/ab12…`.
pub fn path(state: &AppState, id: i64) -> String {
    format!("/pay/{}", token(state, id))
}

/// The links other modules ask for through the kernel's
/// [`PaymentLinks`]: wl-scheduling's prepayment of a self-booking, expiring
/// when the hold does. Decision: without a gateway there is no online
/// payment, so the answer is `None` and the asking module decides.
pub struct Links;

impl PaymentLinks for Links {
    fn create<'a>(
        &'a self,
        state: &'a AppState,
        request: PaymentLinkRequest,
    ) -> PaymentFuture<'a, Option<PaymentLinkRef>> {
        Box::pin(async move {
            if installed_gateway(state).is_none() {
                return Ok(None);
            }
            let mut new = NewLink::new(
                request.party,
                request.legal_entity_id,
                request.amount,
                &request.currency,
            )
            .prepaid(request.prepaid)
            .expires_at(request.expires_at);
            if let Some(user) = request.created_by {
                new = new.created_by(user);
            }
            let link = create(state, new)
                .await
                .map_err(TransitionError::into_error)?;
            Ok(Some(PaymentLinkRef {
                link: link.subject(),
                path: path(state, link.id),
            }))
        })
    }

    fn cancel<'a>(
        &'a self,
        state: &'a AppState,
        link: &'a Subject,
    ) -> PaymentFuture<'a, bool> {
        Box::pin(async move {
            if link.subject_type != PAYMENT_LINK_SUBJECT_TYPE {
                return Ok(false);
            }
            let Some(found) = PaymentLink::find(&state.db, link.subject_id).await? else {
                return Ok(false);
            };
            match found
                .transition(WITHDRAW)
                .by_system()
                .input(&LinkInput::default())
                .run(state)
                .await
            {
                Ok(_) => Ok(true),
                Err(TransitionError::NotFrom(_) | TransitionError::Conflict) => Ok(false),
                Err(other) => Err(other.into_error()),
            }
        })
    }
}

/// Registers the listener that emits `PaymentLinkExpired` after an `expire`
/// commits, and the hourly task and its command.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: Transitioned<PaymentLink>, state: AppState| async move {
            if event.transition != EXPIRE {
                return Ok(());
            }
            // The committed row.
            let link = PaymentLink::find_or_404(&state.db, event.key).await?;
            state
                .emit(PaymentLinkExpired {
                    payment_link: link.subject(),
                    party: link.party(),
                    amount: link.amount,
                    currency: link.currency.clone(),
                    prepaid: link.prepaid(),
                })
                .await
        },
    );
    app.schedule().hourly(TASK, |state: AppState| async move {
        expire_due(&state, renox::db::now()).await.map(|_| ())
    });
    app.command(
        TASK,
        "Expire the payment links whose expiry has passed",
        |_args, state: AppState| async move {
            let expired = expire_due(&state, renox::db::now()).await?;
            println!("{expired} payment links expired.");
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_has_four_states_and_starts_open() {
        let machine = PaymentLink::machine();
        let names: Vec<_> = machine.states().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["open", "paid", "expired", "cancelled"]);
        assert_eq!(machine.initial_states(), ["open"]);
        assert!(machine.states().iter().all(|s| s.tone.is_some()));
        let terminal: Vec<_> = machine
            .states()
            .iter()
            .filter(|s| s.terminal)
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(terminal, ["paid", "expired", "cancelled"]);
    }

    #[test]
    fn pay_and_expire_are_the_systems_and_cancel_is_staffs() {
        let machine = PaymentLink::machine();
        for (name, system) in [(PAY, true), (EXPIRE, true), (CANCEL, false), (WITHDRAW, true)] {
            let found = machine.find(name).expect("transition");
            assert_eq!(found.is_system(), system, "{name}");
            assert_eq!(found.from_states(), ["open"], "{name}");
        }
        assert_eq!(machine.find(PAY).unwrap().to_state(), Some("paid"));
        assert_eq!(machine.find(EXPIRE).unwrap().to_state(), Some("expired"));
        assert_eq!(machine.find(CANCEL).unwrap().to_state(), Some("cancelled"));
        assert_eq!(machine.find(WITHDRAW).unwrap().to_state(), Some("cancelled"));
    }

    fn link_expiring(at: DateTime) -> PaymentLink {
        PaymentLink {
            expires_at: at,
            ..Default::default()
        }
    }

    fn run_at(at: Option<DateTime>) -> TransitionCtx {
        let input = LinkInput {
            payment_id: None,
            at,
        };
        TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
    }

    #[renox::test]
    async fn a_link_expires_only_at_or_after_its_expiry() {
        let machine = PaymentLink::machine();
        let expire = machine.find(EXPIRE).expect("transition");
        let expiry = renox::db::now();
        let link = link_expiring(expiry);
        assert!(
            expire
                .check_guards(&link, &run_at(Some(expiry)))
                .await
                .is_ok()
        );
        assert!(
            expire
                .check_guards(&link, &run_at(Some(expiry + TimeDelta::minutes(1))))
                .await
                .is_ok()
        );
        for ctx in [
            run_at(Some(expiry - TimeDelta::minutes(1))),
            run_at(None),
            TransitionCtx::new(None),
        ] {
            assert!(matches!(
                expire.check_guards(&link, &ctx).await,
                Err(TransitionError::Guard(_))
            ));
        }
    }

    #[test]
    fn the_documents_of_a_link_cannot_take_more_than_it_asks() {
        let invoice = Subject::new("invoicing.invoice", 9);
        let fine = NewLink::new(PartyId(1), 1, 25_000, "aed").target(&invoice, 25_000);
        assert_eq!(fine.currency, "AED");
        assert_eq!(link_problem(&fine), None);
        let over = NewLink::new(PartyId(1), 1, 25_000, "AED").target(&invoice, 25_001);
        assert!(link_problem(&over).is_some());
        let zero = NewLink::new(PartyId(1), 1, 0, "AED");
        assert!(link_problem(&zero).is_some());
        let none = NewLink::new(PartyId(1), 1, 100, "AED").target(&invoice, 0);
        assert!(link_problem(&none).is_some());
    }

    #[test]
    fn a_target_round_trips_through_its_subject() {
        let subject = Subject::new("invoicing.invoice", 9);
        let target = LinkTarget::new(&subject, 500);
        assert_eq!(target.subject(), subject);
        let link = PaymentLink {
            id: 4,
            prepaid_type: Some("scheduling.booking".into()),
            prepaid_id: Some(3),
            ..Default::default()
        };
        assert_eq!(link.subject().to_string(), "payments.payment_link:4");
        assert_eq!(link.prepaid(), Some(Subject::new("scheduling.booking", 3)));
        assert!(LinkStatus::Open.is_open());
        assert!(!LinkStatus::Paid.is_open());
    }
}
