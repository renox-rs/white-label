//! The first steps of the seed: the parties the demo adds to the ones the
//! shared modules seed, the staff logins and the documents of the
//! subcontractors.

use renox::Upload;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_documents::{Documents, NewDocument};
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::{PartyKind, Subject};
use wl_parties::{
    Channel, CompanyDetails, ContactPoint, ContactType, Language, NewParty, Party, PartyRole,
    create,
};

use super::people::{FIRMS, LOGINS, email_of};
use super::roles::ADMIN;
use super::{Seed, day};
use crate::app::demo_data::specimen;
use crate::app::seed::PASSWORD;

impl Seed {
    /// The parties of the demo, by English name.
    pub(crate) async fn parties(&mut self) -> Result {
        let mut made = 0;
        for firm in FIRMS {
            let exists = Party::query()
                .where_(Party::LEGAL_NAME_EN.eq(firm.en))
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
            create(
                &self.state,
                NewParty {
                    kind: PartyKind::Company,
                    display_name: String::new(),
                    legal_name_en: firm.en.to_owned(),
                    legal_name_ar: Some(firm.ar.to_owned()),
                    default_language: Language::En,
                    preferred_channel: Channel::Email,
                    person: None,
                    company: Some(CompanyDetails {
                        legal_form: Some("llc".to_owned()),
                        licence_authority: Some("DEMO".to_owned()),
                        trade_licence_no: Some(format!("DEMO-{:06}", firm.n)),
                        vat_registered: true,
                        trn: Some(format!("100000000{:06}", firm.n)),
                        ..Default::default()
                    }),
                    roles: firm
                        .kind
                        .roles()
                        .iter()
                        .map(|role| PartyRole {
                            role_key: (*role).to_owned(),
                            ..Default::default()
                        })
                        .collect(),
                    contacts: vec![
                        contact(ContactType::Email, format!("{}.example", firm.mail)),
                        contact(
                            ContactType::Mobile,
                            format!("+971500000{:03}", firm.n % 1000),
                        ),
                    ],
                    source: "seed".to_owned(),
                },
            )
            .await?;
            made += 1;
        }
        self.load_parties().await?;
        self.report.add("parties added", made);
        Ok(())
    }

    /// Reads the parties again, by English name.
    pub(crate) async fn load_parties(&mut self) -> Result {
        for party in Party::query().order_by(Party::ID).get(self.db()).await? {
            self.parties.insert(party.legal_name_en.clone(), party);
        }
        Ok(())
    }

    /// The staff logins, one per role. Signed in for the requests that follow.
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
                    user.assign_role(self.db(), login.role).await?;
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

    /// Attaches a document of `kind` to `subject` and has the company admin
    /// accept it; returns its id.
    pub(crate) async fn document(
        &mut self,
        subject: Subject,
        kind: &str,
        title: &str,
        number: Option<String>,
        expires: Option<NaiveDate>,
        issued: Option<NaiveDate>,
    ) -> Result<i64> {
        let id = self
            .attach_document(subject, kind, title, number, expires, issued)
            .await?;
        self.driver
            .post(email_of(ADMIN), &format!("/documents/{id}/accept"), &[])
            .await?;
        Ok(id)
    }

    /// Attaches a document and leaves it waiting for review.
    pub(crate) async fn attach_document(
        &mut self,
        subject: Subject,
        kind: &str,
        title: &str,
        number: Option<String>,
        expires: Option<NaiveDate>,
        issued: Option<NaiveDate>,
    ) -> Result<i64> {
        let mut new = NewDocument::new(
            subject,
            kind,
            Upload::new(
                format!("{kind}.pdf"),
                "application/pdf",
                specimen::pdf(title),
            ),
        )
        .title(title);
        if let Some(number) = &number {
            new = new.number(number);
        }
        if let Some(date) = expires {
            new = new.expires_on(date);
        }
        if let Some(date) = issued {
            new = new.issued_on(date);
        }
        let document = match Documents::attach(&self.state, None, new).await? {
            Ok(document) => document,
            Err(refused) => {
                return Err(renox::anyhow::anyhow!(
                    "demo seed: the {kind} document was refused: {refused}"
                )
                .into());
            }
        };
        self.report.add("documents", 1);
        Ok(document.id)
    }

    /// The Dubai Municipality registrations of the two Dubai subcontractors:
    /// one valid for months, one that expired 40 days ago, which is why a
    /// purchase order to the second waits for the project manager.
    pub(crate) async fn registrations(&mut self) -> Result {
        for (name, expires, number) in [
            (super::people::MEP, day(210), "DM-CR-0041"),
            (super::people::GLASS, day(-40), "DM-CR-0078"),
        ] {
            let id = self.party(name).id;
            self.document(
                Subject::new(PARTY_SUBJECT_TYPE, id),
                construction::registration::REGISTRATION_KIND,
                "Dubai Municipality contractor registration",
                Some(number.to_owned()),
                Some(expires),
                Some(day(-520)),
            )
            .await?;
        }
        Ok(())
    }
}
