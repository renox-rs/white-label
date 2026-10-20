//! The module's [`PartyDirectory`] (wl-parties §6): how other modules read
//! parties through the kernel. [`crate::WlParties`] provides it with
//! [`wl_kernel::parties::provide_directory`].
//!
//! Summaries carry the party's `hold_reason`, which is already the display
//! code (`on_hold` for a restricted reason), so no restricted code can leave
//! the module (`PTY.ACL.02`).
//!
//! Contacts give `wl-notify` the party's language, preferred channel and its
//! e-mail and mobile contact points (landlines and postal addresses carry no
//! message channel).
//!
//! The billing identity gives `wl-invoicing` what an invoice is addressed
//! to: the legal names, the company's TRN when it is VAT registered, and the
//! primary (else the first) address and e-mail contact points, and the
//! company's e-invoicing recipient identifier when it has one.

use renox::prelude::*;
use wl_kernel::notify::{
    ContactKind, ContactPurpose, ContactRelation, NotifyChannel, PartyContactPoint, PartyContacts,
};
use wl_kernel::parties::{
    BillingIdentity, InvalidatedContact, PartyDirectory, PartyFuture, RelatedContact,
};
use wl_kernel::{PartyId, PartyKind, PartyStatus, PartySummary};

use crate::contacts::{ContactPoint, ContactStatus, ContactType};
use crate::details::CompanyDetails;
use crate::party::{Channel, Party};
use crate::relationships::{self, Relationship, RelationshipType};

/// Reads the `wl_parties_parties` table for the kernel contract.
#[derive(Debug, Clone, Copy, Default)]
pub struct Directory;

impl Party {
    /// What other modules see of this party: the display block reason only
    /// while it is blocked.
    pub fn summary(&self) -> PartySummary {
        PartySummary {
            id: self.party_id(),
            code: self.code.clone(),
            kind: self.kind,
            display_name: self.display_name.clone(),
            status: self.status,
            display_block_reason: match self.status {
                PartyStatus::Blocked => self
                    .hold_reason
                    .clone()
                    .filter(|reason| !reason.trim().is_empty()),
                _ => None,
            },
        }
    }
}

impl PartyDirectory for Directory {
    fn find<'a>(&'a self, db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async move { Ok(Party::find(db, party.0).await?.map(|p| p.summary())) })
    }

    fn find_by_code<'a>(
        &'a self,
        db: &'a Db,
        code: &'a str,
    ) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async move {
            let party = Party::query()
                .where_(Party::CODE.eq(code.trim()))
                .first(db)
                .await?;
            Ok(party.map(|p| p.summary()))
        })
    }

    fn search<'a>(
        &'a self,
        db: &'a Db,
        text: &'a str,
        limit: u64,
    ) -> PartyFuture<'a, Vec<PartySummary>> {
        Box::pin(async move {
            let mut query = Party::query();
            let text = text.trim();
            if !text.is_empty() {
                let pattern = format!("%{text}%");
                query = query.where_any(|q| {
                    q.where_(Party::CODE.like(&pattern))
                        .where_(Party::DISPLAY_NAME.like(&pattern))
                });
            }
            let parties = query
                .order_by(Party::DISPLAY_NAME)
                .order_by(Party::ID)
                .limit(limit)
                .get(db)
                .await?;
            Ok(parties.iter().map(Party::summary).collect())
        })
    }

    fn display_name<'a>(
        &'a self,
        db: &'a Db,
        party: PartyId,
        lang: &'a str,
    ) -> PartyFuture<'a, Option<String>> {
        Box::pin(async move {
            let party = Party::find(db, party.0).await?;
            Ok(party.map(|p| p.name_in(lang).to_owned()))
        })
    }

    fn status<'a>(&'a self, db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartyStatus>> {
        Box::pin(async move { Ok(Party::find(db, party.0).await?.map(|p| p.status)) })
    }

    fn contacts<'a>(
        &'a self,
        db: &'a Db,
        party: PartyId,
    ) -> PartyFuture<'a, Option<PartyContacts>> {
        Box::pin(async move {
            let Some(party) = Party::find(db, party.0).await? else {
                return Ok(None);
            };
            let points = ContactPoint::query()
                .where_(ContactPoint::PARTY_ID.eq(party.id))
                .order_by_desc(ContactPoint::IS_PRIMARY)
                .order_by(ContactPoint::ID)
                .get(db)
                .await?;
            Ok(Some(party.contacts(&points)))
        })
    }

    fn invalidate_contact<'a>(
        &'a self,
        db: &'a Db,
        point: i64,
        since: DateTime,
        reason: &'a str,
    ) -> PartyFuture<'a, Option<InvalidatedContact>> {
        Box::pin(async move {
            let Some(found) = ContactPoint::find(db, point).await? else {
                return Ok(None);
            };
            // Landlines and postal addresses carry no message channel.
            let Some(kind) = message_kind(found.contact_type) else {
                return Ok(None);
            };
            // Only valid -> invalid: the guard in the statement keeps the
            // first since and reason, also when two runs race.
            let changed = renox::db::sql(format!(
                "UPDATE {} SET status = 'invalid', invalid_since = ?, invalid_reason = ?, \
                 updated_at = ? WHERE id = ? AND status = 'valid'",
                ContactPoint::TABLE
            ))
            .bind(since)
            .bind(reason.to_owned())
            .bind(renox::db::now())
            .bind(point)
            .execute(db)
            .await?;
            Ok((changed > 0).then_some(InvalidatedContact {
                party: PartyId(found.party_id),
                kind,
            }))
        })
    }

    fn related_contacts<'a>(
        &'a self,
        db: &'a Db,
        of: PartyId,
        purpose: &'a ContactPurpose,
    ) -> PartyFuture<'a, Vec<RelatedContact>> {
        Box::pin(async move {
            let relation = match purpose.relation {
                ContactRelation::ContactOf => RelationshipType::ContactOf,
                ContactRelation::GuardianOf => RelationshipType::GuardianOf,
            };
            let links = Relationship::query()
                .where_(Relationship::TO_PARTY_ID.eq(of.0))
                .where_(Relationship::RELATION_TYPE.eq(relation))
                .get(db)
                .await?;
            Ok(relationships::related_contacts(
                &links,
                relation,
                purpose.position.as_deref(),
                wl_kernel::documents::today(),
            ))
        })
    }

    fn billing_identity<'a>(
        &'a self,
        db: &'a Db,
        party: PartyId,
    ) -> PartyFuture<'a, Option<BillingIdentity>> {
        Box::pin(async move {
            let Some(party) = Party::find(db, party.0).await? else {
                return Ok(None);
            };
            let company = match party.kind {
                PartyKind::Company => {
                    CompanyDetails::query()
                        .where_(CompanyDetails::PARTY_ID.eq(party.id))
                        .first(db)
                        .await?
                }
                PartyKind::Person => None,
            };
            let points = ContactPoint::query()
                .where_(ContactPoint::PARTY_ID.eq(party.id))
                .order_by_desc(ContactPoint::IS_PRIMARY)
                .order_by(ContactPoint::ID)
                .get(db)
                .await?;
            Ok(Some(party.billing_identity(company.as_ref(), &points)))
        })
    }
}

impl Party {
    /// Who an invoice to this party is addressed to: its legal names, the
    /// TRN of `company` when it is VAT registered, and the first address and
    /// e-mail of `points` (ordered primary first).
    pub fn billing_identity(
        &self,
        company: Option<&CompanyDetails>,
        points: &[ContactPoint],
    ) -> BillingIdentity {
        let first = |contact_type: ContactType| {
            points
                .iter()
                .find(|p| p.contact_type == contact_type && !p.value.trim().is_empty())
                .map(|p| p.value.trim().to_owned())
        };
        let legal_name_en = if self.legal_name_en.trim().is_empty() {
            self.display_name.clone()
        } else {
            self.legal_name_en.clone()
        };
        BillingIdentity {
            party: self.party_id(),
            legal_name_en,
            legal_name_ar: self
                .legal_name_ar
                .clone()
                .filter(|name| !name.trim().is_empty()),
            trn: company
                .filter(|c| c.vat_registered)
                .and_then(|c| c.trn.clone())
                .filter(|trn| !trn.trim().is_empty()),
            billing_address: first(ContactType::Address),
            billing_email: first(ContactType::Email),
            einvoice_recipient_id: company
                .and_then(|c| c.einvoice_recipient_id.as_deref())
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_owned),
        }
    }
}

/// The message kind of a contact type; `None` for landlines and addresses.
fn message_kind(contact_type: ContactType) -> Option<ContactKind> {
    match contact_type {
        ContactType::Email => Some(ContactKind::Email),
        ContactType::Mobile => Some(ContactKind::Mobile),
        ContactType::Landline | ContactType::Address => None,
    }
}

impl Party {
    /// How to reach this party through `points`, its contact points: the
    /// e-mail and mobile ones, in the given order.
    pub fn contacts(&self, points: &[ContactPoint]) -> PartyContacts {
        PartyContacts {
            party: self.party_id(),
            language: self.default_language.as_str().to_owned(),
            preferred_channel: Some(match self.preferred_channel {
                Channel::Email => NotifyChannel::Email,
                Channel::Sms => NotifyChannel::Sms,
                Channel::Whatsapp => NotifyChannel::Whatsapp,
            }),
            points: points
                .iter()
                .filter_map(|point| {
                    let kind = message_kind(point.contact_type)?;
                    Some(PartyContactPoint {
                        id: point.id,
                        kind,
                        address: point.value.clone(),
                        primary: point.is_primary,
                        verified: point.verified_at.is_some(),
                        valid: point.status == ContactStatus::Valid,
                    })
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::PartyKind;

    fn party(status: PartyStatus, hold_reason: Option<&str>) -> Party {
        Party {
            id: 7,
            code: "P-00007".into(),
            kind: PartyKind::Company,
            display_name: "Al Noor Trading".into(),
            legal_name_en: "Al Noor Trading LLC".into(),
            status,
            hold_reason: hold_reason.map(str::to_owned),
            ..Default::default()
        }
    }

    #[test]
    fn an_active_party_has_no_display_reason() {
        let summary = party(PartyStatus::Active, None).summary();
        assert_eq!(summary.id, PartyId(7));
        assert_eq!(summary.code, "P-00007");
        assert_eq!(summary.kind, PartyKind::Company);
        assert_eq!(summary.display_name, "Al Noor Trading");
        assert_eq!(summary.status, PartyStatus::Active);
        assert_eq!(summary.display_block_reason, None);
    }

    #[test]
    fn a_blocked_party_shows_its_display_reason() {
        let summary = party(PartyStatus::Blocked, Some("on_hold")).summary();
        assert_eq!(summary.display_block_reason.as_deref(), Some("on_hold"));
        let summary = party(PartyStatus::Blocked, Some("non_payment")).summary();
        assert_eq!(summary.display_block_reason.as_deref(), Some("non_payment"));
    }

    #[test]
    fn contacts_keep_the_email_and_mobile_points() {
        let point = |id, contact_type, value: &str, is_primary| ContactPoint {
            id,
            party_id: 7,
            contact_type,
            value: value.into(),
            is_primary,
            ..Default::default()
        };
        let mut blue_dunes = party(PartyStatus::Active, None);
        blue_dunes.default_language = crate::party::Language::Ar;
        blue_dunes.preferred_channel = Channel::Sms;
        let contacts = blue_dunes.contacts(&[
            point(1, ContactType::Email, "ap@bluedunes.ae", true),
            point(2, ContactType::Landline, "+97143334444", true),
            point(3, ContactType::Mobile, "+971501234567", false),
            point(4, ContactType::Address, "Dubai", false),
        ]);
        assert_eq!(contacts.party, PartyId(7));
        assert_eq!(contacts.language, "ar");
        assert_eq!(contacts.preferred_channel, Some(NotifyChannel::Sms));
        let ids: Vec<_> = contacts.points.iter().map(|p| p.id).collect();
        assert_eq!(ids, [1, 3]);
        let email = contacts.point_for(NotifyChannel::Email).unwrap();
        assert_eq!(
            (
                email.kind,
                email.address.as_str(),
                email.primary,
                email.verified
            ),
            (ContactKind::Email, "ap@bluedunes.ae", true, false)
        );
        let sms = contacts.point_for(NotifyChannel::Sms).unwrap();
        assert_eq!(sms.address, "+971501234567");
    }

    #[test]
    fn an_invalid_point_is_listed_but_not_valid_and_stays_primary() {
        let mut email = ContactPoint {
            id: 1,
            party_id: 7,
            contact_type: ContactType::Email,
            value: "ali@bluedunes.ae".into(),
            is_primary: true,
            ..Default::default()
        };
        let sms = ContactPoint {
            id: 2,
            party_id: 7,
            contact_type: ContactType::Mobile,
            value: "+971501234567".into(),
            ..Default::default()
        };
        email.status = ContactStatus::Invalid;
        let contacts = party(PartyStatus::Active, None).contacts(&[email, sms]);
        let flags: Vec<_> = contacts
            .points
            .iter()
            .map(|p| (p.id, p.valid, p.primary))
            .collect();
        assert_eq!(flags, [(1, false, true), (2, true, false)]);
        assert_eq!(contacts.point_for(NotifyChannel::Email).map(|p| p.id), None);
        assert_eq!(
            contacts.point_for(NotifyChannel::Sms).map(|p| p.id),
            Some(2)
        );
    }

    #[test]
    fn only_email_and_mobile_have_a_message_kind() {
        assert_eq!(message_kind(ContactType::Email), Some(ContactKind::Email));
        assert_eq!(message_kind(ContactType::Mobile), Some(ContactKind::Mobile));
        assert_eq!(message_kind(ContactType::Landline), None);
        assert_eq!(message_kind(ContactType::Address), None);
    }

    #[test]
    fn the_billing_identity_takes_the_primary_points_and_a_registered_trn() {
        let point = |id, contact_type, value: &str, is_primary| ContactPoint {
            id,
            party_id: 7,
            contact_type,
            value: value.into(),
            is_primary,
            ..Default::default()
        };
        let company = CompanyDetails {
            party_id: 7,
            vat_registered: true,
            trn: Some("100000000000003".into()),
            einvoice_recipient_id: Some("0235:100000000000003".into()),
            ..Default::default()
        };
        let points = [
            point(2, ContactType::Email, "ap@alnoor.ae", true),
            point(4, ContactType::Address, "Office 12, Deira, Dubai", true),
            point(1, ContactType::Email, "info@alnoor.ae", false),
        ];
        let identity = party(PartyStatus::Active, None).billing_identity(Some(&company), &points);
        assert_eq!(identity.party, PartyId(7));
        assert_eq!(identity.legal_name_en, "Al Noor Trading LLC");
        assert_eq!(identity.legal_name_ar, None);
        assert_eq!(identity.trn.as_deref(), Some("100000000000003"));
        assert_eq!(
            identity.billing_address.as_deref(),
            Some("Office 12, Deira, Dubai")
        );
        assert_eq!(identity.billing_email.as_deref(), Some("ap@alnoor.ae"));
        assert_eq!(
            identity.einvoice_recipient_id.as_deref(),
            Some("0235:100000000000003")
        );

        let unregistered = CompanyDetails {
            vat_registered: false,
            ..company
        };
        let identity = party(PartyStatus::Active, None).billing_identity(Some(&unregistered), &[]);
        assert_eq!(identity.trn, None);
        assert_eq!(identity.billing_email, None);
        assert_eq!(identity.billing_address, None);
    }

    #[test]
    fn a_stale_hold_reason_is_not_shown_once_active() {
        let summary = party(PartyStatus::Active, Some("non_payment")).summary();
        assert_eq!(summary.display_block_reason, None);
    }
}
