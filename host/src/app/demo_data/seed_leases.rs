//! Making the leases: the draft, the documents, the signature, the deposit,
//! the activation, the Ejari registration, the money, and then what became of
//! the lease. Each step is taken as the demo user who does it, on the day it
//! happened: the clock of the step is set to that day ([`super::clock`]).

use renox::chrono::{Datelike, Duration, Months, NaiveDate};
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::approvals::ApprovalRequest;

use super::leases::{Fate, PLANS, Plan, Quirk};
use super::people::{login_of, manager_of};
use super::real_today as today;
use super::roles::{ACCOUNTANT, LEASING_AGENT};
use super::seed_documents::party_number;
use super::{Seed, day};

/// A date as the forms take it.
pub(crate) fn text(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// Days from today to `date`.
fn offset(date: NaiveDate) -> i64 {
    (date - today()).num_days()
}

/// The earlier of `date` and today: a step cannot happen in the future.
pub(crate) fn upto(date: NaiveDate) -> NaiveDate {
    date.min(today())
}

impl Seed {
    pub(crate) async fn leases(&mut self) -> Result {
        for plan in PLANS {
            self.lease(plan).await?;
        }
        Ok(())
    }

    /// The term of `plan`: its first and last day.
    pub(crate) fn term(plan: &Plan) -> (NaiveDate, NaiveDate) {
        let start = day(plan.start);
        let end = start
            .checked_add_months(Months::new(plan.months))
            .and_then(|d| d.pred_opt())
            .unwrap_or(start);
        (start, end)
    }

    /// The annual rent of `plan` in dirhams.
    fn rent_of(&self, plan: &Plan) -> i64 {
        if plan.rent > 0 {
            plan.rent
        } else {
            self.unit(plan.unit.0, plan.unit.1).asking_rent.unwrap_or(0) / 100
        }
    }

    /// Makes the lease of `plan` and takes it as far as its fate says.
    pub(crate) async fn lease(&mut self, plan: &Plan) -> Result<i64> {
        let (start, _) = Self::term(plan);
        let future = start > today();
        // A lease that has not started was prepared in the last days; one that
        // has was prepared two weeks before it started.
        let created = if future {
            today() - Duration::days(6)
        } else {
            start - Duration::days(14)
        };
        let submitted = if future {
            today() - Duration::days(4)
        } else {
            start - Duration::days(8)
        };
        let agent = login_of(LEASING_AGENT).email;
        let id = at!(self, created, {
            self.papers(plan.tenant, plan.papers).await?;
            self.create_lease(plan).await
        })?;
        self.leases.insert(plan.key, id);
        self.report.add("lease versions", 1);
        if plan.fate == Fate::Draft {
            return Ok(id);
        }
        at!(self, submitted, {
            self.driver
                .post(agent, &format!("/property/leases/{id}/submit"), &[])
                .await
        })?;
        match plan.fate {
            Fate::Pending => return Ok(id),
            Fate::Cancelled => {
                at!(self, today() - Duration::days(2), {
                    self.driver
                        .post(
                            agent,
                            &format!("/property/leases/{id}/cancel"),
                            &[("reason", "The prospect withdrew before signing")],
                        )
                        .await
                })?;
                return Ok(id);
            }
            Fate::CancelAsked | Fate::CancelApproved => {
                self.cancel_with_money(plan, id).await?;
                return Ok(id);
            }
            _ => {}
        }
        self.activate(plan, id).await?;
        self.life(plan, id).await?;
        Ok(id)
    }

    async fn create_lease(&mut self, plan: &Plan) -> Result<i64> {
        let unit = self.unit(plan.unit.0, plan.unit.1).clone();
        let tenant = self.party(plan.tenant).clone();
        let (start, end) = Self::term(plan);
        let first = text(start);
        let last = text(end);
        let rent_text = self.rent_of(plan).to_string();
        let deposit_text = plan.deposit.to_string();
        let instalments = plan.instalments.to_string();
        let reply = self
            .driver
            .post(
                login_of(LEASING_AGENT).email,
                &format!("/property/units/{}/leases", unit.id),
                &[
                    ("tenant", tenant.code.as_str()),
                    ("start_date", first.as_str()),
                    ("end_date", last.as_str()),
                    ("annual_rent", rent_text.as_str()),
                    ("deposit", deposit_text.as_str()),
                    ("instalments", instalments.as_str()),
                    ("first_due", first.as_str()),
                    ("terms", ""),
                ],
            )
            .await?;
        reply
            .redirected_id()
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no lease id for {}", plan.key).into())
    }

    /// The deposit, the signed contract, the activation, the Ejari
    /// registration and the cheques of the submitted lease `id`.
    async fn activate(&mut self, plan: &Plan, id: i64) -> Result {
        let manager = manager_of(plan.unit.0);
        let (start, end) = Self::term(plan);
        let follow_up = self.is_follow_up(id).await?;
        let future = start > today();
        let paid = if future {
            today() - Duration::days(3)
        } else {
            start - Duration::days(5)
        };
        let signed = if future {
            today() - Duration::days(2)
        } else {
            start - Duration::days(4)
        };
        if !follow_up {
            at!(self, paid, { self.deposit(plan, id, paid).await })?;
        }
        at!(self, signed, {
            self.document(
                Subject::new("property.lease", id),
                "signed_tenancy_contract",
                "Signed tenancy contract",
                None,
                None,
                Some(signed),
            )
            .await
        })?;
        at!(self, upto(start), {
            self.driver
                .post(manager, &format!("/property/leases/{id}/activate"), &[])
                .await
        })?;
        let registered = upto(start + Duration::days(2));
        at!(self, registered, {
            self.ejari(plan, id, manager, registered, end).await
        })?;
        // The post-dated cheques come with the signed lease.
        at!(self, upto(start), { self.cheques_of(plan, id).await })
    }

    /// Whether lease `id` is a renewal or an amendment (its deposit is the
    /// first version's).
    async fn is_follow_up(&self, id: i64) -> Result<bool> {
        Ok(property::Lease::find_or_404(self.db(), id)
            .await?
            .is_follow_up())
    }

    /// The life of the active lease `id`: its instalments invoiced and paid on
    /// their days, and what became of it.
    async fn life(&mut self, plan: &Plan, id: i64) -> Result {
        let (_, end) = Self::term(plan);
        let until = match plan.fate {
            Fate::Terminated { move_out, .. } => day(move_out),
            _ => today().min(end),
        };
        self.money(plan, id, until).await?;
        self.invoice_quirk(plan, id).await?;
        self.fate(plan, id).await
    }

    /// What became of the active lease `id`.
    async fn fate(&mut self, plan: &Plan, id: i64) -> Result {
        let manager = manager_of(plan.unit.0);
        let agent = login_of(LEASING_AGENT).email;
        let path = format!("/property/leases/{id}");
        let (_, end) = Self::term(plan);
        match plan.fate {
            Fate::Window | Fate::Expired => {
                self.window(plan, id).await?;
            }
            Fate::Decided(decision) => {
                self.window(plan, id).await?;
                at!(self, today() - Duration::days(3), {
                    self.driver
                        .post(
                            manager,
                            &format!("{path}/renewal-decision"),
                            &[("decision", decision)],
                        )
                        .await?;
                    if decision == "changed_terms" {
                        self.driver
                            .post(manager, &format!("{path}/changed-terms-notice"), &[])
                            .await?;
                    }
                    Ok::<(), renox::Error>(())
                })?;
            }
            Fate::RenewalSent => {
                self.window(plan, id).await?;
                at!(self, today() - Duration::days(5), {
                    self.driver
                        .post(
                            manager,
                            &format!("{path}/renewal-decision"),
                            &[("decision", "same_terms")],
                        )
                        .await?;
                    let renewal = self.version(plan, id, "renewal", None).await?;
                    self.driver
                        .post(agent, &format!("/property/leases/{renewal}/submit"), &[])
                        .await
                })?;
            }
            Fate::Notice {
                by_tenant,
                move_out,
                penalty,
            } => {
                let planned = text(day(move_out));
                let given = today() - Duration::days(9);
                let date = text(given);
                let penalty = penalty.to_string();
                let mut form = vec![("date", date.as_str())];
                if by_tenant {
                    form.push(("party", "tenant"));
                    form.push((
                        "reason",
                        if penalty != "0" {
                            "early_termination"
                        } else {
                            "end_of_term"
                        },
                    ));
                    form.push(("planned_move_out", planned.as_str()));
                    form.push(("penalty", penalty.as_str()));
                } else {
                    form.push(("party", "landlord"));
                    form.push(("reason", "landlords_ground"));
                    form.push(("ground", "The owner needs the unit for a family member"));
                }
                at!(self, given, {
                    self.driver
                        .post(manager, &format!("{path}/notice"), &form)
                        .await
                })?;
            }
            Fate::Terminated {
                notice,
                move_out,
                defects,
            } => {
                self.terminate(plan, id, notice, move_out, defects).await?;
            }
            Fate::Renewed => {
                // The window opens, the manager decides, the renewal is
                // prepared, signed and starts the day after this one ends.
                let flag_day = end - Duration::days(110);
                at!(self, flag_day, {
                    property::renewal::run(&self.state, flag_day)
                        .await
                        .map(|_| ())
                })?;
                let renewal = at!(self, end - Duration::days(70), {
                    self.driver
                        .post(
                            manager,
                            &format!("{path}/renewal-decision"),
                            &[("decision", "same_terms")],
                        )
                        .await?;
                    self.version(plan, id, "renewal", None).await
                })?;
                self.activate_version(plan, renewal, true, end - Duration::days(3))
                    .await?;
            }
            Fate::Amended { effective, rent } => {
                let given = today() - Duration::days(2);
                let amended = at!(self, given, {
                    self.version(plan, id, "amendment", Some((day(effective), rent)))
                        .await
                })?;
                self.activate_version(plan, amended, false, given).await?;
            }
            _ => {}
        }
        Ok(())
    }

    /// The renewal window of an active lease: the daily rule opens it on its
    /// day, and flags the lease that ended as expired the day after it ended.
    async fn window(&mut self, plan: &Plan, _id: i64) -> Result {
        let (start, end) = Self::term(plan);
        let opens = (end - Duration::days(120)).max(start + Duration::days(1));
        let opened = upto(opens);
        at!(self, opened, {
            property::renewal::run(&self.state, opened)
                .await
                .map(|_| ())
        })?;
        if end < today() {
            let after = end + Duration::days(1);
            at!(self, after, {
                property::renewal::run(&self.state, after).await.map(|_| ())
            })?;
            // The holding-over periods are billed.
            let now = today();
            at!(self, now, { self.bill_through(now).await })?;
        }
        Ok(())
    }

    /// Prepares the next version of lease `id` (kind `renewal`: the next 12
    /// months at the same rent; `amendment`: the same term with a new rent
    /// from the effective day); the id of the draft.
    async fn version(
        &mut self,
        plan: &Plan,
        id: i64,
        kind: &str,
        amendment: Option<(NaiveDate, i64)>,
    ) -> Result<i64> {
        let agent = login_of(LEASING_AGENT).email;
        let (start, end) = Self::term(plan);
        let (new_start, new_end) = if amendment.is_some() {
            (start, end)
        } else {
            let next = end + Duration::days(1);
            (
                next,
                next.checked_add_months(Months::new(12))
                    .and_then(|d| d.pred_opt())
                    .unwrap_or(next),
            )
        };
        let rent = amendment.map_or_else(|| self.rent_of(plan), |(_, rent)| rent);
        let rent = rent.to_string();
        let deposit = plan.deposit.to_string();
        let instalments = plan.instalments.to_string();
        let first = text(new_start);
        let last = text(new_end);
        let effective = amendment.map(|(day, _)| text(day)).unwrap_or_default();
        let reply = self
            .driver
            .post(
                agent,
                &format!("/property/leases/{id}/versions"),
                &[
                    ("kind", kind),
                    ("start_date", first.as_str()),
                    ("end_date", last.as_str()),
                    ("annual_rent", rent.as_str()),
                    ("deposit", deposit.as_str()),
                    ("instalments", instalments.as_str()),
                    ("effective_date", effective.as_str()),
                ],
            )
            .await?;
        let next = reply
            .redirected_id()
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no version id for {}", plan.key))?;
        self.report.add("lease versions", 1);
        Ok(next)
    }

    /// Submits the version `id` of `plan` and activates it on `signed`; its
    /// life follows.
    async fn activate_version(
        &mut self,
        plan: &Plan,
        id: i64,
        renewal: bool,
        signed: NaiveDate,
    ) -> Result {
        let agent = login_of(LEASING_AGENT).email;
        let manager = manager_of(plan.unit.0);
        let (_, end) = Self::term(plan);
        at!(self, signed, {
            if !renewal {
                // An amendment's plan covers the whole term, the lease it
                // replaces only what is still to come, so the check for
                // changed terms asks the submitter; the rent went down and
                // nothing else changed.
                self.driver
                    .post(
                        agent,
                        &format!("/property/leases/{id}/terms-answer"),
                        &[
                            ("against", "no"),
                            ("reason", "Rent reduced by agreement; no other term changed"),
                        ],
                    )
                    .await?;
            }
            self.driver
                .post(agent, &format!("/property/leases/{id}/submit"), &[])
                .await?;
            self.document(
                Subject::new("property.lease", id),
                "signed_tenancy_contract",
                "Signed tenancy contract",
                None,
                None,
                Some(signed),
            )
            .await?;
            self.driver
                .post(manager, &format!("/property/leases/{id}/activate"), &[])
                .await
        })?;
        let mut next = *plan;
        next.fate = Fate::Active;
        next.quirk = Quirk::None;
        if renewal {
            next.start = offset(end + Duration::days(1));
        }
        let (start, finish) = Self::term(&next);
        let registered = upto(start + Duration::days(2));
        if registered >= signed {
            at!(self, registered, {
                self.ejari(&next, id, manager, registered, finish).await
            })?;
        }
        at!(self, upto(start.max(signed)), {
            self.cheques_of(&next, id).await
        })?;
        // The version's own life; boxed, since a lease's fate leads here.
        Box::pin(self.life(&next, id)).await
    }

    /// Notice, inspection, keys and termination of lease `id`, each on its
    /// day.
    async fn terminate(
        &mut self,
        plan: &Plan,
        id: i64,
        notice: i64,
        move_out: i64,
        defects: bool,
    ) -> Result {
        let manager = manager_of(plan.unit.0);
        let path = format!("/property/leases/{id}");
        let notice_date = text(day(notice));
        let planned = text(day(move_out));
        at!(self, day(notice), {
            self.driver
                .post(
                    manager,
                    &format!("{path}/notice"),
                    &[
                        ("date", notice_date.as_str()),
                        ("party", "tenant"),
                        ("reason", "end_of_term"),
                        ("planned_move_out", planned.as_str()),
                    ],
                )
                .await
        })?;
        at!(self, day(move_out), {
            self.driver
                .post(
                    manager,
                    &format!("{path}/move-out-inspection"),
                    &[
                        ("done_on", planned.as_str()),
                        ("keys_returned_on", planned.as_str()),
                        ("defects", if defects { "yes" } else { "no" }),
                    ],
                )
                .await?;
            self.driver
                .post(
                    manager,
                    &format!("{path}/terminate"),
                    &[("move_out", planned.as_str()), ("credit", "refund")],
                )
                .await
        })?;
        Ok(())
    }

    /// A lease that was paid for before it started, cancelled: the cheques
    /// and the deposit received, the leasing agent asks to cancel, and the
    /// property manager approves when the fate says so.
    async fn cancel_with_money(&mut self, plan: &Plan, id: i64) -> Result {
        let agent = login_of(LEASING_AGENT).email;
        let manager = manager_of(plan.unit.0);
        let paid = today() - Duration::days(3);
        at!(self, paid, {
            self.deposit(plan, id, paid).await?;
            self.cheques_of(plan, id).await
        })?;
        at!(self, today() - Duration::days(2), {
            self.driver
                .post(
                    agent,
                    &format!("/property/leases/{id}/cancel"),
                    &[("reason", "The prospect found a place closer to work")],
                )
                .await
        })?;
        if plan.fate == Fate::CancelApproved {
            let request = ApprovalRequest::query()
                .where_(ApprovalRequest::SUBJECT_TYPE.eq("property.lease"))
                .where_(ApprovalRequest::SUBJECT_ID.eq(id))
                .first(self.db())
                .await?
                .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no cancellation request"))?;
            at!(self, today() - Duration::days(1), {
                self.driver
                    .post(manager, &format!("/approvals/{}/approve", request.id), &[])
                    .await
            })?;
        }
        Ok(())
    }

    /// The deposit of the lease: received on `date` and held when it has one.
    async fn deposit(&mut self, plan: &Plan, id: i64, date: NaiveDate) -> Result {
        if plan.deposit <= 0 {
            return Ok(());
        }
        let party = self.party(plan.tenant).clone();
        let entity = self.entity_of(plan.unit.0).await?;
        let amount = (plan.deposit * 100).to_string();
        let party_id = party.id.to_string();
        let entity_id = entity.to_string();
        let lease = id.to_string();
        let reference = format!("DEP-{}-{}", party_number(&party.code), id);
        let date = text(date);
        self.driver
            .post(
                login_of(ACCOUNTANT).email,
                "/demo/deposits",
                &[
                    ("party", party_id.as_str()),
                    ("entity", entity_id.as_str()),
                    ("lease", lease.as_str()),
                    ("amount", amount.as_str()),
                    ("reference", reference.as_str()),
                    ("date", date.as_str()),
                ],
            )
            .await?;
        self.report.add("deposits held", 1);
        Ok(())
    }

    /// The id of the legal entity that issues for building `code`.
    pub(crate) async fn entity_of(&self, code: &str) -> Result<i64> {
        let building = self.building(code);
        Ok(wl_kernel::LegalEntity::for_scope(
            self.db(),
            Some((property::building::BUILDING_SUBJECT_TYPE, building.id)),
        )
        .await?
        .id)
    }

    /// The Ejari or Tawtheeq registration of an active lease, `on` the day it
    /// was registered.
    async fn ejari(
        &mut self,
        plan: &Plan,
        id: i64,
        manager: &str,
        on: NaiveDate,
        end: NaiveDate,
    ) -> Result {
        let number = format!("DEMO-EJ-{:07}", 4_000 + id * 7);
        self.document(
            Subject::new("property.lease", id),
            "ejari_certificate",
            if plan.unit.0 == "ADR" {
                "Tawtheeq certificate"
            } else {
                "Ejari certificate"
            },
            Some(number.clone()),
            Some(end),
            Some(on),
        )
        .await?;
        self.driver
            .post(
                manager,
                &format!("/property/leases/{id}/ejari"),
                &[
                    ("number", number.as_str()),
                    ("registered_on", text(on).as_str()),
                    ("expires_on", text(end).as_str()),
                ],
            )
            .await?;
        let _ = Datelike::year(&on);
        Ok(())
    }
}
