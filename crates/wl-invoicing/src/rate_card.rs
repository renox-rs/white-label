//! Rate cards (wl-invoicing §3 *Rate card*, *Rate card line*, `FIN.RAT.01`;
//! 09 Manpower `MPW.ACL.16`): the prices of one source contract, in versions.
//!
//! A rate card belongs to one source contract (`manpower.client_contract:4`).
//! A change of price is never an edit: it is a new [`RateCardVersion`] with
//! an effective date that the client accepts. Only an `accepted` version
//! prices anything, from its effective date until the day before the next
//! accepted version; the lines of a version are edited only while it is
//! `pending`.
//!
//! ```text
//! [*] --> pending --accept--> accepted                           (not dated in the past)
//!         pending --accept_past_dated--> awaiting_approval       (dated in the past)
//!         awaiting_approval --accept_past_dated_approved--> accepted
//!         awaiting_approval --back_to_draft--> rejected          (no, withdrawn, expired)
//! ```
//!
//! All four transitions are the system's: [`crate::rate_cards::accept`]
//! checks who may accept (the client in the portal, `DOC.SIG.01`, or staff
//! recording the signed acceptance) and runs the right one. A version is
//! *dated in the past* when its effective date is before the day it was made
//! (Asia/Dubai); the first version of a card is never dated in the past, it
//! is a start, not a change. `accept_past_dated` asks the holders of
//! `rate_cards.approve` (the MD) for approval, never the user who accepted
//! (SoD); until the last yes the version prices nothing, and on a no it is
//! `rejected` and never used. Two versions of a card never share an
//! effective date (a rejected one does not count).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{self, PolicyDefault, Require, Required};
use wl_kernel::machines::SubjectMachine;
use wl_kernel::{PartyId, Subject};

use crate::budget_category::refused;
use crate::permissions;
use crate::texts::english;

/// The subject type of a rate card version in the kernel registry.
pub const VERSION_SUBJECT_TYPE: &str = "invoicing.rate_card_version";

/// The approval policies' action key of a version dated in the past
/// (`MPW.ACL.16`).
pub const ACTION: &str = "invoicing.rate_card_past_dated";

/// Hours an approval request may wait before it can expire.
pub const EXPIRY_HOURS: i64 = 168;

/// `pending → accepted`.
pub const ACCEPT: &str = "accept";
/// `pending → awaiting_approval`.
pub const ACCEPT_PAST_DATED: &str = "accept_past_dated";
/// `awaiting_approval → accepted`: the kernel runs it on the last yes.
pub const ACCEPT_PAST_DATED_APPROVED: &str = "accept_past_dated_approved";
/// `awaiting_approval → rejected`: the kernel runs it when the request ends
/// without a yes.
pub const BACK_TO_DRAFT: &str = "back_to_draft";

/// The rates of one source contract.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_rate_cards",
    unique(source_type, source_id),
    index(party_id)
)]
pub struct RateCard {
    /// The primary key.
    pub id: i64,
    /// Who is billed.
    pub party_id: i64,
    /// The type of the contract the card prices.
    pub source_type: String,
    /// The id of that contract.
    pub source_id: i64,
    /// The ISO currency code.
    pub currency: String,
    /// The user who made it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl RateCard {
    /// The contract the card prices, `manpower.client_contract:4`.
    pub fn source(&self) -> Subject {
        Subject::new(&self.source_type, self.source_id)
    }

    /// Who is billed.
    pub fn party(&self) -> PartyId {
        PartyId(self.party_id)
    }

    /// The card of `source`, if it has one.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_source(
        db: impl renox::db::Executor<'_>,
        source: &Subject,
    ) -> Result<Option<RateCard>> {
        RateCard::query()
            .where_(RateCard::SOURCE_TYPE.eq(source.subject_type.as_str()))
            .where_(RateCard::SOURCE_ID.eq(source.subject_id))
            .first(db)
            .await
    }
}

/// Where a version stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VersionStatus {
    /// Waiting for the client to accept; prices nothing.
    #[default]
    Pending,
    /// Accepted by the client but dated in the past: waiting for the MD;
    /// prices nothing.
    AwaitingApproval,
    /// In force from its effective date.
    Accepted,
    /// The approval was refused or ended without a yes; never used.
    Rejected,
}

/// A set of lines with an effective date.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_rate_card_versions",
    state_machine,
    index(rate_card_id),
    index(status)
)]
pub struct RateCardVersion {
    /// The primary key.
    pub id: i64,
    /// The card.
    pub rate_card_id: i64,
    /// 1, 2, 3 ... per card.
    pub version: i64,
    /// The first day the version prices.
    pub effective_from: NaiveDate,
    /// The day it was made (Asia/Dubai).
    pub submitted_on: NaiveDate,
    /// Whether it changes prices from before the day it was made.
    pub past_dated: bool,
    /// The lifecycle state; changed only by a transition.
    pub status: VersionStatus,
    /// The user who accepted for the client (a portal user, or the staff
    /// member who recorded the signed acceptance).
    pub accepted_by: Option<i64>,
    /// The day it was accepted.
    pub accepted_on: Option<NaiveDate>,
    /// The signed acceptance staff recorded for the client.
    pub accepted_document: Option<String>,
    /// The approval request of a version dated in the past.
    pub approval_request_id: Option<i64>,
    /// The user who gave the last yes.
    pub approved_by: Option<i64>,
    /// Why it was not approved.
    pub rejected_reason: Option<String>,
    /// The user who made it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl RateCardVersion {
    /// The version as a [`Subject`]: `invoicing.rate_card_version:4`.
    pub fn subject(&self) -> Subject {
        Subject::new(VERSION_SUBJECT_TYPE, self.id)
    }

    /// The lines of the version, in the order they were added.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn lines(&self, db: impl renox::db::Executor<'_>) -> Result<Vec<RateCardLine>> {
        RateCardLine::query()
            .where_(RateCardLine::VERSION_ID.eq(self.id))
            .order_by(RateCardLine::ID)
            .get(db)
            .await
    }

    /// The versions of card `card_id`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_card(
        db: impl renox::db::Executor<'_>,
        card_id: i64,
    ) -> Result<Vec<RateCardVersion>> {
        RateCardVersion::query()
            .where_(RateCardVersion::RATE_CARD_ID.eq(card_id))
            .order_by(RateCardVersion::VERSION)
            .get(db)
            .await
    }
}

/// What a line is priced per.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Unit {
    /// Per hour worked.
    #[default]
    Hour,
    /// Per day worked.
    Day,
    /// Per calendar month, pro rata by days when the version changes inside
    /// it.
    Month,
}

/// One bill rate of a version: a role on a shift type.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_invoicing_rate_card_lines", index(version_id))]
pub struct RateCardLine {
    /// The primary key.
    pub id: i64,
    /// The version.
    pub version_id: i64,
    /// The role, lower case.
    pub role: String,
    /// The shift type, lower case.
    pub shift_type: String,
    /// What the rate is per.
    pub unit: Unit,
    /// The bill rate of one unit, in fils.
    pub rate: i64,
    /// The overtime multiplier in hundredths: 125 is 1.25.
    pub ot_multiplier: i64,
    /// The public holiday multiplier in hundredths.
    pub holiday_multiplier: i64,
    /// The minimum hours billed per shift, in thousandths of an hour.
    pub min_hours_milli: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// What the machine's `accept` runs carry: who accepted, and the signed
/// acceptance staff recorded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AcceptInput {
    /// The accepting user.
    pub by: i64,
    /// The signed acceptance staff recorded, if staff accepted.
    pub document: Option<String>,
}

fn input(ctx: &TransitionCtx) -> Option<AcceptInput> {
    ctx.input_as::<AcceptInput>().ok()
}

/// Writes columns other than the status, in the transition's transaction.
async fn record(
    tx: &mut renox::db::Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    let stamp = renox::db::now();
    let mut all: Vec<(&str, &(dyn renox::db::ToDbValue + Sync))> = columns.to_vec();
    all.push(("updated_at", &stamp));
    RateCardVersion::query()
        .where_(RateCardVersion::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

fn accept_transition() -> Transition<RateCardVersion> {
    Transition::new(ACCEPT)
        .label("Accept")
        .from([VersionStatus::Pending])
        .to(VersionStatus::Accepted)
        .system()
        .guard("not_past_dated", |version: &RateCardVersion, _ctx| {
            let past = version.past_dated;
            Box::pin(async move {
                if past {
                    Err(english("wl_invoicing.rate_card.problem.past_dated").to_owned())
                } else {
                    Ok(())
                }
            })
        })
        .effect(|version: &RateCardVersion, ctx, tx| {
            let id = version.id;
            let given = input(ctx).unwrap_or_default();
            Box::pin(async move {
                record(
                    tx,
                    id,
                    &[
                        ("accepted_by", &Some(given.by)),
                        ("accepted_on", &Some(crate::rate_cards::today())),
                        ("accepted_document", &given.document),
                    ],
                )
                .await
            })
        })
}

fn accept_past_dated_transition() -> Transition<RateCardVersion> {
    Transition::new(ACCEPT_PAST_DATED)
        .label("Accept, for approval")
        .from([VersionStatus::Pending])
        .to(VersionStatus::AwaitingApproval)
        .system()
        .guard("is_past_dated", |version: &RateCardVersion, _ctx| {
            let past = version.past_dated;
            Box::pin(async move {
                if past {
                    Ok(())
                } else {
                    Err(english("wl_invoicing.rate_card.problem.not_past_dated").to_owned())
                }
            })
        })
        .effect(|version: &RateCardVersion, ctx, tx| {
            let id = version.id;
            let subject = version.subject();
            let given = input(ctx).unwrap_or_default();
            Box::pin(async move {
                // The kernel runs `back_to_<state before>` when the request ends
                // without a yes; like the drawing approval, the state before is
                // named `draft` so that it is `back_to_draft`.
                let require = Require::new(ACTION, subject, ACCEPT_PAST_DATED, given.by, "draft")
                    .payload(json!({ "version": id }));
                let request = match approvals::require_in(&mut *tx, &require).await? {
                    Required::Pending(request) => request,
                    // Without the policy a version dated in the past would
                    // price unapproved; the transaction rolls back instead.
                    Required::Allowed => {
                        return Err(refused(english(
                            "wl_invoicing.rate_card.problem.no_approval_policy",
                        )));
                    }
                };
                record(
                    tx,
                    id,
                    &[
                        ("accepted_by", &Some(given.by)),
                        ("accepted_on", &Some(crate::rate_cards::today())),
                        ("accepted_document", &given.document),
                        ("approval_request_id", &Some(request.id)),
                    ],
                )
                .await
            })
        })
}

fn approved_transition() -> Transition<RateCardVersion> {
    Transition::new(ACCEPT_PAST_DATED_APPROVED)
        .label("Approve")
        .from([VersionStatus::AwaitingApproval])
        .to(VersionStatus::Accepted)
        .system()
        .effect(|version: &RateCardVersion, ctx, tx| {
            let id = version.id;
            let approver = ctx.meta().get("actor_id").and_then(Value::as_i64);
            Box::pin(async move { record(tx, id, &[("approved_by", &approver)]).await })
        })
}

fn rejected_transition() -> Transition<RateCardVersion> {
    Transition::new(BACK_TO_DRAFT)
        .label("Not approved")
        .from([VersionStatus::AwaitingApproval])
        .to(VersionStatus::Rejected)
        .system()
        .effect(|version: &RateCardVersion, ctx, tx| {
            let id = version.id;
            let reason = ctx
                .reason()
                .map(|r| r.trim().to_owned())
                .filter(|r| !r.is_empty());
            Box::pin(async move { record(tx, id, &[("rejected_reason", &reason)]).await })
        })
}

impl StateMachine for RateCardVersion {
    type State = VersionStatus;
    const COLUMN: renox::db::Col<RateCardVersion, VersionStatus> = RateCardVersion::STATUS;

    fn machine() -> Machine<Self> {
        use VersionStatus::{Accepted, AwaitingApproval, Pending, Rejected};
        Machine::new()
            .initial([Pending])
            .state(Pending)
            .tone("neutral")
            .state(AwaitingApproval)
            .tone("warning")
            .state(Accepted)
            .tone("success")
            .terminal()
            .state(Rejected)
            .tone("error")
            .terminal()
            .transition(accept_transition())
            .transition(accept_past_dated_transition())
            .transition(approved_transition())
            .transition(rejected_transition())
    }
}

/// The default policy: a version dated in the past needs one approval by a
/// holder of `rate_cards.approve` other than the user who accepted it.
pub fn approval_policy() -> PolicyDefault {
    PolicyDefault::new(ACTION)
        .step_permission(permissions::RATE_CARDS_APPROVE)
        .expiry_hours(Some(EXPIRY_HOURS))
}

/// The page of a version, for the approvers' "Needs your action" list.
pub(crate) fn page_of(id: i64) -> String {
    format!("/rate-card-versions/{id}")
}

/// Registers the machine with the kernel (waiting state `awaiting_approval`),
/// the default policy and the page for the approvers.
pub(crate) fn register() {
    SubjectMachine::of::<RateCardVersion>(VERSION_SUBJECT_TYPE)
        .waiting(VersionStatus::AwaitingApproval)
        .register();
    renox::state_machine::register::<RateCardVersion>();
    approval_policy().register();
    approvals::register_subject_link(VERSION_SUBJECT_TYPE, page_of);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_the_four_system_transitions() {
        let machine = RateCardVersion::machine();
        assert_eq!(machine.initial_states(), ["pending"]);
        let find = |name: &str| machine.find(name).expect("transition");
        assert_eq!(find(ACCEPT).from_states(), ["pending"]);
        assert_eq!(find(ACCEPT).to_state(), Some("accepted"));
        assert_eq!(
            find(ACCEPT_PAST_DATED).to_state(),
            Some("awaiting_approval")
        );
        assert_eq!(
            find(ACCEPT_PAST_DATED_APPROVED).from_states(),
            ["awaiting_approval"]
        );
        assert_eq!(
            find(ACCEPT_PAST_DATED_APPROVED).to_state(),
            Some("accepted")
        );
        assert_eq!(find(BACK_TO_DRAFT).to_state(), Some("rejected"));
        assert!(machine.transitions().iter().all(|t| t.is_system()));
    }

    #[test]
    fn the_policy_asks_the_holders_of_rate_cards_approve() {
        let expected = PolicyDefault::new("invoicing.rate_card_past_dated")
            .step_permission("rate_cards.approve")
            .expiry_hours(Some(168));
        assert_eq!(approval_policy(), expected);
        assert_eq!(page_of(4), "/rate-card-versions/4");
    }
}
