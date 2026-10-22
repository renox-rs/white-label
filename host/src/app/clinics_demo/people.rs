//! The people of the demo clinic: the staff with a login and their roles in
//! their branch, the twelve patients and their contacts, the payers, and the
//! employees of wl-hr behind the practitioners (licence and privileges, a
//! leave still waiting for its decision).

use clinics::seed::{LICENCE_KIND, PRIVILEGES_KIND};
use renox::chrono::{Days, Duration, NaiveDate};
use renox::prelude::*;
use wl_documents::{Document, DocumentKind, DocumentStatus};
use wl_hr::{EmployeeStatus, NewEmployee, NewQualification, SalaryStructure, employees};
use wl_kernel::workforce::EmployeeId;
use wl_kernel::{PartyId, PartyKind, Subject};
use wl_parties::relationships::{NewRelationship, RelationshipType, relate};
use wl_parties::{
    Channel, CompanyDetails, ContactPoint, ContactType, Language, NewParty, PartyRole,
    PersonDetails,
};

use super::super::demo_data::day;
use super::super::seed::PASSWORD;
use super::{LOGINS, Seed, branch};

/// A patient of the clinic.
pub(crate) struct Patient {
    pub(crate) name: &'static str,
    pub(crate) born: (i32, u32, u32),
    pub(crate) nationality: &'static str,
    pub(crate) language: Language,
    /// The mobile number, a fictional one.
    pub(crate) mobile: &'static str,
}

pub(crate) const AHMED: usize = 0;
pub(crate) const FATIMA: usize = 1;
pub(crate) const PRIYA: usize = 2;
pub(crate) const JOHN: usize = 3;
pub(crate) const LAYAN: usize = 4;
pub(crate) const MOHAMMED: usize = 5;
pub(crate) const ELENA: usize = 6;
pub(crate) const KHALID: usize = 7;
pub(crate) const SOFIA: usize = 8;
pub(crate) const YOUSEF: usize = 9;
pub(crate) const RANIA: usize = 10;
pub(crate) const DANIEL: usize = 11;

/// The patients, in the order of the constants above.
pub(crate) const PATIENTS: &[Patient] = &[
    Patient {
        name: "Ahmed Al Suwaidi",
        born: (1984, 3, 12),
        nationality: "AE",
        language: Language::Ar,
        mobile: "+971500010001",
    },
    Patient {
        name: "Fatima Al Zaabi",
        born: (1990, 7, 25),
        nationality: "AE",
        language: Language::Ar,
        mobile: "+971500010002",
    },
    Patient {
        name: "Priya Menon",
        born: (1988, 11, 2),
        nationality: "IN",
        language: Language::En,
        mobile: "+971500010003",
    },
    Patient {
        name: "John Whitaker",
        born: (1975, 1, 30),
        nationality: "GB",
        language: Language::En,
        mobile: "+971500010004",
    },
    Patient {
        name: "Layan Haddad",
        born: (1995, 5, 17),
        nationality: "JO",
        language: Language::Ar,
        mobile: "+971500010005",
    },
    Patient {
        name: "Mohammed Raza",
        born: (1982, 9, 8),
        nationality: "PK",
        language: Language::En,
        mobile: "+971500010006",
    },
    Patient {
        name: "Elena Petrova",
        born: (1992, 12, 21),
        nationality: "RU",
        language: Language::En,
        mobile: "+971500010007",
    },
    Patient {
        name: "Khalid Bin Saeed",
        born: (1969, 4, 4),
        nationality: "AE",
        language: Language::Ar,
        mobile: "+971500010008",
    },
    Patient {
        name: "Sofia Garcia",
        born: (1999, 8, 14),
        nationality: "ES",
        language: Language::En,
        mobile: "+971500010009",
    },
    // A child: Ahmed Al Suwaidi is the guardian.
    Patient {
        name: "Yousef Al Suwaidi",
        born: (2016, 6, 9),
        nationality: "AE",
        language: Language::Ar,
        mobile: "+971500010001",
    },
    // Blocked: an unpaid balance.
    Patient {
        name: "Rania Aziz",
        born: (1979, 2, 19),
        nationality: "EG",
        language: Language::Ar,
        mobile: "+971500010011",
    },
    Patient {
        name: "Daniel Okoye",
        born: (1986, 10, 3),
        nationality: "NG",
        language: Language::En,
        mobile: "+971500010012",
    },
];

/// The payers of the clinic.
pub(crate) const PAYERS: &[&str] = &[
    "Gulf Shield Insurance PJSC",
    "Crescent Health TPA LLC",
    "Oasis Medical Network LLC",
];

/// A practitioner with an employee record.
pub(crate) struct Doctor {
    pub(crate) email: &'static str,
    pub(crate) role_title: &'static str,
    pub(crate) branch: i64,
    /// The days left on the licence.
    pub(crate) licence_days: i64,
    pub(crate) licence_no: &'static str,
}

pub(crate) const DOCTORS: &[Doctor] = &[
    Doctor {
        email: "nadia@palmgrove.example",
        role_title: "Family physician",
        branch: super::JUMEIRAH,
        licence_days: 400,
        licence_no: "DHA-P-481516",
    },
    Doctor {
        email: "samir@palmgrove.example",
        role_title: "Dentist",
        branch: super::BUSINESS_BAY,
        licence_days: 310,
        licence_no: "DHA-P-234234",
    },
    // `CLN.RUL.01`: a licence that ends in 20 days, with its 30-day reminder.
    Doctor {
        email: "yara@palmgrove.example",
        role_title: "Physiotherapist",
        branch: super::JUMEIRAH,
        licence_days: 20,
        licence_no: "DHA-P-162342",
    },
];

fn contact(contact_type: ContactType, value: &str) -> ContactPoint {
    ContactPoint {
        contact_type,
        value: value.to_owned(),
        is_primary: true,
        ..ContactPoint::default()
    }
}

impl Seed {
    /// The users, with their role in their branch.
    pub(crate) async fn logins(&mut self) -> Result {
        for login in LOGINS {
            let user = match User::find_by_email(self.db(), login.email).await? {
                Some(user) => user,
                None => {
                    let mut user =
                        User::register(self.db(), login.name, login.email, PASSWORD).await?;
                    user.email_verified_at = Some(renox::db::now());
                    user.save(self.db()).await?;
                    // Held everywhere: the clinics pages ask for the permission
                    // outside any branch, so a role held in a branch alone
                    // opens none of them. The branch is the employee's scope.
                    user.assign_role(self.db(), login.role).await?;
                    user
                }
            };
            self.driver
                .sign_in(&self.state.db, login.email, &user)
                .await?;
            self.users.push((login.email, user));
        }
        Ok(())
    }

    async fn person(&self, patient: &Patient) -> Result<PartyId> {
        let (year, month, date) = patient.born;
        let key = patient.name.to_lowercase().replace(' ', ".");
        let party = wl_parties::create(
            &self.state,
            NewParty {
                kind: PartyKind::Person,
                legal_name_en: patient.name.to_owned(),
                person: Some(PersonDetails {
                    date_of_birth: NaiveDate::from_ymd_opt(year, month, date),
                    nationality: Some(patient.nationality.to_owned()),
                    ..PersonDetails::default()
                }),
                default_language: patient.language,
                preferred_channel: Channel::Whatsapp,
                roles: vec![PartyRole {
                    role_key: "patient".to_owned(),
                    ..PartyRole::default()
                }],
                contacts: vec![
                    contact(ContactType::Mobile, patient.mobile),
                    contact(
                        ContactType::Email,
                        &format!("{key}@patients.palmgrove.example"),
                    ),
                ],
                source: "seed".to_owned(),
                ..NewParty::default()
            },
        )
        .await?;
        Ok(party.party_id())
    }

    /// The twelve patients, the child's guardian, the blocked patient, the
    /// payers and the Emirates IDs on file.
    pub(crate) async fn patients(&mut self) -> Result {
        for patient in PATIENTS {
            let id = self.person(patient).await?;
            self.patients.push(id);
        }
        // `PTY.REL.01`: Yousef is Ahmed's dependant; the guardian is told.
        relate(
            &self.state,
            NewRelationship {
                from: self.patients[AHMED].0,
                to: self.patients[YOUSEF].0,
                relation_type: RelationshipType::GuardianOf,
                primary: true,
                ..NewRelationship::default()
            },
        )
        .await?;
        relate(
            &self.state,
            NewRelationship {
                from: self.patients[FATIMA].0,
                to: self.patients[AHMED].0,
                relation_type: RelationshipType::FamilyMemberOf,
                ..NewRelationship::default()
            },
        )
        .await?;
        for (index, name) in PAYERS.iter().enumerate() {
            let party = wl_parties::create(
                &self.state,
                NewParty {
                    kind: PartyKind::Company,
                    legal_name_en: (*name).to_owned(),
                    company: Some(CompanyDetails {
                        legal_form: Some("llc".to_owned()),
                        licence_authority: Some("Dubai Insurance Authority".to_owned()),
                        trade_licence_no: Some(format!("77100{index}")),
                        ..CompanyDetails::default()
                    }),
                    roles: vec![PartyRole {
                        role_key: "payer".to_owned(),
                        ..PartyRole::default()
                    }],
                    contacts: vec![contact(
                        ContactType::Email,
                        &format!("claims{}@payers.example", index + 1),
                    )],
                    source: "seed".to_owned(),
                    ..NewParty::default()
                },
            )
            .await?;
            self.payers.push(party.party_id());
        }
        // The Emirates IDs on file, one of them ending in 25 days.
        for (index, left) in [(AHMED, 400), (FATIMA, 310), (PRIYA, 25), (KHALID, 180)] {
            self.document(
                Subject::new(
                    wl_kernel::parties::PARTY_SUBJECT_TYPE,
                    self.patients[index].0,
                ),
                clinics::seed::EMIRATES_ID_KIND,
                &format!("784-19{}-{:07}-1", 80 + index, 1_234_000 + index * 111),
                day(left),
            )
            .await?;
        }
        // Rania has not paid her last invoices: a patient who is blocked is
        // not booked (`PTY.BLK.01`).
        let admin = self.user("admin@palmgrove.example").email.clone();
        let blocked = self.patients[RANIA];
        self.driver
            .post(
                &admin,
                &format!("/parties/{}/block", blocked.0),
                &[
                    ("reason_code", "non_payment"),
                    ("note", "Two patient invoices unpaid after three reminders."),
                ],
            )
            .await?;
        Ok(())
    }

    /// A document of `kind` of `subject`, accepted, valid until `until`.
    pub(crate) async fn document(
        &self,
        subject: Subject,
        kind: &str,
        number: &str,
        until: NaiveDate,
    ) -> Result<Option<Document>> {
        let Some(row) = DocumentKind::by_key(self.db(), kind).await? else {
            return Ok(None);
        };
        let document = Document::create(
            self.db(),
            Document {
                subject_type: subject.subject_type,
                subject_id: subject.subject_id,
                kind_id: row.id,
                title: row.name_en.clone(),
                number: Some(number.to_owned()),
                issue_date: until.checked_sub_days(Days::new(730)),
                expiry_date: Some(until),
                issuing_authority: Some("Dubai Health Authority".to_owned()),
                current_version: 1,
                submitted_at: Some(renox::db::now() - Duration::days(400)),
                accepted_at: Some(renox::db::now() - Duration::days(400)),
                status: DocumentStatus::Accepted,
                channel: "back_office".to_owned(),
                ..Document::default()
            },
        )
        .await?;
        Ok(Some(document))
    }

    /// The employees: every login is a member of staff in wl-hr, and the
    /// practitioners hold their licence and branch privileges as accepted
    /// documents and qualifications (`booking.book` is refused without
    /// them); a leave waits for its decision.
    pub(crate) async fn staff(&mut self) -> Result {
        let mut number = 0;
        for login in LOGINS {
            // The administrator, the owner and the accounting roles are not
            // on the floor.
            let role = login.role;
            let user = self.user(login.email).clone();
            let party = wl_parties::create(
                &self.state,
                NewParty {
                    kind: PartyKind::Person,
                    legal_name_en: login.name.to_owned(),
                    person: Some(PersonDetails::default()),
                    default_language: Language::En,
                    preferred_channel: Channel::Email,
                    roles: vec![PartyRole {
                        role_key: "employee".to_owned(),
                        ..PartyRole::default()
                    }],
                    contacts: vec![contact(ContactType::Email, login.email)],
                    source: "seed".to_owned(),
                    ..NewParty::default()
                },
            )
            .await?;
            number += 1;
            let doctor = DOCTORS.iter().find(|d| d.email == login.email);
            let mut new = NewEmployee::new(party.party_id(), role, day(-700 + number * 11))
                .status(EmployeeStatus::Active);
            new.user_id = Some(user.id);
            new.legal_entity_id = Some(self.entity);
            new.employee_number = format!("PGC-{number:04}");
            new.job_title =
                doctor.map_or_else(|| login.label.to_owned(), |d| d.role_title.to_owned());
            if let Some(id) = login.branch {
                new = new.scope(branch(id));
            }
            let employee = employees::create(&self.state, new).await?;
            SalaryStructure::create(
                self.db(),
                SalaryStructure {
                    employee_id: employee.id,
                    effective_from: day(-700 + number * 11),
                    basic: if doctor.is_some() { 1_800_000 } else { 650_000 },
                    housing: if doctor.is_some() { 450_000 } else { 160_000 },
                    transport: 60_000,
                    ..SalaryStructure::default()
                },
            )
            .await?;
            if let Some(doctor) = doctor {
                self.practitioner(EmployeeId(employee.id), doctor).await?;
            }
            self.employees.push((login.email, EmployeeId(employee.id)));
        }
        Ok(())
    }

    /// The licence and the branch privileges of `doctor`.
    async fn practitioner(&mut self, employee: EmployeeId, doctor: &Doctor) -> Result {
        let subject = employee.subject();
        for (kind, number, label, left) in [
            (
                LICENCE_KIND,
                doctor.licence_no.to_owned(),
                "Practitioner licence",
                doctor.licence_days,
            ),
            (
                PRIVILEGES_KIND,
                format!("{}-PRV", doctor.licence_no),
                "Branch privileges",
                doctor.licence_days + 200,
            ),
        ] {
            let document = self
                .document(subject.clone(), kind, &number, day(left))
                .await?;
            let mut qualification = NewQualification::new(employee, kind)
                .number(&number)
                .scope(branch(doctor.branch))
                .valid_to(day(left));
            qualification.label = label.to_owned();
            qualification.issuer = "Dubai Health Authority".to_owned();
            qualification.document = document.map(|d| d.id);
            wl_hr::qualification::record(self.db(), qualification).await?;
        }
        self.doctors.push(employee);
        Ok(())
    }
}
