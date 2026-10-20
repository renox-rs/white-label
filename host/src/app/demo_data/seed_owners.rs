//! Making the management agreements and the owner statements of 15 months.

use property::agreement::ManagementAgreement;
use property::statement::{OwnerStatement, StatementStatus};
use renox::chrono::{Datelike, Duration, Months, NaiveDate};
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::approvals::ApprovalRequest;

use super::owners::{AGREEMENTS, AgreementPlan, Standing};
use super::people::{HEAD_OF_PROPERTY, login_of};
use super::real_today as today;
use super::roles::{ACCOUNTANT, ADMIN};
use super::seed_leases::{text, upto};
use super::{Seed, day};

impl Seed<'_> {
    /// The agreements and their statements.
    pub(crate) async fn owners(&mut self) -> Result {
        let mut made = Vec::new();
        for plan in AGREEMENTS {
            let id = self.agreement(plan).await?;
            made.push((plan, id));
        }
        for (index, (plan, id)) in made.into_iter().enumerate() {
            if matches!(plan.standing, Standing::Active) {
                self.statements(plan, id, index).await?;
            }
        }
        self.revisions().await
    }

    /// Makes one agreement and takes it as far as its standing says.
    async fn agreement(&mut self, plan: &AgreementPlan) -> Result<i64> {
        let admin = login_of(ADMIN).email;
        let owner = self.party(plan.owner).id.to_string();
        let start = text(day(plan.start));
        let created = day(plan.start.min(0) - 5);
        let id = at!(self, created.min(today()), {
            let mut form = vec![
                ("owner", owner.as_str()),
                ("fee", plan.fee),
                ("start", start.as_str()),
            ];
            if !plan.fixed.is_empty() {
                form.push(("fee_fixed", plan.fixed));
            }
            self.driver.post(admin, "/property/agreements", &form).await?;
            let agreement = ManagementAgreement::query()
                .order_by_desc(ManagementAgreement::ID)
                .first(self.db())
                .await?
                .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no agreement made"))?;
            for (code, numbers) in plan.units {
                for number in *numbers {
                    let unit = self.unit(code, number).id.to_string();
                    self.driver
                        .post(
                            admin,
                            &format!("/property/agreements/{}/units", agreement.id),
                            &[("unit", unit.as_str()), ("date_in", start.as_str())],
                        )
                        .await?;
                }
            }
            Ok::<i64, renox::Error>(agreement.id)
        })?;
        self.report.add("management agreements", 1);
        if plan.standing == Standing::Draft {
            return Ok(id);
        }
        let signed = upto(day(plan.start));
        at!(self, signed, {
            self.document(
                Subject::new(property::agreement::AGREEMENT_SUBJECT_TYPE, id),
                "management_agreement",
                "Signed management agreement",
                None,
                None,
                Some(signed),
            )
            .await?;
            self.driver
                .post(admin, &format!("/property/agreements/{id}/activate"), &[])
                .await
        })?;
        match plan.standing {
            Standing::Ending(notice, handover) | Standing::Ended(notice, handover) => {
                let notice_on = text(day(notice));
                let handover_on = text(day(handover));
                at!(self, day(notice), {
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
                        .await
                })?;
                if matches!(plan.standing, Standing::Ended(..)) {
                    self.end_agreement(id, day(handover)).await?;
                }
            }
            _ => {}
        }
        Ok(id)
    }

    /// The handover of an agreement whose units were empty: the final
    /// statement is prepared, approved and published, and the agreement ends.
    async fn end_agreement(&mut self, id: i64, handover: NaiveDate) -> Result {
        let accountant = login_of(ACCOUNTANT).email;
        let admin = login_of(ADMIN).email;
        let path = format!("/property/agreements/{id}");
        let on = upto(handover);
        at!(self, on, {
            self.driver
                .post(accountant, &format!("{path}/final-statement"), &[])
                .await
        })?;
        let statement = OwnerStatement::of_agreement(self.db(), id)
            .await?
            .pop()
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no final statement"))?;
        let transition = format!("/property/statements/{}/transitions", statement.id);
        at!(self, upto(on + Duration::days(1)), {
            self.driver
                .post(admin, &format!("{transition}/approve"), &[])
                .await?;
            self.driver
                .post(accountant, &format!("{transition}/publish"), &[])
                .await?;
            self.driver.post(admin, &format!("{path}/end"), &[]).await
        })?;
        Ok(())
    }

    /// The monthly statements of an active agreement, from the month of its
    /// first collection to the last month, as the rule of the fifth of the
    /// month makes them, and what the accountant and the managers did with
    /// them: older ones are settled, the third from last waits for its payout
    /// to be approved, the last but one is published or approved, the last is
    /// still a draft.
    async fn statements(&mut self, plan: &AgreementPlan, id: i64, number: usize) -> Result {
        let accountant = login_of(ACCOUNTANT).email;
        let admin = login_of(ADMIN).email;
        let head = HEAD_OF_PROPERTY;
        let Some(first) = self.first_collection(id).await? else {
            return Ok(());
        };
        let this_month = today().with_day(1).unwrap_or_else(today);
        let mut months = Vec::new();
        let mut month = first.with_day(1).unwrap_or(first);
        while month < this_month {
            months.push(month);
            month = month
                .checked_add_months(Months::new(1))
                .unwrap_or(this_month);
        }
        let count = months.len();
        for (index, month) in months.into_iter().enumerate() {
            let from_end = count - index;
            let end = month
                .checked_add_months(Months::new(1))
                .map(|d| d - Duration::days(1))
                .unwrap_or(month);
            // The rule runs on the fifth of the next month.
            let drafted = upto(end + Duration::days(5));
            let Some(statement) = at!(self, drafted, {
                property::statement::generate(&self.state, id, month, end).await
            })?
            else {
                continue;
            };
            self.report.add("owner statements", 1);
            if from_end == 1 {
                continue;
            }
            let transition = format!("/property/statements/{}/transitions", statement.id);
            // The accountant checks the draft and adds the cost of the month:
            // from then on the accountant is its preparer.
            at!(self, upto(drafted + Duration::days(1)), {
                self.driver
                    .post(
                        accountant,
                        &format!("/property/statements/{}/lines", statement.id),
                        &[
                            ("kind", "expense"),
                            ("description", "Common area cleaning share"),
                            ("amount", &format!("{}.50", 80 + (statement.id % 7) * 20)),
                        ],
                    )
                    .await?;
                self.driver
                    .post(admin, &format!("{transition}/approve"), &[])
                    .await
            })?;
            if from_end == 2 && (number + plan.owner.len()) % 2 == 0 {
                continue;
            }
            at!(self, upto(drafted + Duration::days(3)), {
                self.driver
                    .post(accountant, &format!("{transition}/publish"), &[])
                    .await
            })?;
            if from_end == 2 {
                continue;
            }
            let settled = OwnerStatement::find_or_404(self.db(), statement.id).await?;
            if settled.balance() <= 0 {
                at!(self, upto(drafted + Duration::days(4)), {
                    self.driver
                        .post(accountant, &format!("{transition}/carry_forward"), &[])
                        .await
                })?;
                continue;
            }
            at!(self, upto(drafted + Duration::days(4)), {
                self.driver
                    .post(
                        accountant,
                        &format!("/property/statements/{}/pay-out", statement.id),
                        &[],
                    )
                    .await
            })?;
            if from_end == 3 {
                // The payout waits for a property manager.
                continue;
            }
            let request = ApprovalRequest::query()
                .where_(ApprovalRequest::SUBJECT_ID.eq(statement.id))
                .where_(ApprovalRequest::ACTION.eq(property::statement_machine::PAYOUT_ACTION))
                .first(self.db())
                .await?;
            if let Some(request) = request {
                at!(self, upto(drafted + Duration::days(7)), {
                    self.driver
                        .post(head, &format!("/approvals/{}/approve", request.id), &[])
                        .await
                })?;
                self.report.add("payouts approved", 1);
            }
        }
        Ok(())
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
        let candidates = OwnerStatement::query()
            .where_(OwnerStatement::STATUS.eq(StatementStatus::Published))
            .order_by_desc(OwnerStatement::ID)
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
            at!(self, today() - Duration::days(6), {
                self.driver
                    .post(
                        accountant,
                        &format!("/property/statements/{}/transitions/revise", statement.id),
                        &[("reason", "The owner asked for the cleaning share to be split by unit")],
                    )
                    .await
            })?;
            revised += 1;
            self.report.add("statements revised", 1);
        }
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
