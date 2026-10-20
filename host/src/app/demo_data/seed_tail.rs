//! The last steps: how the deposits of the leases that ended were settled, and
//! the daily rules of the modules run for today, so that reminders, overdue
//! invoices, the deposit list and the expiry notices are there on the first
//! day.

use property::deposit_watch;
use renox::chrono::Duration;
use renox::prelude::*;
use wl_payments::Deposit;

use super::leases::{Fate, PLANS};
use super::people::login_of;
use super::real_today as today;
use super::roles::{ACCOUNTANT, ADMIN};
use super::seed_leases::upto;
use super::{Seed, day};

/// How a deposit was settled.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// The deductions were published and the rest was refunded.
    Refund(&'static str, i64),
    /// The deposit was kept, with the reason.
    Forfeit(&'static str),
}

/// The lease and how its deposit was settled.
const SETTLED: &[(&str, Outcome)] = &[
    ("b202", Outcome::Refund("Professional cleaning of the unit", 350)),
    ("b201", Outcome::Refund("Repainting the living room walls", 1_200)),
    ("aq3", Outcome::Forfeit("Floor damage and removal of fixtures, beyond the deposit")),
    ("sj401", Outcome::Forfeit("The lease was broken and the unit left damaged")),
];

impl Seed<'_> {
    pub(crate) async fn tail(&mut self) -> Result {
        self.deposits().await?;
        self.transfers().await?;
        let now = today();
        at!(self, now, { self.rules(now).await })
    }

    /// The deposits of the leases that ended: deductions or a forfeiture
    /// published three days after the move-out, and, once the tenant's 14
    /// days to dispute have passed, the refund or the forfeit.
    async fn deposits(&mut self) -> Result {
        let accountant = login_of(ACCOUNTANT).email;
        let admin = login_of(ADMIN).email;
        let admin_id = self.user_id(admin).await?;
        for (key, outcome) in SETTLED {
            let Some(&lease) = self.leases.get(key) else {
                continue;
            };
            let Some(plan) = PLANS.iter().find(|p| p.key == *key) else {
                continue;
            };
            let Fate::Terminated { move_out, .. } = plan.fate else {
                continue;
            };
            let Some(deposit) = Deposit::query()
                .where_(Deposit::SOURCE_ID.eq(lease))
                .where_(Deposit::SOURCE_TYPE.eq("property.lease"))
                .first(self.db())
                .await?
            else {
                continue;
            };
            let path = format!("/deposits/{}", deposit.id);
            let published = upto(day(move_out + 3));
            let settled = upto(day(move_out + 22));
            match outcome {
                Outcome::Refund(what, amount) => {
                    at!(self, published, {
                        self.driver
                            .post(
                                accountant,
                                &format!("{path}/deductions"),
                                &[(
                                    "lines",
                                    format!("{what} | {amount}.00 | Move-out inspection report").as_str(),
                                )],
                            )
                            .await
                    })?;
                    at!(self, settled, {
                        self.driver
                            .post(accountant, &format!("{path}/refund"), &[("method", "transfer")])
                            .await
                    })?;
                }
                Outcome::Forfeit(reason) => {
                    at!(self, published, {
                        self.driver
                            .post(
                                accountant,
                                &format!("{path}/deductions"),
                                &[("forfeiture_reason", *reason)],
                            )
                            .await
                    })?;
                    at!(self, settled, {
                        self.driver
                            .post(
                                accountant,
                                &format!("{path}/forfeit"),
                                &[
                                    ("reason", *reason),
                                    ("approver", admin_id.to_string().as_str()),
                                ],
                            )
                            .await
                    })?;
                }
            }
            self.report.add("deposits settled", 1);
        }
        Ok(())
    }

    /// Two deposits handed to the owner who took the unit over, against the
    /// owner's receipt.
    async fn transfers(&mut self) -> Result {
        let accountant = login_of(ACCOUNTANT).email;
        for (key, owner, receipt, ago) in [
            ("b101", "Mariam Obaid Al Suwaidi", "RCT-OWN-0007", 20),
            ("ad201", "Rashed Saif Al Dhaheri", "RCT-OWN-0011", 15),
        ] {
            let Some(&lease) = self.leases.get(key) else {
                continue;
            };
            let Some(deposit) = Deposit::query()
                .where_(Deposit::SOURCE_ID.eq(lease))
                .where_(Deposit::SOURCE_TYPE.eq("property.lease"))
                .first(self.db())
                .await?
            else {
                continue;
            };
            let code = self.party(owner).code.clone();
            at!(self, day(-ago), {
                self.driver
                    .post(
                        accountant,
                        &format!("/deposits/{}/transfer", deposit.id),
                        &[("new_holder", code.as_str()), ("receipt", receipt)],
                    )
                    .await
            })?;
        }
        Ok(())
    }

    /// The daily rules, run for `now`.
    async fn rules(&mut self, now: renox::chrono::NaiveDate) -> Result {
        wl_invoicing::overdue::run(&self.state).await?;
        wl_invoicing::reminders::run(&self.state, now).await?;
        wl_payments::cheque_deposit::flag_due_run(&self.state, now).await?;
        wl_payments::cheque_chase::run(&self.state, now).await?;
        property::arrears::run(&self.state, now).await?;
        deposit_watch::run(&self.state, now).await?;
        property::chase::run(&self.state, now).await?;
        wl_documents::expiry::run(&self.state, now).await?;
        wl_sales::campaigns::run_daily(&self.state, now).await?;
        let _ = Duration::days(0);
        Ok(())
    }
}
