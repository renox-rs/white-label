//! The people of the demo centre: the staff with a login, the students and
//! their guardians (one student blocked), the company that pays for two
//! employees' course, and the employees of wl-hr behind the staff, with the
//! tutors' documents (a police clearance that ends in 20 days, one that ended
//! 10 days ago, a freelance permit that ends in 34 days).

use edtech::seed::{POLICE_CLEARANCE_KIND, STUDENT_ID_KIND, TUTOR_PERMIT_KIND};
use renox::chrono::{Days, Duration, NaiveDate};
use renox::prelude::*;
use wl_documents::{Document, DocumentKind, DocumentStatus};
use wl_hr::{
    EmployeeStatus, EmploymentType, NewEmployee, NewQualification, SalaryStructure, employees,
};
use wl_kernel::workforce::EmployeeId;
use wl_kernel::{PartyId, PartyKind, Subject};
use wl_parties::relationships::{NewRelationship, RelationshipType, relate};
use wl_parties::{
    Channel, CompanyDetails, ContactPoint, ContactType, Language, NewParty, PartyRole,
    PersonDetails,
};

use super::super::demo_data::day;
use super::super::seed::PASSWORD;
use super::{ADMIN_LOGIN, HIND, LAYLA, LOGINS, OMAR, RAVI, SARA, Seed};

/// A student or guardian.
pub(crate) struct Person {
    pub(crate) name: &'static str,
    pub(crate) name_ar: &'static str,
    pub(crate) born: (i32, u32, u32),
    pub(crate) nationality: &'static str,
    pub(crate) language: Language,
}

const fn person(
    name: &'static str,
    name_ar: &'static str,
    born: (i32, u32, u32),
    nationality: &'static str,
    language: Language,
) -> Person {
    Person {
        name,
        name_ar,
        born,
        nationality,
        language,
    }
}

pub(crate) const AMAL: usize = 0;
pub(crate) const PRIYA: usize = 1;
pub(crate) const JAMES: usize = 2;
pub(crate) const FATIMA: usize = 3;
pub(crate) const ARJUN: usize = 4;
pub(crate) const ELENA: usize = 5;
pub(crate) const HASSAN: usize = 6;
pub(crate) const MARIA: usize = 7;
pub(crate) const KHALID: usize = 8;
pub(crate) const NADIA: usize = 9;
pub(crate) const DANIEL: usize = 10;
/// Blocked: two invoices unpaid after three reminders.
pub(crate) const RANIA: usize = 11;
pub(crate) const LUCAS: usize = 12;
pub(crate) const SOFIA: usize = 13;
pub(crate) const YOUSEF: usize = 14;
pub(crate) const MONA: usize = 15;
pub(crate) const AARAV: usize = 16;
/// Sixteen: a teenager in an adult course, with her father as guardian.
pub(crate) const LAYAN: usize = 17;
pub(crate) const KOFI: usize = 18;
/// Fourteen: the younger sister of Layan, in a French class.
pub(crate) const NOOR: usize = 19;

/// The students, in the order of the constants above.
pub(crate) const STUDENTS: &[Person] = &[
    person(
        "Amal Al Hammadi",
        "أمل الحمادي",
        (1993, 2, 14),
        "AE",
        Language::Ar,
    ),
    person("Priya Nair", "بريا ناير", (1990, 11, 2), "IN", Language::En),
    person(
        "James Carter",
        "جيمس كارتر",
        (1985, 6, 21),
        "GB",
        Language::En,
    ),
    person(
        "Fatima Al Zaabi",
        "فاطمة الزعابي",
        (1996, 7, 25),
        "AE",
        Language::Ar,
    ),
    person(
        "Arjun Patel",
        "أرجون باتل",
        (1992, 3, 9),
        "IN",
        Language::En,
    ),
    person(
        "Elena Petrova",
        "إيلينا بتروفا",
        (1991, 12, 21),
        "RU",
        Language::En,
    ),
    person("Hassan Raza", "حسن رضا", (1988, 9, 8), "PK", Language::En),
    person(
        "Maria Santos",
        "ماريا سانتوس",
        (1994, 5, 30),
        "PH",
        Language::En,
    ),
    person(
        "Khalid Bin Saeed",
        "خالد بن سعيد",
        (1989, 4, 4),
        "AE",
        Language::Ar,
    ),
    person(
        "Nadia Farouk",
        "نادية فاروق",
        (1997, 1, 18),
        "EG",
        Language::Ar,
    ),
    person(
        "Daniel Okoye",
        "دانيال أوكوي",
        (1986, 10, 3),
        "NG",
        Language::En,
    ),
    person(
        "Rania Aziz",
        "رانيا عزيز",
        (1979, 2, 19),
        "EG",
        Language::Ar,
    ),
    person(
        "Lucas Meyer",
        "لوكاس ماير",
        (1995, 8, 12),
        "DE",
        Language::En,
    ),
    person(
        "Sofia Garcia",
        "صوفيا غارسيا",
        (1999, 8, 14),
        "ES",
        Language::En,
    ),
    person(
        "Yousef Al Suwaidi",
        "يوسف السويدي",
        (2017, 6, 9),
        "AE",
        Language::Ar,
    ),
    person(
        "Mona Al Suwaidi",
        "منى السويدي",
        (2019, 3, 3),
        "AE",
        Language::Ar,
    ),
    person(
        "Aarav Menon",
        "آراف مينون",
        (2016, 9, 27),
        "IN",
        Language::En,
    ),
    person(
        "Layan Haddad",
        "لين حداد",
        (2010, 4, 11),
        "JO",
        Language::Ar,
    ),
    person(
        "Kofi Mensah",
        "كوفي منساه",
        (2015, 1, 5),
        "GH",
        Language::En,
    ),
    person(
        "Noor Haddad",
        "نور حداد",
        (2012, 10, 21),
        "JO",
        Language::Ar,
    ),
];

pub(crate) const AHMED: usize = 0;
pub(crate) const SUNITA: usize = 1;
pub(crate) const OMAR_H: usize = 2;
pub(crate) const GRACE: usize = 3;

/// The guardians, in the order of the constants above.
pub(crate) const GUARDIANS: &[Person] = &[
    person(
        "Ahmed Al Suwaidi",
        "أحمد السويدي",
        (1984, 3, 12),
        "AE",
        Language::Ar,
    ),
    person(
        "Sunita Menon",
        "سونيتا مينون",
        (1987, 8, 1),
        "IN",
        Language::En,
    ),
    person("Omar Haddad", "عمر حداد", (1981, 12, 5), "JO", Language::Ar),
    person(
        "Grace Mensah",
        "غريس منساه",
        (1986, 6, 16),
        "GH",
        Language::En,
    ),
];

/// The guardian of a child student (index of [`STUDENTS`] to [`GUARDIANS`]).
pub(crate) const GUARDIAN_OF: &[(usize, usize)] = &[
    (YOUSEF, AHMED),
    (MONA, AHMED),
    (AARAV, SUNITA),
    (LAYAN, OMAR_H),
    (KOFI, GRACE),
    (NOOR, OMAR_H),
];

/// A tutor with an employee record.
struct Tutor {
    email: &'static str,
    freelance: bool,
    /// Days left on the police clearance (negative: ended).
    clearance: i64,
    /// Days left on the permit or freelance visa, for a freelance tutor.
    permit: Option<i64>,
    number: &'static str,
}

const TUTORS: &[Tutor] = &[
    Tutor {
        email: SARA,
        freelance: false,
        clearance: 400,
        permit: None,
        number: "PC-2026-1101",
    },
    // `EDU.RUL.12`: a clearance that ends in 20 days.
    Tutor {
        email: OMAR,
        freelance: false,
        clearance: 20,
        permit: None,
        number: "PC-2026-1102",
    },
    Tutor {
        email: LAYLA,
        freelance: true,
        clearance: 300,
        permit: Some(200),
        number: "PC-2026-1103",
    },
    // A freelance permit that ends in 34 days.
    Tutor {
        email: RAVI,
        freelance: true,
        clearance: 300,
        permit: Some(34),
        number: "PC-2026-1104",
    },
    // A clearance that ended 10 days ago.
    Tutor {
        email: HIND,
        freelance: false,
        clearance: -10,
        permit: None,
        number: "PC-2025-0907",
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

fn key(name: &str) -> String {
    name.to_lowercase().replace(' ', ".")
}

impl Seed {
    /// The users, with their role.
    pub(crate) async fn logins(&mut self) -> Result {
        for login in LOGINS {
            let user = match User::find_by_email(self.db(), login.email).await? {
                Some(user) => user,
                None => {
                    let mut user =
                        User::register(self.db(), login.name, login.email, PASSWORD).await?;
                    user.email_verified_at = Some(renox::db::now());
                    user.save(self.db()).await?;
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

    async fn person(&self, person: &Person, role: &str, serial: usize) -> Result<PartyId> {
        let (year, month, date) = person.born;
        let party = wl_parties::create(
            &self.state,
            NewParty {
                kind: PartyKind::Person,
                legal_name_en: person.name.to_owned(),
                legal_name_ar: Some(person.name_ar.to_owned()),
                person: Some(PersonDetails {
                    date_of_birth: NaiveDate::from_ymd_opt(year, month, date),
                    nationality: Some(person.nationality.to_owned()),
                    ..PersonDetails::default()
                }),
                default_language: person.language,
                preferred_channel: Channel::Whatsapp,
                roles: vec![PartyRole {
                    role_key: role.to_owned(),
                    ..PartyRole::default()
                }],
                contacts: vec![
                    contact(ContactType::Mobile, &format!("+97150002{serial:04}")),
                    contact(
                        ContactType::Email,
                        &format!("{}@students.gulfskills.example", key(person.name)),
                    ),
                ],
                source: "seed".to_owned(),
                ..NewParty::default()
            },
        )
        .await?;
        Ok(party.party_id())
    }

    /// A document of `kind` of `subject`, accepted, valid until `until`.
    pub(crate) async fn document(
        &self,
        subject: Subject,
        kind: &str,
        number: &str,
        until: NaiveDate,
        authority: &str,
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
                issuing_authority: Some(authority.to_owned()),
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

    /// The students, the guardians (the children are their dependants), the
    /// company, the student IDs on file and the blocked student.
    pub(crate) async fn people(&mut self) -> Result {
        for (index, student) in STUDENTS.iter().enumerate() {
            let id = self.person(student, "student", index).await?;
            self.students.push(id);
        }
        for (index, guardian) in GUARDIANS.iter().enumerate() {
            let id = self.person(guardian, "guardian", 100 + index).await?;
            self.guardians.push(id);
        }
        for (child, guardian) in GUARDIAN_OF {
            relate(
                &self.state,
                NewRelationship {
                    from: self.guardians[*guardian].0,
                    to: self.students[*child].0,
                    relation_type: RelationshipType::GuardianOf,
                    primary: true,
                    // From before the first enrolment of the seed.
                    valid_from: Some(day(-800)),
                    ..NewRelationship::default()
                },
            )
            .await?;
        }
        let company = wl_parties::create(
            &self.state,
            NewParty {
                kind: PartyKind::Company,
                legal_name_en: "Najm Express Cargo LLC".to_owned(),
                legal_name_ar: Some("نجم للشحن السريع ذ.م.م".to_owned()),
                company: Some(CompanyDetails {
                    legal_form: Some("llc".to_owned()),
                    licence_authority: Some("Dubai Economy".to_owned()),
                    trade_licence_no: Some("DEMO-710021".to_owned()),
                    ..CompanyDetails::default()
                }),
                roles: vec![PartyRole {
                    role_key: "corporate_client".to_owned(),
                    ..PartyRole::default()
                }],
                contacts: vec![contact(ContactType::Email, "training@najm-cargo.example")],
                source: "seed".to_owned(),
                ..NewParty::default()
            },
        )
        .await?;
        self.company = company.party_id();
        // The student IDs on file, one of them ending in 25 days.
        for (index, left) in [(AMAL, 400), (PRIYA, 25), (KHALID, 180), (YOUSEF, 365)] {
            self.document(
                Subject::new(
                    wl_kernel::parties::PARTY_SUBJECT_TYPE,
                    self.students[index].0,
                ),
                STUDENT_ID_KIND,
                &format!("784-19{}-{:07}-1", 80 + index, 1_234_000 + index * 111),
                day(left),
                "Federal Authority for Identity",
            )
            .await?;
        }
        Ok(())
    }

    /// The student who does not pay is blocked (`PTY.BLK.01`): done after her
    /// enrolment exists, so that it freezes (cause `block`).
    pub(crate) async fn block_rania(&self) -> Result {
        self.driver
            .post(
                ADMIN_LOGIN,
                &format!("/parties/{}/block", self.students[RANIA].0),
                &[
                    ("reason_code", "non_payment"),
                    (
                        "note",
                        "Two invoices unpaid after three reminders; the seat is on hold.",
                    ),
                ],
            )
            .await?;
        Ok(())
    }

    /// The employees: every login is a member of staff in wl-hr; the tutors
    /// hold their police clearance and, when freelance, the permit as accepted
    /// documents and qualifications.
    pub(crate) async fn staff(&mut self) -> Result {
        for (number, login) in LOGINS.iter().enumerate() {
            let number = number as i64 + 1;
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
            let tutor = TUTORS.iter().find(|t| t.email == login.email);
            let mut new = NewEmployee::new(party.party_id(), login.role, day(-700 + number * 17))
                .status(EmployeeStatus::Active);
            new.user_id = Some(user.id);
            new.legal_entity_id = Some(self.entity);
            new.employee_number = format!("GST-{number:04}");
            new.job_title = login.label.to_owned();
            if tutor.is_some_and(|t| t.freelance) {
                new.employment_type = EmploymentType::Freelance;
            }
            let employee = employees::create(&self.state, new).await?;
            if tutor.is_none_or(|t| !t.freelance) {
                SalaryStructure::create(
                    self.db(),
                    SalaryStructure {
                        employee_id: employee.id,
                        effective_from: day(-700 + number * 17),
                        basic: if tutor.is_some() { 900_000 } else { 1_200_000 },
                        housing: 250_000,
                        transport: 60_000,
                        ..SalaryStructure::default()
                    },
                )
                .await?;
            }
            if let Some(tutor) = tutor {
                self.tutor_documents(EmployeeId(employee.id), tutor).await?;
            }
            self.employees.push((login.email, EmployeeId(employee.id)));
        }
        Ok(())
    }

    async fn tutor_documents(&self, employee: EmployeeId, tutor: &Tutor) -> Result {
        let subject = employee.subject();
        let mut kinds = vec![(
            POLICE_CLEARANCE_KIND,
            tutor.number.to_owned(),
            "Police clearance",
            tutor.clearance,
            "Dubai Police",
        )];
        if let Some(left) = tutor.permit {
            kinds.push((
                TUTOR_PERMIT_KIND,
                format!("{}-P", tutor.number),
                "Private tutor permit",
                left,
                "MoHRE",
            ));
        }
        for (kind, number, label, left, authority) in kinds {
            let document = self
                .document(subject.clone(), kind, &number, day(left), authority)
                .await?;
            let mut qualification = NewQualification::new(employee, kind)
                .number(&number)
                .valid_to(day(left));
            qualification.label = label.to_owned();
            qualification.issuer = authority.to_owned();
            qualification.document = document.map(|d| d.id);
            wl_hr::qualification::record(self.db(), qualification).await?;
        }
        Ok(())
    }

    /// The employee behind a login.
    pub(crate) fn employee_of(&self, email: &str) -> i64 {
        self.employees
            .iter()
            .find(|(known, _)| *known == email)
            .map(|(_, employee)| employee.0)
            .unwrap_or_default()
    }
}
