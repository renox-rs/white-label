//! The rate card services (wl-invoicing §3 *Rate card*, `FIN.RAT.01`;
//! 09 Manpower `MPW.ACL.16`): make a card, make a new version of it, keep
//! the lines of a version while it is `pending`, accept it for the client and
//! put it in force.
//!
//! - **A change is a new version.** [`new_version`] numbers the version,
//!   refuses an effective date another version of the card already has
//!   (rejected ones do not count) and marks it *dated in the past* when it is
//!   before today (Asia/Dubai), except for the first version of a card. It may
//!   start as a copy of the latest accepted version, so the user changes only
//!   what changed.
//! - **Lines of an accepted version are never edited.** [`add_line`],
//!   [`edit_line`] and [`remove_line`] work on a `pending` version only; the
//!   refusal for any other offers *new version*.
//! - **Acceptance** ([`accept`], `DOC.SIG.01`) is the client's: a portal user
//!   acting for the card's party with the role `owner` or `approver`, or a
//!   member of staff with `rate_cards.manage` recording the signed acceptance
//!   (the document is required). A version dated in the past waits for the
//!   approval of a holder of `rate_cards.approve` after that; until then it
//!   prices nothing (decision: the MD is asked after the client's
//!   acceptance, as the spec says, and the approval request is created by
//!   the acceptance).
//! - **Putting it in force** ([`listen`]): once a version is `accepted`
//!   (either way), the unbilled hours worked from its effective date that no
//!   `locked` timesheet holds are repriced
//!   ([`crate::rate_pricing::reprice`]). Invoiced hours and locked
//!   timesheets keep the price they had.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError, Transitioned};
use wl_kernel::portal::portal_users;
use wl_kernel::{PartyId, Subject};

use crate::budget_category::refused;
use crate::permissions;
use crate::rate_card::{
    ACCEPT, ACCEPT_PAST_DATED, ACCEPT_PAST_DATED_APPROVED, AcceptInput, RateCard, RateCardLine,
    RateCardVersion, Unit, VersionStatus,
};
use crate::texts::english;

/// The portal roles that accept a version for the client company.
pub const ACCEPT_PORTAL_ROLES: &[&str] = &["owner", "approver"];

/// Today in Asia/Dubai.
pub fn today() -> NaiveDate {
    wl_kernel::documents::today()
}

/// What a line is made of. Rates are in fils, multipliers in hundredths
/// (125 is 1.25), minimum hours in thousandths of an hour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineInput {
    /// The role.
    pub role: String,
    /// The shift type.
    pub shift_type: String,
    /// What the rate is per.
    pub unit: Unit,
    /// The bill rate of one unit, in fils.
    pub rate: i64,
    /// The overtime multiplier in hundredths; 100 is none.
    pub ot_multiplier: i64,
    /// The public holiday multiplier in hundredths; 100 is none.
    pub holiday_multiplier: i64,
    /// The minimum hours billed per shift, in thousandths of an hour.
    pub min_hours_milli: i64,
}

impl LineInput {
    /// A line for `role` on `shift_type` at `rate` fils per `unit`, with no
    /// multiplier and no minimum.
    pub fn new(role: &str, shift_type: &str, unit: Unit, rate: i64) -> Self {
        LineInput {
            role: role.to_owned(),
            shift_type: shift_type.to_owned(),
            unit,
            rate,
            ot_multiplier: 100,
            holiday_multiplier: 100,
            min_hours_milli: 0,
        }
    }

    /// With the overtime multiplier `hundredths` (125 is 1.25).
    pub fn overtime(mut self, hundredths: i64) -> Self {
        self.ot_multiplier = hundredths;
        self
    }

    /// With the public holiday multiplier `hundredths`.
    pub fn holiday(mut self, hundredths: i64) -> Self {
        self.holiday_multiplier = hundredths;
        self
    }

    /// With a minimum of `milli` thousandths of an hour per shift.
    pub fn minimum_hours_milli(mut self, milli: i64) -> Self {
        self.min_hours_milli = milli;
        self
    }

    fn check(&self) -> Result {
        if self.role.trim().is_empty() || self.shift_type.trim().is_empty() {
            return Err(refused(english(
                "wl_invoicing.rate_card.problem.line_fields",
            )));
        }
        if self.rate < 0
            || self.ot_multiplier < 100
            || self.holiday_multiplier < 100
            || self.min_hours_milli < 0
        {
            return Err(refused(english(
                "wl_invoicing.rate_card.problem.line_amounts",
            )));
        }
        Ok(())
    }
}

/// Makes the rate card of `source` for `party`.
///
/// # Errors
///
/// 422 when the source already has a card.
pub async fn create_card(
    db: &Db,
    created_by: Option<i64>,
    party: PartyId,
    source: &Subject,
) -> Result<RateCard> {
    if RateCard::of_source(db, source).await?.is_some() {
        return Err(refused(english(
            "wl_invoicing.rate_card.problem.card_exists",
        )));
    }
    RateCard::create(
        db,
        RateCard {
            party_id: party.0,
            source_type: source.subject_type.clone(),
            source_id: source.subject_id,
            currency: "AED".to_owned(),
            created_by,
            ..RateCard::default()
        },
    )
    .await
}

/// Makes the next version of card `card_id`, `pending`, effective from
/// `effective_from`. With `copy_latest` it starts with the lines of the latest
/// accepted version.
///
/// # Errors
///
/// 422 when another version of the card (not a rejected one) has the same
/// effective date; 404 for an unknown card.
pub async fn new_version(
    db: &Db,
    created_by: Option<i64>,
    card_id: i64,
    effective_from: NaiveDate,
    copy_latest: bool,
) -> Result<RateCardVersion> {
    let card = RateCard::find_or_404(db, card_id).await?;
    let versions = RateCardVersion::of_card(db, card.id).await?;
    if versions
        .iter()
        .any(|v| v.status != VersionStatus::Rejected && v.effective_from == effective_from)
    {
        return Err(refused(english("wl_invoicing.rate_card.problem.same_date")));
    }
    let day = today();
    let number = versions.iter().map(|v| v.version).max().unwrap_or(0) + 1;
    let version = RateCardVersion::create(
        db,
        RateCardVersion {
            rate_card_id: card.id,
            version: number,
            effective_from,
            submitted_on: day,
            past_dated: number > 1 && effective_from < day,
            created_by,
            ..RateCardVersion::default()
        },
    )
    .await?;
    if copy_latest
        && let Some(latest) = versions
            .iter()
            .filter(|v| v.status == VersionStatus::Accepted)
            .max_by_key(|v| v.effective_from)
    {
        for line in latest.lines(db).await? {
            RateCardLine::create(
                db,
                RateCardLine {
                    version_id: version.id,
                    created_at: None,
                    updated_at: None,
                    id: 0,
                    ..line
                },
            )
            .await?;
        }
    }
    Ok(version)
}

/// The refusal for changing a version that is not `pending`: it offers a new
/// version.
fn locked() -> Error {
    refused(english("wl_invoicing.rate_card.problem.new_version"))
}

/// Adds a line to `pending` version `version_id`.
///
/// # Errors
///
/// 422 for a version that is not `pending` (*new version* is offered), a
/// blank role or shift, a negative amount or a multiplier below 1.00, or a
/// line for the same role and shift.
pub async fn add_line(db: &Db, version_id: i64, input: LineInput) -> Result<RateCardLine> {
    let version = RateCardVersion::find_or_404(db, version_id).await?;
    if version.status != VersionStatus::Pending {
        return Err(locked());
    }
    input.check()?;
    let role = input.role.trim().to_lowercase();
    let shift = input.shift_type.trim().to_lowercase();
    if version
        .lines(db)
        .await?
        .iter()
        .any(|l| l.role == role && l.shift_type == shift)
    {
        return Err(refused(english(
            "wl_invoicing.rate_card.problem.line_exists",
        )));
    }
    RateCardLine::create(
        db,
        RateCardLine {
            version_id,
            role,
            shift_type: shift,
            unit: input.unit,
            rate: input.rate,
            ot_multiplier: input.ot_multiplier,
            holiday_multiplier: input.holiday_multiplier,
            min_hours_milli: input.min_hours_milli,
            ..RateCardLine::default()
        },
    )
    .await
}

/// Changes line `line_id` of a `pending` version.
///
/// # Errors
///
/// 422 for a version that is not `pending` (*new version* is offered) or a
/// bad amount.
pub async fn edit_line(db: &Db, line_id: i64, input: LineInput) -> Result<RateCardLine> {
    let mut line = RateCardLine::find_or_404(db, line_id).await?;
    let version = RateCardVersion::find_or_404(db, line.version_id).await?;
    if version.status != VersionStatus::Pending {
        return Err(locked());
    }
    input.check()?;
    line.rate = input.rate;
    line.unit = input.unit;
    line.ot_multiplier = input.ot_multiplier;
    line.holiday_multiplier = input.holiday_multiplier;
    line.min_hours_milli = input.min_hours_milli;
    line.save_only(
        db,
        &[
            "rate",
            "unit",
            "ot_multiplier",
            "holiday_multiplier",
            "min_hours_milli",
        ],
    )
    .await?;
    RateCardLine::find_or_404(db, line_id).await
}

/// Removes line `line_id` of a `pending` version.
///
/// # Errors
///
/// 422 for a version that is not `pending`.
pub async fn remove_line(db: &Db, line_id: i64) -> Result {
    let mut line = RateCardLine::find_or_404(db, line_id).await?;
    let version = RateCardVersion::find_or_404(db, line.version_id).await?;
    if version.status != VersionStatus::Pending {
        return Err(locked());
    }
    line.delete(db).await?;
    Ok(())
}

/// Whether `by` may accept versions of `card` for the client: a portal user
/// of the party with an accepting role. Staff with `rate_cards.manage` may
/// record the signed acceptance instead.
async fn client_accepts(state: &AppState, by: &AuthUser, card: &RateCard) -> Result<bool> {
    let acting = portal_users(state)
        .acting_for(&state.db, by.id, card.party(), today())
        .await?;
    Ok(acting.is_some_and(|user| user.has_any_role(ACCEPT_PORTAL_ROLES)))
}

/// `by` accepts version `version_id` for the client (`DOC.SIG.01`). A version
/// not dated in the past is `accepted` at once; one dated in the past waits
/// in `awaiting_approval` for a holder of `rate_cards.approve`.
///
/// `document` is the signed acceptance a member of staff recorded; a portal
/// user gives none.
///
/// # Errors
///
/// `Forbidden` unless `by` acts for the client in the portal as `owner` or
/// `approver`, or holds `rate_cards.manage` and gives the signed document; a
/// 422 for a version without lines or one that is not `pending`.
pub async fn accept(
    state: &AppState,
    by: &AuthUser,
    version_id: i64,
    document: Option<&str>,
) -> Result<RateCardVersion> {
    let version = RateCardVersion::find_or_404(&state.db, version_id).await?;
    let card = RateCard::find_or_404(&state.db, version.rate_card_id).await?;
    let document = document.map(str::trim).filter(|d| !d.is_empty());
    let by_client = client_accepts(state, by, &card).await?;
    if !by_client {
        let scope = wl_kernel::scopes::scope_of(&state.db, &card.source()).await?;
        if !by.has_permission_in(permissions::RATE_CARDS_MANAGE, &scope)
            && !by.has_permission(permissions::RATE_CARDS_MANAGE)
        {
            return Err(Error::Forbidden);
        }
        if document.is_none() {
            return Err(refused(english("wl_invoicing.rate_card.problem.document")));
        }
    }
    if version.status != VersionStatus::Pending {
        return Err(refused(english(
            "wl_invoicing.rate_card.problem.not_pending",
        )));
    }
    if version.lines(&state.db).await?.is_empty() {
        return Err(refused(english("wl_invoicing.rate_card.problem.no_lines")));
    }
    let name = if version.past_dated {
        ACCEPT_PAST_DATED
    } else {
        ACCEPT
    };
    let mut tx = state.db.begin().await?;
    let outcome = version
        .transition(name)
        .by_system()
        .input(&AcceptInput {
            by: by.id,
            document: document.filter(|_| !by_client).map(str::to_owned),
        })
        .run_in(&mut tx)
        .await
        .map_err(TransitionError::into_error)?;
    tx.commit().await?;
    wl_kernel::approvals::announce(state, &outcome).await?;
    outcome.emit(state).await?;
    RateCardVersion::find_or_404(&state.db, version_id).await
}

/// The listener that puts an accepted version in force: its unbilled, unlocked
/// hours from the effective date are repriced.
pub(crate) fn listen(app: &mut Registry) {
    app.listen(
        |event: Transitioned<RateCardVersion>, state: AppState| async move {
            if event.transition != ACCEPT && event.transition != ACCEPT_PAST_DATED_APPROVED {
                return Ok(());
            }
            let version = RateCardVersion::find_or_404(&state.db, event.key).await?;
            crate::rate_pricing::reprice(&state, &version).await?;
            Ok(())
        },
    );
}
