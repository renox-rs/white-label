//! Making the management agreements and the owner statements of 15 months.

use property::agreement::ManagementAgreement;
use renox::chrono::{Datelike, Duration, Months, NaiveDate};
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::approvals::ApprovalRequest;
use wl_kernel::documents::today;

use super::owners::{AGREEMENTS, AgreementPlan, Standing};
use super::people::login_of;
use super::roles::{ACCOUNTANT, ADMIN};
use super::seed_leases::text;
use super::{Seed, day};

impl Seed<'_> {
    /// The agreements and their statements.
    pub(crate) async fn owners(&mut self) -> Result {
        let mut made = Vec::new();
        for plan in AGREEMENTS {
            let id = self.agreement(plan).await?;
            made.push((plan, id));
        }
        for (plan, id) in made {
            if matches!(plan.standing, Standing::Active) {
                self.statements(plan, id).await?;
            }
        }
        self.approve_payouts().await?;
        self.revisions().await
    }

    /// The accountant revises two published statements the owners queried: the
    /// old one is superseded, and the corrected draft follows.
    async fn revisions(&mut self) -> Result {
        let accountant = login_of(ACCOUNTANT).email;
        let pending: Vec<i64> = ApprovalRequest::query()
            .where_(ApprovalRequest::STATUS.eq(wl_kernel::ApprovalStatus::Pending))
            .get(self.db())
            .await?
            .iter()
            .map(|r| r.subject_id)
            .collect();
        let candidates = property::statement::OwnerStatement::query()
            .where_(
                property::statement::OwnerStatement::STATUS
                    .eq(property::statement::StatementStatus::Published),
            )
            .order_by_desc(property::statement::OwnerStatement::ID)
            .get(self.db())
            .await?;
        let mut revised = 0;
        for statement in candidates {
            if revised == 2 {
                break;
            }
            if pending.contains(&statement.id) {
                continue;
            }
            self.driver
                .post(
                    accountant,
                    &format!("/property/statements/{}/transitions/revise", statement.id),
                    &[(
                        "reason",
                        "The owner asked for the cleaning share to be split by unit",
                    )],
                )
                .await?;
            revised += 1;
            self.report.add("statements revised", 1);
        }
        Ok(())
    }

    /// A property manager other than the accountant approves the payout
    /// requests, except the three newest, which are still waiting.
    async fn approve_payouts(&mut self) -> Result {
        let head = super::people::HEAD_OF_PROPERTY;
        let mut waiting = ApprovalRequest::query()
            .where_(ApprovalRequest::ACTION.eq(property::statement_machine::PAYOUT_ACTION))
            .where_(ApprovalRequest::STATUS.eq(wl_kernel::ApprovalStatus::Pending))
            .order_by(ApprovalRequest::ID)
            .get(self.db())
            .await?;
        let keep = waiting.len().min(3);
        waiting.truncate(waiting.len() - keep);
        for request in waiting {
            self.driver
                .post(head, &format!("/approvals/{}/approve", request.id), &[])
                .await?;
            self.report.add("payouts approved", 1);
        }
        Ok(())
    }

    /// Makes one agreement and takes it as far as its standing says.
    async fn agreement(&mut self, plan: &AgreementPlan) -> Result<i64> {
        let admin = login_of(ADMIN).email;
        let owner = self.party(plan.owner).id.to_string();
        let start = text(day(plan.start));
        let mut form = vec![
            ("owner", owner.as_str()),
            ("fee", plan.fee),
            ("start", start.as_str()),
        ];
        if !plan.fixed.is_empty() {
            form.push(("fee_fixed", plan.fixed));
        }
        let before = ManagementAgreement::query().count(self.db()).await?;
        self.driver
            .post(admin, "/property/agreements", &form)
            .await?;
        let agreement = ManagementAgreement::query()
            .order_by_desc(ManagementAgreement::ID)
            .first(self.db())
            .await?
            .filter(|_| before >= 0)
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no agreement made"))?;
        let id = agreement.id;
        self.report.add("management agreements", 1);
        for (code, numbers) in plan.units {
            for number in *numbers {
                let unit = self.unit(code, number).id.to_string();
                self.driver
                    .post(
                        admin,
                        &format!("/property/agreements/{id}/units"),
                        &[("unit", unit.as_str()), ("date_in", start.as_str())],
                    )
                    .await?;
            }
        }
        if plan.standing == Standing::Draft {
            return Ok(id);
        }
        self.document(
            Subject::new(property::agreement::AGREEMENT_SUBJECT_TYPE, id),
            "management_agreement",
            "Signed management agreement",
            None,
            None,
            Some(day(plan.start)),
        )
        .await?;
        self.driver
            .post(admin, &format!("/property/agreements/{id}/activate"), &[])
            .await?;
        match plan.standing {
            Standing::Ending(notice, handover) | Standing::Ended(notice, handover) => {
                let notice_on = text(day(notice));
                let handover_on = text(day(handover));
                self.driver
                    .post(
                        admin,
                        &format!("/property/agreements/{id}/give-notice"),
                        &[
                            ("notice_on", notice_on.as_str()),
                            ("party", "owner"),
                            ("handover", handover_on.as_str()),
                        ],
                    )
                    .await?;
                if matches!(plan.standing, Standing::Ended(..)) {
                    self.end_agreement(id).await?;
                }
            }
            _ => {}
        }
        Ok(id)
    }

    /// The handover of an agreement whose units were empty: the final
    /// statement is prepared, approved and published, and the agreement ends.
    async fn end_agreement(&mut self, id: i64) -> Result {
        let accountant = login_of(ACCOUNTANT).email;
        let admin = login_of(ADMIN).email;
        let path = format!("/property/agreements/{id}");
        self.driver
            .post(accountant, &format!("{path}/final-statement"), &[])
            .await?;
        let statement = property::statement::OwnerStatement::of_agreement(self.db(), id)
            .await?
            .pop()
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no final statement"))?;
        let transition = format!("/property/statements/{}/transitions", statement.id);
        self.driver
            .post(admin, &format!("{transition}/approve"), &[])
            .await?;
        self.driver
            .post(accountant, &format!("{transition}/publish"), &[])
            .await?;
        self.driver.post(admin, &format!("{path}/end"), &[]).await?;
        Ok(())
    }

    /// The monthly statements of an active agreement, from the month of its
    /// first collection to the last month, as the rule of the fifth of the
    /// month makes them, and what the accountant and the managers did with
    /// them: older ones are settled, the last but one published or approved,
    /// the last still a draft.
    async fn statements(&mut self, plan: &AgreementPlan, id: i64) -> Result {
        let accountant = login_of(ACCOUNTANT).email;
        let admin = login_of(ADMIN).email;
        let first = self.first_collection(id).await?;
        let Some(first) = first else {
            return Ok(());
        };
        let this_month = today().with_day(1).unwrap_or_else(today);
        let mut month = first.with_day(1).unwrap_or(first);
        let mut made = Vec::new();
        while month < this_month {
            let end = month
                .checked_add_months(Months::new(1))
                .map(|d| d - Duration::days(1))
                .unwrap_or(month);
            if let Some(statement) =
                property::statement::generate(&self.state, id, month, end).await?
            {
                made.push(statement.id);
                self.report.add("owner statements", 1);
            }
            month = month
                .checked_add_months(Months::new(1))
                .unwrap_or(this_month);
        }
        let count = made.len();
        for (index, statement) in made.into_iter().enumerate() {
            let from_end = count - index;
            if from_end == 1 {
                continue;
            }
            let transition = format!("/property/statements/{statement}/transitions");
            // The accountant checks the draft and adds the cost of the month:
            // from then on the accountant is its preparer.
            self.driver
                .post(
                    accountant,
                    &format!("/property/statements/{statement}/lines"),
                    &[
                        ("kind", "expense"),
                        ("description", "Common area cleaning share"),
                        ("amount", &format!("{}.50", 180 + (statement % 7) * 20)),
                    ],
                )
                .await?;
            self.driver
                .post(admin, &format!("{transition}/approve"), &[])
                .await?;
            if from_end == 2 && plan.owner.len() % 2 == 0 {
                continue;
            }
            self.driver
                .post(accountant, &format!("{transition}/publish"), &[])
                .await?;
            if from_end <= 3 {
                continue;
            }
            self.driver
                .post(
                    accountant,
                    &format!("/property/statements/{statement}/pay-out"),
                    &[],
                )
                .await?;
        }
        let _ = (NaiveDate::MIN,);
        Ok(())
    }

    /// The first day an invoice of the units of agreement `id` was due.
    async fn first_collection(&self, id: i64) -> Result<Option<NaiveDate>> {
        let agreement = ManagementAgreement::find_or_404(self.db(), id).await?;
        let mut first: Option<NaiveDate> = None;
        for unit in agreement.units(self.db()).await? {
            let leases = property::Lease::query()
                .where_(property::Lease::UNIT_ID.eq(unit.unit_id))
                .get(self.db())
                .await?;
            for lease in leases {
                for charge in lease.charges(self.db()).await? {
                    if charge.invoice_id.is_some() {
                        first = Some(first.map_or(charge.due_date, |d| d.min(charge.due_date)));
                    }
                }
            }
        }
        Ok(first)
    }
}
