//! The staff records: the people behind the logins who are employees, and the
//! workers and supervisors of the sites, hired over the last years; two joined
//! recently and are still in onboarding. Each has a work permit and a
//! residence visa on file, one permit expires soon and one expired.

use super::at;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_hr::OnboardingStep;
use wl_hr::employees::create;
use wl_hr::{Employee, EmployeeStatus, NewEmployee};
use wl_kernel::workforce::EMPLOYEE_SUBJECT_TYPE;
use wl_kernel::{PartyId, PartyKind, Subject};
use wl_parties::{
    Channel, ContactPoint, ContactType, Language, NewParty, Party, PartyRole, PersonDetails,
};

use super::people::email_of;
use super::roles::{HR, PROCUREMENT, QUANTITY_SURVEYOR, STOREKEEPER, SUPERVISOR};
use super::{Seed, day, upto};

/// A worker or supervisor on a site, or a member of the office staff who is
/// an employee.
#[derive(Debug, Clone, Copy)]
pub struct Staff {
    /// The key the rest of the demo finds the employee by.
    pub key: &'static str,
    /// The English name.
    pub en: &'static str,
    /// The Arabic name.
    pub ar: &'static str,
    /// The nationality.
    pub nationality: &'static str,
    /// The year of birth.
    pub born: i32,
    /// The job role the onboarding templates use.
    pub job_role: &'static str,
    /// The title (the trade of a worker).
    pub title: &'static str,
    /// The hire day, days from today.
    pub hired: i64,
    /// The e-mail of the login of the same person, if there is one.
    pub login: Option<&'static str>,
    /// Whether the person is still in onboarding.
    pub onboarding: bool,
    /// The days to the work permit's end.
    pub permit_ends: i64,
    /// The projects the employee works on, by code.
    pub sites: &'static [&'static str],
}

/// The project codes.
pub const TWR: &str = "TWR-01";
/// The fashion store in the mall.
pub const MALL: &str = "MALL-02";
/// The clinic.
pub const CLN: &str = "CLN-03";
/// The restaurant.
pub const RST: &str = "RST-04";

/// The employees, in the order they are hired.
pub const STAFF: &[Staff] = &[
    Staff {
        key: "ramon",
        en: "Ramon Dela Cruz",
        ar: "رامون دي لا كروز",
        nationality: "PH",
        born: 1982,
        job_role: SUPERVISOR,
        title: "Site supervisor",
        hired: -700,
        login: Some("supervisor@alwaha.example"),
        onboarding: false,
        permit_ends: 300,
        sites: &[TWR, MALL],
    },
    Staff {
        key: "nasser",
        en: "Nasser Al Hammadi",
        ar: "ناصر الحمادي",
        nationality: "AE",
        born: 1984,
        job_role: SUPERVISOR,
        title: "Site supervisor",
        hired: -500,
        login: None,
        onboarding: false,
        permit_ends: 420,
        sites: &[CLN, RST],
    },
    Staff {
        key: "hessa",
        en: "Hessa Al Neyadi",
        ar: "حصة النيادي",
        nationality: "AE",
        born: 1990,
        job_role: PROCUREMENT,
        title: "Procurement officer",
        hired: -600,
        login: Some("procurement@alwaha.example"),
        onboarding: false,
        permit_ends: 500,
        sites: &[],
    },
    Staff {
        key: "tariq",
        en: "Tariq Mahmood",
        ar: "طارق محمود",
        nationality: "PK",
        born: 1981,
        job_role: STOREKEEPER,
        title: "Storekeeper",
        hired: -450,
        login: Some("storekeeper@alwaha.example"),
        onboarding: false,
        permit_ends: 260,
        sites: &[],
    },
    Staff {
        key: "karim",
        en: "Abdul Karim Mia",
        ar: "عبد الكريم ميا",
        nationality: "BD",
        born: 1988,
        job_role: "worker",
        title: "Carpenter",
        hired: -400,
        login: None,
        onboarding: false,
        permit_ends: 340,
        sites: &[TWR],
    },
    Staff {
        key: "suresh",
        en: "Suresh Babu Nair",
        ar: "سوريش بابو نائر",
        nationality: "IN",
        born: 1986,
        job_role: "worker",
        title: "Electrician",
        hired: -380,
        login: None,
        onboarding: false,
        permit_ends: 380,
        sites: &[TWR],
    },
    Staff {
        key: "imran",
        en: "Imran Hussain Shah",
        ar: "عمران حسين شاه",
        nationality: "PK",
        born: 1990,
        job_role: "worker",
        title: "Painter",
        hired: -350,
        login: None,
        onboarding: false,
        // The permit that expires soon.
        permit_ends: 20,
        sites: &[MALL],
    },
    Staff {
        key: "joselito",
        en: "Joselito Bautista",
        ar: "خوسيليتو باوتيستا",
        nationality: "PH",
        born: 1985,
        job_role: "worker",
        title: "Plumber",
        hired: -300,
        login: None,
        onboarding: false,
        permit_ends: 230,
        sites: &[MALL],
    },
    Staff {
        key: "mahmoud",
        en: "Mahmoud Farag",
        ar: "محمود فرج",
        nationality: "EG",
        born: 1983,
        job_role: "worker",
        title: "Tiler",
        hired: -260,
        login: None,
        onboarding: false,
        permit_ends: 290,
        sites: &[CLN],
    },
    Staff {
        key: "ravi",
        en: "Ravi Kumar Yadav",
        ar: "رافي كومار ياداف",
        nationality: "IN",
        born: 1991,
        job_role: "worker",
        title: "HVAC technician",
        hired: -240,
        login: None,
        onboarding: false,
        permit_ends: 310,
        sites: &[CLN],
    },
    Staff {
        key: "sher",
        en: "Sher Ali Khan",
        ar: "شير علي خان",
        nationality: "PK",
        born: 1989,
        job_role: "worker",
        title: "Labourer",
        hired: -380,
        login: None,
        onboarding: false,
        permit_ends: 270,
        sites: &[RST],
    },
    Staff {
        key: "bilal",
        en: "Bilal Ahmed Rana",
        ar: "بلال أحمد رانا",
        nationality: "PK",
        born: 1993,
        job_role: "worker",
        title: "Gypsum fixer",
        hired: -340,
        login: None,
        onboarding: false,
        permit_ends: 330,
        sites: &[RST],
    },
    Staff {
        key: "dennis",
        en: "Dennis Villanueva",
        ar: "دينيس فيلانويفا",
        nationality: "PH",
        born: 1987,
        job_role: "worker",
        title: "Painter",
        hired: -320,
        login: None,
        onboarding: false,
        // The permit that expired: the records of the worker are flagged.
        permit_ends: -15,
        sites: &[RST],
    },
    Staff {
        key: "anitha",
        en: "Anitha Krishnan",
        ar: "أنيثا كريشنان",
        nationality: "IN",
        born: 1992,
        job_role: QUANTITY_SURVEYOR,
        title: "Quantity surveyor",
        hired: -9,
        login: Some("quantity_surveyor@alwaha.example"),
        onboarding: true,
        permit_ends: 700,
        sites: &[],
    },
    Staff {
        key: "hamza",
        en: "Hamza Siddiqui",
        ar: "حمزة صديقي",
        nationality: "PK",
        born: 1996,
        job_role: "worker",
        title: "Labourer",
        hired: -4,
        login: None,
        onboarding: true,
        permit_ends: 720,
        sites: &[],
    },
];

/// The Emirates ID-style numbers of the demo (DEMO-DATA.md 1.2).
fn residence_visa(n: u32) -> String {
    format!("DEMO-RV-0000{:03}", n % 1000)
}

impl Seed {
    /// The staff as parties and employees.
    pub(crate) async fn staff(&mut self) -> Result {
        let hr = email_of(HR);
        let entity = wl_kernel::LegalEntity::default_entity(self.db()).await?.id;
        for (index, member) in STAFF.iter().enumerate() {
            let n = 500 + index as u32;
            let existing = Party::query()
                .where_(Party::LEGAL_NAME_EN.eq(member.en))
                .first(self.db())
                .await?;
            let party = match existing {
                Some(party) => party,
                None => {
                    let email = member
                        .login
                        .map_or_else(|| format!("{}@alwaha.example", member.key), str::to_owned);
                    let contact = |contact_type, value: String| ContactPoint {
                        contact_type,
                        value,
                        is_primary: true,
                        ..Default::default()
                    };
                    wl_parties::create(
                        &self.state,
                        NewParty {
                            kind: PartyKind::Person,
                            legal_name_en: member.en.to_owned(),
                            legal_name_ar: Some(member.ar.to_owned()),
                            default_language: Language::En,
                            preferred_channel: Channel::Whatsapp,
                            person: Some(PersonDetails {
                                nationality: Some(member.nationality.to_owned()),
                                date_of_birth: NaiveDate::from_ymd_opt(
                                    member.born,
                                    1 + (n % 12),
                                    1 + (n % 27),
                                ),
                                ..Default::default()
                            }),
                            roles: vec![PartyRole {
                                role_key: "employee".to_owned(),
                                ..Default::default()
                            }],
                            contacts: vec![
                                contact(ContactType::Email, email),
                                contact(ContactType::Mobile, format!("+971500000{n}")),
                            ],
                            source: "seed".to_owned(),
                            ..Default::default()
                        },
                    )
                    .await?
                }
            };
            let mut new = NewEmployee::new(PartyId(party.id), member.job_role, day(member.hired))
                .job_title(member.title);
            if let Some(login) = member.login {
                new = new.user(self.user_id(login).await?);
            }
            new.employee_number = format!("AWI-{:04}", index + 1);
            new.legal_entity_id = Some(entity);
            if !member.onboarding {
                new = new.status(EmployeeStatus::Active);
            }
            let hired = upto(day(member.hired));
            let employee = at!(self, hired, { create(&self.state, new).await })?;
            self.employees.insert(member.key, employee.id);
            self.report.add("employees", 1);
            if member.onboarding {
                self.start_steps(&employee, hr, hired).await?;
            }
        }
        Ok(())
    }

    /// The work permit and residence visa of every employee who works on a
    /// site, accepted by the company admin.
    pub(crate) async fn permits(&mut self) -> Result {
        for (index, member) in STAFF.iter().enumerate() {
            if member.sites.is_empty() {
                continue;
            }
            let id = self.employees[member.key];
            let subject = Subject::new(EMPLOYEE_SUBJECT_TYPE, id);
            let n = 500 + index as u32;
            let hired = upto(day(member.hired + 5));
            at!(self, hired, {
                self.document(
                    subject.clone(),
                    "work_permit",
                    "Work permit",
                    Some(format!("WP-0000{:03}", n % 1000)),
                    Some(day(member.permit_ends)),
                    Some(day(member.permit_ends - 730)),
                )
                .await?;
                self.document(
                    subject,
                    "residence_visa",
                    "Residence visa",
                    Some(residence_visa(n)),
                    Some(day(member.permit_ends.max(60) + 40)),
                    Some(day(member.permit_ends - 730)),
                )
                .await
            })?;
        }
        Ok(())
    }

    /// Puts the workers on the projects they work on (their scopes, which the
    /// absence rule reads), once the projects exist.
    pub(crate) async fn assign_sites(&mut self) -> Result {
        for member in STAFF {
            for code in member.sites {
                let project = construction::Project::query()
                    .where_(construction::Project::CODE.eq(*code))
                    .first(self.db())
                    .await?
                    .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no project {code}"))?;
                wl_hr::employees::set_scope(
                    &self.state,
                    self.employees[member.key],
                    &project.subject(),
                    true,
                )
                .await?;
            }
        }
        Ok(())
    }

    /// HR does the steps it owns of a new hire's onboarding that are due.
    async fn start_steps(&mut self, employee: &Employee, hr: &str, hired: NaiveDate) -> Result {
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
