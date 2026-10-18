//! The party contract ([ARCHITECTURE.md] §5, wl-parties §6): what every
//! module may know about a party (a person or a company the client deals
//! with) without depending on the `wl-parties` crate.
//!
//! `wl-parties` provides a [`PartyDirectory`] with [`provide_directory`] and
//! emits [`PartyCreated`], [`PartyBlocked`] and [`PartyUnblocked`]. Other
//! modules read parties through [`directory`], which falls back to
//! [`NoParties`] when the `parties` module is off.
//!
//! No restricted block reason ever leaves `wl-parties`: summaries and events
//! carry the display reason, which is [`NEUTRAL_BLOCK_REASON`] when the real
//! one is restricted (wl-parties `PTY.ACL.02`, no tipping off).
//!
//! [ARCHITECTURE.md]: https://github.com/renox-rs/white-label/blob/main/docs/ARCHITECTURE.md

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::notify::{ContactPurpose, PartyContacts};
use crate::numbering::{Reset, Series};
use crate::subject::Subject;

/// The subject type of a party, for [`Subject`] references.
pub const PARTY_SUBJECT_TYPE: &str = "parties.party";

/// The neutral block reason, shown as **On hold**: what everyone without
/// `parties.block_reason.view`, every event and every agreement sees for a
/// restricted reason.
pub const NEUTRAL_BLOCK_REASON: &str = "on_hold";

/// Party codes: `P-00001`, across the deployment, never reset; gaps allowed
/// (take them with [`Series::next_allowing_gaps`]).
pub const PARTY_SERIES: Series = Series {
    key: "party",
    pattern: "{prefix}-{00000}",
    prefix: "P",
    reset: Reset::Never,
    gapless: false,
    per_entity: false,
};

/// The id of a party.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct PartyId(pub i64);

impl PartyId {
    /// The party as a [`Subject`]: `parties.party:42`.
    pub fn subject(self) -> Subject {
        Subject::new(PARTY_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for PartyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for PartyId {
    fn from(id: i64) -> Self {
        PartyId(id)
    }
}

/// Whether a party is a natural person or a company.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PartyKind {
    /// A natural person.
    #[default]
    Person,
    /// A company or other legal person.
    Company,
}

/// The lifecycle state of a party (wl-parties §4); the state of the
/// `wl-parties` machine.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PartyStatus {
    /// Business as usual.
    #[default]
    Active,
    /// No new business; shown as **On hold**.
    Blocked,
    /// No longer in use; kept for history.
    Archived,
    /// Merged into another party (terminal).
    Merged,
}

/// What other modules see of a party.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PartySummary {
    /// The id.
    pub id: PartyId,
    /// The code, e.g. `P-00001`.
    pub code: String,
    /// Person or company.
    pub kind: PartyKind,
    /// The name to show.
    pub display_name: String,
    /// The lifecycle state.
    pub status: PartyStatus,
    /// The display block reason while blocked ([`NEUTRAL_BLOCK_REASON`] for a
    /// restricted one); `None` otherwise.
    pub display_block_reason: Option<String>,
}

/// Who an invoice is addressed to: [`PartyDirectory::billing_identity`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BillingIdentity {
    /// The party.
    pub party: PartyId,
    /// The legal name in English.
    pub legal_name_en: String,
    /// The legal name in Arabic, if known.
    pub legal_name_ar: Option<String>,
    /// The tax registration number (15 digits), when VAT registered.
    pub trn: Option<String>,
    /// The billing address, one text.
    pub billing_address: Option<String>,
    /// The e-mail address invoices and notices go to.
    pub billing_email: Option<String>,
}

/// A party related to another one by a [`ContactPurpose`]:
/// [`PartyDirectory::related_contacts`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelatedContact {
    /// The related (from) party, a person.
    pub party: PartyId,
    /// The position of the relationship, e.g. `finance`.
    pub position: Option<String>,
    /// Whether it is the primary one for its party, relation and position.
    pub primary: bool,
}

/// The first of `candidates` (already in preference order: primary first)
/// whose party is not in `excluded`.
pub fn choose_contact<'a>(
    candidates: &'a [RelatedContact],
    excluded: &[PartyId],
) -> Option<&'a RelatedContact> {
    candidates.iter().find(|c| !excluded.contains(&c.party))
}

/// The reason code others may see: the code itself, or
/// [`NEUTRAL_BLOCK_REASON`] when it is restricted.
pub fn display_block_reason(code: &str, restricted: bool) -> &str {
    if restricted {
        NEUTRAL_BLOCK_REASON
    } else {
        code
    }
}

/// The future a [`PartyDirectory`] method returns.
pub type PartyFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Reads parties for other modules. `wl-parties` provides it; without that
/// module, [`directory`] gives [`NoParties`].
pub trait PartyDirectory: Send + Sync + 'static {
    /// The party with this id.
    fn find<'a>(&'a self, db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartySummary>>;

    /// The party with this code, e.g. `P-00001`.
    fn find_by_code<'a>(
        &'a self,
        db: &'a Db,
        code: &'a str,
    ) -> PartyFuture<'a, Option<PartySummary>>;

    /// The party's name in the language `lang` (`en`, `ar`), falling back to
    /// its display name.
    fn display_name<'a>(
        &'a self,
        db: &'a Db,
        party: PartyId,
        lang: &'a str,
    ) -> PartyFuture<'a, Option<String>>;

    /// The party's lifecycle state.
    fn status<'a>(&'a self, db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartyStatus>>;

    /// How to reach the party: its language, preferred channel and e-mail
    /// and mobile contact points, for `wl-notify`. The default knows none.
    fn contacts<'a>(
        &'a self,
        db: &'a Db,
        party: PartyId,
    ) -> PartyFuture<'a, Option<PartyContacts>> {
        let _ = (db, party);
        Box::pin(async { Ok(None) })
    }

    /// The parties related to `of` as `purpose` says (its finance contacts,
    /// its guardians), active today, primary first, then by start date, then
    /// by id. Only relationships whose to-party is `of` count, so a party
    /// outside them is never returned. The default knows none.
    fn related_contacts<'a>(
        &'a self,
        db: &'a Db,
        of: PartyId,
        purpose: &'a ContactPurpose,
    ) -> PartyFuture<'a, Vec<RelatedContact>> {
        let _ = (db, of, purpose);
        Box::pin(async { Ok(Vec::new()) })
    }

    /// Who an invoice to the party is addressed to: legal names, TRN,
    /// billing address and e-mail, copied into the invoice's party snapshot
    /// at issue. The default knows none.
    fn billing_identity<'a>(
        &'a self,
        db: &'a Db,
        party: PartyId,
    ) -> PartyFuture<'a, Option<BillingIdentity>> {
        let _ = (db, party);
        Box::pin(async { Ok(None) })
    }
}

/// The directory when the `parties` module is off: no party is ever found.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoParties;

impl PartyDirectory for NoParties {
    fn find<'a>(&'a self, _db: &'a Db, _party: PartyId) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async { Ok(None) })
    }

    fn find_by_code<'a>(
        &'a self,
        _db: &'a Db,
        _code: &'a str,
    ) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async { Ok(None) })
    }

    fn display_name<'a>(
        &'a self,
        _db: &'a Db,
        _party: PartyId,
        _lang: &'a str,
    ) -> PartyFuture<'a, Option<String>> {
        Box::pin(async { Ok(None) })
    }

    fn status<'a>(&'a self, _db: &'a Db, _party: PartyId) -> PartyFuture<'a, Option<PartyStatus>> {
        Box::pin(async { Ok(None) })
    }
}

/// Makes `directory` the app's [`PartyDirectory`]; `wl-parties` calls it in
/// `register`.
pub fn provide_directory(app: &mut Registry, directory: impl PartyDirectory) {
    let directory: Arc<dyn PartyDirectory> = Arc::new(directory);
    app.provide(directory);
}

/// The app's [`PartyDirectory`], or [`NoParties`] when none is provided.
pub fn directory(state: &AppState) -> Arc<dyn PartyDirectory> {
    match state.provided::<Arc<dyn PartyDirectory>>() {
        Some(directory) => Arc::clone(&directory),
        None => Arc::new(NoParties),
    }
}

/// A party was created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyCreated {
    /// The new party.
    pub party: PartyId,
    /// Person or company.
    pub kind: PartyKind,
    /// The role keys it was created with, e.g. `tenant`.
    pub roles: Vec<String>,
    /// Where it came from, e.g. `back_office`, `import`, `portal`.
    pub source: String,
}

impl Event for PartyCreated {}

/// A party was blocked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyBlocked {
    /// The party.
    pub party: PartyId,
    /// The display reason code: [`NEUTRAL_BLOCK_REASON`] when the real one is
    /// restricted.
    pub display_reason: String,
}

impl Event for PartyBlocked {}

/// A party was unblocked: every open Block record is closed and the party is
/// `active` again.
///
/// Releasing is split by owner. `wl-parties` releases only what it owns
/// (its open review tasks go back to their deciders). A module that paused
/// agreements because of `PartyBlocked` resumes them in its own listener on
/// this event, and only the agreements it paused that are still paused: an
/// agreement that its decider ended while the party was blocked stays ended.
/// The payload carries the display reason only, never the unblock note, the
/// basis or a restricted code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyUnblocked {
    /// The party.
    pub party: PartyId,
    /// The display reason code of the block that ended:
    /// [`NEUTRAL_BLOCK_REASON`] when the real one is restricted.
    pub display_reason: String,
}

impl Event for PartyUnblocked {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restricted_reason_is_shown_as_on_hold() {
        assert_eq!(display_block_reason("aml_concern", true), "on_hold");
        assert_eq!(
            display_block_reason("sanctions", true),
            NEUTRAL_BLOCK_REASON
        );
        assert_eq!(display_block_reason("non_payment", false), "non_payment");
        assert_eq!(display_block_reason("on_hold", false), "on_hold");
    }

    #[test]
    fn statuses_and_kinds_are_stored_in_snake_case() {
        let names: Vec<_> = PartyStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(names, ["active", "blocked", "archived", "merged"]);
        assert_eq!(
            "blocked".parse::<PartyStatus>().unwrap(),
            PartyStatus::Blocked
        );
        assert_eq!(PartyStatus::default(), PartyStatus::Active);
        assert_eq!(PartyKind::Company.as_str(), "company");
        assert_eq!("person".parse::<PartyKind>().unwrap(), PartyKind::Person);
    }

    fn related(party: i64, primary: bool) -> RelatedContact {
        RelatedContact {
            party: PartyId(party),
            position: Some("finance".into()),
            primary,
        }
    }

    #[test]
    fn choose_contact_takes_the_first_not_excluded() {
        let candidates = [related(1, true), related(2, false), related(3, false)];
        assert_eq!(choose_contact(&candidates, &[]).unwrap().party, PartyId(1));
        assert_eq!(
            choose_contact(&candidates, &[PartyId(1)]).unwrap().party,
            PartyId(2)
        );
        assert_eq!(
            choose_contact(&candidates, &[PartyId(1), PartyId(2)])
                .unwrap()
                .party,
            PartyId(3)
        );
        let all = [PartyId(1), PartyId(2), PartyId(3)];
        assert!(choose_contact(&candidates, &all).is_none());
        assert!(choose_contact(&[], &[]).is_none());
    }

    #[test]
    fn a_party_id_is_a_subject() {
        let party = PartyId(42);
        assert_eq!(party.subject().to_string(), "parties.party:42");
        assert_eq!(party.to_string(), "42");
    }

    #[test]
    fn the_party_series_is_p_and_five_digits() {
        assert_eq!(PARTY_SERIES.key, "party");
        assert_eq!(PARTY_SERIES.pattern, "{prefix}-{00000}");
        assert_eq!(PARTY_SERIES.prefix, "P");
        assert_eq!(PARTY_SERIES.reset, Reset::Never);
        assert_eq!(
            (PARTY_SERIES.gapless, PARTY_SERIES.per_entity),
            (false, false)
        );
    }

    #[test]
    fn a_billing_identity_serializes_with_a_plain_party_id() {
        let identity = BillingIdentity {
            party: PartyId(3),
            legal_name_en: "Gulf Trading LLC".into(),
            legal_name_ar: None,
            trn: Some("100000000000003".into()),
            billing_address: None,
            billing_email: Some("billing@gulf.example".into()),
        };
        let json = renox::serde_json::to_value(&identity).unwrap();
        assert_eq!(json["party"], 3);
        assert_eq!(json["trn"], "100000000000003");
        assert_eq!(json["billing_email"], "billing@gulf.example");
        assert!(json["legal_name_ar"].is_null());
    }

    #[test]
    fn events_carry_the_display_reason_only() {
        let blocked = PartyBlocked {
            party: PartyId(7),
            display_reason: display_block_reason("aml_concern", true).to_owned(),
        };
        let json = renox::serde_json::to_string(&blocked).unwrap();
        assert_eq!(json, r#"{"party":7,"display_reason":"on_hold"}"#);
    }
}
