//! The enrolments the counsellor makes at the enrolments page, with the
//! terms accepted at the desk, and the first payment of each: in cash, by
//! card, by transfer or (one) through the online gateway. The first payment
//! activates the enrolment. An enrolment is only taken while its batch is
//! open, so all of them are made before the first session is held; the ones
//! that stay `pending_payment` or lapse are in batches that open later.

use renox::prelude::*;
use wl_kernel::payments::PaymentMethod;

use super::super::demo_data::{at, day};
use super::money::aed;
use super::people::{
    AARAV, AMAL, ARJUN, DANIEL, ELENA, FATIMA, HASSAN, JAMES, KHALID, KOFI, LAYAN, LUCAS, MARIA,
    MONA, NADIA, NOOR, PRIYA, RANIA, SOFIA, YOUSEF,
};
use super::people::{AHMED, GRACE, OMAR_H, SUNITA};
use super::{COUNSELLOR_LOGIN, Seed};

/// How the first invoice is paid.
#[derive(Clone, Copy)]
pub(crate) enum Pay {
    /// Not paid yet.
    Unpaid,
    /// In full, by this method on this day.
    Full(PaymentMethod, i64),
    /// Through the online gateway on this day (the reference).
    Gateway(i64, &'static str),
}

/// Who pays.
#[derive(Clone, Copy)]
pub(crate) enum Payer {
    /// The student (or, for a child, the guardian).
    Default,
    /// The corporate client.
    Company,
}

/// An enrolment.
pub(crate) struct Enrol {
    pub(crate) key: &'static str,
    pub(crate) batch: &'static str,
    pub(crate) student: usize,
    /// The guardian (an index of the guardians) for a child.
    pub(crate) guardian: Option<usize>,
    pub(crate) payer: Payer,
    /// The price in dirhams with VAT.
    pub(crate) price: i64,
    pub(crate) discount: i64,
    pub(crate) registration: i64,
    pub(crate) instalments: i64,
    /// The day the counsellor enrols the student.
    pub(crate) on: i64,
    pub(crate) terms: bool,
    pub(crate) pay: Pay,
}

use PaymentMethod::{Card, Cash, Transfer};

const fn e(
    key: &'static str,
    batch: &'static str,
    student: usize,
    price: i64,
    on: i64,
    pay: Pay,
) -> Enrol {
    Enrol {
        key,
        batch,
        student,
        guardian: None,
        payer: Payer::Default,
        price,
        discount: 0,
        registration: 0,
        instalments: 1,
        on,
        terms: true,
        pay,
    }
}

impl Enrol {
    const fn guardian(mut self, guardian: usize) -> Self {
        self.guardian = Some(guardian);
        self
    }
    const fn company(mut self) -> Self {
        self.payer = Payer::Company;
        self
    }
    const fn registration(mut self, aed: i64) -> Self {
        self.registration = aed;
        self
    }
    const fn instalments(mut self, count: i64) -> Self {
        self.instalments = count;
        self
    }
    const fn discount(mut self, aed: i64) -> Self {
        self.discount = aed;
        self
    }
}

/// The enrolments, batch by batch.
pub(crate) const ENROLS: &[Enrol] = &[
    // A completed English course.
    e("eng.amal", "eng_done", AMAL, 1890, -57, Pay::Full(Cash, -56)).registration(100).instalments(2),
    e("eng.priya", "eng_done", PRIYA, 1890, -56, Pay::Full(Card, -55)),
    e("eng.james", "eng_done", JAMES, 1890, -55, Pay::Full(Transfer, -54)),
    e("eng.fatima", "eng_done", FATIMA, 1890, -54, Pay::Full(Card, -53)),
    e("eng.elena", "eng_done", ELENA, 1890, -53, Pay::Full(Cash, -52)),
    // Excel, paid for by the employer.
    e("xl.khalid", "xl_done", KHALID, 840, -36, Pay::Full(Transfer, -27)).company(),
    e("xl.daniel", "xl_done", DANIEL, 840, -36, Pay::Full(Transfer, -27)).company(),
    // The kids' coding batch that is terminated.
    e("code.yousef", "code_term", YOUSEF, 1050, -31, Pay::Full(Card, -30)).guardian(AHMED),
    e("code.aarav", "code_term", AARAV, 1050, -30, Pay::Full(Cash, -29)).guardian(SUNITA),
    e("code.kofi", "code_term", KOFI, 1050, -29, Pay::Full(Card, -28)).guardian(GRACE),
    // The Excel batch that is terminated.
    e("xlt.hassan", "xl_term", HASSAN, 840, -27, Pay::Full(Cash, -26)),
    e("xlt.maria", "xl_term", MARIA, 840, -26, Pay::Full(Transfer, -25)),
    // The French batch that is cancelled before it starts.
    e("frc.arjun", "fr_cancel", ARJUN, 1575, -28, Pay::Full(Card, -27)),
    e("frc.lucas", "fr_cancel", LUCAS, 1575, -27, Pay::Full(Cash, -26)),
    e("frc.sofia", "fr_cancel", SOFIA, 1575, -20, Pay::Unpaid),
    // IELTS, running.
    e("ielts.james", "ielts_run", JAMES, 2520, -25, Pay::Full(Card, -24)).instalments(3),
    e("ielts.nadia", "ielts_run", NADIA, 2520, -25, Pay::Full(Transfer, -24)),
    e("ielts.maria", "ielts_run", MARIA, 2520, -24, Pay::Full(Cash, -23)).registration(105),
    e("ielts.rania", "ielts_run", RANIA, 2520, -23, Pay::Full(Cash, -22)),
    e("ielts.daniel", "ielts_run", DANIEL, 2520, -23, Pay::Full(Card, -22)),
    e("ielts.khalid", "ielts_run", KHALID, 2520, -22, Pay::Full(Cash, -21)),
    e("ielts.fatima", "ielts_run", FATIMA, 2520, -21, Pay::Gateway(-20, "GW-88231")),
    // Python, with a sixteen-year-old and her father as guardian.
    e("py.arjun", "py_run", ARJUN, 2100, -21, Pay::Full(Card, -20)),
    e("py.layan", "py_run", LAYAN, 2100, -20, Pay::Full(Card, -19)).guardian(OMAR_H),
    e("py.priya", "py_run", PRIYA, 2100, -19, Pay::Full(Transfer, -18)).instalments(2),
    // Arabic for kids.
    e("arb.mona", "arb_run", MONA, 1260, -31, Pay::Full(Card, -30)).guardian(AHMED),
    e("arb.yousef", "arb_run", YOUSEF, 1260, -31, Pay::Full(Card, -30)).guardian(AHMED),
    e("arb.aarav", "arb_run", AARAV, 1260, -30, Pay::Full(Cash, -29)).guardian(SUNITA),
    // GMAT.
    e("gmat.amal", "gmat_run", AMAL, 3150, -24, Pay::Full(Transfer, -22)),
    e("gmat.arjun", "gmat_run", ARJUN, 3150, -23, Pay::Full(Card, -21)),
    // French, with a fourteen-year-old.
    e("fr.noor", "fr_run", NOOR, 1575, -26, Pay::Full(Cash, -25)).guardian(OMAR_H),
    e("fr.sofia", "fr_run", SOFIA, 1575, -25, Pay::Full(Card, -24)),
    e("fr.maria", "fr_run", MARIA, 1575, -24, Pay::Full(Transfer, -23)),
    // The open French batch with three seats: two of them here, the third by
    // the transfer of the cancelled batch.
    e("fro.nadia", "fr_open", NADIA, 1575, -5, Pay::Full(Cash, -4)),
    e("fro.james", "fr_open", JAMES, 1575, -1, Pay::Unpaid),
    // The weekend IELTS batch: one seat held for 48 hours that nobody paid
    // for, one that waits for the centre manager to approve a discount.
    e("ieltso.lucas", "ielts_open", LUCAS, 2520, -5, Pay::Unpaid),
    e("ieltso.elena", "ielts_open", ELENA, 2520, -1, Pay::Unpaid).discount(400),
];

impl Seed {
    /// Makes one enrolment at the enrolments page as the counsellor, and has
    /// the first invoice paid.
    pub(crate) async fn enrol_one(&mut self, plan: &Enrol) -> Result {
        let batch = self.batches[plan.batch].to_string();
        let student = self.students[plan.student].0.to_string();
        let guardian = plan
            .guardian
            .map(|g| self.guardians[g].0.to_string())
            .unwrap_or_default();
        let payer = match plan.payer {
            Payer::Company => self.company.0.to_string(),
            Payer::Default => String::new(),
        };
        let price = plan.price.to_string();
        let discount = if plan.discount > 0 {
            aed(plan.discount * 100)
        } else {
            String::new()
        };
        let registration = if plan.registration > 0 {
            plan.registration.to_string()
        } else {
            String::new()
        };
        let instalments = plan.instalments.to_string();
        let mut form = vec![
            ("kind", "course"),
            ("student", student.as_str()),
            ("guardian", guardian.as_str()),
            ("payer", payer.as_str()),
            ("batch", batch.as_str()),
            ("price", price.as_str()),
            ("discount", discount.as_str()),
            ("registration_fee", registration.as_str()),
            ("instalments", instalments.as_str()),
        ];
        if plan.terms {
            form.push(("terms", "on"));
        }
        let reply = at!(self, day(plan.on), {
            self.driver
                .post(COUNSELLOR_LOGIN, "/edtech/enrolments", &form)
                .await
        })?;
        let id = reply.redirected_id().ok_or_else(|| {
            renox::anyhow::anyhow!("demo seed: enrolment {} has no page", plan.key)
        })?;
        self.enrolments.insert(plan.key.to_owned(), id);
        match plan.pay {
            Pay::Unpaid => {}
            Pay::Full(method, on) => {
                self.pay_enrolment(plan.key, method, None, on).await?;
            }
            Pay::Gateway(on, reference) => {
                self.gateway_payment(plan.key, on, reference).await?;
            }
        }
        Ok(())
    }

    /// All the enrolments of [`ENROLS`], and the package of private sessions.
    pub(crate) async fn enrol(&mut self) -> Result {
        for plan in ENROLS {
            self.enrol_one(plan).await?;
        }
        self.package().await
    }

    /// Hassan buys eight private sessions; a credit is used each time he is
    /// marked on the roster of a class he is added to.
    async fn package(&mut self) -> Result {
        let student = self.students[HASSAN].0.to_string();
        let form = [
            ("kind", "package"),
            ("student", student.as_str()),
            ("item_code", "private_tutoring_8"),
            ("item_name", "Private tutoring, 8 sessions"),
            ("credits", "8"),
            ("price", "2400"),
            ("terms", "on"),
        ];
        let reply = at!(self, day(-30), {
            self.driver
                .post(COUNSELLOR_LOGIN, "/edtech/enrolments", &form)
                .await
        })?;
        let id = reply
            .redirected_id()
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the package has no page"))?;
        self.enrolments.insert("pkg.hassan".to_owned(), id);
        self.pay_enrolment("pkg.hassan", Card, None, -30).await?;
        Ok(())
    }
}
