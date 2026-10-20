//! Making the leases: the draft, the documents, the signature, the deposit,
//! the activation, the Ejari registration, the money, and then what became of
//! the lease, each step as the demo user who does it.

use renox::chrono::{Duration, Months, NaiveDate};
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::approvals::ApprovalRequest;
use wl_kernel::documents::today;

use super::leases::{Fate, PLANS, Plan, Quirk};
use super::people::{login_of, manager_of};
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

impl Seed<'_> {
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
        self.papers(plan.tenant, plan.papers).await?;
        let unit = self.unit(plan.unit.0, plan.unit.1).clone();
        let tenant = self.party(plan.tenant).clone();
        let agent = login_of(LEASING_AGENT).email;
        let (start, end) = Self::term(plan);
        let first = text(start);
        let last = text(end);
        let rent_text = self.rent_of(plan).to_string();
        let deposit_text = plan.deposit.to_string();
        let instalments = plan.instalments.to_string();
        let reply = self
            .driver
            .post(
                agent,
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
        let id = reply
            .redirected_id()
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no lease id for {}", plan.key))?;
        self.leases.insert(plan.key, id);
        self.report.add("lease versions", 1);
        if plan.fate == Fate::Draft {
            return Ok(id);
        }
        self.driver
            .post(agent, &format!("/property/leases/{id}/submit"), &[])
            .await?;
        match plan.fate {
            Fate::Pending => return Ok(id),
            Fate::Cancelled => {
                self.driver
                    .post(
                        agent,
                        &format!("/property/leases/{id}/cancel"),
                        &[("reason", "The prospect withdrew before signing")],
                    )
                    .await?;
                return Ok(id);
            }
            Fate::CancelAsked | Fate::CancelApproved => {
                self.cancel_with_money(plan, id).await?;
                return Ok(id);
            }
            _ => {}
        }
        self.activate(plan, id).await?;
        self.after_activation(plan, id).await?;
        Ok(id)
    }

    /// The deposit, the signed contract, the activation and the Ejari
    /// registration of the submitted lease `id`.
    async fn activate(&mut self, plan: &Plan, id: i64) -> Result {
        let manager = manager_of(plan.unit.0);
        let (start, end) = Self::term(plan);
        if !self.is_follow_up(id).await? {
            self.deposit(plan, id).await?;
        }
        self.document(
            Subject::new("property.lease", id),
            "signed_tenancy_contract",
            "Signed tenancy contract",
            None,
            None,
            Some(start),
        )
        .await?;
        self.driver
            .post(manager, &format!("/property/leases/{id}/activate"), &[])
            .await?;
        // The certificate of a lease that has ended is expired, and the
        // document guard refuses to record a registration with an expired
        // one: those leases keep the registration they had in their day.
        if end < today() {
            return Ok(());
        }
        self.ejari(plan, id, manager, start, end).await
    }

    /// Whether lease `id` is a renewal or an amendment (its deposit is the
    /// first version's).
    async fn is_follow_up(&self, id: i64) -> Result<bool> {
        Ok(property::Lease::find_or_404(self.db(), id)
            .await?
            .is_follow_up())
    }

    /// The money of the active lease `id` and what became of it.
    async fn after_activation(&mut self, plan: &Plan, id: i64) -> Result {
        let (_, end) = Self::term(plan);
        let money_until = match plan.fate {
            Fate::Terminated { move_out, .. } => day(move_out),
            _ => today().min(end),
        };
        self.invoice_through(money_until).await?;
        self.cheques_of(plan, id).await?;
        self.bank_run(plan, id).await?;
        self.invoice_quirk(plan, id).await?;
        self.fate(plan, id).await
    }

    /// What became of the active lease `id`.
    async fn fate(&mut self, plan: &Plan, id: i64) -> Result {
        let manager = manager_of(plan.unit.0);
        let agent = login_of(LEASING_AGENT).email;
        let path = format!("/property/leases/{id}");
        match plan.fate {
            Fate::Window | Fate::Expired => {
                self.rules().await?;
            }
            Fate::Decided(decision) => {
                self.rules().await?;
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
            }
            Fate::RenewalSent => {
                self.rules().await?;
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
                    .await?;
            }
            Fate::Notice {
                by_tenant,
                move_out,
                penalty,
            } => {
                let planned = text(day(move_out));
                let date = text(today() - Duration::days(9));
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
                self.driver
                    .post(manager, &format!("{path}/notice"), &form)
                    .await?;
            }
            Fate::Terminated {
                notice,
                move_out,
                defects,
            } => {
                self.terminate(plan, id, notice, move_out, defects).await?;
            }
            Fate::Renewed => {
                self.rules().await?;
                let renewal = self.version(plan, id, "renewal", None).await?;
                self.activate_version(plan, renewal, true).await?;
            }
            Fate::Amended { effective, rent } => {
                let amended = self
                    .version(plan, id, "amendment", Some((day(effective), rent)))
                    .await?;
                self.activate_version(plan, amended, false).await?;
            }
            _ => {}
        }
        Ok(())
    }

    /// The daily rules of the lease module, for today: the renewal window,
    /// leases that ended, and the billing of the periods they hold over.
    pub(crate) async fn rules(&mut self) -> Result {
        let flagged = property::renewal::run(&self.state, today()).await?;
        let _ = flagged;
        self.invoice_through(today()).await
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

    /// Submits the version `id` of `plan` and activates it; its money follows.
    async fn activate_version(&mut self, plan: &Plan, id: i64, renewal: bool) -> Result {
        let agent = login_of(LEASING_AGENT).email;
        if !renewal {
            // An amendment's plan covers the whole term, the lease it replaces
            // only what is still to come, so the check for changed terms asks
            // the submitter; the rent went down and nothing else changed.
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
        let (_, end) = Self::term(plan);
        let mut next = *plan;
        next.fate = Fate::Active;
        next.quirk = Quirk::None;
        if renewal {
            next.start = offset(end + Duration::days(1));
        }
        self.activate(&next, id).await?;
        // The version's own money; boxed, since a lease's fate leads here.
        Box::pin(self.after_activation(&next, id)).await
    }

    /// Notice, inspection, keys and termination of lease `id`.
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
            .await?;
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
            .await?;
        Ok(())
    }

    /// A lease that was paid for before it started, cancelled: the cheques
    /// and the deposit received, the leasing agent asks to cancel, and the
    /// property manager approves when the fate says so.
    async fn cancel_with_money(&mut self, plan: &Plan, id: i64) -> Result {
        let agent = login_of(LEASING_AGENT).email;
        let manager = manager_of(plan.unit.0);
        self.deposit(plan, id).await?;
        self.cheques_of(plan, id).await?;
        self.driver
            .post(
                agent,
                &format!("/property/leases/{id}/cancel"),
                &[("reason", "The prospect found a place closer to work")],
            )
            .await?;
        if plan.fate == Fate::CancelApproved {
            let request = ApprovalRequest::query()
                .where_(ApprovalRequest::SUBJECT_TYPE.eq("property.lease"))
                .where_(ApprovalRequest::SUBJECT_ID.eq(id))
                .first(self.db())
                .await?
                .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no cancellation request"))?;
            self.driver
                .post(manager, &format!("/approvals/{}/approve", request.id), &[])
                .await?;
        }
        Ok(())
    }

    /// The deposit of the lease: received and held when it has one.
    async fn deposit(&mut self, plan: &Plan, id: i64) -> Result {
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
        let date = text(day(plan.start - 3).min(today()));
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

    /// The Ejari or Tawtheeq registration of an active lease.
    async fn ejari(
        &mut self,
        plan: &Plan,
        id: i64,
        manager: &str,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result {
        let number = format!("DEMO-EJ-{:07}", 4_000 + id * 7);
        let registered = start + Duration::days(2);
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
            Some(registered),
        )
        .await?;
        self.driver
            .post(
                manager,
                &format!("/property/leases/{id}/ejari"),
                &[
                    ("number", number.as_str()),
                    ("registered_on", text(registered).as_str()),
                    ("expires_on", text(end).as_str()),
                ],
            )
            .await?;
        Ok(())
    }
}
