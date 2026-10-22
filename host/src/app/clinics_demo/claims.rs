//! The claims of the demo clinic, taken to every state a claim reaches: the
//! visits billed in the desk story each left a draft claim; the billing
//! coordinator validates, submits and records the payer's answers through the
//! claims page, and the clinic manager writes off the claim whose window
//! passed.

use clinics::{Claim, ClaimLine};
use renox::prelude::*;

use super::super::demo_data::day;
use super::Seed;
use super::people::{AHMED, FATIMA, LAYAN, PRIYA};

const BILLING: &str = "billing@palmgrove.example";
const MANAGER: &str = "manager.jumeirah@palmgrove.example";

/// Fils as the dirhams the remittance form reads: `350.00`.
fn aed(fils: i64) -> String {
    format!("{}.{:02}", fils / 100, fils % 100)
}

impl Seed {
    /// The claim of the visit of `patient` on `days` from today.
    async fn claim_of(&self, patient: usize, days: i64) -> Result<Claim> {
        Claim::query()
            .where_(Claim::PATIENT_PARTY_ID.eq(self.patients[patient].0))
            .where_(Claim::VISIT_DATE.eq(day(days)))
            .first(self.db())
            .await?
            .ok_or_else(|| {
                renox::anyhow::anyhow!("demo seed: no claim for patient {patient} on day {days}")
                    .into()
            })
    }

    async fn claim_post(
        &self,
        who: &str,
        claim: &Claim,
        action: &str,
        form: &[(&str, &str)],
    ) -> Result {
        self.driver
            .post(who, &format!("/clinics/claims/{}/{action}", claim.id), form)
            .await?;
        Ok(())
    }

    /// Validates and submits `claim`.
    async fn send_claim(&self, claim: &Claim, reference: &str) -> Result {
        self.claim_post(BILLING, claim, "validate", &[]).await?;
        self.claim_post(BILLING, claim, "submit", &[("reference", reference)])
            .await
    }

    /// The remittance for every line of `claim`: paid in full, or denied.
    async fn remit(&self, claim: &Claim, reference: &str, denied: Option<(&str, &str)>) -> Result {
        let lines = ClaimLine::of_claim(self.db(), claim.id).await?;
        let mut form: Vec<(String, String)> = vec![
            ("reference".into(), reference.into()),
            ("date".into(), day(-1).to_string()),
        ];
        for line in &lines {
            match denied {
                None => form.push((format!("line_{}_paid", line.id), aed(line.claimed))),
                Some((code, reason)) => {
                    form.push((format!("line_{}_denied", line.id), aed(line.claimed)));
                    form.push((format!("line_{}_code", line.id), code.into()));
                    form.push((format!("line_{}_reason", line.id), reason.into()));
                }
            }
        }
        let pairs: Vec<(&str, &str)> = form.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        self.claim_post(BILLING, claim, "remittance", &pairs).await
    }

    pub(crate) async fn claims(&mut self) -> Result {
        // Paid in full.
        let paid = self.claim_of(AHMED, -12).await?;
        self.send_claim(&paid, "GS-SUB-2201").await?;
        self.remit(&paid, "GS-RA-8841", None).await?;
        // Sent, waiting for the payer.
        let sent = self.claim_of(FATIMA, -11).await?;
        self.send_claim(&sent, "CH-SUB-1180").await?;
        // Denied: the payer does not cover the service for the plan.
        let denied = self.claim_of(PRIYA, -10).await?;
        self.send_claim(&denied, "OM-SUB-5530").await?;
        self.remit(
            &denied,
            "OM-RA-3302",
            Some(("E104", "Service not covered under the plan")),
        )
        .await?;
        // Ready to send: validated, not yet submitted.
        let ready = self.claim_of(super::people::YOUSEF, -3).await?;
        self.claim_post(BILLING, &ready, "validate", &[]).await?;
        // Written off: the payer's 21 days to receive it ended and it accepts
        // no late claim.
        let late = self.claim_of(LAYAN, -24).await?;
        self.claim_post(
            MANAGER,
            &late,
            "write-off",
            &[("reason", "Not sent inside the payer's 21 days")],
        )
        .await?;
        // The others stay drafts, with their reminders running.
        Ok(())
    }
}
