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
use super::ipa_plans::{BOQ, Cycle, Decide};
use super::people::email_of;
use super::roles::{ACCOUNTANT, PROJECT_MANAGER, QUANTITY_SURVEYOR, RECEIVABLES};
use super::seed_projects::plan_of;
use super::{Seed, aed, day, text, upto};

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
