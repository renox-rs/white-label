//! The first steps of the seed: the parties the demo adds to the ones the
//! shared modules seed, and the staff logins.

use renox::prelude::*;
use wl_kernel::PartyKind;
use wl_parties::{
    Channel, CompanyDetails, ContactPoint, ContactType, NewParty, Party, PartyRole, PersonDetails,
    create,
};

use super::people::{LOGINS, PARTIES, Reach, Who};
use super::{PASSWORD, Seed};

/// The Emirates ID of demo person `n` born in `born`: the block after the
/// year starts with `0000` (DEMO-DATA.md 1.2).
pub fn emirates_id(n: u32, born: i32) -> String {
    format!("784-{born}-0000{:03}-{}", n % 1000, n % 10)
}

/// The passport number of demo person `n`.
pub fn passport(n: u32) -> String {
    format!("X0000{:03}", n % 1000)
}

/// The residence visa number of demo person `n`.
pub fn visa(n: u32) -> String {
    format!("DEMO-RV-0000{:03}", n % 1000)
}

impl Seed<'_> {
    /// The parties of the demo, by English name: the ones the shared modules
    /// seed and the ones added here.
    pub(crate) async fn parties(&mut self) -> Result {
        let mut made = 0;
        for demo in PARTIES {
            let exists = Party::query()
                .where_(Party::LEGAL_NAME_EN.eq(demo.en))
                .exists(self.db())
                .await?;
            if exists {
                continue;
            }
            let contact = |contact_type, value: String| ContactPoint {
                contact_type,
                value,
                is_primary: true,
                ..Default::default()
            };
            let (kind, person, company) = match demo.who {
                Who::Person(nationality, born) => (
                    PartyKind::Person,
                    Some(PersonDetails {
                        nationality: Some(nationality.to_owned()),
                        date_of_birth: renox::chrono::NaiveDate::from_ymd_opt(
                            born,
                            1 + (demo.n % 12),
                            1 + (demo.n % 27),
                        ),
                        ..Default::default()
                    }),
                    None,
                ),
                Who::Company(form) => (
                    PartyKind::Company,
                    None,
                    Some(CompanyDetails {
                        legal_form: Some(form.to_owned()),
                        licence_authority: Some("DEMO".to_owned()),
                        trade_licence_no: Some(format!("DEMO-{:06}", demo.n)),
                        vat_registered: true,
                        trn: Some(format!("100000000{:06}", demo.n)),
                        ..Default::default()
                    }),
                ),
            };
            let mail = if demo.mail.contains('@') {
                format!("{}.example", demo.mail)
            } else {
                format!("{}@mail.example", demo.mail)
            };
            create(
                &self.state,
                NewParty {
                    kind,
                    display_name: String::new(),
                    legal_name_en: demo.en.to_owned(),
                    legal_name_ar: Some(demo.ar.to_owned()),
                    default_language: demo.lang,
                    preferred_channel: if kind == PartyKind::Person {
                        Channel::Whatsapp
                    } else {
                        Channel::Email
                    },
                    person,
                    company,
                    roles: vec![PartyRole {
                        role_key: demo.role.to_owned(),
                        ..Default::default()
                    }],
                    contacts: vec![
                        contact(ContactType::Email, mail),
                        contact(
                            ContactType::Mobile,
                            format!("+971500000{:03}", demo.n % 1000),
                        ),
                    ],
                    source: "seed".to_owned(),
                },
            )
            .await?;
            made += 1;
        }
        for party in Party::query().order_by(Party::ID).get(self.db()).await? {
            self.parties.insert(party.legal_name_en.clone(), party);
        }
        self.report.add("parties added", made);
        Ok(())
    }

    /// The party named `en`.
    pub(crate) fn party(&self, en: &str) -> &Party {
        self.parties
            .get(en)
            .unwrap_or_else(|| panic!("the demo party {en}"))
    }

    /// The staff logins: one per role, scoped to buildings where the role is.
    /// Signed in for the requests that follow.
    pub(crate) async fn logins(&mut self) -> Result {
        let mut made = 0;
        for login in LOGINS {
            let user = match User::find_by_email(self.db(), login.email).await? {
                Some(user) => user,
                None => {
                    let mut user =
                        User::register(self.db(), login.name, login.email, PASSWORD).await?;
                    // Seeded people need no verification mail.
                    user.email_verified_at = Some(renox::db::now());
                    user.save(self.db()).await?;
                    match login.reach {
                        Reach::Global => user.assign_role(self.db(), login.role).await?,
                        Reach::Buildings(codes) => {
                            for code in codes {
                                let building = self.building(code);
                                user.assign_role_in(self.db(), login.role, &building.scope())
                                    .await?;
                            }
                        }
                    }
                    made += 1;
                    user
                }
            };
            let db = self.state.db.clone();
            self.driver.sign_in(&db, login.email, &user).await?;
        }
        self.report.add("staff logins", made);
        Ok(())
    }
}
