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
    (
        "b202",
        Outcome::Refund("Professional cleaning of the unit", 350),
    ),
    (
        "b201",
        Outcome::Refund("Repainting the living room walls", 1_200),
    ),
    (
        "aq3",
        Outcome::Forfeit("Floor damage and removal of fixtures, beyond the deposit"),
    ),
    (
        "sj401",
        Outcome::Forfeit("The lease was broken and the unit left damaged"),
    ),
];

impl Seed {
    pub(crate) async fn tail(&mut self) -> Result {
        self.deposits().await?;
        self.transfers().await?;
        self.refund_decisions().await?;
        self.withdraw_units().await?;
        self.document_states().await?;
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
                                    format!("{what} | {amount}.00 | Move-out inspection report")
                                        .as_str(),
                                )],
                            )
                            .await
                    })?;
                    at!(self, settled, {
                        self.driver
                            .post(
                                accountant,
                                &format!("{path}/refund"),
                                &[("method", "transfer")],
                            )
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

    /// Documents in the other states of the review: two replaced by a newer
    /// Emirates ID, two waiting for review, two rejected with a reason. The
    /// tenants are those with full papers; the two that fail the lease check
    /// on purpose are left as they are. An archived document is not made: the
    /// module has no page for it (`Documents::archive` needs a signed-in
    /// user).
    async fn document_states(&mut self) -> Result {
        use super::seed_documents::{Papers, party_number};
        use wl_kernel::Subject;
        use wl_kernel::parties::PARTY_SUBJECT_TYPE;
        let admin = login_of(ADMIN).email;
        let people: Vec<String> = PLANS
            .iter()
            .filter(|p| p.papers == Papers::Full)
            .map(|p| p.tenant.to_owned())
            .filter(|name| self.papered.contains(name))
            .filter(|name| self.party(name).kind == wl_kernel::PartyKind::Person)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        for (i, name) in people.iter().take(6).enumerate() {
            let party = self.party(name).clone();
            let n = party_number(&party.code);
            let subject = Subject::new(PARTY_SUBJECT_TYPE, party.id);
            match i / 2 {
                // A renewed Emirates ID takes the place of the old one.
                0 => {
                    self.document(
                        subject,
                        "emirates_id",
                        "Emirates ID (renewed)",
                        Some(super::seed_people::emirates_id(n, 1990)),
                        Some(day(1_400 + i as i64)),
                        Some(day(-20)),
                    )
                    .await?;
                }
                // Uploaded from the portal, not reviewed yet.
                1 => {
                    self.attach_document(
                        subject,
                        "passport",
                        "Passport (new copy)",
                        Some(super::seed_people::passport(n + 500)),
                        Some(day(2_000)),
                        Some(day(-3)),
                    )
                    .await?;
                }
                _ => {
                    let id = self
                        .attach_document(
                            subject,
                            "residence_visa",
                            "Residence visa (blurred scan)",
                            Some(super::seed_people::visa(n + 500)),
                            Some(day(900)),
                            Some(day(-5)),
                        )
                        .await?;
                    self.driver
                        .post(
                            admin,
                            &format!("/documents/{id}/reject"),
                            &[("reason", "The scan is blurred: the number cannot be read")],
                        )
                        .await?;
                }
            }
        }
        Ok(())
    }

    /// Two empty units taken off the market (a sale in progress, a
    /// refurbishment). Reserved units are not made: the module refuses to
    /// reserve until booking deposits exist (`property.units.reserve_later`),
    /// and a unit is not inserted in a state the module itself refuses.
    async fn withdraw_units(&mut self) -> Result {
        let admin = login_of(ADMIN).email;
        let mut empty: Vec<i64> = property::Unit::query()
            .where_(property::Unit::STATUS.eq(property::UnitStatus::Available))
            .order_by_desc(property::Unit::ID)
            .get(self.db())
            .await?
            .into_iter()
            .map(|u| u.id)
            .collect();
        empty.truncate(2);
        for id in empty {
            self.driver
                .post(
                    admin,
                    &format!("/property/units/{id}/transitions/withdraw"),
                    &[("reason", "Taken off the market for a refurbishment")],
                )
                .await?;
            self.report.add("units withdrawn", 1);
        }
        Ok(())
    }

    /// The head of the company decides four of the six refund requests (none
    /// of them is by the admin, so the separation of duties holds): two are
    /// approved, two rejected with a reason; two stay waiting.
    async fn refund_decisions(&mut self) -> Result {
        let admin = login_of(ADMIN).email;
        let mut ids: Vec<i64> = wl_payments::Refund::query()
            .order_by(wl_payments::Refund::ID)
            .get(self.db())
            .await?
            .into_iter()
            .filter(|r| r.requested_by.is_some())
            .map(|r| r.id)
            .collect();
        ids.truncate(4);
        for (i, id) in ids.into_iter().enumerate() {
            let path = format!("/refunds/{id}/transitions");
            if i % 2 == 0 {
                self.driver
                    .post(admin, &format!("{path}/approve"), &[])
                    .await?;
            } else {
                self.driver
                    .post(
                        admin,
                        &format!("{path}/reject"),
                        &[(
                            "reason",
                            "The credit note was issued in error; no money is due",
                        )],
                    )
                    .await?;
            }
            self.report.add("refunds decided", 1);
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
