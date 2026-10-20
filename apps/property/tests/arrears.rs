//! BL-PRP-14 (#264): `PRP.RUL.10`. Thirty days after an invoice's due date the
//! property manager gets one task per lease to send a formal payment demand
//! listing the overdue invoices; 30 days after the demand's proof of delivery,
//! a second task asks to open a legal case or agree a payment plan.

mod support;

use property::arrears::{self, ArrearsDemand, NewPlan, demand_key, legal_key};
use property::notices::PAYMENT_DEMAND;
use property::{Lease, NewLease};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::invoicing::InvoiceStatus;
use wl_kernel::notify::NoticeStatus;

use support::{AGENT, Booted, MANAGER, TENANT, aed, boot, d, day};

struct Scene {
    t: Booted,
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
        unit,
        agent,
        manager,
    }
}

impl Scene {
    /// An `active` lease from 2027-03-01 at AED 12,000 a month.
    async fn leased(&self) -> Lease {
        let new = NewLease::new(
            self.unit.id,
            TENANT,
            d(3, 1),
            ymd(2028, 2, 29),
            aed(144_000),
            12,
        );
        self.activated(new).await
    }

    async fn activated(&self, new: NewLease) -> Lease {
        let lease = property::leases::create_draft(self.t.state(), None, new)
            .await
            .unwrap();
        self.t.submit(&self.agent, &lease).await.assert_ok();
        self.t.activate(&self.manager, &lease).await.assert_ok();
        self.t.reload(&lease).await
    }

    /// The daily invoicing run for `on`; the drafts are issued, unpaid.
    async fn invoice(&self, on: NaiveDate) {
        property::invoicing::run(self.t.state(), on).await.unwrap();
        for invoice in self.t.invoices.all() {
            if invoice.summary.status == InvoiceStatus::Draft {
                self.t.invoices.issue(invoice.summary.id.0);
            }
        }
    }

    async fn run(&self, on: NaiveDate) -> arrears::Report {
        arrears::run(self.t.state(), on).await.unwrap()
    }

    async fn demands(&self, lease: &Lease) -> Vec<ArrearsDemand> {
        ArrearsDemand::of_root(self.t.db(), lease.root_id)
            .await
            .unwrap()
    }

    /// The open tasks about the demands to send.
    fn demand_tasks(&self) -> Vec<support::Task> {
        self.t
            .tasks
            .with_key("property.arrears_demand:")
            .into_iter()
            .filter(|t| t.open)
            .collect()
    }

    /// The open tasks about a legal case.
    fn legal_tasks(&self) -> Vec<support::Task> {
        self.t
            .tasks
            .with_key("property.arrears_legal:")
            .into_iter()
            .filter(|t| t.open)
            .collect()
    }

    /// The manager sends demand `demand` on `on`.
    async fn send(&self, lease: &Lease, demand: &ArrearsDemand, on: NaiveDate) -> ArrearsDemand {
        arrears::send_demand_as(
            self.t.state(),
            Some(self.manager.id),
            lease.id,
            demand.id,
            on,
        )
        .await
        .unwrap()
    }
}

fn ymd(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

/// AC1: an invoice of AED 12,000 due 2027-03-01, unpaid and not disputed: the
/// property manager has one task "send formal payment demand" on 2027-03-31,
/// not on the 30th, and only once.
#[renox::test]
async fn bl_prp_14_the_demand_task_comes_thirty_days_after_the_due_date() {
    let s = scene().await;
    let lease = s.leased().await;
    s.invoice(d(3, 1)).await;

    let early = s.run(d(3, 30)).await;
    assert_eq!(early.demands, 0);
    assert!(s.demand_tasks().is_empty());

    let on_time = s.run(d(3, 31)).await;
    assert_eq!(on_time.demands, 1);
    let tasks = s.demand_tasks();
    assert_eq!(tasks.len(), 1);
    assert!(
        tasks[0].title.contains("formal payment demand"),
        "{}",
        tasks[0].title
    );
    assert!(tasks[0].title.contains("L-0000"), "{}", tasks[0].title);
    assert!(
        tasks[0].description.contains("AED 12,000.00"),
        "{}",
        tasks[0].description
    );
    assert_eq!(tasks[0].subject, lease.subject());

    // A second run changes nothing.
    let again = s.run(d(3, 31)).await;
    assert_eq!((again.demands, again.added), (0, 0));
    assert_eq!(s.demand_tasks().len(), 1);
    let demands = s.demands(&lease).await;
    assert_eq!(demands.len(), 1);
    assert_eq!(demands[0].invoices(s.t.db()).await.unwrap().len(), 1);
}

/// AC2: with that demand not yet sent, a second invoice of the lease due
/// 2027-04-01 is added to the open task on 2027-05-01, not a new task.
#[renox::test]
async fn bl_prp_14_a_later_invoice_is_added_to_the_open_demand() {
    let s = scene().await;
    let lease = s.leased().await;
    s.invoice(d(3, 1)).await;
    s.run(d(3, 31)).await;
    s.invoice(d(4, 1)).await;

    let before = s.run(d(4, 30)).await;
    assert_eq!(before.added, 0);
    let report = s.run(d(5, 1)).await;
    assert_eq!((report.demands, report.added), (0, 1));

    assert_eq!(s.demand_tasks().len(), 1);
    let demands = s.demands(&lease).await;
    assert_eq!(demands.len(), 1);
    assert_eq!(demands[0].invoices(s.t.db()).await.unwrap().len(), 2);
}

/// Sending the demand is the property manager's: a statutory notice to the
/// tenant that closes the task and freezes the list.
#[renox::test]
async fn bl_prp_14_sending_the_demand_closes_the_task_and_freezes_the_list() {
    let s = scene().await;
    let lease = s.leased().await;
    s.invoice(d(3, 1)).await;
    s.run(d(3, 31)).await;
    let demand = s.demands(&lease).await.remove(0);

    let sent = s.send(&lease, &demand, d(4, 3)).await;
    assert_eq!(sent.sent_on, Some(d(4, 3)));
    assert_eq!(sent.sent_by, Some(s.manager.id));
    assert!(s.demand_tasks().is_empty());
    let notices = s.t.notices.of_type(PAYMENT_DEMAND);
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].subject, demand.subject());
    // The statutory notice has not been delivered: no clock yet.
    assert_eq!(
        arrears::delivered_on(s.t.state(), &sent).await.unwrap(),
        None
    );

    // Sent twice: refused.
    let again = arrears::send_demand_as(
        s.t.state(),
        Some(s.manager.id),
        lease.id,
        demand.id,
        d(4, 4),
    )
    .await;
    assert!(matches!(
        again,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    ));
    assert_eq!(s.t.notices.of_type(PAYMENT_DEMAND).len(), 1);

    // The invoices on a sent demand are not listed again.
    let report = s.run(d(5, 1)).await;
    assert_eq!(report.demands, 0);
    assert!(s.demand_tasks().is_empty());
}

/// AC3: demand delivered with proof on 2027-04-05, still unpaid on 2027-05-05:
/// the task "open a legal case or agree a payment plan"; not a day earlier.
#[renox::test]
async fn bl_prp_14_the_legal_case_task_comes_thirty_days_after_the_proof() {
    let s = scene().await;
    let lease = s.leased().await;
    s.invoice(d(3, 1)).await;
    s.run(d(3, 31)).await;
    let demand = s.demands(&lease).await.remove(0);
    s.send(&lease, &demand, d(4, 3)).await;
    s.t.notices.set_notice(
        &demand.subject(),
        PAYMENT_DEMAND,
        NoticeStatus::ProofRecorded {
            delivered_on: d(4, 5),
        },
    );

    let early = s.run(d(5, 4)).await;
    assert_eq!(early.legal_tasks, 0);
    assert!(s.legal_tasks().is_empty());

    let on_time = s.run(d(5, 5)).await;
    assert_eq!(on_time.legal_tasks, 1);
    let tasks = s.legal_tasks();
    assert_eq!(tasks.len(), 1);
    assert!(
        tasks[0]
            .title
            .contains("Open a legal case or agree a payment plan"),
        "{}",
        tasks[0].title
    );
    assert_eq!(tasks[0].key.as_deref(), Some(legal_key(demand.id).as_str()));
    let stored = ArrearsDemand::find(s.t.db(), demand.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.delivered_on, Some(d(4, 5)));
    assert_eq!(stored.legal_task_on, Some(d(5, 5)));

    // Once.
    assert_eq!(s.run(d(5, 6)).await.legal_tasks, 0);
    assert_eq!(s.legal_tasks().len(), 1);
}

/// AC4: the demand message gave up and no proof is recorded: no second task.
#[renox::test]
async fn bl_prp_14_without_a_proof_of_delivery_there_is_no_second_task() {
    let s = scene().await;
    let lease = s.leased().await;
    s.invoice(d(3, 1)).await;
    s.run(d(3, 31)).await;
    let demand = s.demands(&lease).await.remove(0);
    s.send(&lease, &demand, d(4, 3)).await;

    for on in [d(5, 5), d(6, 1), d(9, 1)] {
        assert_eq!(s.run(on).await.legal_tasks, 0);
    }
    assert!(s.legal_tasks().is_empty());
}

/// AC4, with the notice given up: the demand message gave up (the contract
/// reports `NotDelivered`, which is how a notice given up without a proof
/// reads) and staff record no proof: no second task for months. When staff
/// later record a proof of delivery dated 2027-06-10, the task comes 30 days
/// after it, on 2027-07-10, not before.
#[renox::test]
async fn bl_prp_14_a_demand_that_gave_up_opens_no_second_task_until_a_proof_is_recorded() {
    let s = scene().await;
    let lease = s.leased().await;
    s.invoice(d(3, 1)).await;
    s.run(d(3, 31)).await;
    let demand = s.demands(&lease).await.remove(0);
    s.send(&lease, &demand, d(4, 3)).await;
    s.t.notices.set_notice(
        &demand.subject(),
        PAYMENT_DEMAND,
        NoticeStatus::NotDelivered,
    );

    for on in [d(5, 5), d(6, 1), d(9, 1), d(12, 1)] {
        assert_eq!(s.run(on).await.legal_tasks, 0, "{on}");
    }
    assert!(s.legal_tasks().is_empty());
    assert!(s.t.tasks.with_key("property.arrears_legal:").is_empty());

    // Staff record a proof of delivery by other means.
    s.t.notices.set_notice(
        &demand.subject(),
        PAYMENT_DEMAND,
        NoticeStatus::ProofRecorded {
            delivered_on: d(6, 10),
        },
    );
    assert_eq!(s.run(d(7, 9)).await.legal_tasks, 0);
    assert_eq!(s.run(d(7, 10)).await.legal_tasks, 1);
    assert_eq!(s.legal_tasks().len(), 1);
}

/// AC5: the demand for the March invoice sent on 2027-04-03 (task closed); the
/// April invoice (due 04-01) reaching 30 days on 05-01 opens a new demand
/// listing only that invoice; the first demand's legal-case task still comes
/// on day 30 after its own proof.
#[renox::test]
async fn bl_prp_14_an_invoice_after_a_sent_demand_opens_a_new_demand() {
    let s = scene().await;
    let lease = s.leased().await;
    s.invoice(d(3, 1)).await;
    s.run(d(3, 31)).await;
    let first = s.demands(&lease).await.remove(0);
    s.send(&lease, &first, d(4, 3)).await;
    s.t.notices.set_notice(
        &first.subject(),
        PAYMENT_DEMAND,
        NoticeStatus::ProofRecorded {
            delivered_on: d(4, 5),
        },
    );
    s.invoice(d(4, 1)).await;

    let report = s.run(d(5, 1)).await;
    assert_eq!(report.demands, 1);
    let demands = s.demands(&lease).await;
    assert_eq!(demands.len(), 2);
    let second = &demands[1];
    assert!(second.is_open());
    let listed = second.invoices(s.t.db()).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].due_date, d(4, 1));
    let tasks = s.demand_tasks();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].key.as_deref(),
        Some(demand_key(second.id).as_str())
    );
    // The first demand lists only the March invoice.
    assert_eq!(first.invoices(s.t.db()).await.unwrap().len(), 1);

    let later = s.run(d(5, 5)).await;
    assert_eq!(later.legal_tasks, 1);
    assert_eq!(
        s.legal_tasks()[0].key.as_deref(),
        Some(legal_key(first.id).as_str())
    );
}

/// AC6: an invoice disputed on 2027-03-20 gets no task on 2027-03-31; it is
/// taken off a demand that was waiting, and the task closes when nothing is
/// left.
#[renox::test]
async fn bl_prp_14_a_disputed_invoice_is_not_escalated() {
    let s = scene().await;
    let lease = s.leased().await;
    s.invoice(d(3, 1)).await;
    let invoice_id = s.t.invoices.all()[0].summary.id.0;
    s.t.invoices.dispute(invoice_id, true);
    assert_eq!(s.run(d(3, 31)).await.demands, 0);
    assert!(s.demand_tasks().is_empty());

    // Disputed after the task was made: removed, and the task closes.
    s.t.invoices.dispute(invoice_id, false);
    s.run(d(3, 31)).await;
    assert_eq!(s.demand_tasks().len(), 1);
    s.t.invoices.dispute(invoice_id, true);
    let report = s.run(d(4, 2)).await;
    assert_eq!((report.removed, report.closed), (1, 1));
    assert!(s.demand_tasks().is_empty());
    let demand = s.demands(&lease).await.remove(0);
    assert_eq!(demand.closed_on, Some(d(4, 2)));
}

/// An invoice that is paid or written off meanwhile is removed from the
/// demand; the task closes when nothing is left.
#[renox::test]
async fn bl_prp_14_a_paid_or_written_off_invoice_leaves_the_demand() {
    let s = scene().await;
    let lease = s.leased().await;
    s.invoice(d(3, 1)).await;
    s.invoice(d(4, 1)).await;
    s.run(d(5, 1)).await;
    let demand = s.demands(&lease).await.remove(0);
    assert_eq!(demand.invoices(s.t.db()).await.unwrap().len(), 2);

    let ids: Vec<i64> = s.t.invoices.all().iter().map(|i| i.summary.id.0).collect();
    s.t.invoices.pay(
        ids[0],
        &Subject::new("payments.payment", 1),
        s.t.invoices.get(ids[0]).unwrap().summary.gross_total,
    );
    let report = s.run(d(5, 2)).await;
    assert_eq!(report.removed, 1);
    assert_eq!(demand.invoices(s.t.db()).await.unwrap().len(), 1);
    assert_eq!(s.demand_tasks().len(), 1);

    s.t.invoices.set_status(ids[1], InvoiceStatus::WrittenOff);
    let report = s.run(d(5, 3)).await;
    assert_eq!((report.removed, report.closed), (1, 1));
    assert!(s.demand_tasks().is_empty());
}

/// AC7: with a legal case flag on the lease, no task.
#[renox::test]
async fn bl_prp_14_a_legal_case_stops_the_rule() {
    let s = scene().await;
    let lease = s.leased().await;
    s.invoice(d(3, 1)).await;
    s.t.post(
        &s.manager,
        &format!("/property/leases/{}/legal-cases", lease.id),
        &[
            ("case_type", "arrears"),
            ("reference", "RDC-1"),
            ("ticket", "new"),
        ],
    )
    .await
    .assert_ok();

    assert_eq!(s.run(d(3, 31)).await.demands, 0);
    assert!(s.demand_tasks().is_empty());
    assert!(s.demands(&lease).await.is_empty());
}

/// An agreed payment plan is a note with the agreed dates: the legal-case task
/// closes, the rule is quiet until the latest agreed date and then looks at the
/// unpaid invoices again; the invoices are not changed.
#[renox::test]
async fn bl_prp_14_a_payment_plan_quiets_the_rule_until_its_last_date() {
    let s = scene().await;
    let lease = s.leased().await;
    s.invoice(d(3, 1)).await;
    s.run(d(3, 31)).await;
    let demand = s.demands(&lease).await.remove(0);
    s.send(&lease, &demand, d(4, 3)).await;
    s.t.notices.set_notice(
        &demand.subject(),
        PAYMENT_DEMAND,
        NoticeStatus::ProofRecorded {
            delivered_on: d(4, 5),
        },
    );
    s.run(d(5, 5)).await;
    assert_eq!(s.legal_tasks().len(), 1);

    let plan = arrears::record_plan_as(
        s.t.state(),
        Some(s.manager.id),
        lease.id,
        NewPlan {
            note: "AED 12,000 on 10 June and on 20 June".into(),
            latest_on: d(6, 20),
        },
        d(5, 6),
    )
    .await
    .unwrap();
    assert_eq!(plan.latest_on, d(6, 20));
    assert!(s.legal_tasks().is_empty());
    let stored = ArrearsDemand::find(s.t.db(), demand.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.legal_task_on, None);

    // Quiet until the latest agreed date.
    assert_eq!(s.run(d(6, 1)).await.legal_tasks, 0);
    assert_eq!(s.run(d(6, 20)).await.legal_tasks, 0);
    assert!(s.legal_tasks().is_empty());
    // Then it runs again, if the invoice is still unpaid.
    assert_eq!(s.run(d(6, 21)).await.legal_tasks, 1);
    assert_eq!(s.legal_tasks().len(), 1);
    // The invoice was not changed.
    let invoice = s.t.invoices.all().remove(0);
    assert_eq!(invoice.summary.status, InvoiceStatus::Issued);
    assert_eq!(invoice.summary.balance, aed(12_000));
}

/// A plan needs its dates and a latest date that is not past.
#[renox::test]
async fn bl_prp_14_a_payment_plan_needs_its_dates() {
    let s = scene().await;
    let lease = s.leased().await;
    let blank = arrears::record_plan_as(
        s.t.state(),
        None,
        lease.id,
        NewPlan {
            note: "  ".into(),
            latest_on: d(6, 20),
        },
        d(5, 6),
    )
    .await;
    assert!(matches!(
        blank,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    ));
    let past = arrears::record_plan_as(
        s.t.state(),
        None,
        lease.id,
        NewPlan {
            note: "10 June".into(),
            latest_on: d(5, 5),
        },
        d(5, 6),
    )
    .await;
    assert!(matches!(
        past,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    ));
}

/// The pages: the property manager sends the demand and records the plan from
/// the lease; the leasing agent cannot.
#[renox::test]
async fn bl_prp_14_the_lease_page_sends_the_demand_and_records_the_plan() {
    let s = scene().await;
    // Invoiced 45 days ago, so the demand is due today.
    let new = NewLease::new(s.unit.id, TENANT, day(-45), day(319), aed(144_000), 12);
    let lease = s.activated(new).await;
    property::invoicing::run(s.t.state(), day(-45))
        .await
        .unwrap();
    for invoice in s.t.invoices.all() {
        s.t.invoices.issue(invoice.summary.id.0);
    }
    arrears::run(s.t.state(), day(0)).await.unwrap();
    let demand = s.demands(&lease).await.remove(0);
    assert!(demand.is_open());

    s.t.app.acting_as(&s.manager);
    s.t.app
        .get(&format!("/property/leases/{}", lease.id))
        .await
        .assert_ok()
        .assert_see("Send the formal payment demand");

    let path = format!("/property/leases/{}/arrears-demand", lease.id);
    let id = demand.id.to_string();
    s.t.post(&s.agent, &path, &[("demand", id.as_str())])
        .await
        .assert_forbidden();
    s.t.post(&s.manager, &path, &[("demand", id.as_str())])
        .await
        .assert_ok();
    let sent = ArrearsDemand::find(s.t.db(), demand.id)
        .await
        .unwrap()
        .unwrap();
    assert!(sent.is_sent());
    assert_eq!(s.t.notices.of_type(PAYMENT_DEMAND).len(), 1);

    let plan_path = format!("/property/leases/{}/payment-plan", lease.id);
    let until = day(20).to_string();
    s.t.post(
        &s.agent,
        &plan_path,
        &[("note", "20 days"), ("latest", until.as_str())],
    )
    .await
    .assert_forbidden();
    s.t.post(
        &s.manager,
        &plan_path,
        &[("note", "20 days"), ("latest", until.as_str())],
    )
    .await
    .assert_ok();
    s.t.app.acting_as(&s.manager);
    s.t.app
        .get(&format!("/property/leases/{}", lease.id))
        .await
        .assert_ok()
        .assert_see("Payment plan until");
}
