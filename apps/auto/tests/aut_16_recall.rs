//! BL-AUT-16 (#412): a recalled part traced to vehicle owners, fleets and
//! customers (08 §7.1 settings *Recall*, `TracePartyResolver`). Parts are not
//! tracked by lot, so wl-inventory traces the issues and sales of the part
//! from the first receipt in the supplier's date range, each line *possible*;
//! the vertical says who is behind each record. The trace itself is
//! wl-inventory's own (its tests and `host/tests/auto_recall_real.rs` run it
//! with the real modules); here a stand-in gives the lines and the resolver,
//! the list, the notice and the page are the app's own.

mod support;

use auto::recalls::{self, RECALL_NOTICE, TraceParties};
use auto::vehicles::{self, NewVehicle};
use auto::{Emirate, FleetContract, FleetStatus, JobCard, JobCardStatus, Vehicle};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use support::fleetfx::{CONTACT, FLEET, active_contract, draft_contract, fatima};
use support::{Garage, Messages, boot};
use wl_kernel::inventory::{TraceMark, TraceParty, TracePartyResolver};
use wl_kernel::notify::{
    MessageGaveUp, MessageId, NotifyChannel, NotifyRecipient, NotifyRequest,
};
use wl_kernel::sales::SaleCustomer;
use wl_kernel::{PartyId, Subject};

/// The day the part was fitted or sold.
fn sold_on() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, 3).unwrap()
}

/// The Owner / GM: opens recalls and tells the customers.
async fn owner(t: &Garage) -> AuthUser {
    let user = t
        .user("Omar", &[auto::permissions::RECALLS_MANAGE])
        .await;
    t.auth(&user).await
}

/// The records of the scenario.
struct World {
    /// A job card of a vehicle of owner O1 (party 7).
    hilux: JobCard,
    /// A job card of a vehicle under a fleet contract in force (fleet F1).
    van: JobCard,
    /// A job card of a vehicle whose fleet contract has ended (owner party 8).
    patrol: JobCard,
    /// A sales order to workshop W1 (party 30).
    order: Subject,
    /// Four counter sales that name nobody.
    counter: Vec<Subject>,
}

/// JC for the Hilux of Mona (party 7), JC for a van of the fleet (party 20,
/// contact 21), JC for a Patrol of Rashed (party 8) whose fleet contract has
/// ended, SO-15 to the workshop W1 (party 30) and four walk-in counter sales.
async fn world(t: &Garage) -> World {
    t.people.add(8, "Rashed Salem");
    t.people.add(30, "Quick Fit Workshop");
    let hilux = t.hilux().await;
    let hilux = t.card_in(&hilux, JobCardStatus::InProgress).await;
    let (_contract, van, _plan) = active_contract(t, 60_000).await;
    let van = t.card_in(&van, JobCardStatus::InProgress).await;

    let fatima = fatima(t).await;
    let old = draft_contract(t, -400, -30).await;
    let patrol = vehicles::create(
        t.state(),
        &fatima,
        NewVehicle::new(Emirate::Dubai, "C", "777", "Nissan", "Patrol", 8).odometer(80_000),
    )
    .await
    .unwrap();
    auto::fleet_contracts::add_vehicle(t.state(), &fatima, old.id, patrol.id)
        .await
        .unwrap();
    FleetContract::query()
        .where_(FleetContract::ID.eq(old.id))
        .update(t.db(), &[("status", &FleetStatus::Ended)])
        .await
        .unwrap();
    let patrol = Vehicle::find_or_404(t.db(), patrol.id).await.unwrap();
    assert_eq!(patrol.fleet_contract_id, Some(old.id));
    let patrol = t.card_in(&patrol, JobCardStatus::InProgress).await;

    let order = Subject::new("sales.sales_order", 15);
    t.customers
        .set(&order, SaleCustomer::Party(PartyId(30)));
    let counter: Vec<Subject> = (21..=24)
        .map(|n| Subject::new("sales.counter_sale", n))
        .collect();
    for sale in &counter {
        t.customers.set(sale, SaleCustomer::WalkIn);
    }
    World {
        hilux,
        van,
        patrol,
        order,
        counter,
    }
}

/// Puts the records into the recall's trace the way wl-inventory does: the
/// party of each subject comes from the app's resolver, walk-in sales are
/// counted per branch and day.
async fn traced(t: &Garage, subjects: &[Subject]) {
    let mut walk_in = 0;
    for subject in subjects {
        match TraceParties.party(t.state(), subject).await.unwrap() {
            TraceParty::Party(party) => t.trace.line(subject, Some(party.0), sold_on()),
            TraceParty::Unknown => t.trace.line(subject, None, sold_on()),
            TraceParty::WalkIn => walk_in += 1,
        }
    }
    if walk_in > 0 {
        t.trace
            .walk_in(&Subject::new("inventory.location", 1), walk_in, sold_on());
    }
}

/// Done when 1: the part fitted on a job card of owner O1, on one of a fleet
/// vehicle, sold on a sales order to workshop W1 and in four walk-in sales at
/// Branch 1 on 2026-09-03: lines for O1, F1 and W1, all *possible*, and one
/// line "4 sales, Branch 1, 2026-09-03".
#[renox::test]
async fn aut_16_the_trace_has_a_line_for_each_owner_fleet_and_customer_and_a_count_of_walk_in_sales()
{
    let t = boot().await;
    let w = world(&t).await;
    wl_kernel::documents::register_subject_label("inventory.location", |_db, id| {
        Box::pin(async move { Ok((id == 1).then(|| "Branch 1".to_owned())) })
    });
    let mut subjects = vec![w.hilux.subject(), w.van.subject(), w.order.clone()];
    subjects.extend(w.counter.clone());
    traced(&t, &subjects).await;

    let rows = recalls::trace(t.state(), 9).await.unwrap();

    let parties: Vec<Option<i64>> = rows.iter().filter(|r| !r.walk_in).map(|r| r.party).collect();
    assert_eq!(parties, vec![Some(7), Some(20), Some(30)]);
    assert!(rows.iter().all(|row| row.mark == TraceMark::Possible));
    assert!(rows.iter().all(|row| row.notice == "pending"));
    let counted: Vec<_> = rows.iter().filter(|r| r.walk_in).collect();
    assert_eq!(counted.len(), 1);
    assert_eq!(counted[0].sales, 4);
    assert_eq!(counted[0].branch, "Branch 1");
    assert_eq!(counted[0].on, Some(sold_on()));
    assert_eq!(counted[0].party, None);
    // The job cards name their vehicle, the order its number.
    let lines: Vec<_> = rows.iter().filter(|r| !r.walk_in).collect();
    assert_eq!(lines[0].record, w.hilux.number());
    assert_eq!(lines[0].vehicle.as_deref(), Some("Dubai A 12345"));
    assert_eq!(lines[1].vehicle.as_deref(), Some("Dubai B 555"));
    assert_eq!(lines[2].vehicle, None);
    // The fleet's contact is told about the fleet's vehicle.
    assert_eq!(lines[1].addressee, Some(CONTACT));
    assert_eq!(lines[0].addressee, Some(7));
    assert_eq!(lines[2].addressee, Some(30));
}

/// Done when 3: a job card for a vehicle whose fleet contract has `ended`
/// names the vehicle owner; one under a contract in force names the fleet.
#[renox::test]
async fn aut_16_a_vehicle_of_an_ended_fleet_contract_names_its_owner() {
    let t = boot().await;
    let w = world(&t).await;

    assert_eq!(
        TraceParties
            .party(t.state(), &w.patrol.subject())
            .await
            .unwrap(),
        TraceParty::Party(PartyId(8))
    );
    assert_eq!(
        TraceParties
            .party(t.state(), &w.van.subject())
            .await
            .unwrap(),
        TraceParty::Party(PartyId(FLEET))
    );
    assert_eq!(
        TraceParties
            .party(t.state(), &w.hilux.subject())
            .await
            .unwrap(),
        TraceParty::Party(PartyId(7))
    );
    // A sale is its customer, a counter sale with no customer is walk-in,
    // anything else is unknown.
    assert_eq!(
        TraceParties.party(t.state(), &w.order).await.unwrap(),
        TraceParty::Party(PartyId(30))
    );
    assert_eq!(
        TraceParties
            .party(t.state(), &w.counter[0])
            .await
            .unwrap(),
        TraceParty::WalkIn
    );
    assert_eq!(
        TraceParties
            .party(t.state(), &Subject::new("auto.job_card", 9_999))
            .await
            .unwrap(),
        TraceParty::Unknown
    );
    assert_eq!(
        TraceParties
            .party(t.state(), &Subject::new("clinics.visit", 3))
            .await
            .unwrap(),
        TraceParty::Unknown
    );
}

/// The users and parties a notice went to.
fn parties_of(request: &NotifyRequest) -> Vec<i64> {
    request
        .recipients
        .iter()
        .filter_map(|r| match r {
            NotifyRecipient::Party { id } => Some(id.0),
            _ => None,
        })
        .collect()
}

/// A notice names only its own party's vehicle or purchase: the owner reads
/// her plate, the fleet contact the plate of the fleet's van, the workshop its
/// order; nobody reads another customer's plate or job.
#[renox::test]
async fn aut_16_a_notice_names_only_the_vehicle_or_purchase_of_its_own_party() {
    let t = boot().await;
    let w = world(&t).await;
    let mut subjects = vec![w.hilux.subject(), w.van.subject(), w.order.clone()];
    subjects.extend(w.counter.clone());
    traced(&t, &subjects).await;
    let omar = owner(&t).await;

    let sent = recalls::send(t.state(), &omar, 9).await.unwrap();

    assert_eq!(sent, 3);
    let notices = t.messages.of_type(RECALL_NOTICE);
    assert_eq!(notices.len(), 3);
    let by_party = |party: i64| {
        notices
            .iter()
            .find(|n| parties_of(n) == vec![party])
            .map(|n| Messages::text(n, NotifyChannel::Email))
            .unwrap_or_else(|| panic!("a notice for party {party}"))
    };
    let owner_text = by_party(7);
    assert!(owner_text.contains("your vehicle Dubai A 12345"), "{owner_text}");
    assert!(!owner_text.contains("B 555"), "{owner_text}");
    let fleet_text = by_party(CONTACT);
    assert!(
        fleet_text.contains("the vehicle Dubai B 555 of your fleet"),
        "{fleet_text}"
    );
    assert!(!fleet_text.contains("A 12345"), "{fleet_text}");
    let workshop_text = by_party(30);
    assert!(workshop_text.contains("your purchase"), "{workshop_text}");
    assert!(!workshop_text.contains("12345"), "{workshop_text}");
    assert!(!workshop_text.contains("555"), "{workshop_text}");
    // The party is told in Arabic as well.
    let def = wl_kernel::notify::notification_type(RECALL_NOTICE).expect("registered");
    assert!(def.template_for(NotifyChannel::Email, "ar").is_some());
    // The lines are sent, the walk-in count is not (it has no party).
    let now = t.trace.notices();
    assert_eq!(now.values().filter(|n| *n == "sent").count(), 3);
    assert_eq!(now.values().filter(|n| *n == "pending").count(), 1);
    // Nobody is left to tell.
    let again = recalls::send(t.state(), &omar, 9).await.unwrap_err();
    assert!(matches!(again, Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _)));
}

/// Only a holder of `recalls.manage` tells the customers.
#[renox::test]
async fn aut_16_only_a_recall_manager_tells_the_customers() {
    let t = boot().await;
    let w = world(&t).await;
    traced(&t, &[w.hilux.subject()]).await;
    let sami = t.advisor().await;

    let refused = recalls::send(t.state(), &sami, 9).await.unwrap_err();

    assert!(matches!(refused, Error::Forbidden));
    assert!(t.messages.of_type(RECALL_NOTICE).is_empty());
    t.app.acting_as(&t.advisor_user().await);
    t.app.get("/auto/recalls/9").await.assert_status(403);
}

/// A notice that gives up returns its line to pending.
#[renox::test]
async fn aut_16_a_notice_that_gives_up_returns_the_line_to_pending() {
    let t = boot().await;
    let w = world(&t).await;
    traced(&t, &[w.hilux.subject()]).await;
    let omar = owner(&t).await;
    recalls::send(t.state(), &omar, 9).await.unwrap();
    assert_eq!(t.trace.notices().values().next().unwrap(), "sent");
    let line = Subject::new("inventory.recall_line", 1);

    t.state()
        .emit(MessageGaveUp {
            message: MessageId(1),
            type_key: RECALL_NOTICE.to_owned(),
            subject: line.clone(),
            subject_label: None,
            recipient: wl_kernel::notify::NotifyRecipient::Party { id: PartyId(7) },
            gave_up_at: renox::db::now(),
            contact_point: None,
            channels_tried: Vec::new(),
            statutory: false,
            reactions: Vec::new(),
            alternative_of: None,
            contact_of: None,
        })
        .await
        .unwrap();

    assert_eq!(t.trace.notices().values().next().unwrap(), "pending");
}

/// The page lists the lines in words: the customer, the plate, the day, the
/// evidence *Possible* and the count of walk-in sales; the owner sees the
/// button that tells the customers.
#[renox::test]
async fn aut_16_the_page_lists_the_lines_and_the_walk_in_count() {
    let t = boot().await;
    let w = world(&t).await;
    let mut subjects = vec![w.hilux.subject(), w.van.subject(), w.order.clone()];
    subjects.extend(w.counter.clone());
    traced(&t, &subjects).await;
    let omar = t
        .user("Omar", &[auto::permissions::RECALLS_MANAGE])
        .await;
    t.app.acting_as(&omar);

    let page = t.app.get("/auto/recalls/9").await;

    page.assert_ok();
    page.assert_see("Mona Hassan");
    page.assert_see("Al Noor Logistics");
    page.assert_see("Quick Fit Workshop");
    page.assert_see("Dubai A 12345");
    page.assert_see("2026-09-03");
    page.assert_see("Possible");
    page.assert_see("Walk-in sales");
    page.assert_see("4 sales");
    page.assert_see("Send the notices");
}
