//! Demo parties for `db:seed` ([DEMO-DATA.md] §1, 01 Property): tenants,
//! owners and vendors of the demo client, among them the company
//! "Al Noor Trading". All of them are fictional, bilingual and use the
//! demo identifier formats (§1.2: TRN `100000000NNNNNN`, trade licence
//! `DEMO-NNNNNN`, phone `+971 50 000 NNNN`, e-mail on `.example`).
//!
//! The list is fixed, so every seed gives the same parties; seeding twice
//! changes nothing (a party whose English legal name exists is skipped).
//!
//! [DEMO-DATA.md]: https://github.com/renox-rs/white-label/blob/main/docs/DEMO-DATA.md

use renox::prelude::*;
use wl_kernel::PartyKind;

use crate::contacts::{ContactPoint, ContactType};
use crate::create::{NewParty, create};
use crate::details::{CompanyDetails, PersonDetails};
use crate::party::{Channel, Language, Party};
use crate::roles::PartyRole;
use crate::sanctions_lists::SanctionsList;
use crate::screening::ScreeningSettings;

/// One demo party.
#[derive(Debug, Clone, Copy)]
pub struct DemoParty {
    /// Person or company.
    pub kind: PartyKind,
    /// The English legal name (also the display name).
    pub name_en: &'static str,
    /// The Arabic legal name.
    pub name_ar: &'static str,
    /// The language it is written to in.
    pub language: Language,
    /// Its preferred channel.
    pub channel: Channel,
    /// Its role key: `tenant`, `owner` or `vendor`.
    pub role: &'static str,
    /// Its primary e-mail.
    pub email: &'static str,
    /// Its primary mobile.
    pub mobile: &'static str,
    /// A person's nationality (ISO 3166 alpha-2).
    pub nationality: Option<&'static str>,
    /// A company's trade licence number.
    pub trade_licence_no: Option<&'static str>,
    /// A company's TRN; `None` when it is not VAT registered.
    pub trn: Option<&'static str>,
}

const fn person(
    name_en: &'static str,
    name_ar: &'static str,
    language: Language,
    role: &'static str,
    email: &'static str,
    mobile: &'static str,
    nationality: &'static str,
) -> DemoParty {
    DemoParty {
        kind: PartyKind::Person,
        name_en,
        name_ar,
        language,
        channel: Channel::Whatsapp,
        role,
        email,
        mobile,
        nationality: Some(nationality),
        trade_licence_no: None,
        trn: None,
    }
}

const fn company(
    name_en: &'static str,
    name_ar: &'static str,
    role: &'static str,
    email: &'static str,
    mobile: &'static str,
    trade_licence_no: &'static str,
    trn: Option<&'static str>,
) -> DemoParty {
    DemoParty {
        kind: PartyKind::Company,
        name_en,
        name_ar,
        language: Language::En,
        channel: Channel::Email,
        role,
        email,
        mobile,
        nationality: None,
        trade_licence_no: Some(trade_licence_no),
        trn,
    }
}

/// The demo parties, in the order they are created (and numbered).
pub const DEMO_PARTIES: &[DemoParty] = &[
    person(
        "Khalid Saeed Al Mansoori",
        "خالد سعيد المنصوري",
        Language::Ar,
        "tenant",
        "khalid.almansoori@mail.example",
        "+971500000101",
        "AE",
    ),
    person(
        "Priya Raghunathan",
        "بريا راغوناثان",
        Language::En,
        "tenant",
        "priya.raghunathan@mail.example",
        "+971500000102",
        "IN",
    ),
    person(
        "Maria Teresa Dizon",
        "ماريا تيريزا ديزون",
        Language::En,
        "tenant",
        "maria.dizon@mail.example",
        "+971500000103",
        "PH",
    ),
    person(
        "Fatima Yousef Haddad",
        "فاطمة يوسف حداد",
        Language::Ar,
        "tenant",
        "fatima.haddad@mail.example",
        "+971500000104",
        "JO",
    ),
    person(
        "Oliver Brennan",
        "أوليفر برينان",
        Language::En,
        "owner",
        "oliver.brennan@mail.example",
        "+971500000105",
        "IE",
    ),
    person(
        "Hamdan Rashid Al Ketbi",
        "حمدان راشد الكتبي",
        Language::Ar,
        "owner",
        "hamdan.alketbi@mail.example",
        "+971500000106",
        "AE",
    ),
    company(
        "Al Noor Trading LLC",
        "النور للتجارة ش.ذ.م.م",
        "vendor",
        "accounts@alnoortrading.example",
        "+971500000201",
        "DEMO-000201",
        Some("100000000000201"),
    ),
    company(
        "Gulf Build Materials LLC",
        "الخليج لمواد البناء ش.ذ.م.م",
        "vendor",
        "sales@gulfbuild.example",
        "+971500000202",
        "DEMO-000202",
        Some("100000000000202"),
    ),
    company(
        "Coolbreeze AC Services LLC",
        "كول بريز لخدمات التكييف ش.ذ.م.م",
        "vendor",
        "service@coolbreeze.example",
        "+971500000203",
        "DEMO-000203",
        Some("100000000000203"),
    ),
    company(
        "Bluewave Logistics FZE",
        "بلو ويف للخدمات اللوجستية م.م.ح",
        "tenant",
        "finance@bluewave.example",
        "+971500000204",
        "DEMO-000204",
        Some("100000000000204"),
    ),
    company(
        "Sandline Holdings LLC",
        "ساندلاين القابضة ش.ذ.م.م",
        "owner",
        "office@sandline.example",
        "+971500000205",
        "DEMO-000205",
        None,
    ),
];

impl DemoParty {
    /// The party to create.
    pub fn new_party(&self) -> NewParty {
        let contact = |contact_type, value: &str| ContactPoint {
            contact_type,
            value: value.to_owned(),
            is_primary: true,
            ..Default::default()
        };
        NewParty {
            kind: self.kind,
            display_name: display_name(self.name_en).to_owned(),
            legal_name_en: self.name_en.to_owned(),
            legal_name_ar: Some(self.name_ar.to_owned()),
            default_language: self.language,
            preferred_channel: self.channel,
            person: (self.kind == PartyKind::Person).then(|| PersonDetails {
                nationality: self.nationality.map(str::to_owned),
                ..Default::default()
            }),
            company: (self.kind == PartyKind::Company).then(|| CompanyDetails {
                legal_form: Some(legal_form(self.name_en).to_owned()),
                licence_authority: Some("DEMO".to_owned()),
                trade_licence_no: self.trade_licence_no.map(str::to_owned),
                vat_registered: self.trn.is_some(),
                trn: self.trn.map(str::to_owned),
                ..Default::default()
            }),
            roles: vec![PartyRole {
                role_key: self.role.to_owned(),
                ..Default::default()
            }],
            contacts: vec![
                contact(ContactType::Email, self.email),
                contact(ContactType::Mobile, self.mobile),
            ],
            source: "seed".to_owned(),
        }
    }
}

/// A company's name without its legal form: "Al Noor Trading" for
/// "Al Noor Trading LLC".
fn display_name(name_en: &str) -> &str {
    ["LLC", "FZE"]
        .iter()
        .find_map(|form| name_en.strip_suffix(form))
        .map_or(name_en, str::trim_end)
}

fn legal_form(name_en: &str) -> &'static str {
    if name_en.ends_with("FZE") {
        "fze"
    } else {
        "llc"
    }
}

/// Creates the [`DEMO_PARTIES`] that do not exist yet; returns how many it
/// created. The module's seeder.
pub async fn run(state: &AppState) -> Result<usize> {
    let mut created = 0;
    for demo in DEMO_PARTIES {
        let exists = Party::query()
            .where_(Party::LEGAL_NAME_EN.eq(demo.name_en))
            .exists(&state.db)
            .await?;
        if !exists {
            create(state, demo.new_party()).await?;
            created += 1;
        }
    }
    Ok(created)
}

/// The sanctions lists seeded when screening is on (wl-parties §7): name and
/// source.
pub const DEFAULT_LISTS: &[(&str, &str)] = &[
    ("UN Consolidated List", "United Nations Security Council"),
    (
        "UAE Local Terrorist List",
        "UAE Executive Office for Control and Non-Proliferation",
    ),
];

/// Creates the [`DEFAULT_LISTS`] that do not exist yet, stale and never
/// updated until the first download; returns how many it created. Does
/// nothing when screening is off.
pub async fn lists(state: &AppState) -> Result<usize> {
    if !ScreeningSettings::of(state).on {
        return Ok(0);
    }
    let mut created = 0;
    for (name, source) in DEFAULT_LISTS {
        let exists = SanctionsList::query()
            .where_(SanctionsList::NAME.eq(*name))
            .exists(&state.db)
            .await?;
        if !exists {
            let mut list = SanctionsList {
                name: (*name).to_owned(),
                source: (*source).to_owned(),
                ..SanctionsList::default()
            };
            list.insert(&state.db).await?;
            created += 1;
        }
    }
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contacts::second_primary;
    use wl_kernel::entities::is_trn;

    #[test]
    fn al_noor_trading_is_a_demo_company() {
        let al_noor = DEMO_PARTIES
            .iter()
            .find(|p| p.name_en.starts_with("Al Noor Trading"))
            .expect("Al Noor Trading is seeded");
        assert_eq!(al_noor.kind, PartyKind::Company);
        let new = al_noor.new_party();
        assert_eq!(new.display_name, "Al Noor Trading");
        assert_eq!(new.source, "seed");
        assert!(new.person.is_none());
        assert_eq!(
            new.company.and_then(|c| c.trn).as_deref(),
            Some("100000000000201")
        );
    }

    #[test]
    fn every_demo_party_is_bilingual_fictional_and_valid() {
        let mut names = Vec::new();
        for demo in DEMO_PARTIES {
            assert!(!demo.name_ar.trim().is_empty(), "{}", demo.name_en);
            assert!(demo.email.ends_with(".example"), "{}", demo.email);
            assert!(demo.mobile.starts_with("+97150000"), "{}", demo.mobile);
            if let Some(trn) = demo.trn {
                assert!(is_trn(trn) && trn.starts_with("100000000"), "{trn}");
            }
            if let Some(licence) = demo.trade_licence_no {
                assert!(licence.starts_with("DEMO-"), "{licence}");
            }
            let new = demo.new_party();
            assert_eq!(new.kind == PartyKind::Person, new.person.is_some());
            assert_eq!(new.kind == PartyKind::Company, new.company.is_some());
            assert!(second_primary(&new.contacts).is_none());
            assert!(!names.contains(&demo.name_en), "{} twice", demo.name_en);
            names.push(demo.name_en);
        }
    }

    #[test]
    fn there_are_tenants_owners_and_vendors_of_both_kinds() {
        for role in ["tenant", "owner"] {
            for kind in [PartyKind::Person, PartyKind::Company] {
                assert!(
                    DEMO_PARTIES
                        .iter()
                        .any(|p| p.role == role && p.kind == kind),
                    "{role} {kind:?}"
                );
            }
        }
        assert!(DEMO_PARTIES.iter().any(|p| p.role == "vendor"));
    }

    #[test]
    fn display_names_drop_the_legal_form() {
        assert_eq!(display_name("Bluewave Logistics FZE"), "Bluewave Logistics");
        assert_eq!(display_name("Oliver Brennan"), "Oliver Brennan");
        assert_eq!(legal_form("Bluewave Logistics FZE"), "fze");
        assert_eq!(legal_form("Al Noor Trading LLC"), "llc");
    }
}
