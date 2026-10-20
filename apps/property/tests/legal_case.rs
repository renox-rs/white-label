//! BL-PRP-13 (#263): the legal case flag of a lease. While a flag is open the
//! reminders on the invoices of the lease pause (the chase is held), renewal
//! and amendment versions are refused, the tenant gets no automatic message,
//! and every flag has its ticket. A flag is cleared with an outcome; a bounced
//! cheque referred to legal sets one.

mod support;

use property::lease_machine::FLAG_RENEWAL;
use property::legal_case::{self, CaseOutcome, CaseStatus, CaseType, LegalCase, ticket_key};
use property::notices::{RENEWAL_DUE, RENEWAL_WINDOW};
use property::renewal;
use property::{Lease, LeaseStatus, NewLease, NewVersion};
use renox::prelude::*;
use wl_kernel::chase_holds::{ChaseHold, hold};
use wl_kernel::cheques::{ChequeId, ChequeReferredToLegal};
use wl_kernel::documents::today;
use wl_kernel::tickets::{Assignee, OpenTicket, TicketId, task_creator};

use support::{AGENT, Booted, MANAGER, TENANT, aed, boot, d, day};

struct Scene {
    t: Booted,
    building: property::Building,
    unit: property::Unit,
    agent: User,
    manager: User,
}

async fn scene() -> Scene {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    Scene {
        t,
        building,
        unit,
        agent,
        manager,
    }
}

impl Scene {
    /// An `active` lease (its term began 300 days ago and ends in 64 days).
    async fn lease(&self) -> Lease {
        self.t
            .active_lease(&self.unit, &self.agent, &self.manager)
            .await
    }

    /// `POST` that sets a flag.
    async fn set(&self, lease: &Lease, pairs: &[(&str, &str)]) -> renox::testing::TestResponse {
        self.t
            .post(
                &self.manager,
                &format!("/property/leases/{}/legal-cases", lease.id),
                pairs,
            )
            .await
    }

    /// Sets an arrears flag with a reference and a new ticket.
    async fn flag(&self, lease: &Lease) -> LegalCase {
        self.set(
            lease,
            &[
                ("case_type", "arrears"),
                ("reference", "RDC-2027-1234"),
                ("ticket", "new"),
            ],
        )
        .await
        .assert_ok();
        self.cases(lease).await.pop().expect("a flag")
    }

    async fn cases(&self, lease: &Lease) -> Vec<LegalCase> {
        LegalCase::of_root(self.t.db(), lease.root_id)
            .await
            .unwrap()
    }

    /// `POST` that clears the flag `case` with `outcome`.
    async fn clear(
        &self,
        lease: &Lease,
        case: &LegalCase,
        outcome: &str,
    ) -> renox::testing::TestResponse {
        self.t
            .post(
                &self.manager,
                &format!("/property/leases/{}/legal-cases/clear", lease.id),
                &[("case", case.id.to_string().as_str()), ("outcome", outcome)],
            )
            .await
    }

    async fn held(&self, lease: &Lease) -> ChaseHold {
        hold(self.t.db(), &lease.subject()).await.unwrap()
    }
}

/// AC1, AC2: a flag of type arrears with the reference RDC-2027-1234 creates
/// one *legal case* ticket for the property manager, with the lease as its
/// subject; the chase of the invoices of every version of the lease is held.
#[renox::test]
async fn bl_prp_13_a_flag_creates_its_ticket_and_holds_the_chase() {
    let s = scene().await;
    let lease = s.lease().await;
    assert_eq!(s.held(&lease).await, ChaseHold::Free);

    let case = s.flag(&lease).await;
    assert_eq!(case.status, CaseStatus::Open);
    assert_eq!(case.case_type, CaseType::Arrears);
    assert_eq!(case.reference.as_deref(), Some("RDC-2027-1234"));
    assert_eq!(case.root_id, lease.root_id);
    assert_eq!(case.opened_by, Some(s.manager.id));

    let tickets = s.t.tasks.with_key(&ticket_key(case.id));
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0].category, "legal_case");
    assert_eq!(tickets[0].subject, lease.subject());
    assert_eq!(
        tickets[0].assignee,
        Some(Assignee::User { id: s.manager.id })
    );
    assert_eq!(case.ticket_id, Some(tickets[0].id));
    assert!(case.ticket_number.is_some());

    // The chase is held for the lease and for a later version of it.
    assert_eq!(s.held(&lease).await, ChaseHold::Held);
    assert!(legal_case::has_open(s.t.db(), lease.root_id).await.unwrap());
    s.t.system(&lease, FLAG_RENEWAL).await;
    let renewal = property::leases::create_version(
        s.t.state(),
        Some(s.agent.id),
        lease.id,
        NewVersion::renewal(day(65), day(429), aed(100_000), aed(5_000), 4),
    )
    .await
    .unwrap();
    assert_eq!(s.held(&renewal).await, ChaseHold::Held);
    // Another lease is not held.
    let other_unit = s.t.unit(&s.building, "1300").await;
    let other = s.t.active_lease(&other_unit, &s.agent, &s.manager).await;
    assert_eq!(s.held(&other).await, ChaseHold::Free);

    // The page says so.
    s.t.app.acting_as(&s.manager);
    s.t.app
        .get(&format!("/property/leases/{}", lease.id))
        .await
        .assert_ok()
        .assert_see("Legal case open")
        .assert_see("RDC-2027-1234");
}

/// AC2: an open *legal case* ticket of the lease that has no flag yet can be
/// picked instead of a new one; no new ticket is created. A ticket that is not
/// the lease's, or already has a flag, is refused.
#[renox::test]
async fn bl_prp_13_an_existing_ticket_is_linked_instead_of_a_new_one() {
    let s = scene().await;
    let lease = s.lease().await;
    let state = s.t.state();
    let creator = task_creator(state);
    let own = creator
        .open(
            state,
            OpenTicket::new(lease.subject(), "legal_case", "Prepared case"),
        )
        .await
        .unwrap()
        .expect("a ticket");
    let elsewhere = creator
        .open(
            state,
            OpenTicket::new(s.unit.subject(), "legal_case", "Not a lease"),
        )
        .await
        .unwrap()
        .expect("a ticket");
    let before = s.t.tasks.all().len();

    // A ticket that is not the lease's: refused.
    s.set(
        &lease,
        &[
            ("case_type", "eviction"),
            ("ticket", elsewhere.id.0.to_string().as_str()),
        ],
    )
    .await
    .assert_status(422);
    assert!(s.cases(&lease).await.is_empty());

    s.set(
        &lease,
        &[
            ("case_type", "eviction"),
            ("ticket", own.id.0.to_string().as_str()),
        ],
    )
    .await
    .assert_ok();
    let cases = s.cases(&lease).await;
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].ticket_id, Some(own.id.0));
    assert_eq!(cases[0].reference, None, "the reference can wait");
    assert_eq!(s.t.tasks.all().len(), before, "no new ticket");

    // The same ticket cannot carry a second flag.
    s.set(
        &lease,
        &[
            ("case_type", "other"),
            ("ticket", own.id.0.to_string().as_str()),
        ],
    )
    .await
    .assert_status(422);
    assert_eq!(s.cases(&lease).await.len(), 1);
}

/// AC3: resolving the ticket by hand leaves the flag open; clearing the flag
/// with outcome paid resolves its ticket if it is still open.
#[renox::test]
async fn bl_prp_13_the_ticket_and_the_flag_clear_separately() {
    let s = scene().await;
    let lease = s.lease().await;
    let state = s.t.state();

    // Resolved by hand: the flag stays open.
    let first = s.flag(&lease).await;
    let ticket = TicketId(first.ticket_id.unwrap());
    assert!(
        task_creator(state)
            .complete(state, ticket, "Done by hand")
            .await
            .unwrap()
    );
    assert_eq!(s.held(&lease).await, ChaseHold::Held);
    assert!(s.cases(&lease).await[0].is_open());

    // Cleared with outcome paid: its ticket is resolved if still open.
    s.set(&lease, &[("case_type", "arrears")]).await.assert_ok();
    let second = s.cases(&lease).await.pop().unwrap();
    let key = ticket_key(second.id);
    assert!(s.t.tasks.with_key(&key)[0].open);
    s.clear(&lease, &second, "paid").await.assert_ok();
    let cleared = LegalCase::find_or_404(s.t.db(), second.id).await.unwrap();
    assert_eq!(cleared.status, CaseStatus::Cleared);
    assert_eq!(cleared.outcome, Some(CaseOutcome::Paid));
    assert_eq!(cleared.cleared_on, Some(today()));
    assert_eq!(cleared.cleared_by, Some(s.manager.id));
    let task = &s.t.tasks.with_key(&key)[0];
    assert!(!task.open);
    assert!(task.note.contains("paid"), "{}", task.note);
    // The first flag is still open, so the chase is still held.
    assert_eq!(s.held(&lease).await, ChaseHold::Held);
}

/// AC4: a renewal version cannot be submitted, or activated, while a flag is
/// open: "legal case open". A draft stays.
#[renox::test]
async fn bl_prp_13_a_renewal_is_refused_while_a_flag_is_open() {
    let s = scene().await;
    let v1 = s.lease().await;
    s.t.system(&v1, FLAG_RENEWAL).await;
    let case = s.flag(&v1).await;
    let v2 = property::leases::create_version(
        s.t.state(),
        Some(s.agent.id),
        v1.id,
        NewVersion::renewal(day(65), day(429), aed(100_000), aed(5_000), 4),
    )
    .await
    .unwrap();
    // The rent and the dates may differ from the version it renews.
    s.t.accept(&v2);

    let refused = s.t.submit(&s.agent, &v2).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("Legal case open"),
        "{}",
        refused.text()
    );
    assert_eq!(s.t.status(&v2).await, LeaseStatus::Draft, "the draft stays");

    // Cleared: it can be submitted; flagged again before activation: refused.
    s.clear(&v1, &case, "settled").await.assert_ok();
    s.t.submit(&s.agent, &v2).await.assert_ok();
    s.set(&v1, &[("case_type", "arrears")]).await.assert_ok();
    let refused = s.t.activate(&s.manager, &v2).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("Legal case open"),
        "{}",
        refused.text()
    );
    assert_eq!(s.t.status(&v2).await, LeaseStatus::PendingSignature);

    let second = s.cases(&v1).await.pop().unwrap();
    s.clear(&v1, &second, "paid").await.assert_ok();
    s.t.activate(&s.manager, &v2).await.assert_ok();
    assert_eq!(s.t.status(&v2).await, LeaseStatus::Active);
}

/// AC5: a bounced cheque referred to legal sets a flag of type arrears linked
/// to the cheque and the lease, with its ticket; once per cheque; a cheque of
/// another agreement is not ours.
#[renox::test]
async fn bl_prp_13_a_cheque_referred_to_legal_sets_a_flag() {
    let s = scene().await;
    let lease = s.lease().await;
    let referred = ChequeReferredToLegal {
        cheque: ChequeId(41),
        party: TENANT,
        agreement: Some(lease.agreement()),
        open_amount: aed(25_000),
        currency: "AED".to_owned(),
        case_owner: s.manager.id,
    };
    s.t.state().emit(referred.clone()).await.unwrap();

    let cases = s.cases(&lease).await;
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].case_type, CaseType::Arrears);
    assert_eq!(cases[0].cheque_id, Some(41));
    assert_eq!(cases[0].lease_id, lease.id);
    assert_eq!(cases[0].opened_by, Some(s.manager.id));
    assert_eq!(
        cases[0].reference, None,
        "the property manager completes it"
    );
    assert_eq!(s.t.tasks.with_key(&ticket_key(cases[0].id)).len(), 1);
    assert_eq!(s.held(&lease).await, ChaseHold::Held);

    // Once per cheque.
    s.t.state().emit(referred).await.unwrap();
    assert_eq!(s.cases(&lease).await.len(), 1);

    // A cheque received with something else, or with nothing.
    for agreement in [None, Some(wl_kernel::Subject::new("sales.order", 3))] {
        s.t.state()
            .emit(ChequeReferredToLegal {
                cheque: ChequeId(42),
                party: TENANT,
                agreement,
                open_amount: 100,
                currency: "AED".to_owned(),
                case_owner: s.manager.id,
            })
            .await
            .unwrap();
    }
    assert_eq!(s.cases(&lease).await.len(), 1);
}

/// AC6: clearing without an outcome is refused; clearing with `settled`
/// resumes the chase from the next step. A judgment needs the reference.
#[renox::test]
async fn bl_prp_13_a_flag_is_cleared_with_an_outcome() {
    let s = scene().await;
    let lease = s.lease().await;
    let case = s.flag(&lease).await;

    s.clear(&lease, &case, "").await.assert_status(422);
    s.clear(&lease, &case, "won").await.assert_status(422);
    assert!(s.cases(&lease).await[0].is_open());
    assert_eq!(s.held(&lease).await, ChaseHold::Held);

    s.clear(&lease, &case, "settled").await.assert_ok();
    assert_eq!(
        s.held(&lease).await,
        ChaseHold::Resumed { on: today() },
        "reminders resume from the next scheduled step"
    );
    assert!(!legal_case::has_open(s.t.db(), lease.root_id).await.unwrap());
    // Cleared once.
    s.clear(&lease, &case, "paid").await.assert_status(422);

    // A judgment needs the reference, which may be empty while it is prepared.
    s.set(&lease, &[("case_type", "eviction")])
        .await
        .assert_ok();
    let bare = s.cases(&lease).await.pop().unwrap();
    assert_eq!(bare.reference, None);
    let refused = s.clear(&lease, &bare, "judgment").await;
    refused.assert_status(422);
    assert!(refused.text().contains("reference"), "{}", refused.text());
    LegalCase::query()
        .where_(LegalCase::ID.eq(bare.id))
        .update(s.t.db(), &[("reference", &Some("CASE-77".to_owned()))])
        .await
        .unwrap();
    s.clear(&lease, &bare, "judgment").await.assert_ok();
    let done = LegalCase::find_or_404(s.t.db(), bare.id).await.unwrap();
    assert_eq!(done.outcome, Some(CaseOutcome::Judgment));
}

/// A lease may carry several flags: the effects apply while any is open, and
/// each is cleared separately.
#[renox::test]
async fn bl_prp_13_several_flags_hold_until_the_last_is_cleared() {
    let s = scene().await;
    let lease = s.lease().await;
    let one = s.flag(&lease).await;
    s.set(&lease, &[("case_type", "eviction")])
        .await
        .assert_ok();
    let cases = s.cases(&lease).await;
    assert_eq!(cases.len(), 2);

    s.clear(&lease, &one, "paid").await.assert_ok();
    assert_eq!(s.held(&lease).await, ChaseHold::Held, "one is still open");
    s.clear(&lease, &cases[1], "settled").await.assert_ok();
    assert_eq!(s.held(&lease).await, ChaseHold::Resumed { on: today() });
}

/// AC7: with a deposit dispute case open, the deposit's deductions dispute
/// flag stays until the case is cleared.
#[renox::test]
async fn bl_prp_13_a_deposit_dispute_keeps_the_deposit_flag_until_cleared() {
    let s = scene().await;
    let lease = s.lease().await;
    s.set(
        &lease,
        &[("case_type", "deposit_dispute"), ("reference", "RDC-9")],
    )
    .await
    .assert_ok();
    s.set(&lease, &[("case_type", "deposit_dispute")])
        .await
        .assert_ok();
    let source = lease.agreement();
    assert_eq!(
        s.t.deposits.legal(),
        vec![(source.clone(), true), (source.clone(), true)]
    );

    let cases = s.cases(&lease).await;
    s.clear(&lease, &cases[0], "settled").await.assert_ok();
    // Another deposit dispute case is open: the flag stays.
    assert_eq!(s.t.deposits.legal().len(), 2);
    s.clear(&lease, &cases[1], "paid").await.assert_ok();
    assert_eq!(s.t.deposits.legal().last(), Some(&(source, false)));
}

/// A terminated lease can keep a flag open; a lease that never started cannot
/// have one. Only a property manager sets and clears flags.
#[renox::test]
async fn bl_prp_13_who_and_when() {
    let s = scene().await;
    let lease = s.lease().await;
    let case = s.flag(&lease).await;

    // The leasing agent holds no `leases.manage`.
    s.t.post(
        &s.agent,
        &format!("/property/leases/{}/legal-cases", lease.id),
        &[("case_type", "arrears")],
    )
    .await
    .assert_forbidden();
    s.t.post(
        &s.agent,
        &format!("/property/leases/{}/legal-cases/clear", lease.id),
        &[("case", case.id.to_string().as_str()), ("outcome", "paid")],
    )
    .await
    .assert_forbidden();
    assert!(s.cases(&lease).await[0].is_open());

    // A terminated lease keeps its open flag, and can get a new one.
    s.t.set(&lease, &[("status", &LeaseStatus::Terminated)])
        .await;
    assert_eq!(s.held(&lease).await, ChaseHold::Held);
    s.set(&lease, &[("case_type", "arrears")]).await.assert_ok();

    // A draft has no case.
    let other_unit = s.t.unit(&s.building, "1300").await;
    let draft = s.t.lease(&other_unit, 0).await;
    let refused = s.set(&draft, &[("case_type", "arrears")]).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("never started"),
        "{}",
        refused.text()
    );
    assert!(s.cases(&draft).await.is_empty());
}

/// The tenant is sent no automatic message while a flag is open: the renewal
/// window notice is not sent for it; it is for a lease with no flag.
#[renox::test]
async fn bl_prp_13_the_tenant_gets_no_automatic_message_while_a_flag_is_open() {
    let s = scene().await;
    let mut leases = Vec::new();
    for number in ["1204", "1300"] {
        let unit = if number == "1204" {
            s.unit.clone()
        } else {
            s.t.unit(&s.building, number).await
        };
        let new = NewLease::new(unit.id, TENANT, d(1, 1), d(12, 31), aed(120_000), 12);
        let lease = property::leases::create_draft(s.t.state(), None, new)
            .await
            .unwrap();
        s.t.submit(&s.agent, &lease).await.assert_ok();
        s.t.activate(&s.manager, &lease).await.assert_ok();
        leases.push(s.t.reload(&lease).await);
    }
    s.flag(&leases[0]).await;

    renewal::run(s.t.state(), d(9, 2)).await.unwrap();
    for lease in &leases {
        assert_eq!(s.t.status(lease).await, LeaseStatus::RenewalDue);
    }
    // The managers are told about both; the tenant of the flagged lease is not.
    assert_eq!(s.t.notices.of_type(RENEWAL_DUE).len(), 2);
    let told: Vec<_> = s.t.notices.of_type(RENEWAL_WINDOW);
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].subject, leases[1].subject());
}

/// AC1 with the real invoicing: a lease with two overdue invoices and a flag
/// gets no reminder (the real reminders run skips the held source) and no
/// `PRP.RUL.10` task, yet the invoices still become `overdue`; an overdue
/// invoice of a source with no flag is reminded in the same run.
#[renox::test]
async fn bl_prp_13_no_reminder_is_sent_while_a_flag_is_open_but_the_invoices_become_overdue() {
    use wl_invoicing::reminders::{self, AFTER_DUE};
    use wl_invoicing::{Invoice, ReminderLog};
    use wl_kernel::invoicing::{DraftLine, InvoiceDraft, InvoiceStatus, VatCategory, invoicer};

    let t = support::boot_with(support::Real::INVOICING).await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    renox::auth::permissions::define_role(
        t.db(),
        "invoice_clerk",
        &[
            wl_invoicing::permissions::VIEW,
            wl_invoicing::permissions::ISSUE,
        ],
    )
    .await
    .unwrap();
    let clerk = t.user_with("Clara", "invoice_clerk").await;
    let lease = t.active_lease(&unit, &agent, &manager).await;

    // Two instalments fell due long ago (300 and 209 days back): invoiced,
    // issued, unpaid.
    property::invoicing::run(t.state(), day(-150))
        .await
        .unwrap();
    let mut ids: Vec<i64> = lease
        .charges(t.db())
        .await
        .unwrap()
        .iter()
        .filter_map(|c| c.invoice_id)
        .collect();
    ids.sort_unstable();
    assert_eq!(ids.len(), 2);
    // An overdue invoice of a source with no flag, to show the run does remind.
    let free = invoicer(t.state())
        .draft(
            t.db(),
            InvoiceDraft::new(TENANT, wl_kernel::Subject::new("property.lease", 9999))
                .due_date(day(-60))
                .line(DraftLine::new("Unflagged", aed(1_000), VatCategory::Exempt)),
        )
        .await
        .unwrap()
        .expect("invoicing is on")
        .0;
    for id in ids.iter().copied().chain([free]) {
        let issued = t.post(&clerk, &format!("/invoices/{id}/issue"), &[]).await;
        assert!(
            issued.status.is_success() || issued.status.is_redirection(),
            "{}",
            issued.text()
        );
    }

    // The flag is set; the chase is held.
    let s = Scene {
        t,
        building,
        unit,
        agent,
        manager,
    };
    let case = s.flag(&lease).await;
    assert_eq!(s.held(&lease).await, ChaseHold::Held);

    // FIN.RUL.02: the invoices still become overdue, flag or not.
    assert_eq!(wl_invoicing::overdue::run(s.t.state()).await.unwrap(), 3);
    for id in &ids {
        let invoice = Invoice::find_or_404(s.t.db(), *id).await.unwrap();
        assert_eq!(invoice.status, InvoiceStatus::Overdue, "invoice {id}");
    }

    // FIN.RUL.01: only the unflagged invoice is reminded.
    assert_eq!(reminders::run(s.t.state(), day(0)).await.unwrap(), 1);
    assert_eq!(s.t.notices.of_type(AFTER_DUE).len(), 1);
    for id in &ids {
        let logged = ReminderLog::query()
            .where_(ReminderLog::INVOICE_ID.eq(*id))
            .count(s.t.db())
            .await
            .unwrap();
        assert_eq!(logged, 0, "invoice {id} was not reminded");
    }
    assert_eq!(
        ReminderLog::query()
            .where_(ReminderLog::INVOICE_ID.eq(free))
            .count(s.t.db())
            .await
            .unwrap(),
        1
    );

    // PRP.RUL.10: no demand task for the flagged lease.
    let report = property::arrears::run(s.t.state(), day(0)).await.unwrap();
    assert_eq!(report.demands, 0);
    assert!(s.t.tasks.with_key("property.arrears_demand:").is_empty());

    // The flag clears with an outcome and the chase resumes.
    s.clear(&lease, &case, "paid").await.assert_ok();
    assert!(matches!(s.held(&lease).await, ChaseHold::Resumed { .. }));
}
