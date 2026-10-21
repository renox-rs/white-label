//! Making the staff records: the people behind the demo logins, hired over
//! the last years; two joined recently and are still in onboarding.

use renox::prelude::*;
use wl_hr::OnboardingStep;
use wl_hr::employees::create;
use wl_hr::{Employee, EmployeeStatus, NewEmployee};
use wl_kernel::{PartyId, PartyKind, Subject};
use wl_parties::{
    Channel, ContactPoint, ContactType, Language, NewParty, Party, PartyRole, PersonDetails,
};

use super::people::login_of;
use super::roles::{
    ACCOUNTANT, ADMIN, COORDINATOR, HR, LEASING_AGENT, PROPERTY_MANAGER, RECEIVABLES, TECHNICIAN,
};
use super::seed_leases::upto;
use super::{Seed, day};

/// One member of staff: the login's role, the Arabic name, the job role the
/// onboarding templates use, the hire day (days from today) and the title.
struct Staff {
    email: &'static str,
    ar: &'static str,
    nationality: &'static str,
    job_role: &'static str,
    hired: i64,
    title: &'static str,
    onboarding: bool,
}

fn staff() -> Vec<Staff> {
    vec![
        Staff {
            email: login_of(ADMIN).email,
            ar: "ليلى حسن",
            nationality: "AE",
            job_role: "admin",
            hired: -1_900,
            title: "General manager",
            onboarding: false,
        },
        Staff {
            email: login_of(PROPERTY_MANAGER).email,
            ar: "راشد الملا",
            nationality: "AE",
            job_role: "property_manager",
            hired: -1_450,
            title: "Property manager, Towers",
            onboarding: false,
        },
        Staff {
            email: "property_manager.bay@palmgrove.example",
            ar: "سارة نائر",
            nationality: "IN",
            job_role: "property_manager",
            hired: -980,
            title: "Property manager, Business Bay",
            onboarding: false,
        },
        Staff {
            email: login_of(LEASING_AGENT).email,
            ar: "دانا ميركادو",
            nationality: "PH",
            job_role: "leasing_agent",
            hired: -640,
            title: "Leasing agent",
            onboarding: false,
        },
        Staff {
            email: login_of(ACCOUNTANT).email,
            ar: "فيكرام أيير",
            nationality: "IN",
            job_role: "accountant",
            hired: -1_100,
            title: "Senior accountant",
            onboarding: false,
        },
        Staff {
            email: login_of(RECEIVABLES).email,
            ar: "هند الزعابي",
            nationality: "AE",
            job_role: "accountant",
            hired: -430,
            title: "Receivables officer",
            onboarding: false,
        },
        Staff {
            email: login_of(COORDINATOR).email,
            ar: "بلال خان",
            nationality: "PK",
            job_role: "coordinator",
            hired: -12,
            title: "Maintenance coordinator",
            onboarding: true,
        },
        Staff {
            email: login_of(TECHNICIAN).email,
            ar: "جويل راموس",
            nationality: "PH",
            job_role: "technician",
            hired: -5,
            title: "Technician",
            onboarding: true,
        },
    ]
}

impl Seed {
    /// The staff as parties and employees.
    pub(crate) async fn staff(&mut self) -> Result {
        let hr = login_of(HR).email;
        let entity = wl_kernel::LegalEntity::default_entity(self.db()).await?.id;
        for member in staff() {
            let login = super::people::LOGINS
                .iter()
                .find(|l| l.email == member.email)
                .expect("a login for the staff member");
            let existing = Party::query()
                .where_(Party::LEGAL_NAME_EN.eq(login.name))
                .first(self.db())
                .await?;
            let party = match existing {
                Some(party) => party,
                None => {
                    let n = 500 + self.report.made.get("staff").copied().unwrap_or(0) as u32;
                    self.report.add("staff", 1);
                    wl_parties::create(
                        &self.state,
                        NewParty {
                            kind: PartyKind::Person,
                            legal_name_en: login.name.to_owned(),
                            legal_name_ar: Some(member.ar.to_owned()),
                            default_language: Language::En,
                            preferred_channel: Channel::Email,
                            person: Some(PersonDetails {
                                nationality: Some(member.nationality.to_owned()),
                                ..Default::default()
                            }),
                            roles: vec![PartyRole {
                                role_key: "employee".to_owned(),
                                ..Default::default()
                            }],
                            contacts: vec![
                                ContactPoint {
                                    contact_type: ContactType::Email,
                                    value: member.email.to_owned(),
                                    is_primary: true,
                                    ..Default::default()
                                },
                                ContactPoint {
                                    contact_type: ContactType::Mobile,
                                    value: format!("+971500000{n}"),
                                    is_primary: true,
                                    ..Default::default()
                                },
                            ],
                            source: "seed".to_owned(),
                            ..Default::default()
                        },
                    )
                    .await?
                }
            };
            let user = self.user_id(member.email).await?;
            let mut new = NewEmployee::new(PartyId(party.id), member.job_role, day(member.hired))
                .user(user)
                .job_title(member.title);
            new.employee_number = format!("PGP-{:04}", party.id);
            new.legal_entity_id = Some(entity);
            if let super::people::Reach::Buildings(codes) = login.reach {
                for code in codes {
                    new = new.scope(Subject::new(
                        property::building::BUILDING_SUBJECT_TYPE,
                        self.building(code).id,
                    ));
                }
            }
            if !member.onboarding {
                new = new.status(EmployeeStatus::Active);
            }
            let hired = upto(day(member.hired));
            let employee = at!(self, hired, { create(&self.state, new).await })?;
            self.report.add("employees", 1);
            if member.onboarding {
                self.start_steps(&employee, hr, hired).await?;
            }
        }
        Ok(())
    }

    /// HR does the steps it owns of a new hire's onboarding that are due.
    async fn start_steps(
        &mut self,
        employee: &Employee,
        hr: &str,
        hired: renox::chrono::NaiveDate,
    ) -> Result {
        let onboarding = wl_hr::Onboarding::query()
            .where_(wl_hr::Onboarding::EMPLOYEE_ID.eq(employee.id))
            .first(self.db())
            .await?;
        let Some(onboarding) = onboarding else {
            return Ok(());
        };
        let steps = OnboardingStep::query()
            .where_(OnboardingStep::ONBOARDING_ID.eq(onboarding.id))
            .where_(OnboardingStep::OWNER.eq("hr"))
            .order_by(OnboardingStep::POSITION)
            .get(self.db())
            .await?;
        for step in steps.into_iter().take(1) {
            at!(self, upto(hired + renox::chrono::Duration::days(1)), {
                self.driver
                    .post(
                        hr,
                        &format!("/hr/steps/{}/done", step.id),
                        &[("note", "Done on the first day")],
                    )
                    .await
            })?;
        }
        Ok(())
    }
}
