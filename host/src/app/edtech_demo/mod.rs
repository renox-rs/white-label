//! The demo centre of the EdTech client (DEMO-DATA.md, 10; *Gulf Skills
//! Training Centre LLC*, all fictional): `edtech:demo` fills a local database
//! so that every screen of the app has something to show the moment someone
//! logs in, and `db:seed` runs it in `local`, never in production.
//!
//! It follows the Clinics and Manpower demos (`super::clinics_demo`):
//!
//! - **Through the real services.** What a person does at a screen (define a
//!   course, plan and open a batch, enrol a student, accept the terms, freeze,
//!   withdraw, transfer, mark the attendance, cancel a session, cancel or
//!   terminate a batch, issue or revoke a certificate, put a student on the
//!   waiting list) is a request to the app's own router as the demo user who
//!   does it ([`Driver`]), so the permissions, the guards and the state
//!   machines are the real ones and a refusal stops the seed with the page's
//!   message. Money goes through wl-invoicing (the enrolment's invoice,
//!   issued by the accountant) and wl-payments (the payment recorder, the
//!   refund and the chargeback of the accountant's own pages). A record in a
//!   state the machines only reach after days of real life is inserted in
//!   that state with `insert_in_state`, and the place says why: the only one
//!   is the card payment of the online gateway, because no gateway adapter is
//!   installed in the demo (as in the clinics demo).
//! - **Relative dates.** Every date is `day(n)`, the anchor day plus or minus
//!   days (Asia/Dubai). The anchor is the latest Friday up to today
//!   ([`anchor_day`]): the batches meet on fixed weekdays and the stories
//!   hold and mark their sessions on fixed days, so the demo tells the same
//!   story whatever weekday it is seeded on (on a Friday the anchor is today). A step of the past runs on the seed's [`Clock`] set to its
//!   day: a batch is planned a month before it starts, its students enrol
//!   and pay in the weeks before, each session is marked on the evening it
//!   was taught, so the numbers, the invoices and the audit trail carry the
//!   right days.
//! - **Money in fils**, VAT 5 % inside the prices.
//! - **Deterministic**: names, amounts and days are written out below.
//! - **Idempotent.** The seed is done once its marker exists (an audit entry
//!   of the subject `edtech_demo`); a login that exists is kept.
//!
//! # The centre
//!
//! One licensed training centre in Dubai (the client file has one legal entity
//! and no campuses), eight courses (language, IT, exam preparation, kids), five
//! tutors (two freelance), twenty-odd students (adults and children with their
//! guardians, one blocked), thirteen batches and the stories on the screens:
//!
//! | Screen | What it shows |
//! |---|---|
//! | Courses | the four programmes with fees, two with a regulator approval on file |
//! | Batches | planned, open (one full, with a waiting list), running, completed, cancelled and terminated |
//! | Sessions | scheduled, held and cancelled (with a make-up), flagged for a tutor without a valid document |
//! | Enrolments | pending payment, active, frozen (by the student, by a block), withdrawn with a refund, lapsed, completed, transferred, a package |
//! | Certificates | draft, issued (public verification page), revoked, reissued |
//! | Invoices, payments | instalments, cash, card and transfer, a payment charged back, a refund |
//! | Documents | the centre permit that ends in 45 days, a tutor clearance that ends in 20, one that ended 10 days ago |
//! | Leads, campaigns, tickets | leads in every state of a counsellor's board, two campaigns, complaints and tasks |
//!
//! Every login has the password of [`super::seed::PASSWORD`].

mod batches;
mod centre;
mod enrolments;
mod hold;
mod money;
mod notes;
mod people;
mod sales;
mod stories;

use std::collections::BTreeMap;

use renox::Environment;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::workforce::EmployeeId;

use super::demo_data::clock::Clock;
use super::demo_data::driver::Driver;
use super::roles::{
    ACADEMIC_COORDINATOR, ACCOUNTANT, ADMIN, CENTRE_MANAGER, COUNSELLOR, OWNER, TUTOR,
};
use super::seed::PASSWORD;

/// The command that fills a local database with the demo centre.
pub const COMMAND: &str = "edtech:demo";

/// The audit subject of the marker of a finished seed.
const MARKER_SUBJECT: &str = "edtech_demo";
const DONE_ACTION: &str = "edtech_demo.seeded";

/// One demo login.
#[derive(Debug, Clone, Copy)]
pub struct Login {
    /// The name shown on the login page.
    pub name: &'static str,
    /// The sign-in e-mail.
    pub email: &'static str,
    /// The role.
    pub role: &'static str,
    /// What it is for, shown beside the name.
    pub label: &'static str,
}

/// The e-mails the steps below act as.
pub(crate) const ADMIN_LOGIN: &str = "admin@gulfskills.example";
pub(crate) const OWNER_LOGIN: &str = "owner@gulfskills.example";
pub(crate) const MANAGER: &str = "manager@gulfskills.example";
pub(crate) const COUNSELLOR_LOGIN: &str = "counsellor@gulfskills.example";
pub(crate) const COORDINATOR: &str = "coordinator@gulfskills.example";
pub(crate) const ACCOUNTANT_LOGIN: &str = "accountant@gulfskills.example";
pub(crate) const SARA: &str = "tutor@gulfskills.example";
pub(crate) const OMAR: &str = "omar@gulfskills.example";
pub(crate) const LAYLA: &str = "layla@gulfskills.example";
pub(crate) const RAVI: &str = "ravi@gulfskills.example";
pub(crate) const HIND: &str = "hind@gulfskills.example";

/// The demo people, one per role (and one per tutor).
pub const LOGINS: &[Login] = &[
    Login {
        name: "Noura Al Mazrouei",
        email: ADMIN_LOGIN,
        role: ADMIN,
        label: "Company administrator",
    },
    Login {
        name: "Khalid Al Suwaidi",
        email: OWNER_LOGIN,
        role: OWNER,
        label: "Owner of the centre",
    },
    Login {
        name: "Mariam Haddad",
        email: MANAGER,
        role: CENTRE_MANAGER,
        label: "Centre manager",
    },
    Login {
        name: "Rania Khoury",
        email: COUNSELLOR_LOGIN,
        role: COUNSELLOR,
        label: "Admissions counsellor (front desk)",
    },
    Login {
        name: "Dr Tariq Nasser",
        email: COORDINATOR,
        role: ACADEMIC_COORDINATOR,
        label: "Academic coordinator",
    },
    Login {
        name: "Sara Whitfield",
        email: SARA,
        role: TUTOR,
        label: "Tutor (English and IELTS), employed",
    },
    Login {
        name: "Omar Siddiqui",
        email: OMAR,
        role: TUTOR,
        label: "Tutor (IT), employed, police clearance ends in 20 days",
    },
    Login {
        name: "Layla Hassan",
        email: LAYLA,
        role: TUTOR,
        label: "Tutor (Arabic, kids), freelance",
    },
    Login {
        name: "Ravi Menon",
        email: RAVI,
        role: TUTOR,
        label: "Tutor (GMAT), freelance, permit ends in 34 days",
    },
    Login {
        name: "Hind Al Shamsi",
        email: HIND,
        role: TUTOR,
        label: "Tutor (French), employed, police clearance ended 10 days ago",
    },
    Login {
        name: "Faisal Gupta",
        email: ACCOUNTANT_LOGIN,
        role: ACCOUNTANT,
        label: "Accountant",
    },
];

/// The module that adds the command and the seeder.
pub struct EdtechDemo {
    client: wl_kernel::ClientConfig,
}

impl EdtechDemo {
    /// The module for the client `config`.
    pub fn new(config: wl_kernel::ClientConfig) -> Self {
        Self { client: config }
    }
}

impl Module for EdtechDemo {
    fn name(&self) -> &'static str {
        "edtech_demo"
    }

    fn register(&self, app: &mut Registry) {
        let client = self.client.clone();
        app.command(
            COMMAND,
            "Fill the database with the Gulf Skills demo centre (staff, tutors, students, batches, sessions, enrolments, certificates, payments); local and testing only",
            move |_args, state: AppState| {
                let client = client.clone();
                async move { run_command(client, state, true).await }
            },
        );
        let client = self.client.clone();
        app.seeder(move |state: AppState| {
            let client = client.clone();
            async move { run_command(client, state, false).await }
        });
    }
}

/// `edtech:demo` and the seeder: boots the app a second time on the same
/// database to get its router (the demo users do their work through it), then
/// seeds. The command works in `local` and `testing`; the seeder of
/// `db:seed` only in `local`.
async fn run_command(client: wl_kernel::ClientConfig, state: AppState, said: bool) -> Result {
    let env = state.config.env;
    let allowed = if said {
        matches!(env, Environment::Local | Environment::Testing)
    } else {
        env == Environment::Local
    };
    if !allowed {
        if said {
            return Err(renox::anyhow::anyhow!(
                "the demo centre is made in local or testing only, never in {env:?}"
            )
            .into());
        }
        return Ok(());
    }
    if state.config.database_url.contains(":memory:") {
        if said {
            return Err(renox::anyhow::anyhow!(
                "the demo centre needs a database file, not an in-memory database"
            )
            .into());
        }
        return Ok(());
    }
    let app = crate::app_for(client).map_err(|e| renox::anyhow::anyhow!("{e}"))?;
    let kernel = app.config((*state.config).clone()).boot().await?;
    if !seed(&kernel).await? {
        println!("The demo centre is there already.");
        return Ok(());
    }
    println!("The demo centre is made. Log in with any of these (password {PASSWORD}):");
    for login in LOGINS {
        println!("  {:<42} {}", login.email, login.label);
    }
    Ok(())
}

/// Seeds the demo centre into the database `kernel` is on; `false` when it is
/// there already and nothing was made. (The app must have been installed and
/// seeded: the legal entity of `client.toml` and the service types exist.)
///
/// # Errors
///
/// The database's errors, and the message of the rule a demo record broke.
pub async fn seed(kernel: &renox::Kernel) -> Result<bool> {
    let state = kernel.state().clone();
    if is_seeded(&state.db).await? {
        return Ok(false);
    }
    let driver = Driver::new(kernel.router());
    let clock = std::sync::Arc::new(Clock::new().await);
    Box::pin(Seed::new(state, driver, clock).run()).await?;
    Ok(true)
}

/// The demo is there once its last step recorded that it is done.
pub async fn is_seeded(db: &Db) -> Result<bool> {
    Ok(!renox::audit::for_subject(db, MARKER_SUBJECT, 1, 1)
        .await?
        .is_empty())
}

/// The state of a seed in progress: what the later steps need of the earlier.
pub(crate) struct Seed {
    pub(crate) state: AppState,
    pub(crate) driver: Driver,
    pub(crate) clock: std::sync::Arc<Clock>,
    /// The legal entity of the centre.
    pub(crate) entity: i64,
    /// The demo users by e-mail.
    pub(crate) users: Vec<(&'static str, User)>,
    /// The students, in the order of [`people::STUDENTS`].
    pub(crate) students: Vec<PartyId>,
    /// The guardians, in the order of [`people::GUARDIANS`].
    pub(crate) guardians: Vec<PartyId>,
    /// The company that pays for two employees' course.
    pub(crate) company: PartyId,
    /// Every employee by the e-mail of the login behind it.
    pub(crate) employees: Vec<(&'static str, EmployeeId)>,
    /// The tutors' wl-scheduling resources by the e-mail of the login.
    pub(crate) tutors: Vec<(&'static str, i64)>,
    /// The rooms by name.
    pub(crate) rooms: BTreeMap<&'static str, i64>,
    /// The courses by code.
    pub(crate) courses: BTreeMap<&'static str, i64>,
    /// The batches by their key in [`batches::BATCHES`].
    pub(crate) batches: BTreeMap<&'static str, i64>,
    /// The enrolments by the key of the story (`ielts.amal`).
    pub(crate) enrolments: BTreeMap<String, i64>,
}

/// The day the seed's dates are counted from: the latest Friday up to
/// `today`. The sessions of a batch fall on weekdays, so the number of
/// sessions before a given day, and who is on the roster of each, depend on
/// the weekday of day 0; a Friday is the weekday the stories are written for.
pub(crate) fn anchor_day(today: renox::chrono::NaiveDate) -> renox::chrono::NaiveDate {
    use renox::chrono::{Datelike, Days};
    let back = (today.weekday().number_from_monday() + 2) % 7;
    today - Days::new(u64::from(back))
}

impl Seed {
    fn new(state: AppState, driver: Driver, clock: std::sync::Arc<Clock>) -> Self {
        Self {
            state,
            driver,
            clock,
            entity: 0,
            users: Vec::new(),
            students: Vec::new(),
            guardians: Vec::new(),
            company: PartyId(0),
            employees: Vec::new(),
            tutors: Vec::new(),
            rooms: BTreeMap::new(),
            courses: BTreeMap::new(),
            batches: BTreeMap::new(),
            enrolments: BTreeMap::new(),
        }
    }

    pub(crate) fn db(&self) -> &Db {
        &self.state.db
    }

    /// The demo user with this e-mail.
    pub(crate) fn user(&self, email: &str) -> &User {
        self.users
            .iter()
            .find(|(known, _)| *known == email)
            .map(|(_, user)| user)
            .unwrap_or_else(|| panic!("the demo login {email}"))
    }

    async fn run(&mut self) -> Result {
        // The seed's dates are relative to the anchor day, also inside a step
        // that runs on the clock of a day in the past.
        *super::demo_data::REAL_TODAY
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(anchor_day(wl_kernel::documents::today()));
        // Not cleared afterwards: seeds that run at the same time in one
        // process (the tests) share it, and the first to end would send the
        // others back to the clock of their step. The next seed sets it again.
        self.story().await
    }

    async fn story(&mut self) -> Result {
        super::roles::define(self.db()).await?;
        self.entity = wl_kernel::LegalEntity::default_entity(self.db()).await?.id;
        self.logins().await?;
        self.people().await?;
        self.staff().await?;
        self.centre().await?;
        self.courses_and_batches().await?;
        self.sales().await?;
        self.enrol().await?;
        self.stories().await?;
        self.notes().await?;
        renox::audit::record(
            self.db(),
            renox::audit::Entry::new(DONE_ACTION).subject(MARKER_SUBJECT, 1),
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::anchor_day;
    use renox::chrono::{Datelike, NaiveDate, Weekday};

    /// Whatever the weekday, the anchor is a Friday, and it is today on a
    /// Friday and never more than six days before.
    #[test]
    fn the_anchor_is_the_latest_friday() {
        let first = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap_or_default();
        for n in 0..14 {
            let today = first + renox::chrono::Days::new(n);
            let anchor = anchor_day(today);
            assert_eq!(anchor.weekday(), Weekday::Fri, "{today}");
            let back = (today - anchor).num_days();
            assert!((0..=6).contains(&back), "{today}: {back}");
            assert_eq!(back == 0, today.weekday() == Weekday::Fri);
        }
    }
}
