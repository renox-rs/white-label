//! The interim payment applications: the BOQ of each project, the monthly
//! drafts the cut-off rule makes, and what happens to each IPA: progress,
//! submit (the project manager), the QS records the deductions and the
//! certificate, the project manager accepts or contests the deductions,
//! receivables invoices the certified IPA, and the accountant records the
//! client's payment.
//!
//! The BOQ lines and the progress on them are made by the services of the
//! vertical, and the claimed amount of a draft is set from the progress: the
//! calculation of an IPA (and the screens that enter the BOQ) come with their
//! own issues, so there is no route to either.

use construction::boq::{self, BoqLine};
use construction::cutoff;
use construction::{Ipa, IpaDeduction};
use renox::chrono::{Datelike, Days, NaiveDate};
use renox::prelude::*;
use wl_invoicing::Invoice;

use super::at;
use super::people::email_of;
use super::roles::{ACCOUNTANT, PROJECT_MANAGER, QUANTITY_SURVEYOR, RECEIVABLES};
use super::seed_projects::plan_of;
use super::{Seed, aed, day, text, upto};

/// What the project manager does with a deduction line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decide {
    /// Accepts it.
    Accept,
    /// Contests it.
    Contest,
    /// Tries to accept it: above AED 25,000 the MD is asked and the line stays
    /// undecided until then.
    AskMd,
    /// Leaves it undecided.
    Wait,
}

/// A deduction the client takes off a certificate.
#[derive(Debug, Clone, Copy)]
pub struct Ded {
    /// `back_charge`, `contra_charge`, `liquidated_damages` or `other`.
    pub kind: &'static str,
    /// The client's reference.
    pub reference: &'static str,
    /// The amount excluding VAT, in dirhams.
    pub amount: i64,
    /// What the project manager does with it.
    pub decide: Decide,
}

/// The client's payment of an invoice.
#[derive(Debug, Clone, Copy)]
pub struct Pay {
    /// Days after the invoice.
    pub after: i64,
    /// The share of the open balance paid, in percent.
    pub percent: i64,
}

/// One IPA of the demo and what happens to it.
#[derive(Debug, Clone, Copy)]
pub struct Cycle {
    /// The project.
    pub code: &'static str,
    /// The number of the IPA in the project.
    pub number: i64,
    /// The cut-off day the draft is made on, days from today.
    pub cutoff: i64,
    /// The cumulative work claimed, in basis points of the contract value.
    pub claimed_bp: i64,
    /// Days after the cut-off that it is submitted.
    pub submit: i64,
    /// The number of the pending IPA it replaces.
    pub replaces: Option<i64>,
    /// Days after the submission that the client certifies, and the certified
    /// share of the claim, in percent.
    pub certify: Option<(i64, i64)>,
    /// The deductions on the certificate.
    pub deductions: &'static [Ded],
    /// Days after the certificate that receivables invoices it.
    pub invoice: Option<i64>,
    /// The client's payment.
    pub pay: Option<Pay>,
}

const NONE: &[Ded] = &[];

const fn paid(after: i64, percent: i64) -> Option<Pay> {
    Some(Pay { after, percent })
}

#[allow(clippy::too_many_arguments)]
const fn cycle(
    code: &'static str,
    number: i64,
    cutoff: i64,
    claimed_bp: i64,
    submit: i64,
    certify: Option<(i64, i64)>,
    invoice: Option<i64>,
    pay: Option<Pay>,
) -> Cycle {
    Cycle {
        code,
        number,
        cutoff,
        claimed_bp,
        submit,
        replaces: None,
        certify,
        deductions: NONE,
        invoice,
        pay,
    }
}

/// The IPAs of the cut-offs of the office fit-out that was completed.
pub const COMPLETED: &[Cycle] = &[
    cycle(
        "OFC-06",
        1,
        -210,
        1_800,
        3,
        Some((14, 95)),
        Some(2),
        paid(20, 100),
    ),
    cycle(
        "OFC-06",
        2,
        -179,
        5_000,
        3,
        Some((14, 96)),
        Some(2),
        paid(20, 100),
    ),
    cycle(
        "OFC-06",
        3,
        -149,
        9_600,
        3,
        Some((14, 97)),
        Some(2),
        paid(25, 100),
    ),
];

/// The IPAs of the cut-off of day -118.
pub const JUNE: &[Cycle] = &[
    cycle(
        "TRM-07",
        1,
        -118,
        2_200,
        2,
        Some((12, 95)),
        Some(2),
        paid(25, 100),
    ),
    cycle(
        "TWR-01",
        1,
        -118,
        1_400,
        3,
        Some((16, 94)),
        Some(2),
        paid(25, 100),
    ),
    cycle(
        "MALL-02",
        1,
        -118,
        1_000,
        3,
        Some((12, 100)),
        Some(2),
        paid(20, 100),
    ),
];

/// The IPAs of the cut-off of day -88.
pub const JULY: &[Cycle] = &[
    Cycle {
        deductions: &[
            Ded {
                kind: "liquidated_damages",
                reference: "LD-TRM-002",
                amount: 12_000,
                decide: Decide::Accept,
            },
            Ded {
                kind: "contra_charge",
                reference: "CC-TRM-011",
                amount: 24_000,
                decide: Decide::Contest,
            },
        ],
        ..cycle("TRM-07", 2, -88, 5_000, 2, Some((14, 96)), Some(2), None)
    },
    Cycle {
        deductions: &[
            Ded {
                kind: "back_charge",
                reference: "BC-TWR-017",
                amount: 8_500,
                decide: Decide::Accept,
            },
            Ded {
                kind: "liquidated_damages",
                reference: "LD-TWR-004",
                amount: 20_000,
                decide: Decide::Accept,
            },
            Ded {
                kind: "contra_charge",
                reference: "CC-TWR-009",
                amount: 6_000,
                decide: Decide::Contest,
            },
        ],
        ..cycle(
            "TWR-01",
            2,
            -88,
            3_300,
            3,
            Some((18, 96)),
            Some(2),
            paid(30, 55),
        )
    },
    cycle(
        "MALL-02",
        2,
        -88,
        3_000,
        3,
        Some((14, 95)),
        Some(2),
        paid(30, 100),
    ),
    // The client pays this one by cheque, which bounces.
    cycle("CLN-03", 1, -88, 1_100, 3, Some((15, 95)), Some(2), None),
];

/// The IPAs of the cut-off of day -57.
pub const AUGUST: &[Cycle] = &[
    // Never certified: the next IPA replaces it.
    cycle("TWR-01", 3, -57, 5_100, 2, None, None, None),
    // Certified, and invoiced late: the invoice is issued and not yet due.
    cycle("MALL-02", 3, -57, 5_300, 3, Some((18, 96)), Some(32), None),
    cycle("CLN-03", 2, -57, 2_600, 3, Some((16, 95)), Some(2), None),
    cycle(
        "RST-04",
        1,
        -57,
        1_500,
        3,
        Some((12, 96)),
        Some(2),
        paid(20, 100),
    ),
];

/// The IPAs of the cut-off of day -26.
pub const SEPTEMBER: &[Cycle] = &[
    Cycle {
        replaces: Some(3),
        deductions: &[
            Ded {
                kind: "liquidated_damages",
                reference: "LD-TWR-011",
                amount: 30_000,
                decide: Decide::AskMd,
            },
            Ded {
                kind: "back_charge",
                reference: "BC-TWR-022",
                amount: 9_000,
                decide: Decide::Wait,
            },
        ],
        ..cycle("TWR-01", 4, -26, 5_700, 2, None, None, None)
    },
    cycle("MALL-02", 4, -26, 6_800, 17, None, None, None),
    cycle("CLN-03", 3, -26, 3_700, 4, None, None, None),
    cycle("RST-04", 2, -26, 3_600, 16, None, None, None),
];

/// The final account of the terminated showroom: the draft of the cut-off of
/// day -57 became it when the project was terminated.
pub const FINAL_ACCOUNT: Cycle = cycle("TRM-07", 3, -57, 6_400, 17, Some((15, 96)), None, None);

/// The BOQ of a project: the share of the contract value (basis points), the
/// number, the description, the unit and the rate in dirhams.
const BOQ: [(i64, &str, &str, &str, i64); 5] = [
    (3_000, "1.1", "Gypsum partitions and ceilings", "m2", 180),
    (
        2_500,
        "2.1",
        "Joinery, doors and built-in units",
        "nr",
        2_500,
    ),
    (2_000, "3.1", "Flooring and wall finishes", "m2", 220),
    (1_000, "4.1", "Painting and decorative finishes", "m2", 38),
    (1_500, "5.1", "MEP first and second fix", "m", 95),
];

/// `date` plus `days`, never later than the real today.
pub(crate) fn after(date: NaiveDate, days: i64) -> NaiveDate {
    upto(
        date.checked_add_days(Days::new(days.max(0) as u64))
            .unwrap_or(date),
    )
}

impl Seed {
    /// The BOQ lines of project `code`, which add up to its contract value.
    pub(crate) async fn boq(&mut self, code: &str) -> Result {
        let plan = plan_of(code);
        let project = self.project(code).await?;
        for (share, number, description, unit, rate) in BOQ {
            let rate = rate * 100;
            let quantity = plan.value * 100 * share / 10_000 / rate;
            let mut line = BoqLine::new(project.id, number, quantity, rate);
            line.description = description.to_owned();
            line.unit = unit.to_owned();
            line.save(self.db()).await?;
            self.report.add("BOQ lines", 1);
        }
        Ok(())
    }

    /// The cut-off rule runs on the day `offset`: every project in progress
    /// without a draft gets one.
    pub(crate) async fn cutoff(&mut self, offset: i64) -> Result {
        let date = day(offset);
        let made = at!(self, date, {
            cutoff::run_on(&self.state, date, date.day()).await
        })?;
        self.report.add("IPA drafts", made.drafts.len());
        Ok(())
    }

    /// The IPA `number` of project `code`.
    pub(crate) async fn ipa_of(&self, code: &str, number: i64) -> Result<Ipa> {
        let project = self.project(code).await?;
        Ipa::query()
            .where_(Ipa::PROJECT_ID.eq(project.id))
            .where_(Ipa::NUMBER.eq(number))
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no IPA {number} of {code}").into())
    }

    /// Records the progress of the draft `ipa` on the BOQ (`bp` of every
    /// line's quantity, cumulative) and sets the claim from it.
    pub(crate) async fn progress(&mut self, ipa: &Ipa, bp: i64) -> Result {
        let lines = BoqLine::query()
            .where_(BoqLine::PROJECT_ID.eq(ipa.project_id))
            .order_by(BoqLine::ID)
            .get(self.db())
            .await?;
        let mut claimed = 0;
        for line in lines {
            let done = line.quantity * bp / 10_000;
            boq::record_progress(self.db(), ipa.id, line.id, done).await?;
            claimed += done * line.rate;
        }
        Ipa::query()
            .where_(Ipa::ID.eq(ipa.id))
            .update(self.db(), &[("claimed_amount", &claimed)])
            .await?;
        Ok(())
    }

    /// The cut-off of day `offset` and what happens to each of its IPAs.
    pub(crate) async fn month(&mut self, offset: i64, cycles: &[Cycle]) -> Result {
        self.cutoff(offset).await?;
        for c in cycles {
            self.cycle(c).await?;
        }
        Ok(())
    }

    /// Everything that happens to one IPA after its cut-off.
    pub(crate) async fn cycle(&mut self, c: &Cycle) -> Result {
        let pm = email_of(PROJECT_MANAGER);
        let qs = email_of(QUANTITY_SURVEYOR);
        let cutoff = day(c.cutoff);
        let ipa = self.ipa_of(c.code, c.number).await?;
        at!(self, after(cutoff, 1), {
            self.progress(&ipa, c.claimed_bp).await
        })?;
        let submitted = after(cutoff, c.submit);
        let replaces = match c.replaces {
            Some(number) => self.ipa_of(c.code, number).await?.id.to_string(),
            None => String::new(),
        };
        at!(self, submitted, {
            self.driver
                .post(
                    pm,
                    &format!("/construction/ipas/{}/submit", ipa.id),
                    &[("replaces", replaces.as_str())],
                )
                .await
        })?;
        self.report.add("IPAs submitted", 1);
        let certified_on = match c.certify {
            Some((days, _)) => after(submitted, days),
            None => after(submitted, 3),
        };
        // The QS records the deductions when the certificate arrives, the
        // project manager decides on them.
        for ded in c.deductions {
            let amount = (ded.amount * 100).to_string();
            at!(self, certified_on, {
                self.driver
                    .post(
                        qs,
                        &format!("/construction/ipas/{}/deductions", ipa.id),
                        &[
                            ("kind", ded.kind),
                            ("reference", ded.reference),
                            ("amount", amount.as_str()),
                            ("description", ""),
                        ],
                    )
                    .await
            })?;
            let line = IpaDeduction::query()
                .where_(IpaDeduction::IPA_ID.eq(ipa.id))
                .where_(IpaDeduction::REFERENCE.eq(ded.reference))
                .first(self.db())
                .await?
                .ok_or_else(|| {
                    renox::anyhow::anyhow!("demo seed: no deduction {}", ded.reference)
                })?;
            let verb = match ded.decide {
                Decide::Accept | Decide::AskMd => "accept",
                Decide::Contest => "contest",
                Decide::Wait => continue,
            };
            at!(self, after(certified_on, 1), {
                self.driver
                    .post(
                        pm,
                        &format!("/construction/deductions/{}/{verb}", line.id),
                        &[],
                    )
                    .await
            })?;
            self.report.add("deductions decided", 1);
        }
        let Some((_, percent)) = c.certify else {
            return Ok(());
        };
        let reload = Ipa::find_or_404(self.db(), ipa.id).await?;
        let certified = reload.claimed_amount * percent / 100 / 10_000 * 10_000;
        let certificate = format!("CERT-{}-{:02}.pdf", c.code, c.number);
        let certified_text = certified.to_string();
        let certificate_day = after(certified_on, 1);
        let on = text(certificate_day);
        at!(self, certificate_day, {
            self.driver
                .post(
                    qs,
                    &format!("/construction/ipas/{}/certificate", ipa.id),
                    &[
                        ("certified_amount", certified_text.as_str()),
                        ("certificate", certificate.as_str()),
                        ("certified_on", on.as_str()),
                    ],
                )
                .await
        })?;
        self.report.add("IPAs certified", 1);
        let Some(days) = c.invoice else {
            return Ok(());
        };
        let invoiced = after(certificate_day, days);
        at!(self, invoiced, {
            self.driver
                .post(
                    email_of(RECEIVABLES),
                    &format!("/construction/ipas/{}/invoice", ipa.id),
                    &[],
                )
                .await
        })?;
        self.report.add("IPAs invoiced", 1);
        if let Some(pay) = c.pay {
            self.receive(c.code, ipa.id, after(invoiced, pay.after), pay.percent)
                .await?;
        }
        Ok(())
    }

    /// The client pays `percent` of what is open on the invoice of IPA `ipa`
    /// by transfer (see [`Seed::pay_invoice`]).
    pub(crate) async fn receive(
        &mut self,
        code: &str,
        ipa: i64,
        on: NaiveDate,
        percent: i64,
    ) -> Result {
        let ipa = Ipa::find_or_404(self.db(), ipa).await?;
        let Some(invoice_id) = ipa.invoice_id else {
            return Ok(());
        };
        let client = plan_of(code).client;
        let reference = format!("TRF-{}-{:02}-{:02}", code, ipa.number, percent % 100);
        self.pay_invoice(client, invoice_id, on, percent, &reference)
            .await
    }

    /// `client` pays `percent` of what is open on invoice `invoice_id` by
    /// transfer on `on`: the accountant records the payment and allocates it.
    pub(crate) async fn pay_invoice(
        &mut self,
        client: &str,
        invoice_id: i64,
        on: NaiveDate,
        percent: i64,
        reference: &str,
    ) -> Result {
        let accountant = email_of(ACCOUNTANT);
        let invoice = Invoice::find_or_404(self.db(), invoice_id).await?;
        let amount = invoice.balance * percent / 100;
        if amount <= 0 {
            return Ok(());
        }
        let client = self.party(client).clone();
        let entity = wl_kernel::LegalEntity::default_entity(self.db())
            .await?
            .id
            .to_string();
        let amount_text = aed(amount);
        let date = text(on);
        let invoice_text = invoice_id.to_string();
        at!(self, on, {
            let reply = self
                .driver
                .post(
                    accountant,
                    "/payments",
                    &[
                        ("party_code", client.code.as_str()),
                        ("legal_entity_id", entity.as_str()),
                        ("method", "transfer"),
                        ("amount", amount_text.as_str()),
                        ("payment_date", date.as_str()),
                        ("reference", reference),
                        (
                            "bank_account",
                            "Gulf Demo Bank AE00 0000 0000 0000 0001 234",
                        ),
                    ],
                )
                .await?;
            let payment = reply
                .redirected_id()
                .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no payment id"))?;
            self.driver
                .post(
                    accountant,
                    &format!("/payments/{payment}/allocate"),
                    &[
                        ("invoice_id", invoice_text.as_str()),
                        ("amount", amount_text.as_str()),
                    ],
                )
                .await
        })?;
        self.report.add("client payments", 1);
        Ok(())
    }
}
