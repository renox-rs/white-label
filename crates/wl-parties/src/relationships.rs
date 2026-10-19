//! Relationships between parties (wl-parties §3, `PTY.REL.01`): a directed
//! link from one party to another. This is the minimal record that recipient
//! rules need (`contact_of` with a position, `guardian_of`); editing on the
//! party page, guardian basis and ending at an age threshold come later.
//!
//! [`relate`] keeps two rules: a `contact_of` or `guardian_of` link starts
//! from a person, and there is at most one primary per (to party, type,
//! position): a new primary clears the old one.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::validation::{Errors, ValidationError};
use serde::{Deserialize, Serialize};
use wl_kernel::parties::RelatedContact;
use wl_kernel::{PartyId, PartyKind};

use crate::party::Party;
use crate::texts;

/// What the link says the from party is to the to party.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RelationshipType {
    /// A person who is a guardian of another.
    GuardianOf,
    /// A person who is a contact of a party, in a position.
    #[default]
    ContactOf,
    /// An employee of a company.
    EmployeeOf,
    /// A shareholder of a company.
    ShareholderOf,
    /// A member of another's family.
    FamilyMemberOf,
    /// A member of a group.
    MemberOfGroup,
    /// A person who is an ultimate beneficial owner of a company.
    UboOf,
    /// A person who is a director of a company.
    DirectorOf,
    /// A person who is an authorised signatory of a company.
    SignatoryOf,
}

impl RelationshipType {
    /// Whether the from party must be a person.
    pub fn needs_person(self) -> bool {
        matches!(
            self,
            RelationshipType::ContactOf | RelationshipType::GuardianOf
        )
    }
}

/// One link between two parties.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_parties_relationships",
    index(to_party_id, relation_type),
    index(from_party_id)
)]
pub struct Relationship {
    /// The primary key.
    pub id: i64,
    /// The party the link starts from (the contact, the guardian).
    pub from_party_id: i64,
    /// The party the link points to (the company, the dependant).
    pub to_party_id: i64,
    /// What the from party is to the to party.
    pub relation_type: RelationshipType,
    /// For contacts, e.g. `finance`.
    pub position: Option<String>,
    /// Whether it is the primary one for its (to party, type, position).
    pub is_primary: bool,
    /// The first day it holds.
    pub valid_from: NaiveDate,
    /// The day it stops holding; `None` while it has no end.
    pub valid_until: Option<NaiveDate>,
    /// Why it ended.
    pub end_reason: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Relationship {
    /// Whether it holds on `date`: started, and not ended (`valid until`
    /// empty or after `date`, §3).
    pub fn is_active_on(&self, date: NaiveDate) -> bool {
        self.valid_from <= date && self.valid_until.is_none_or(|until| until > date)
    }

    /// Whether it is a link of `relation` in `position` (any position when
    /// `None`).
    pub fn matches(&self, relation: RelationshipType, position: Option<&str>) -> bool {
        self.relation_type == relation
            && position.is_none_or(|p| self.position.as_deref() == Some(p))
    }
}

/// The contacts among `links` (all pointing to one party) that hold on
/// `today`, for `relation` and `position`: primary first, then by `valid
/// from`, then by id.
pub fn related_contacts(
    links: &[Relationship],
    relation: RelationshipType,
    position: Option<&str>,
    today: NaiveDate,
) -> Vec<RelatedContact> {
    let mut active: Vec<&Relationship> = links
        .iter()
        .filter(|l| l.matches(relation, position) && l.is_active_on(today))
        .collect();
    active.sort_by_key(|l| (!l.is_primary, l.valid_from, l.id));
    active
        .into_iter()
        .map(|l| RelatedContact {
            party: PartyId(l.from_party_id),
            position: l.position.clone(),
            primary: l.is_primary,
        })
        .collect()
}

/// The links that reach a company when its owner, director or signatory is
/// a confirmed sanctions match. A shareholder link is not one.
pub const OWNERSHIP_LINKS: [RelationshipType; 3] = [
    RelationshipType::UboOf,
    RelationshipType::DirectorOf,
    RelationshipType::SignatoryOf,
];

/// The companies `party` is a UBO, director or signatory of on `today`
/// (`valid_from <= today < valid_until`), each once, by id. A `merged`
/// company is skipped. When ownership records land (`PTY.UBO.01`), their UBO
/// rows are added here.
///
/// # Errors
///
/// A database error.
pub async fn related_companies(db: &Db, party: i64, today: NaiveDate) -> Result<Vec<Party>> {
    let links = Relationship::query()
        .where_(Relationship::FROM_PARTY_ID.eq(party))
        .order_by(Relationship::TO_PARTY_ID)
        .order_by(Relationship::ID)
        .get(db)
        .await?;
    let mut companies: Vec<Party> = Vec::new();
    for link in links {
        if !OWNERSHIP_LINKS.contains(&link.relation_type)
            || !link.is_active_on(today)
            || companies.iter().any(|c| c.id == link.to_party_id)
        {
            continue;
        }
        if let Some(company) = Party::find(db, link.to_party_id).await?
            && company.kind == PartyKind::Company
            && company.status != wl_kernel::PartyStatus::Merged
        {
            companies.push(company);
        }
    }
    Ok(companies)
}

/// What a new relationship is made of. Id and timestamps are set by the
/// service.
#[derive(Debug, Clone, Default)]
pub struct NewRelationship {
    /// The party the link starts from.
    pub from: i64,
    /// The party the link points to.
    pub to: i64,
    /// The type.
    pub relation_type: RelationshipType,
    /// For contacts, e.g. `finance`; blank means none.
    pub position: Option<String>,
    /// Whether it becomes the primary one for its (to party, type, position).
    pub primary: bool,
    /// The first day it holds; today (Asia/Dubai) when `None`.
    pub valid_from: Option<NaiveDate>,
    /// The day it stops holding.
    pub valid_until: Option<NaiveDate>,
}

/// Records the relationship in `new`. Both parties must exist; `contact_of`
/// and `guardian_of` start from a person. A primary one clears the primary
/// flag of the others of its (to party, type, position), in one transaction.
/// Invalid input is a validation error (422) naming the field.
pub async fn relate(state: &AppState, new: NewRelationship) -> Result<Relationship> {
    let from = Party::find(&state.db, new.from).await?;
    let to = Party::find(&state.db, new.to).await?;
    let lang = state.current_lang();
    let valid_from = new.valid_from.unwrap_or_else(wl_kernel::documents::today);
    let mut errors = Errors::new();
    check(
        &new,
        valid_from,
        from.as_ref().map(|p| p.kind),
        to.is_some(),
        &mut errors,
        &|key| texts::t(&lang, key),
    );
    if !errors.is_empty() {
        return Err(Error::Validation(ValidationError::new(errors)));
    }

    let position = new
        .position
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_owned);
    let mut link = Relationship {
        from_party_id: new.from,
        to_party_id: new.to,
        relation_type: new.relation_type,
        position,
        is_primary: new.primary,
        valid_from,
        valid_until: new.valid_until,
        ..Default::default()
    };

    let mut tx = state.db.begin_immediate().await?;
    if link.is_primary {
        let others = Relationship::query()
            .where_(Relationship::TO_PARTY_ID.eq(link.to_party_id))
            .where_(Relationship::RELATION_TYPE.eq(link.relation_type))
            .where_(Relationship::IS_PRIMARY.eq(true))
            .get(&mut tx)
            .await?;
        for mut other in others.into_iter().filter(|o| o.position == link.position) {
            other.is_primary = false;
            other.save(&mut tx).await?;
        }
    }
    link.insert(&mut tx).await?;
    tx.commit().await?;
    Ok(link)
}

/// Collects what is wrong with `new`, given the kind of the from party (if
/// it exists) and whether the to party exists.
fn check(
    new: &NewRelationship,
    valid_from: NaiveDate,
    from_kind: Option<PartyKind>,
    to_exists: bool,
    errors: &mut Errors,
    t: &dyn Fn(&str) -> String,
) {
    match from_kind {
        None => errors.add("from", t("wl_parties.errors.relationship_party_missing")),
        Some(kind) if new.relation_type.needs_person() && kind != PartyKind::Person => {
            errors.add("from", t("wl_parties.errors.relationship_person"));
        }
        Some(_) => {}
    }
    if !to_exists {
        errors.add("to", t("wl_parties.errors.relationship_party_missing"));
    }
    if new.from == new.to {
        errors.add("to", t("wl_parties.errors.relationship_self"));
    }
    if new.valid_until.is_some_and(|until| until <= valid_from) {
        errors.add("valid_until", t("wl_parties.errors.relationship_dates"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create::{NewParty, create};

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap()
    }

    fn link(id: i64, from: i64, primary: bool, position: Option<&str>) -> Relationship {
        Relationship {
            id,
            from_party_id: from,
            to_party_id: 1,
            relation_type: RelationshipType::ContactOf,
            position: position.map(str::to_owned),
            is_primary: primary,
            valid_from: day(1, 1),
            ..Default::default()
        }
    }

    #[test]
    fn a_link_is_active_from_its_start_until_its_end() {
        let mut l = link(1, 2, false, None);
        l.valid_from = day(3, 1);
        l.valid_until = Some(day(6, 1));
        assert!(!l.is_active_on(day(2, 28)));
        assert!(l.is_active_on(day(3, 1)));
        assert!(l.is_active_on(day(5, 31)));
        assert!(!l.is_active_on(day(6, 1)));
        l.valid_until = None;
        assert!(l.is_active_on(day(12, 31)));
    }

    #[test]
    fn related_contacts_are_active_of_the_position_and_primary_first() {
        let mut ended = link(4, 40, false, Some("finance"));
        ended.valid_until = Some(day(5, 1));
        let mut earlier = link(2, 20, false, Some("finance"));
        earlier.valid_from = day(1, 1);
        let mut later = link(1, 10, false, Some("finance"));
        later.valid_from = day(2, 1);
        let mut guardian = link(6, 60, true, Some("finance"));
        guardian.relation_type = RelationshipType::GuardianOf;
        let links = [
            later,
            link(3, 30, true, Some("finance")),
            link(5, 50, true, Some("operations")),
            ended,
            guardian,
            earlier,
        ];
        let today = day(6, 1);
        let got = related_contacts(&links, RelationshipType::ContactOf, Some("finance"), today);
        let parties: Vec<_> = got.iter().map(|c| c.party.0).collect();
        assert_eq!(parties, [30, 20, 10]);
        assert!(got[0].primary && !got[1].primary);
        assert_eq!(got[0].position.as_deref(), Some("finance"));

        // Without a position, any position matches.
        let any = related_contacts(&links, RelationshipType::ContactOf, None, today);
        let parties: Vec<_> = any.iter().map(|c| c.party.0).collect();
        assert_eq!(parties, [30, 50, 20, 10]);
        let guardians = related_contacts(&links, RelationshipType::GuardianOf, None, today);
        assert_eq!(guardians.len(), 1);
    }

    async fn party(state: &AppState, kind: PartyKind, name: &str) -> Party {
        create(
            state,
            NewParty {
                kind,
                legal_name_en: name.into(),
                source: "test".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
    }

    fn contact(from: &Party, to: &Party, position: &str, primary: bool) -> NewRelationship {
        NewRelationship {
            from: from.id,
            to: to.id,
            relation_type: RelationshipType::ContactOf,
            position: Some(position.into()),
            primary,
            ..Default::default()
        }
    }

    #[renox::test]
    async fn related_companies_are_the_active_ubo_director_and_signatory_links() {
        let (app, _party, _views) = crate::block_effects::tests::boot().await;
        let state = app.state();
        let omar = party(state, PartyKind::Person, "Omar K.").await;
        let falcon = party(state, PartyKind::Company, "Falcon Holdings LLC").await;
        let dune = party(state, PartyKind::Company, "Dune Logistics").await;
        let sign = party(state, PartyKind::Company, "Signet LLC").await;
        let share = party(state, PartyKind::Company, "Shares LLC").await;
        let old = party(state, PartyKind::Company, "Old LLC").await;
        let merged = party(state, PartyKind::Company, "Merged LLC").await;
        let today = wl_kernel::documents::today();
        let new = |to: &Party, relation_type| NewRelationship {
            from: omar.id,
            to: to.id,
            relation_type,
            ..Default::default()
        };
        for (to, kind) in [
            (&falcon, RelationshipType::UboOf),
            (&dune, RelationshipType::DirectorOf),
            (&dune, RelationshipType::SignatoryOf),
            (&sign, RelationshipType::SignatoryOf),
            (&share, RelationshipType::ShareholderOf),
            (&merged, RelationshipType::UboOf),
        ] {
            relate(state, new(to, kind)).await.unwrap();
        }
        let mut ended = new(&old, RelationshipType::UboOf);
        ended.valid_from = Some(today - renox::chrono::Duration::days(30));
        ended.valid_until = Some(today - renox::chrono::Duration::days(1));
        relate(state, ended).await.unwrap();
        renox::db::sql("UPDATE wl_parties_parties SET status = 'merged' WHERE id = ?")
            .bind(merged.id)
            .execute(&state.db)
            .await
            .unwrap();

        let got = related_companies(&state.db, omar.id, today).await.unwrap();
        let names: Vec<_> = got.iter().map(|c| c.legal_name_en.as_str()).collect();
        assert_eq!(
            names,
            ["Falcon Holdings LLC", "Dune Logistics", "Signet LLC"]
        );
        assert!(
            related_companies(&state.db, falcon.id, today)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[renox::test]
    async fn a_company_cannot_be_a_contact_and_both_parties_must_exist() {
        let (app, _party, _views) = crate::block_effects::tests::boot().await;
        let state = app.state();
        let blue_dunes = party(state, PartyKind::Company, "Blue Dunes LLC").await;
        let acme = party(state, PartyKind::Company, "Acme LLC").await;
        let ali = party(state, PartyKind::Person, "Ali").await;

        let err = relate(state, contact(&acme, &blue_dunes, "finance", true))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Validation(_)));
        let mut missing = contact(&ali, &blue_dunes, "finance", false);
        missing.to = 9999;
        assert!(matches!(
            relate(state, missing).await.unwrap_err(),
            Error::Validation(_)
        ));
        assert!(matches!(
            relate(state, contact(&ali, &ali, "finance", false))
                .await
                .unwrap_err(),
            Error::Validation(_)
        ));
        let mut shareholder = contact(&acme, &blue_dunes, "", false);
        shareholder.relation_type = RelationshipType::ShareholderOf;
        let saved = relate(state, shareholder).await.unwrap();
        assert_eq!(saved.position, None);
        assert_eq!(Relationship::query().count(&state.db).await.unwrap(), 1);
    }

    #[renox::test]
    async fn a_new_primary_clears_the_old_one_of_the_same_position() {
        let (app, _party, _views) = crate::block_effects::tests::boot().await;
        let state = app.state();
        let blue_dunes = party(state, PartyKind::Company, "Blue Dunes LLC").await;
        let ali = party(state, PartyKind::Person, "Ali").await;
        let mona = party(state, PartyKind::Person, "Mona").await;
        let omar = party(state, PartyKind::Person, "Omar").await;

        let first = relate(state, contact(&ali, &blue_dunes, "finance", true))
            .await
            .unwrap();
        let ops = relate(state, contact(&omar, &blue_dunes, "operations", true))
            .await
            .unwrap();
        let second = relate(state, contact(&mona, &blue_dunes, "finance", true))
            .await
            .unwrap();
        assert!(second.is_primary);
        let first = Relationship::find(&state.db, first.id)
            .await
            .unwrap()
            .unwrap();
        let ops = Relationship::find(&state.db, ops.id)
            .await
            .unwrap()
            .unwrap();
        assert!(!first.is_primary);
        assert!(ops.is_primary);
    }
}
