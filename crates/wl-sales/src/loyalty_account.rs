//! The loyalty account (wl-sales §3 *Loyalty account*, §4.7, §5.9 step 1;
//! BL-SAL-22): a party's membership of a programme. It has two states,
//! `active` (earns) and `closed` (terminal; a party enrols again with a zero
//! balance); closing is BL-SAL-26. The balance moves only with a
//! [`PointsMovement`](crate::loyalty_points::PointsMovement).
//!
//! Enrolment records the version of the terms the member (or a guardian for
//! a minor, `PTY.REL.01`) accepted. **It is not marketing consent**: no
//! consent is created or implied (`PTY.CNS.01` asks for that separately).
//! A mobile number belongs to one open account per programme (`SAL.ACL.08`).
//!
//! Decisions:
//! - The member's code (`LYL-0001`) is what the cashier reads or scans; the
//!   one-time code of `wl-notify` that confirms a redemption is BL-SAL-23.
//! - A mobile number is stored in international form: a leading `00` becomes
//!   `+`, and a local number starting with `0` is a UAE number (`+971`).

use renox::chrono::NaiveDate;
use renox::db::{DateTime, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::numbering::{Reset, Series};
use wl_kernel::sales::{LOYALTY_ACCOUNT_SUBJECT_TYPE, LoyaltyAccountId};
use wl_kernel::{PartyId, Subject};

use crate::loyalty_programme::LoyaltyProgramme;
use crate::texts::english;

/// The numbering of the member's code: `LYL-0001`.
pub const ACCOUNT_SERIES: Series = Series {
    key: "loyalty_account",
    pattern: "{prefix}-{0000}",
    prefix: "LYL",
    reset: Reset::Never,
    gapless: false,
    per_entity: false,
};

/// Where a loyalty account is (§4.7).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AccountStatus {
    /// Earns (and redeems).
    #[default]
    Active,
    /// Terminal: the member left, staff closed it, it was merged or erased.
    Closed,
}

/// A party's membership of a loyalty programme.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_loyalty_accounts",
    unique(code),
    index(programme_id, party_id),
    index(mobile)
)]
pub struct LoyaltyAccount {
    /// The primary key.
    pub id: i64,
    /// The programme.
    #[model(references = "wl_sales_loyalty_programmes")]
    pub programme_id: i64,
    /// The member.
    pub party_id: i64,
    /// The member's code, e.g. `LYL-0001`.
    pub code: String,
    /// The guardian who accepted the terms for a minor.
    pub accepted_by_party_id: Option<i64>,
    /// The mobile number the codes go to, in international form.
    pub mobile: String,
    /// When the member enrolled.
    pub enrolled_at: Option<DateTime>,
    /// `counter`, `portal` or `public`.
    pub enrolled_via: String,
    /// The staff user who enrolled the member at the counter.
    pub enrolled_by_user_id: Option<i64>,
    /// The version of the terms accepted.
    pub terms_version: String,
    /// The points (a per-AED programme); negative after a reversal.
    pub points_balance: i64,
    /// The points held for a redemption in progress (BL-SAL-23).
    pub points_held: i64,
    /// The visits since the last reward (a per-visit programme).
    pub visits: i64,
    /// The day a reward made available is valid until.
    pub reward_available_until: Option<NaiveDate>,
    /// Whether the party is blocked: nothing is earned (BL-SAL-26).
    pub paused: bool,
    /// Active or closed.
    pub status: AccountStatus,
    /// Why it was closed.
    pub closed_reason: Option<String>,
    /// When it was closed.
    pub closed_at: Option<DateTime>,
    /// When the last expiry reminder gave up; the account shows *reminder
    /// not delivered* (BL-SAL-24).
    pub expiry_reminder_failed_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl LoyaltyAccount {
    /// The account as an id.
    pub fn account_id(&self) -> LoyaltyAccountId {
        LoyaltyAccountId(self.id)
    }

    /// The account as a kernel subject.
    pub fn subject(&self) -> Subject {
        Subject::new(LOYALTY_ACCOUNT_SUBJECT_TYPE, self.id)
    }

    /// The member.
    pub fn party(&self) -> PartyId {
        PartyId(self.party_id)
    }

    /// The party told about the account: the guardian for a minor, else the
    /// member.
    pub fn notify_party(&self) -> PartyId {
        PartyId(self.accepted_by_party_id.unwrap_or(self.party_id))
    }

    /// Whether the account is open.
    pub fn is_active(&self) -> bool {
        self.status == AccountStatus::Active
    }

    /// Whether the account earns: open and not paused.
    pub fn earns(&self) -> bool {
        self.is_active() && !self.paused
    }

    /// The open account with the code `code` (any case).
    pub async fn by_code(db: &Db, code: &str) -> Result<Option<LoyaltyAccount>> {
        LoyaltyAccount::query()
            .where_(LoyaltyAccount::CODE.eq(code.trim().to_uppercase()))
            .where_(LoyaltyAccount::STATUS.eq(AccountStatus::Active))
            .first(db)
            .await
    }

    /// The open account with the mobile number `mobile` (any way of writing
    /// it), in any programme.
    pub async fn by_mobile(db: &Db, mobile: &str) -> Result<Option<LoyaltyAccount>> {
        let Some(mobile) = normalize_mobile(mobile) else {
            return Ok(None);
        };
        LoyaltyAccount::query()
            .where_(LoyaltyAccount::MOBILE.eq(mobile))
            .where_(LoyaltyAccount::STATUS.eq(AccountStatus::Active))
            .order_by(LoyaltyAccount::ID)
            .first(db)
            .await
    }

    /// The open account of `party`, in any programme.
    pub async fn of_party(db: &Db, party: PartyId) -> Result<Option<LoyaltyAccount>> {
        LoyaltyAccount::query()
            .where_(LoyaltyAccount::PARTY_ID.eq(party.0))
            .where_(LoyaltyAccount::STATUS.eq(AccountStatus::Active))
            .order_by(LoyaltyAccount::ID)
            .first(db)
            .await
    }
}

/// The refusal at the counter for a member whose party is on hold: the
/// neutral 422 "This party is on hold", never the reason (`PTY.ACL.03`).
pub(crate) fn on_hold(party: PartyId) -> Error {
    wl_kernel::block_guard::BlockRefusal::neutral(
        party,
        wl_kernel::block_guard::ActionKind::Start,
        "",
    )
    .into_error()
}

/// `text` as a mobile number in international form (`+971501234567`), or
/// `None` when it does not have 8 to 15 digits.
pub fn normalize_mobile(text: &str) -> Option<String> {
    let text = text.trim();
    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
    if !(8..=15).contains(&digits.len()) {
        return None;
    }
    if text.starts_with('+') {
        Some(format!("+{digits}"))
    } else if let Some(rest) = digits.strip_prefix("00") {
        Some(format!("+{rest}"))
    } else if let Some(rest) = digits.strip_prefix('0') {
        Some(format!("+971{rest}"))
    } else {
        Some(format!("+{digits}"))
    }
}

/// A member to enrol.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Enrolment {
    /// The member.
    pub party: PartyId,
    /// The programme; the first active one when `None`.
    pub programme: Option<i64>,
    /// The mobile number the codes go to.
    pub mobile: String,
    /// The guardian who accepts the terms for a minor.
    pub guardian: Option<PartyId>,
    /// `counter`, `portal` or `public`.
    pub via: String,
    /// The staff user who enrols the member at the counter.
    pub by_user: Option<i64>,
    /// Whether the member (or guardian) accepted the terms.
    pub accepted_terms: bool,
}

impl Enrolment {
    /// `party` enrolling with the mobile number `mobile`, at the counter,
    /// terms not yet accepted.
    pub fn new(party: PartyId, mobile: &str) -> Self {
        Enrolment {
            party,
            programme: None,
            mobile: mobile.to_owned(),
            guardian: None,
            via: "counter".to_owned(),
            by_user: None,
            accepted_terms: false,
        }
    }

    /// In the programme `id`.
    pub fn programme(mut self, id: i64) -> Self {
        self.programme = Some(id);
        self
    }

    /// A minor, enrolled by the guardian `guardian`, who accepts the terms.
    pub fn guardian(mut self, guardian: PartyId) -> Self {
        self.guardian = Some(guardian);
        self
    }

    /// Through `via` (`counter`, `portal` or `public`).
    pub fn via(mut self, via: &str) -> Self {
        self.via = via.to_owned();
        self
    }

    /// By the staff user `user`.
    pub fn by_user(mut self, user: i64) -> Self {
        self.by_user = Some(user);
        self
    }

    /// The member accepted the terms.
    pub fn accepted_terms(mut self) -> Self {
        self.accepted_terms = true;
        self
    }
}

/// A 422 with the English text `key`.
fn refused(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]))
}

/// Enrols a member and records the version of the terms accepted. No
/// marketing consent is created.
///
/// # Errors
///
/// 422 when there is no active programme, the terms were not accepted, the
/// mobile number is not a number, the party already has an open account in
/// the programme, or the mobile number is on another open account of it.
pub async fn enrol(state: &AppState, e: Enrolment) -> Result<LoyaltyAccount> {
    let programme = match e.programme {
        Some(id) => LoyaltyProgramme::find(&state.db, id)
            .await?
            .filter(|p| p.active),
        None => LoyaltyProgramme::first_active(&state.db).await?,
    };
    // A programme that was turned off takes no new members.
    let programme = programme.filter(|p| p.closing_date.is_none());
    let Some(programme) = programme else {
        return Err(refused("wl_sales.loyalty.problem.no_programme"));
    };
    if !e.accepted_terms {
        return Err(refused("wl_sales.loyalty.problem.terms_not_accepted"));
    }
    let Some(mobile) = normalize_mobile(&e.mobile) else {
        return Err(refused("wl_sales.loyalty.problem.mobile"));
    };
    let party_taken = LoyaltyAccount::query()
        .where_(LoyaltyAccount::PROGRAMME_ID.eq(programme.id))
        .where_(LoyaltyAccount::PARTY_ID.eq(e.party.0))
        .where_(LoyaltyAccount::STATUS.eq(AccountStatus::Active))
        .exists(&state.db)
        .await?;
    if party_taken {
        return Err(refused("wl_sales.loyalty.problem.already_member"));
    }
    let mobile_taken = LoyaltyAccount::query()
        .where_(LoyaltyAccount::PROGRAMME_ID.eq(programme.id))
        .where_(LoyaltyAccount::MOBILE.eq(mobile.clone()))
        .where_(LoyaltyAccount::STATUS.eq(AccountStatus::Active))
        .exists(&state.db)
        .await?;
    if mobile_taken {
        return Err(refused("wl_sales.loyalty.problem.mobile_taken"));
    }
    let code = ACCOUNT_SERIES
        .next_allowing_gaps(&state.db, None, None, wl_kernel::documents::today())
        .await?;
    LoyaltyAccount::create(
        &state.db,
        LoyaltyAccount {
            programme_id: programme.id,
            party_id: e.party.0,
            code,
            accepted_by_party_id: e.guardian.map(|g| g.0),
            mobile,
            enrolled_at: Some(now()),
            enrolled_via: e.via,
            enrolled_by_user_id: e.by_user,
            terms_version: programme.terms_version.clone(),
            ..LoyaltyAccount::default()
        },
    )
    .await
}

/// The member of the account as the page of a sale shows it, `LYL-0001
/// (party 7)`; `None` for no account.
pub async fn label_of(db: &Db, account: Option<i64>) -> Result<Option<String>> {
    let Some(id) = account else {
        return Ok(None);
    };
    Ok(LoyaltyAccount::find(db, id)
        .await?
        .map(|a| format!("{} (party {})", a.code, a.party_id)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mobile_number_has_one_international_form() {
        assert_eq!(
            normalize_mobile("050 123 4567").as_deref(),
            Some("+971501234567")
        );
        assert_eq!(
            normalize_mobile("+971 50 123 4567").as_deref(),
            Some("+971501234567")
        );
        assert_eq!(
            normalize_mobile("00971501234567").as_deref(),
            Some("+971501234567")
        );
        assert_eq!(
            normalize_mobile("971501234567").as_deref(),
            Some("+971501234567")
        );
        assert_eq!(normalize_mobile("12345"), None);
        assert_eq!(normalize_mobile(""), None);
    }

    #[test]
    fn a_member_on_hold_is_refused_with_the_neutral_text() {
        match on_hold(PartyId(5)) {
            Error::Status(code, message) => {
                assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
                assert_eq!(message, "This party is on hold");
            }
            other => panic!("unexpected error {other:?}"),
        }
    }

    #[test]
    fn a_minors_codes_go_to_the_guardian() {
        let account = LoyaltyAccount {
            party_id: 5,
            accepted_by_party_id: Some(9),
            ..LoyaltyAccount::default()
        };
        assert_eq!(account.notify_party(), PartyId(9));
        let own = LoyaltyAccount {
            party_id: 5,
            ..LoyaltyAccount::default()
        };
        assert_eq!(own.notify_party(), PartyId(5));
        assert!(own.earns());
    }

    #[test]
    fn a_paused_or_closed_account_does_not_earn() {
        let paused = LoyaltyAccount {
            paused: true,
            ..LoyaltyAccount::default()
        };
        assert!(!paused.earns());
        let closed = LoyaltyAccount {
            status: AccountStatus::Closed,
            ..LoyaltyAccount::default()
        };
        assert!(!closed.earns() && !closed.is_active());
    }
}
