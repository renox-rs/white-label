//! How projects end: the office fit-out is handed over (practical
//! completion) and its first retention part is invoiced and paid, the
//! showroom is terminated by the project manager with the MD's approval.

use construction::Retention;
use construction::retention::RetentionPart;
use renox::prelude::*;
use wl_kernel::approvals::ApprovalRequest;

use super::at;
use super::people::{email_of, login_of};
use super::roles::{ADMIN, PROJECT_MANAGER, RECEIVABLES};
use super::seed_ipas::after;
use super::{Seed, day, text};

impl Seed {
    /// The project manager hands the project `code` over on `offset`; the
    /// first retention part falls due and receivables invoices it five days
    /// later, the client pays it a month after.
    pub(crate) async fn complete(&mut self, code: &str, offset: i64) -> Result {
        let project = self.project(code).await?;
        let pm = email_of(PROJECT_MANAGER);
        let date = text(day(offset));
        let certificate = format!("HANDOVER-{code}.pdf");
        at!(self, day(offset), {
            self.driver
                .post(
                    pm,
                    &format!("/construction/projects/{}/complete", project.id),
                    &[
                        ("completed_on", date.as_str()),
                        ("certificate", certificate.as_str()),
                    ],
                )
                .await
        })?;
        let first = Retention::query()
            .where_(Retention::PROJECT_ID.eq(project.id))
            .where_(Retention::PART.eq(RetentionPart::First))
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no retention of {code}"))?;
        let invoiced = after(day(offset), 5);
        at!(self, invoiced, {
            self.driver
                .post(
                    email_of(RECEIVABLES),
                    &format!("/construction/retentions/{}/invoice", first.id),
                    &[],
                )
                .await
        })?;
        self.report.add("retention parts invoiced", 1);
        Ok(())
    }

    /// The project manager asks to terminate project `code`; the MD approves
    /// the request the next day.
    pub(crate) async fn terminate(&mut self, code: &str, offset: i64, effective: i64) -> Result {
        let project = self.project(code).await?;
        let pm = email_of(PROJECT_MANAGER);
        let effective_on = text(day(effective));
        at!(self, day(offset), {
            self.driver
                .post(
                    pm,
                    &format!("/construction/projects/{}/terminate", project.id),
                    &[
                        ("reason", "terminated_by_client"),
                        ("notice", "TERMINATION-NOTICE-TRM-07.pdf"),
                        ("effective_on", effective_on.as_str()),
                    ],
                )
                .await
        })?;
        let request = ApprovalRequest::pending_about(self.db(), &project.subject())
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no termination request"))?;
        at!(self, after(day(offset), 1), {
            self.driver
                .post(
                    login_of(ADMIN).email,
                    &format!("/approvals/{}/approve", request.id),
                    &[],
                )
                .await
        })?;
        Ok(())
    }

    /// Receivables invoices the `part` of the retention of project `code` on
    /// `offset`.
    pub(crate) async fn invoice_retention(
        &mut self,
        code: &str,
        part: RetentionPart,
        offset: i64,
    ) -> Result {
        let project = self.project(code).await?;
        let held = Retention::query()
            .where_(Retention::PROJECT_ID.eq(project.id))
            .where_(Retention::PART.eq(part))
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no retention of {code}"))?;
        at!(self, day(offset), {
            self.driver
                .post(
                    email_of(RECEIVABLES),
                    &format!("/construction/retentions/{}/invoice", held.id),
                    &[],
                )
                .await
        })?;
        self.report.add("retention parts invoiced", 1);
        Ok(())
    }
}
