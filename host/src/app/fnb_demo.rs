//! The demo company of the F&B client (DEMO-DATA.md, 06: *Zaatar & Co.
//! Kitchens*): `fnb:demo` fills a local database with the people of the
//! company, a kitchen with its stock, the brands, outlets and menu, three
//! customers and a few orders in every state a person clicks through, and
//! `db:seed` runs it in `local`, never anywhere else.
//!
//! The orders are sales orders with the kitchen's fulfilment on them. A seed
//! has no signed-in user, and an order in the middle of its life can only be
//! reached through people's actions, so the rows are stored in the state of
//! the story (the way `wl-sales` stores its demo orders): `placed` and waiting
//! for the manager, `preparing` with a ticket cooking, `ready` with its
//! delivery asked for, `completed`, a prepaid order in `delivery_failed` (the
//! customer was not home), another in `delivery_failed` on the outlet's side
//! (the run was aborted), a closed one with no refund that the cashier can
//! ask a refund for, and a customer whose cash on delivery is off after two
//! failures. Seeding twice changes nothing: an order whose external number is
//! used is not made again, a person who exists is kept. The people log in
//! with the password of [`super::seed::PASSWORD`].
//!
//! The demo also calls the helper seeds the modules left for 06: the online,
//! changed and cancelled orders of `wl-sales`, and the till sessions of
//! `wl-payments` (one short, one balanced). The rows they make for the first
//! customer get their kitchen record here.

use fnb::order::{cancel_reason, decision, party, reason, side};
use fnb::ticket::{KitchenTicket, TicketStatus};
use fnb::{CustomerFlag, FnbOrder, FnbOrderStatus, Outlet};
use renox::Environment;
use renox::chrono::TimeDelta;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_delivery::{Attempt, AttemptOutcome, Deliveries, Delivery, DeliveryStatus, Proof};
use wl_inventory::item::value_of;
use wl_inventory::location::NegativeStockPolicy;
use wl_inventory::{Item, Location, StockMovement};
use wl_kernel::delivery::{DeliveryItem, DeliveryRequest};
use wl_kernel::documents::today;
use wl_kernel::sales::SalesOrderId;
use wl_kernel::{PartyId, PartyKind, Subject};
use wl_portal::{AccessGrant, GrantStatus};
use wl_sales::sale_line::{NewLine, SaleLine, totals};
use wl_sales::sales_order::{
    ORDER_SERIES, OrderPaymentStatus, PAYMENT_ONLINE, SalesOrder, SalesOrderStatus,
};
use wl_sales::sales_orders::external_key;

use super::roles::{
    self, ACCOUNTANT, AREA_MANAGER, CASHIER, DRIVER, HEAD_CHEF, KITCHEN_MANAGER, LINE_COOK, MEMBER,
    MENU_MANAGER, OPERATIONS_DIRECTOR, STOREKEEPER,
};
use super::seed::PASSWORD;

/// The command that fills a local database with the demo company.
pub const COMMAND: &str = "fnb:demo";

/// One demo login.
#[derive(Debug, Clone, Copy)]
pub struct Login {
    /// The name shown on the login page.
    pub name: &'static str,
    /// The sign-in e-mail.
    pub email: &'static str,
    /// The role.
    pub role: &'static str,
    /// What it is for, shown beside the name.
    pub label: &'static str,
}

/// The demo people, one per role of 06 §3.1 the demo uses.
pub const LOGINS: &[Login] = &[
    Login {
        name: "Fadi Al Zaabi",
        email: "director@zaatar.example",
        role: OPERATIONS_DIRECTOR,
        label: "Operations director",
    },
    Login {
        name: "Huda Rashid",
        email: "area@zaatar.example",
        role: AREA_MANAGER,
        label: "Area manager",
    },
    Login {
        name: "Mounir Haddad",
        email: "kitchen@zaatar.example",
        role: KITCHEN_MANAGER,
        label: "Kitchen manager",
    },
    Login {
        name: "Chef Samir Nasr",
        email: "chef@zaatar.example",
        role: HEAD_CHEF,
        label: "Head chef (Marina Kitchen and the central kitchen)",
    },
    Login {
        name: "Chef Hana Saleh",
        email: "chef.sharjah@zaatar.example",
        role: HEAD_CHEF,
        label: "Head chef (Sharjah Kitchen)",
    },
    Login {
        name: "Rizwan Ahmed",
        email: "cook@zaatar.example",
        role: LINE_COOK,
        label: "Line cook (KDS)",
    },
    Login {
        name: "Carla Haddad",
        email: "cashier@zaatar.example",
        role: CASHIER,
        label: "Cashier",
    },
    Login {
        name: "Bilal Qureshi",
        email: "store@zaatar.example",
        role: STOREKEEPER,
        label: "Storekeeper",
    },
    Login {
        name: "Dalia Farouk",
        email: "menu@zaatar.example",
        role: MENU_MANAGER,
        label: "Menu manager",
    },
    Login {
        name: "Nour Aziz",
        email: "accounts@zaatar.example",
        role: ACCOUNTANT,
        label: "Accountant",
    },
    Login {
        name: "Imran Siddiqui",
        email: "rider@zaatar.example",
        role: DRIVER,
        label: "Rider (own fleet)",
    },
    Login {
        name: "Layla Mansoori",
        email: CUSTOMER_EMAIL,
        role: MEMBER,
        label: "Customer (follows and cancels her own order)",
    },
];

/// The sign-in of the first demo customer, who acts for herself in the
/// portal: `/my/orders/{id}` is hers.
const CUSTOMER_EMAIL: &str = "layla@customer.example";

/// The customers of the demo: name, and whether they get the failed cash
/// orders of the cash-on-delivery story.
const CUSTOMERS: [&str; 3] = ["Layla Mansoori", "Omar Haddad", "Reem Farouk"];

/// The stock the kitchen starts with: code, name, kilos, cost of a kilo (fils).
const STOCK: [(&str, &str, i64, i64); 4] = [
    ("CHICKEN", "Chicken", 40_000, 1_400),
    ("RICE", "Rice", 60_000, 600),
    ("BREAD", "Flatbread", 30_000, 800),
    ("HUMMUS", "Hummus", 20_000, 900),
];

/// The module that adds the command and the seeder.
pub struct FnbDemo;

impl Module for FnbDemo {
    fn name(&self) -> &'static str {
        "fnb_demo"
    }

    fn register(&self, app: &mut Registry) {
        app.command(
            COMMAND,
            "Fill the database with the Zaatar & Co. demo company (people, kitchen stock, menu, customers, orders in every state); local and testing only",
            |_args, state: AppState| async move { run(state, true).await },
        );
        app.seeder(|state: AppState| async move { run(state, false).await });
    }
}

async fn run(state: AppState, forced: bool) -> Result {
    match state.config.env {
        Environment::Production => {
            return Err(
                renox::anyhow::anyhow!("the demo company is never made in production").into(),
            );
        }
        Environment::Local => {}
        _ if forced => {}
        _ => return Ok(()),
    }
    let db = &state.db;
    roles::define(db).await?;
    let mut cashier = None;
    // A head chef decides the hold lines of the kitchen they cook in, not of
    // the other (`STK.ACL.05`, BL-FNB-17): the role is assigned in the scope of
    // the outlet once the outlets exist (below).
    let mut chefs = Vec::new();
    for login in LOGINS {
        let user = match User::find_by_email(db, login.email).await? {
            Some(user) => user,
            None => {
                let mut user = User::register(db, login.name, login.email, PASSWORD).await?;
                user.email_verified_at = Some(renox::db::now());
                user.save(db).await?;
                if login.role == HEAD_CHEF {
                    chefs.push(user.clone());
                } else {
                    user.assign_role(db, login.role).await?;
                }
                user
            }
        };
        if login.role == CASHIER {
            cashier = Some(user.id);
        }
    }
    let location = kitchen_stock(&state).await?;
    let ingredients: Vec<(&str, i64)> = {
        let mut found = Vec::new();
        for (key, code) in [
            ("chicken", "CHICKEN"),
            ("rice", "RICE"),
            ("bread", "BREAD"),
            ("hummus", "HUMMUS"),
        ] {
            if let Some(item) = Item::query().where_(Item::CODE.eq(code)).first(db).await? {
                found.push((key, item.id));
            }
        }
        found
    };
    let installed = fnb::seed::demo_catalogue(db, Some(location.id), &ingredients).await?;
    println!(
        "{} brands, {} outlets and {} menu items added to the Zaatar & Co. demo.",
        installed.brands, installed.outlets, installed.items
    );
    // Each kitchen has its own stock location, scoped to its outlet, with the
    // negative stock policy of 06 (`STK.RUL.06`), and each head chef decides
    // the hold lines of their own outlet only.
    sharjah_stock(db).await?;
    central_stock(&state).await?;
    let located = super::fnb_compliance::outlet_stock(db).await?;
    println!("{located} outlet stock locations scoped.");
    scope_chefs(db, &chefs).await?;
    let mut customers = Vec::new();
    for name in CUSTOMERS {
        customers.push(customer(&state, name).await?);
    }
    customer_access(&state, customers[0]).await?;
    // The helper seeds of the modules, for the first customer.
    let first = customers[0].0;
    wl_sales::seed::demo_online_orders(db, first).await?;
    wl_sales::seed::demo_order_changes(db, first).await?;
    wl_sales::seed::demo_order_cancellations(db, first).await?;
    adopt_confirmed_orders(&state).await?;
    if let Some(cashier) = cashier {
        wl_payments::cash_till::demo(&state, cashier, today()).await?;
        // A real till session, closed AED 35.00 short: its closing made the
        // cash handover of the shortage (BL-PAY-19).
        wl_sales::seed::demo_till_sessions(&state, cashier, today()).await?;
    }
    // The channels of the two kitchens (BL-FNB-14).
    let channels = fnb::seed::demo_channels(db).await?;
    println!("{channels} outlet channels added.");
    let made = orders(&state, &customers).await?;
    println!("{made} demo orders made for the Zaatar & Co. kitchen.");
    let refunds = complaint_refund(&state).await?;
    let statements = payout_statements(&state).await?;
    println!("{refunds} refund after completion and {statements} payout statements made.");
    // The central kitchen's batches: one produced from a good lot, one on hold
    // because its lot is held, one planned (BL-FNB-11).
    let batches = production(&state).await?;
    println!("{batches} production batches made for the central kitchen.");
    // The fridge that read 9 degrees: its chicken and cream are on hold
    // (BL-FNB-12).
    let units = cold_units(&state).await?;
    println!("{units} cold units made for the Marina kitchen.");
    // The recalled tomato lot traced through the sauce batch to the orders and
    // customers it may have reached (BL-FNB-13).
    let recalls = recall_story(&state).await?;
    println!("{recalls} recall opened and traced to orders and customers.");
    // Marina Kitchen in an outage with its channels paused, a paper order, and
    // the closings of earlier days: a submitted one with a till shortage and a
    // late entry the area manager approved, an approved one that refused a
    // late entry (BL-FNB-14, BL-FNB-15, BL-PAY-19).
    let area = User::find_by_email(db, "area@zaatar.example").await?;
    let days = outage_and_closings(&state, cashier, area.as_ref()).await?;
    println!("{days} outage and closing stories made.");
    // The rider who does not arrive (`FNB.RUL.16`): the kitchen manager is
    // told for the order ready for 20 minutes and has the task for the one
    // ready for 50.
    let watched = fnb::pickup::run(&state, renox::db::now()).await?;
    println!(
        "{} aggregator orders to chase, {} tasks to record what happened.",
        watched.alerts, watched.tasks
    );
    Ok(())
}

/// Sharjah Kitchen's stock location, with the outlet of the same name pointing
/// at it.
async fn sharjah_stock(db: &Db) -> Result {
    let location = match Location::query()
        .where_(Location::CODE.eq("SHARJAH"))
        .first(db)
        .await?
    {
        Some(location) => location,
        None => {
            let mut location = Location::new("SHARJAH", "Sharjah Kitchen");
            location.save(db).await?;
            location
        }
    };
    if let Some(mut outlet) = Outlet::by_code(db, "sharjah").await?
        && outlet.location_id.is_none()
    {
        outlet.location_id = Some(location.id);
        outlet.save(db).await?;
    }
    Ok(())
}

/// Assigns the head chefs made now their role in the scope of their outlet:
/// `chef@` cooks at Marina and is the central kitchen's chef too (the batches
/// are made there), `chef.sharjah@` at Sharjah.
async fn scope_chefs(db: &Db, chefs: &[User]) -> Result {
    for chef in chefs {
        let code = if chef.email == "chef.sharjah@zaatar.example" {
            "sharjah"
        } else {
            "marina"
        };
        let mut codes = vec![code];
        if code == "marina" {
            codes.push("central");
        }
        for code in codes {
            if let Some(outlet) = Outlet::by_code(db, code).await? {
                chef.assign_role_in(db, HEAD_CHEF, &outlet.scope()).await?;
            }
        }
    }
    Ok(())
}

/// The first customer's portal access to herself: with it she follows her
/// orders and cancels a placed one (`cod::customer_of`).
async fn customer_access(state: &AppState, party: PartyId) -> Result {
    let db = &state.db;
    let Some(user) = User::find_by_email(db, CUSTOMER_EMAIL).await? else {
        return Ok(());
    };
    let granted = AccessGrant::query()
        .where_(AccessGrant::USER_ID.eq(Some(user.id)))
        .first(db)
        .await?;
    if granted.is_some() {
        return Ok(());
    }
    let mut grant = AccessGrant {
        user_id: Some(user.id),
        person_party_id: party.0,
        party_id: party.0,
        audience: "tenant".to_owned(),
        roles: "owner".to_owned(),
        source: "self_registration".to_owned(),
        valid_from: today() - TimeDelta::days(1),
        status: GrantStatus::Active,
        ..AccessGrant::default()
    };
    let mut tx = db.begin().await?;
    grant
        .insert_in_state(&mut tx, GrantStatus::Active, "demo seed")
        .await?;
    tx.commit().await?;
    Ok(())
}

/// The kitchen's store with its stock, where the 06 outlet takes its
/// ingredients from. A kitchen consumes what it cooks before the receipts are
/// keyed in, so it allows consumption below zero (`STK.RUL.06`, #238).
async fn kitchen_stock(state: &AppState) -> Result<Location> {
    let db = &state.db;
    let location = match Location::query()
        .where_(Location::CODE.eq("MARINA"))
        .first(db)
        .await?
    {
        Some(location) => location,
        None => {
            let mut location = Location::new("MARINA", "Marina Kitchen");
            location.negative_stock_policy = NegativeStockPolicy::AllowConsumption;
            location.save(db).await?;
            location
        }
    };
    for (code, name, quantity_milli, cost) in STOCK {
        let item = match Item::query().where_(Item::CODE.eq(code)).first(db).await? {
            Some(item) => item,
            None => {
                let mut item = Item::new(code, name, "kg");
                item.average_cost = cost;
                item.save(db).await?;
                item
            }
        };
        let stocked = StockMovement::query()
            .where_(StockMovement::ITEM_ID.eq(item.id))
            .count(db)
            .await?;
        if stocked == 0 {
            let source = Subject::new("inventory.goods_receipt", 9_000);
            let mut moved = StockMovement::receive(
                item.id,
                None,
                location.id,
                quantity_milli,
                cost,
                value_of(quantity_milli, cost),
                &source,
                None,
            );
            moved.save(db).await?;
        }
    }
    Ok(location)
}

/// A customer party named `name`, made when there is none.
async fn customer(state: &AppState, name: &str) -> Result<PartyId> {
    let existing = wl_parties::Party::query()
        .where_(wl_parties::Party::LEGAL_NAME_EN.eq(name))
        .first(&state.db)
        .await?;
    Ok(match existing {
        Some(party) => PartyId(party.id),
        None => {
            let party = wl_parties::create(
                state,
                wl_parties::NewParty {
                    kind: PartyKind::Company,
                    legal_name_en: name.to_owned(),
                    source: "seed".to_owned(),
                    ..wl_parties::NewParty::default()
                },
            )
            .await?;
            PartyId(party.id)
        }
    })
}

/// The kitchen's record for the confirmed sales orders the modules' helper
/// seeds made: accepted, with their tickets.
async fn adopt_confirmed_orders(state: &AppState) -> Result {
    let confirmed = SalesOrder::query()
        .where_(SalesOrder::STATUS.eq(SalesOrderStatus::Confirmed))
        .get(&state.db)
        .await?;
    for sales in confirmed {
        if FnbOrder::of_sales_order(&state.db, sales.id)
            .await?
            .is_some()
        {
            continue;
        }
        if let Some(order) =
            fnb::placing::open(state, SalesOrderId(sales.id), FnbOrderStatus::Accepted).await?
        {
            fnb::kitchen::on_accepted(state, &order).await?;
        }
    }
    Ok(())
}

/// One order of the story.
struct Story {
    /// The external number: the order is made once.
    key: &'static str,
    /// `website` (waits for the outlet) or `phone`.
    channel: &'static str,
    /// Which customer, by position in [`CUSTOMERS`].
    customer: usize,
    /// `online` (paid in advance) or `cash_on_delivery`.
    mode: &'static str,
    /// The menu item and its quantity.
    lines: &'static [(&'static str, i64)],
    /// Where the sales order is.
    sales: SalesOrderStatus,
    /// Where the kitchen's record is.
    status: FnbOrderStatus,
    /// How many hours ago it was placed.
    placed_hours_ago: i64,
    /// For an order that is `ready`: how many minutes ago it became ready
    /// (the clock of `FNB.RUL.16`); 0 for the others.
    ready_minutes_ago: i64,
}

const STORIES: [Story; 18] = [
    Story {
        key: "FNBD-1",
        channel: "website",
        customer: 0,
        mode: "online",
        lines: &[("shawarma-plate", 2)],
        sales: SalesOrderStatus::PendingAcceptance,
        status: FnbOrderStatus::Placed,
        placed_hours_ago: 0,
        ready_minutes_ago: 0,
    },
    Story {
        key: "FNBD-2",
        channel: "phone",
        customer: 1,
        mode: "cash_on_delivery",
        lines: &[("falafel-wrap", 3)],
        sales: SalesOrderStatus::Confirmed,
        status: FnbOrderStatus::Preparing,
        placed_hours_ago: 0,
        ready_minutes_ago: 0,
    },
    Story {
        key: "FNBD-3",
        channel: "phone",
        customer: 0,
        mode: "cash_on_delivery",
        lines: &[("hummus-bowl", 2)],
        sales: SalesOrderStatus::Confirmed,
        status: FnbOrderStatus::Ready,
        placed_hours_ago: 0,
        ready_minutes_ago: 10,
    },
    Story {
        key: "FNBD-4",
        channel: "website",
        customer: 1,
        mode: "online",
        lines: &[("kunafa-tray", 1)],
        sales: SalesOrderStatus::Fulfilled,
        status: FnbOrderStatus::Completed,
        placed_hours_ago: 26,
        ready_minutes_ago: 0,
    },
    Story {
        key: "FNBD-5",
        channel: "website",
        customer: 0,
        mode: "online",
        lines: &[("shawarma-plate", 1)],
        sales: SalesOrderStatus::Confirmed,
        status: FnbOrderStatus::DeliveryFailed,
        placed_hours_ago: 3,
        ready_minutes_ago: 0,
    },
    Story {
        key: "FNBD-6",
        channel: "website",
        customer: 1,
        mode: "online",
        lines: &[("falafel-wrap", 2), ("hummus-bowl", 1)],
        sales: SalesOrderStatus::Confirmed,
        status: FnbOrderStatus::DeliveryFailed,
        placed_hours_ago: 2,
        ready_minutes_ago: 0,
    },
    Story {
        key: "FNBD-7",
        channel: "website",
        customer: 1,
        mode: "online",
        lines: &[("shawarma-plate", 2)],
        sales: SalesOrderStatus::Cancelled,
        status: FnbOrderStatus::Cancelled,
        placed_hours_ago: 50,
        ready_minutes_ago: 0,
    },
    Story {
        key: "FNBD-8",
        channel: "phone",
        customer: 2,
        mode: "cash_on_delivery",
        lines: &[("shawarma-plate", 1)],
        sales: SalesOrderStatus::Cancelled,
        status: FnbOrderStatus::Cancelled,
        placed_hours_ago: 24 * 40,
        ready_minutes_ago: 0,
    },
    Story {
        key: "FNBD-9",
        channel: "phone",
        customer: 2,
        mode: "cash_on_delivery",
        lines: &[("falafel-wrap", 2)],
        sales: SalesOrderStatus::Confirmed,
        status: FnbOrderStatus::DeliveryFailed,
        placed_hours_ago: 1,
        ready_minutes_ago: 0,
    },
    // BL-FNB-04: accepted and not started, above AED 50: the cashier's cancel
    // waits for the kitchen manager.
    Story {
        key: "FNBD-10",
        channel: "website",
        customer: 0,
        mode: "online",
        lines: &[("shawarma-plate", 2), ("hummus-bowl", 1)],
        sales: SalesOrderStatus::Confirmed,
        status: FnbOrderStatus::Accepted,
        placed_hours_ago: 0,
        ready_minutes_ago: 0,
    },
    // BL-FNB-04: cancelled by the customer from the order page while it was
    // placed, refunded in full.
    Story {
        key: "FNBD-11",
        channel: "website",
        customer: 0,
        mode: "online",
        lines: &[("falafel-wrap", 2)],
        sales: SalesOrderStatus::Cancelled,
        status: FnbOrderStatus::Cancelled,
        placed_hours_ago: 5,
        ready_minutes_ago: 0,
    },
    // BL-FNB-04: an aggregator's order cancelled with the aggregator
    // responsible: no refund, compensation expected on the payout.
    Story {
        key: "FNBD-12",
        channel: "aggregator_a",
        customer: 2,
        mode: "on_account",
        lines: &[("shawarma-plate", 1)],
        sales: SalesOrderStatus::Cancelled,
        status: FnbOrderStatus::Cancelled,
        placed_hours_ago: 6,
        ready_minutes_ago: 0,
    },
    // BL-FNB-05: an aggregator's accepted order of two lines: an item ran
    // out, so the cashier records the cancelled line here (no refund, the
    // line is flagged for the payout); Amend is not offered on it.
    Story {
        key: "FNBD-13",
        channel: "aggregator_a",
        customer: 2,
        mode: "on_account",
        lines: &[("falafel-wrap", 2), ("hummus-bowl", 1)],
        sales: SalesOrderStatus::Confirmed,
        status: FnbOrderStatus::Accepted,
        placed_hours_ago: 0,
        ready_minutes_ago: 0,
    },
    // BL-FNB-06: an aggregator's order ready for 20 minutes: the kitchen
    // manager was told to chase the aggregator on its tablet.
    Story {
        key: "FNBD-14",
        channel: "aggregator_a",
        customer: 2,
        mode: "on_account",
        lines: &[("shawarma-plate", 1)],
        sales: SalesOrderStatus::Confirmed,
        status: FnbOrderStatus::Ready,
        placed_hours_ago: 1,
        ready_minutes_ago: 20,
    },
    // BL-FNB-06: ready for 50 minutes: the kitchen manager also has the task
    // to record whether the rider came late or the aggregator cancelled.
    Story {
        key: "FNBD-15",
        channel: "aggregator_a",
        customer: 1,
        mode: "on_account",
        lines: &[("falafel-wrap", 2)],
        sales: SalesOrderStatus::Confirmed,
        status: FnbOrderStatus::Ready,
        placed_hours_ago: 1,
        ready_minutes_ago: 50,
    },
    // BL-FNB-07: a completed cash order whose delivery is delivered with the
    // cash recorded: the customer says it never arrived, and the kitchen
    // manager reverses the delivery (credit note only: nobody handed the cash
    // over).
    Story {
        key: "FNBD-16",
        channel: "phone",
        customer: 2,
        mode: "cash_on_delivery",
        lines: &[("hummus-bowl", 3)],
        sales: SalesOrderStatus::Fulfilled,
        status: FnbOrderStatus::Completed,
        placed_hours_ago: 3,
        ready_minutes_ago: 0,
    },
    // BL-FNB-10: an aggregator's completed order whose payout statement has
    // a refund line of AED 25.00 ("Food was cold"): a sales return refunded by
    // the channel.
    Story {
        key: "FNBD-17",
        channel: "aggregator_a",
        customer: 2,
        mode: "on_account",
        lines: &[("shawarma-plate", 1)],
        sales: SalesOrderStatus::Fulfilled,
        status: FnbOrderStatus::Completed,
        placed_hours_ago: 30,
        ready_minutes_ago: 0,
    },
    // BL-FNB-10: another completed aggregator order on the same statement,
    // with no refund.
    Story {
        key: "FNBD-18",
        channel: "aggregator_a",
        customer: 1,
        mode: "on_account",
        lines: &[("falafel-wrap", 2)],
        sales: SalesOrderStatus::Fulfilled,
        status: FnbOrderStatus::Completed,
        placed_hours_ago: 29,
        ready_minutes_ago: 0,
    },
];

/// Makes the orders of the stories that are not there yet; returns how many.
async fn orders(state: &AppState, customers: &[PartyId]) -> Result<usize> {
    let mut made = 0;
    for story in &STORIES {
        if let Some(key) = external_key(story.key)
            && SalesOrder::of_external(&state.db, story.channel, &key)
                .await?
                .is_some()
        {
            continue;
        }
        let party = customers[story.customer];
        let name = CUSTOMERS[story.customer];
        let sales = sales_order(state, story, party).await?;
        let order = kitchen_record(state, story, &sales, party, name).await?;
        tickets_for(state, story, &order, &sales).await?;
        if story.status == FnbOrderStatus::Ready && order.is_delivery() {
            ask_delivery(state, &order).await?;
        }
        // The completed orders were delivered: their deliveries can be put
        // under review and reversed.
        if story.status == FnbOrderStatus::Completed && order.is_delivery() {
            delivered_delivery(state, &order).await?;
        }
        made += 1;
    }
    // Reem Farouk's cash on delivery is off after two customer-side failures
    // (`FNB.RUL.14`): the kitchen manager can turn it back on.
    let reem = customers[2];
    if CustomerFlag::of(&state.db, reem).await?.is_none() {
        let failed = FnbOrder::query()
            .where_(FnbOrder::PARTY_ID.eq(reem.0))
            .where_(FnbOrder::STATUS.eq(FnbOrderStatus::DeliveryFailed))
            .first(&state.db)
            .await?;
        let mut flag = CustomerFlag {
            party_id: reem.0,
            cod_allowed: false,
            cod_off_at: Some(renox::db::now() - TimeDelta::hours(1)),
            cod_off_order_id: failed.map(|order| order.id),
            ..CustomerFlag::default()
        };
        flag.insert(&state.db).await?;
    }
    Ok(made)
}

/// The sales order of the story, with its lines and totals.
async fn sales_order(state: &AppState, story: &Story, party: PartyId) -> Result<SalesOrder> {
    let db = &state.db;
    let number = ORDER_SERIES
        .next_allowing_gaps(db, None, None, today())
        .await?;
    let prepaid = story.mode == PAYMENT_ONLINE;
    let at = renox::db::now() - TimeDelta::hours(story.placed_hours_ago);
    let mut order = SalesOrder {
        number,
        party_id: party.0,
        channel: story.channel.to_owned(),
        payment_mode: story.mode.to_owned(),
        currency: "AED".to_owned(),
        revision: 1,
        external_number: Some(story.key.to_owned()),
        external_key: external_key(story.key),
        payment_status: if prepaid {
            OrderPaymentStatus::Received
        } else {
            OrderPaymentStatus::NotRequired
        },
        confirmed_at: (story.sales != SalesOrderStatus::PendingAcceptance).then_some(at),
        status: story.sales,
        ..SalesOrder::default()
    };
    let why = match story.key {
        "FNBD-11" => "Cancelled by the customer",
        "FNBD-12" => "Cancelled by the aggregator",
        _ => "failed delivery",
    };
    if story.sales == SalesOrderStatus::Cancelled {
        order.reason = Some(why.to_owned());
    }
    let mut tx = db.begin().await?;
    order.insert_in_state(&mut tx, story.sales, "demo").await?;
    tx.commit().await?;
    let order = SalesOrder::find_or_404(db, order.id).await?;
    for (code, quantity) in story.lines {
        let item = fnb::MenuItem::by_code(db, code).await?;
        let (price, name) = item
            .map(|item| (item.price, item.name))
            .unwrap_or((2_500, (*code).to_owned()));
        let line = SaleLine::add(
            db,
            &order.subject(),
            NewLine::new(code, price)
                .description(&name)
                .quantity(*quantity),
        )
        .await?;
        let quantity_milli = line.quantity_milli;
        match story.sales {
            SalesOrderStatus::Fulfilled => {
                SaleLine::query()
                    .where_(SaleLine::ID.eq(line.id))
                    .update(db, &[("fulfilled_milli", &quantity_milli)])
                    .await?;
            }
            SalesOrderStatus::Cancelled => {
                let outcome = Some("scrap".to_owned());
                let why = Some(why.to_owned());
                SaleLine::query()
                    .where_(SaleLine::ID.eq(line.id))
                    .update(
                        db,
                        &[
                            ("cancelled_milli", &quantity_milli),
                            ("cancel_outcome", &outcome),
                            ("cancel_reason", &why),
                        ],
                    )
                    .await?;
            }
            _ => {}
        }
    }
    let lines = SaleLine::of(db, &order.subject()).await?;
    let t = totals(&lines);
    SalesOrder::query()
        .where_(SalesOrder::ID.eq(order.id))
        .update(
            db,
            &[
                ("net_total", &t.net),
                ("vat_total", &t.vat),
                ("gross_total", &t.gross),
            ],
        )
        .await?;
    SalesOrder::find_or_404(db, order.id).await
}

/// The kitchen's record of the story, in the state of the story.
async fn kitchen_record(
    state: &AppState,
    story: &Story,
    sales: &SalesOrder,
    party: PartyId,
    name: &str,
) -> Result<FnbOrder> {
    let db = &state.db;
    let outlet = Outlet::query()
        .where_(Outlet::ACTIVE.eq(true))
        .order_by(Outlet::ID)
        .first(db)
        .await?;
    let brand = match story.lines.first() {
        Some((code, _)) => fnb::MenuItem::by_code(db, code)
            .await?
            .and_then(|item| item.brand_id),
        None => None,
    };
    let placed = renox::db::now() - TimeDelta::hours(story.placed_hours_ago);
    // The trading day by the outlet's cut-off, and that day's closing.
    let business_date = outlet.as_ref().map(|outlet| outlet.business_date(placed));
    if let (Some(outlet), Some(date)) = (&outlet, business_date) {
        fnb::closings::ensure(db, outlet.id, date).await?;
    }
    let prepaid = story.mode == PAYMENT_ONLINE;
    let mut order = FnbOrder {
        number: sales.number.clone(),
        sales_order_id: sales.id,
        party_id: party.0,
        brand_id: brand,
        outlet_id: outlet.map(|outlet| outlet.id),
        business_date,
        channel: story.channel.to_owned(),
        kind: fnb::placing::kind_for(story.channel).to_owned(),
        payment_mode: story.mode.to_owned(),
        paid_amount: if prepaid { sales.gross_total } else { 0 },
        total: sales.gross_total,
        recipient_name: name.to_owned(),
        recipient_phone: "+971500000000".to_owned(),
        address: "Villa 12, Al Barsha 2".to_owned(),
        area: "Al Barsha".to_owned(),
        placed_at: Some(placed),
        ready_at: (story.ready_minutes_ago > 0)
            .then(|| renox::db::now() - TimeDelta::minutes(story.ready_minutes_ago)),
        accepted_at: (story.status != FnbOrderStatus::Placed).then_some(placed),
        status: story.status,
        ..FnbOrder::default()
    };
    match story.key {
        // The customer was not home (customer side).
        "FNBD-5" => failed(
            &mut order,
            "not_home",
            reason::CUSTOMER_UNREACHABLE,
            side::CUSTOMER,
            party::CUSTOMER,
        ),
        // The run was aborted (outlet side).
        "FNBD-6" => failed(
            &mut order,
            "run_aborted",
            reason::RUN_ABORTED,
            side::OUTLET,
            party::BUSINESS,
        ),
        // Closed after a customer-side failure: nothing refunded, the cashier
        // may ask for a refund within 7 days.
        "FNBD-7" => {
            failed(
                &mut order,
                "not_home",
                reason::CUSTOMER_UNREACHABLE,
                side::CUSTOMER,
                party::CUSTOMER,
            );
            closed(&mut order, decision::NO_REFUND, 0, 48);
        }
        // A cash order failed and closed 40 days ago, and one failed now.
        "FNBD-8" => {
            failed(
                &mut order,
                "no_cash",
                reason::NO_CASH,
                side::CUSTOMER,
                party::CUSTOMER,
            );
            closed(&mut order, decision::NOT_APPLICABLE, 0, 24 * 40 - 1);
        }
        "FNBD-9" => failed(
            &mut order,
            "refused",
            reason::REFUSED,
            side::CUSTOMER,
            party::CUSTOMER,
        ),
        "FNBD-4" | "FNBD-17" | "FNBD-18" => {
            order.completed_at = Some(placed + TimeDelta::minutes(40));
        }
        // Cancelled by the customer from the order page, refunded in full.
        "FNBD-11" => cancelled(
            &mut order,
            cancel_reason::CUSTOMER_REQUEST,
            party::CUSTOMER,
            true,
            5,
        ),
        // The aggregator cancelled it: no refund, the aggregator pays.
        "FNBD-12" => cancelled(
            &mut order,
            cancel_reason::AGGREGATOR_CANCELLED,
            party::AGGREGATOR,
            false,
            6,
        ),
        _ => {}
    }
    let mut tx = db.begin().await?;
    order.insert_in_state(&mut tx, story.status, "demo").await?;
    tx.commit().await?;
    FnbOrder::find_or_404(db, order.id).await
}

/// Marks `order` as failed on a delivery.
fn failed(order: &mut FnbOrder, delivery_reason: &str, why: &str, on: &str, to_blame: &str) {
    order.failure_reason = Some(why.to_owned());
    order.delivery_reason = Some(delivery_reason.to_owned());
    order.failure_side = Some(on.to_owned());
    order.failure_attempt = 1;
    order.failed_at = order
        .placed_at
        .map(|placed| placed + TimeDelta::minutes(55));
    order.responsible_party = Some(to_blame.to_owned());
}

/// Marks `order` as cancelled `hours_ago` hours ago with `why` (BL-FNB-04).
fn cancelled(order: &mut FnbOrder, why: &str, to_blame: &str, by_customer: bool, hours_ago: i64) {
    order.cancel_reason_code = Some(why.to_owned());
    order.cancel_party = Some(to_blame.to_owned());
    order.cancelled_by_customer = by_customer;
    order.cancelled_at = Some(renox::db::now() - TimeDelta::hours(hours_ago));
    order.close_reason = Some(fnb::cancel::reason_text(why, ""));
    order.compensation_expected = to_blame == party::AGGREGATOR;
    if order.paid_amount > 0 {
        order.refund_decision = Some(decision::FULL_REFUND.to_owned());
        order.refund_amount = order.paid_amount;
    } else {
        order.refund_decision = Some(decision::NOT_APPLICABLE.to_owned());
    }
}

/// Marks `order` as closed `hours_ago` hours ago with `chosen`.
fn closed(order: &mut FnbOrder, chosen: &str, amount: i64, hours_ago: i64) {
    order.refund_decision = Some(chosen.to_owned());
    order.refund_amount = amount;
    order.closed_at = Some(renox::db::now() - TimeDelta::hours(hours_ago));
    order.close_reason = Some("failed delivery".to_owned());
    order.wastage_posted = true;
}

/// The tickets of the story, one per item line: none for a placed order,
/// cooking for a preparing one, bumped for one past the kitchen. A ticket in
/// the middle of its life is stored in that state, as the seed's exception.
async fn tickets_for(
    state: &AppState,
    story: &Story,
    order: &FnbOrder,
    sales: &SalesOrder,
) -> Result {
    let wanted = match story.status {
        FnbOrderStatus::Placed => return Ok(()),
        // The customer cancelled before the kitchen had it; the aggregator
        // cancelled after: its tickets were voided.
        FnbOrderStatus::Cancelled if story.key == "FNBD-11" => return Ok(()),
        FnbOrderStatus::Cancelled if story.key == "FNBD-12" => TicketStatus::Voided,
        FnbOrderStatus::Accepted => TicketStatus::Queued,
        FnbOrderStatus::Preparing => TicketStatus::InPrep,
        _ => TicketStatus::Done,
    };
    let at = order.placed_at.unwrap_or_else(renox::db::now);
    for line in SaleLine::of(&state.db, &sales.subject()).await? {
        let mut ticket = KitchenTicket {
            order_id: order.id,
            sale_line_id: line.id,
            station: fnb::ticket::DEFAULT_STATION.to_owned(),
            description: line.description.clone(),
            quantity_milli: line.quantity_milli,
            started_at: matches!(wanted, TicketStatus::InPrep | TicketStatus::Done)
                .then_some(at + TimeDelta::minutes(5)),
            bumped_at: (wanted == TicketStatus::Done).then_some(at + TimeDelta::minutes(25)),
            status: wanted,
            ..KitchenTicket::default()
        };
        let mut tx = state.db.begin().await?;
        ticket.insert_in_state(&mut tx, wanted, "demo").await?;
        tx.commit().await?;
    }
    Ok(())
}

/// The delivered delivery of a completed order: the stop was handed over 20
/// minutes ago with a photo (and the cash of a cash order). A seed has no
/// signed-in rider, so the rows are stored in the state of the story, the way
/// the orders are; the delivery has no run.
async fn delivered_delivery(state: &AppState, order: &FnbOrder) -> Result {
    let db = &state.db;
    let collected = if order.is_cash_on_delivery() {
        order.total
    } else {
        0
    };
    let request = DeliveryRequest::new(order.subject(), &order.recipient_name)
        .address(&order.address, &order.area)
        .phone(&order.recipient_phone)
        .perishable()
        .promise(order.placed_at.unwrap_or_else(renox::db::now), 45)
        .collect(collected)
        .item(DeliveryItem::new(&format!("Order {}", order.number), 1_000));
    let delivery = Deliveries::request(state, request).await?;
    let mut attempt = Attempt {
        delivery_id: delivery.id,
        number: 1,
        outcome: AttemptOutcome::Delivered,
        counts: true,
        occurred_at: renox::db::now() - TimeDelta::minutes(20),
        ..Attempt::default()
    };
    attempt.insert(db).await?;
    let mut proof = Proof {
        delivery_id: delivery.id,
        attempt_id: attempt.id,
        methods: "photo".to_owned(),
        recipient_name: order.recipient_name.clone(),
        photo_ref: Some("demo-photo".to_owned()),
        cash_expected: collected,
        cash_collected: collected,
        ..Proof::default()
    };
    proof.insert(db).await?;
    Delivery::query()
        .where_(Delivery::ID.eq(delivery.id))
        .update(
            db,
            &[("status", &DeliveryStatus::Delivered), ("attempts", &1_i64)],
        )
        .await?;
    FnbOrder::query()
        .where_(FnbOrder::ID.eq(order.id))
        .update(db, &[("delivery_id", &Some(delivery.id))])
        .await?;
    Ok(())
}

/// Asks wl-delivery for the delivery of a `ready` order, as the kitchen does
/// when the last ticket is bumped: a dispatcher puts it on a run.
async fn ask_delivery(state: &AppState, order: &FnbOrder) -> Result {
    let request = DeliveryRequest::new(order.subject(), &order.recipient_name)
        .address(&order.address, &order.area)
        .phone(&order.recipient_phone)
        .perishable()
        .promise(order.placed_at.unwrap_or_else(renox::db::now), 45)
        .collect(if order.is_cash_on_delivery() {
            order.total
        } else {
            0
        })
        .item(DeliveryItem::new(&format!("Order {}", order.number), 1_000));
    let delivery = Deliveries::request(state, request).await?;
    FnbOrder::query()
        .where_(FnbOrder::ID.eq(order.id))
        .update(&state.db, &[("delivery_id", &Some(delivery.id))])
        .await?;
    Ok(())
}

/// BL-FNB-08: the customer of the delivered prepaid order FNBD-4 complained
/// that the food was cold; the rider was at fault. The sales return is raised
/// and resolved as the system, by credit note only: the demo order has no
/// payment record a refund could be paid from. The refunds report shows AED
/// 30.00 under *cold* / *rider* and the rider has a fault entry. Returns how
/// many it made.
async fn complaint_refund(state: &AppState) -> Result<usize> {
    let db = &state.db;
    let Some(sales) = SalesOrder::query()
        .where_(SalesOrder::EXTERNAL_KEY.eq("fnbd-4"))
        .first(db)
        .await?
    else {
        return Ok(0);
    };
    if fnb::RefundFact::query()
        .where_(fnb::RefundFact::SALES_ORDER_ID.eq(sales.id))
        .first(db)
        .await?
        .is_some()
    {
        return Ok(0);
    }
    let Some(order) = FnbOrder::of_sales_order(db, sales.id).await? else {
        return Ok(0);
    };
    // The rider of the order's last run.
    if let Some(rider) = User::find_by_email(db, "rider@zaatar.example").await? {
        FnbOrder::query()
            .where_(FnbOrder::ID.eq(order.id))
            .update(db, &[("rider_user_id", &Some(rider.id))])
            .await?;
    }
    let Some(line) = SaleLine::of(db, &sales.subject()).await?.into_iter().next() else {
        return Ok(0);
    };
    let new = wl_sales::sales_returns::NewReturn::no_goods(sales.subject())
        .reason("The food arrived cold")
        .complaint("cold", "driver_or_rider")
        .refund_line(line.id, 3_000);
    wl_sales::sales_returns::refund_by_vertical(state, new, true).await?;
    Ok(1)
}

/// The statement of the week the demo company imported and matched: one
/// refund line of AED 25.00 for FNBD-17 ("Food was cold", mapped to *cold* and
/// *business*), the aggregator's compensation for the order it cancelled
/// (FNBD-12) and a plain order.
const MATCHED_STATEMENT: &str = "Order ID,Gross,Commission,Promo,Refund,Net,Refund reason\n\
FNBD-17,31.50,6.30,0,25.00,0.20,Food was cold\n\
FNBD-18,37.80,7.56,0,0,30.24,\n\
FNBD-12,31.50,6.30,0,0,25.20,\n";

/// A statement imported and not yet matched: it names FNBD-99, an order the
/// kitchen never recorded (the cashier enters it as a late entry), and has a
/// refund of a reason nobody mapped ("Customer changed mind"): the accountant
/// sets its party before Match is allowed. Importing `AGG-A-2026-W41` again is
/// refused, naming the statement that exists.
const OPEN_STATEMENT: &str = "Order ID,Gross,Commission,Promo,Refund,Net,Refund reason\n\
FNBD-14,31.50,6.30,0,0,25.20,\n\
FNBD-99,42.00,8.40,0,0,33.60,\n\
FNBD-18,37.80,7.56,0,10.00,20.24,Customer changed mind\n";

/// The statements of the demo (BL-FNB-09, BL-FNB-10): `AGG-A-2026-W41`
/// matched with a refund line, `AGG-A-2026-W42` imported with a missing order
/// and an unmapped refund, and a wrong file `AGG-A-2026-W40` that was voided
/// and imported again under the same reference. Returns how many it made.
async fn payout_statements(state: &AppState) -> Result<usize> {
    use fnb::payout::{PayoutLine, PayoutStatement, PayoutStatus, line_state};
    use fnb::payouts::{ImportAsk, import_as, match_statement};
    let db = &state.db;
    // The column presets and the refund reasons the statements are read with.
    fnb::seed::install_payouts(db).await?;
    let accountant = User::find_by_email(db, "accounts@zaatar.example")
        .await?
        .map(|user| user.id);
    let mut made = 0;
    let day = today();
    let ask = |reference: &str, content: &str, back: i64| ImportAsk {
        aggregator: "aggregator_a".to_owned(),
        reference: reference.to_owned(),
        file_name: format!("{reference}.csv"),
        period_from: Some(day - TimeDelta::days(back + 6)),
        period_to: Some(day - TimeDelta::days(back)),
        content: content.to_owned(),
    };
    // The week that was matched: the receivable is booked and the refund
    // line is a sales return, refunded by the channel.
    if PayoutStatement::using(db, "aggregator_a", "AGG-A-2026-W41")
        .await?
        .is_none()
    {
        let statement = import_as(
            state,
            accountant,
            &ask("AGG-A-2026-W41", MATCHED_STATEMENT, 14),
        )
        .await?;
        match_statement(state, None, statement.id).await?;
        made += 1;
    }
    // The week still open.
    if PayoutStatement::using(db, "aggregator_a", "AGG-A-2026-W42")
        .await?
        .is_none()
    {
        import_as(state, accountant, &ask("AGG-A-2026-W42", OPEN_STATEMENT, 7)).await?;
        made += 1;
    }
    // A wrong file, voided before it was settled; its reference was imported
    // again with the right content. The wrong one is stored in its state, as
    // a seed has no signed-in accountant to void it.
    if PayoutStatement::using(db, "aggregator_a", "AGG-A-2026-W40")
        .await?
        .is_none()
    {
        let number = fnb::STATEMENT_SERIES
            .next_allowing_gaps(db, None, None, today())
            .await?;
        let mut wrong = PayoutStatement {
            number,
            aggregator: "aggregator_a".to_owned(),
            reference: "AGG-A-2026-W40".to_owned(),
            reference_key: "agg-a-2026-w40".to_owned(),
            file_name: "AGG-A-2026-W40-wrong.csv".to_owned(),
            preset: "Aggregator A weekly CSV".to_owned(),
            period_from: Some(day - TimeDelta::days(27)),
            period_to: Some(day - TimeDelta::days(21)),
            line_count: 1,
            gross_total: 3_150,
            commission_total: 630,
            net_total: 2_520,
            void_reason: Some("wrong week".to_owned()),
            voided_at: Some(renox::db::now() - TimeDelta::days(20)),
            imported_by: accountant,
            status: PayoutStatus::Voided,
            ..PayoutStatement::default()
        };
        let mut tx = db.begin().await?;
        wrong
            .insert_in_state(&mut tx, PayoutStatus::Voided, "demo seed")
            .await?;
        let mut line = PayoutLine {
            statement_id: wrong.id,
            line_no: 1,
            order_number: "FNBD-17".to_owned(),
            order_key: "fnbd-17".to_owned(),
            gross: 3_150,
            commission: 630,
            net: 2_520,
            match_state: line_state::UNMATCHED.to_owned(),
            ..PayoutLine::default()
        };
        line.insert(&mut tx).await?;
        tx.commit().await?;
        let content = "Order ID,Gross,Commission,Promo,Refund,Net,Refund reason\nFNBD-18,37.80,7.56,0,0,30.24,\n";
        import_as(state, accountant, &ask("AGG-A-2026-W40", content, 21)).await?;
        made += 2;
    }
    Ok(made)
}

/// A lot-tracked kilogram item `code`, made when there is none, at `cost`
/// fils a kilogram.
async fn tracked_item(db: &Db, code: &str, name: &str, cost: i64) -> Result<Item> {
    if let Some(item) = Item::query().where_(Item::CODE.eq(code)).first(db).await? {
        return Ok(item);
    }
    let mut item = Item::new(code, name, "kg").lot_tracked();
    item.average_cost = cost;
    item.save(db).await?;
    Ok(item)
}

/// The central production kitchen's stock location, with the outlet of the
/// same name pointing at it.
async fn central_stock(state: &AppState) -> Result<Location> {
    let db = &state.db;
    let location = match Location::query()
        .where_(Location::CODE.eq("CENTRAL"))
        .first(db)
        .await?
    {
        Some(location) => location,
        None => {
            // The central kitchen blocks a movement below zero (`STK.RUL.06`,
            // BL-FNB-17): its stock is lot-tracked and counted.
            let mut location = Location::new("CENTRAL", "Central Production Kitchen");
            location.negative_stock_policy = NegativeStockPolicy::Block;
            location.save(db).await?;
            location
        }
    };
    if let Some(mut outlet) = Outlet::by_code(db, "central").await?
        && outlet.location_id.is_none()
    {
        outlet.location_id = Some(location.id);
        outlet.save(db).await?;
    }
    Ok(location)
}

/// Lot `number` of `item`, `quantity_milli` of it received at `location`,
/// expiring in `days` days; the lot that exists is kept.
async fn received_lot(
    db: &Db,
    item: &Item,
    number: &str,
    location: &Location,
    quantity_milli: i64,
    days: i64,
) -> Result<wl_inventory::Lot> {
    use wl_inventory::{Lot, LotStatus};
    if let Some(lot) = Lot::query()
        .where_(Lot::ITEM_ID.eq(item.id))
        .where_(Lot::NUMBER.eq(number))
        .first(db)
        .await?
    {
        return Ok(lot);
    }
    let source = Subject::new("inventory.goods_receipt", 9_100);
    let mut lot = Lot {
        item_id: item.id,
        number: number.to_owned(),
        source_type: Some(source.subject_type.clone()),
        source_id: Some(source.subject_id),
        expiry_date: Some(today() + TimeDelta::days(days)),
        status: LotStatus::Released,
        ..Lot::default()
    };
    lot.save(db).await?;
    let mut moved = StockMovement::receive(
        item.id,
        Some(lot.id),
        location.id,
        quantity_milli,
        item.average_cost,
        value_of(quantity_milli, item.average_cost),
        &source,
        None,
    );
    moved.save(db).await?;
    Ok(lot)
}

/// A produced batch of `recipe` made from `used_milli` of `ingredient`, with
/// the output lot numbered like the batch, in stock at `location`, expiring in
/// `days` days. Stored in its state: a seed has no signed-in chef.
async fn produced_batch(
    state: &AppState,
    recipe: &fnb::BatchRecipe,
    sauce: &Item,
    location: &Location,
    ingredient: (&Item, &wl_inventory::Lot, i64),
    made_milli: i64,
    days: i64,
) -> Result<(fnb::ProductionBatch, wl_inventory::Lot)> {
    use fnb::batch::{BatchInput, BatchStatus, ProductionBatch};
    use wl_inventory::{Lot, LotStatus, MovementType};
    let db = &state.db;
    let (tomato, tomato_lot, used_milli) = ingredient;
    let number = fnb::BATCH_SERIES
        .next_allowing_gaps(db, None, None, today())
        .await?;
    let outlet = Outlet::by_code(db, "central").await?;
    let mut batch = ProductionBatch {
        number: number.clone(),
        recipe_id: recipe.id,
        outlet_id: outlet.map(|outlet| outlet.id),
        location_id: location.id,
        planned_yield_milli: made_milli,
        actual_yield_milli: Some(made_milli),
        expiry_date: Some(today() + TimeDelta::days(days)),
        finished_at: Some(renox::db::now() - TimeDelta::hours(5)),
        started_at: Some(renox::db::now() - TimeDelta::hours(8)),
        status: BatchStatus::Produced,
        ..ProductionBatch::default()
    };
    let mut tx = db.begin().await?;
    batch
        .insert_in_state(&mut tx, BatchStatus::Produced, "demo seed")
        .await?;
    let mut input = BatchInput {
        batch_id: batch.id,
        item_id: tomato.id,
        lot_id: tomato_lot.id,
        quantity_milli: used_milli,
        ..BatchInput::default()
    };
    input.insert(&mut tx).await?;
    let source = batch.subject();
    let value = value_of(used_milli, tomato.average_cost);
    let mut consumed = StockMovement::leaving(
        MovementType::Consume,
        tomato.id,
        Some(tomato_lot.id),
        location.id,
        used_milli,
        tomato.average_cost,
        value,
        &source,
        None,
        None,
    );
    consumed.insert(&mut tx).await?;
    let mut out = Lot {
        item_id: sauce.id,
        number,
        source_type: Some(source.subject_type.clone()),
        source_id: Some(source.subject_id),
        expiry_date: batch.expiry_date,
        status: LotStatus::Released,
        ..Lot::default()
    };
    out.insert(&mut tx).await?;
    let unit_cost = i64::try_from(i128::from(value) * 1_000 / i128::from(made_milli)).unwrap_or(0);
    let mut made = StockMovement::moving(
        MovementType::Produce,
        sauce.id,
        Some(out.id),
        None,
        Some(location.id),
        made_milli,
        unit_cost,
        value,
        &source,
        None,
    );
    made.insert(&mut tx).await?;
    ProductionBatch::query()
        .where_(ProductionBatch::ID.eq(batch.id))
        .update(&mut tx, &[("lot_id", &Some(out.id))])
        .await?;
    tx.commit().await?;
    Ok((ProductionBatch::find_or_404(db, batch.id).await?, out))
}

/// The central kitchen's batches (BL-FNB-11): `BAT-0001` was made from tomato
/// lot T7 and its sauce lot is on hold (a quality hold, so the batch is on
/// hold and blocked everywhere), `BAT-0002` was made from tomato lot T5 and is
/// in stock, and `BAT-0003` is planned, so the chef can start it and see the
/// lot on hold refused. Returns how many batches it made.
async fn production(state: &AppState) -> Result<usize> {
    use fnb::batch::{BatchStatus, ProductionBatch};
    use wl_inventory::hold_service::{NewHold, NewScope, place};
    let db = &state.db;
    if ProductionBatch::query().count(db).await? > 0 {
        return Ok(0);
    }
    let tomato = tracked_item(db, "TOMATO", "Tomato", 800).await?;
    let sauce = tracked_item(db, "TOMATO-SAUCE", "Tomato sauce", 1_500).await?;
    let central = central_stock(state).await?;
    let recipe = fnb::seed::demo_batch_recipe(
        db,
        "tomato-sauce",
        "Tomato sauce",
        sauce.id,
        10_000,
        &[(tomato.id, "TOMATO", 12_000)],
    )
    .await?;
    let t5 = received_lot(db, &tomato, "T5", &central, 40_000, 6).await?;
    let t7 = received_lot(db, &tomato, "T7", &central, 30_000, 3).await?;
    // Made from T7; the sauce lot is then held.
    let (held, held_lot) = produced_batch(
        state,
        &recipe,
        &sauce,
        &central,
        (&tomato, &t7, 12_000),
        10_000,
        2,
    )
    .await?;
    place(
        state,
        NewHold {
            reason: "quality".to_owned(),
            reference: held.subject(),
            scope: NewScope::Lots(vec![held_lot.id]),
            by: None,
        },
    )
    .await?;
    // Made from T5 and fine.
    produced_batch(
        state,
        &recipe,
        &sauce,
        &central,
        (&tomato, &t5, 12_000),
        8_000,
        4,
    )
    .await?;
    // Planned: the chef starts it.
    let number = fnb::BATCH_SERIES
        .next_allowing_gaps(db, None, None, today())
        .await?;
    let outlet = Outlet::by_code(db, "central").await?;
    let mut planned = ProductionBatch {
        number,
        recipe_id: recipe.id,
        outlet_id: outlet.map(|outlet| outlet.id),
        location_id: central.id,
        planned_yield_milli: 10_000,
        status: BatchStatus::Planned,
        ..ProductionBatch::default()
    };
    let mut tx = db.begin().await?;
    planned
        .insert_in_state(&mut tx, BatchStatus::Planned, "demo seed")
        .await?;
    tx.commit().await?;
    Ok(3)
}

/// A location `code` named `name` under `parent`, made when there is none.
async fn sub_location(db: &Db, code: &str, name: &str, parent: &Location) -> Result<Location> {
    if let Some(found) = Location::query()
        .where_(Location::CODE.eq(code))
        .first(db)
        .await?
    {
        return Ok(found);
    }
    let mut location = Location::new(code, name);
    location.parent_id = Some(parent.id);
    location.save(db).await?;
    Ok(location)
}

/// `quantity_milli` of the plain (not lot-tracked) item `code`, received at
/// `location` when nothing of it was.
async fn stocked(
    db: &Db,
    code: &str,
    name: &str,
    location: &Location,
    quantity_milli: i64,
    cost: i64,
) -> Result {
    let item = match Item::query().where_(Item::CODE.eq(code)).first(db).await? {
        Some(item) => item,
        None => {
            let mut item = Item::new(code, name, "kg");
            item.average_cost = cost;
            item.save(db).await?;
            item
        }
    };
    let there = StockMovement::query()
        .where_(StockMovement::ITEM_ID.eq(item.id))
        .where_(StockMovement::TO_LOCATION_ID.eq(location.id))
        .count(db)
        .await?;
    if there == 0 {
        let source = Subject::new("inventory.goods_receipt", 9_200);
        let mut moved = StockMovement::receive(
            item.id,
            None,
            location.id,
            quantity_milli,
            item.average_cost,
            value_of(quantity_milli, item.average_cost),
            &source,
            None,
        );
        moved.save(db).await?;
    }
    Ok(())
}

/// The Marina kitchen's cold units (BL-FNB-12): Fridge 2 (OUT1-F2) holds 6 kg
/// of chicken and 2 kg of cream and read 9 degrees at 14:00 and 8 degrees half
/// an hour later, so its stock is on one temperature hold with one equipment
/// task, and Freezer 1 read minus 20 and is fine. Returns how many units it
/// made. The readings are stored and reacted to by the system: a seed has no
/// signed-in cook.
async fn cold_units(state: &AppState) -> Result<usize> {
    use fnb::cold::{ColdUnit, TemperatureReading, default_range, kind, react};
    let db = &state.db;
    if ColdUnit::query().count(db).await? > 0 {
        return Ok(0);
    }
    let marina = kitchen_stock(state).await?;
    let fridge_location = sub_location(db, "OUT1-F2", "Fridge 2", &marina).await?;
    let freezer_location = sub_location(db, "OUT1-FR1", "Freezer 1", &marina).await?;
    stocked(
        db,
        "CHICKEN-FRESH",
        "Fresh chicken",
        &fridge_location,
        6_000,
        1_800,
    )
    .await?;
    stocked(db, "CREAM", "Cream", &fridge_location, 2_000, 1_200).await?;
    stocked(
        db,
        "CHICKEN-FROZEN",
        "Frozen chicken",
        &freezer_location,
        20_000,
        1_500,
    )
    .await?;
    let outlet = Outlet::by_code(db, "marina").await?.map(|outlet| outlet.id);
    let mut made = Vec::new();
    for (code, name, unit_kind, location) in [
        ("OUT1-F2", "Fridge 2", kind::FRIDGE, &fridge_location),
        ("OUT1-FR1", "Freezer 1", kind::FREEZER, &freezer_location),
    ] {
        let (min, max) = default_range(unit_kind);
        let mut unit = ColdUnit {
            code: code.to_owned(),
            name: name.to_owned(),
            kind: unit_kind.to_owned(),
            outlet_id: outlet,
            location_id: location.id,
            min_tenths: min,
            max_tenths: max,
            active: true,
            ..ColdUnit::default()
        };
        unit.insert(db).await?;
        made.push(unit);
    }
    let now = renox::db::now();
    for (unit, tenths, minutes_ago) in [
        (&made[1], -200, 60),
        (&made[0], 40, 300),
        (&made[0], 90, 180),
        (&made[0], 80, 150),
    ] {
        let mut reading = TemperatureReading {
            unit_id: unit.id,
            tenths,
            in_range: unit.in_range(tenths),
            recorded_at: now - TimeDelta::minutes(minutes_ago),
            ..TemperatureReading::default()
        };
        reading.insert(db).await?;
        if !reading.in_range {
            react(state, unit.id, reading.id).await?;
        }
    }
    Ok(made.len())
}

/// The tomato lot T5 is recalled by the supplier (BL-FNB-13). The sauce lot of
/// the batch made from it was moved to the Marina kitchen three days ago, and
/// every menu item uses 40 g of the sauce, so the completed orders (two direct
/// orders of customers, two of an aggregator) consumed it by recipe: the trace
/// lists the customers with their possible orders and the sales without a
/// known customer per day, and the owner has the task to decide the public
/// notice. Returns how many recalls it opened.
async fn recall_story(state: &AppState) -> Result<usize> {
    use fnb::{MenuItem, RecipeLine};
    use wl_inventory::recall::{NewRecall, ScopeEntry, open_for};
    use wl_inventory::{Lot, MovementType, Recall, RecallSource};
    let db = &state.db;
    if Recall::query().count(db).await? > 0 {
        return Ok(0);
    }
    let Some(tomato) = Item::query()
        .where_(Item::CODE.eq("TOMATO"))
        .first(db)
        .await?
    else {
        return Ok(0);
    };
    let Some(sauce) = Item::query()
        .where_(Item::CODE.eq("TOMATO-SAUCE"))
        .first(db)
        .await?
    else {
        return Ok(0);
    };
    let marina = kitchen_stock(state).await?;
    let central = central_stock(state).await?;
    // Every menu item uses the sauce.
    for code in [
        "shawarma-plate",
        "falafel-wrap",
        "hummus-bowl",
        "kunafa-tray",
    ] {
        let Some(item) = MenuItem::by_code(db, code).await? else {
            continue;
        };
        let used = RecipeLine::query()
            .where_(RecipeLine::MENU_ITEM_ID.eq(item.id))
            .where_(RecipeLine::ITEM_ID.eq(sauce.id))
            .exists(db)
            .await?;
        if !used {
            let mut line = RecipeLine {
                menu_item_id: item.id,
                item_id: sauce.id,
                per_unit_milli: 40,
                ..RecipeLine::default()
            };
            line.insert(db).await?;
        }
    }
    // The sauce lot of the batch made from T5 goes to Marina three days ago.
    let batches = fnb::ProductionBatch::query()
        .order_by(fnb::ProductionBatch::ID)
        .get(db)
        .await?;
    let t5 = Lot::query()
        .where_(Lot::ITEM_ID.eq(tomato.id))
        .where_(Lot::NUMBER.eq("T5"))
        .first(db)
        .await?;
    let made_from_t5 = match &t5 {
        Some(t5) => {
            let mut found = None;
            for batch in &batches {
                if batch
                    .inputs(db)
                    .await?
                    .iter()
                    .any(|input| input.lot_id == t5.id)
                {
                    found = batch.lot_id;
                }
            }
            found
        }
        None => None,
    };
    let Some(sauce_lot) = made_from_t5 else {
        return Ok(0);
    };
    let source = Subject::new("inventory.transfer", 9_300);
    let mut moved = StockMovement::moving(
        MovementType::TransferIn,
        sauce.id,
        Some(sauce_lot),
        Some(central.id),
        Some(marina.id),
        6_000,
        sauce.average_cost,
        value_of(6_000, sauce.average_cost),
        &source,
        None,
    );
    moved.moved_at = renox::db::now() - TimeDelta::days(3);
    moved.save(db).await?;
    // The completed orders consumed the sauce by recipe.
    for order in FnbOrder::query()
        .where_(FnbOrder::STATUS.eq(FnbOrderStatus::Completed))
        .order_by(FnbOrder::ID)
        .get(db)
        .await?
    {
        fnb::consumption::post(state, order.id).await?;
    }
    open_for(
        state,
        None,
        NewRecall {
            source: RecallSource::Supplier,
            reason: "The supplier found contamination in tomato lot T5".to_owned(),
            supplier_party_id: None,
            scope: vec![ScopeEntry::lot(tomato.id, "T5")],
        },
    )
    .await?;
    Ok(1)
}

/// The local date and `HH:MM` of `at`, as the paper-order form takes them.
fn local_parts(at: renox::db::DateTime) -> (renox::chrono::NaiveDate, String) {
    let local = fnb::business_day::local(at);
    (local.date(), local.format("%H:%M").to_string())
}

/// A paper order of `outlet` that happened at `at`.
fn paper_ask(
    outlet: &Outlet,
    reference: &str,
    amount: i64,
    at: renox::db::DateTime,
    reason: &str,
) -> fnb::paper_order::PaperAsk {
    let (day, time) = local_parts(at);
    fnb::paper_order::PaperAsk {
        outlet_id: outlet.id,
        channel: "counter".to_owned(),
        reference: reference.to_owned(),
        amount,
        method: "cash".to_owned(),
        note: String::new(),
        occurred_on: Some(day),
        occurred_time: time,
        reason: reason.to_owned(),
        outage_reference: String::new(),
    }
}

/// The outage and closing stories (BL-FNB-14, BL-FNB-15):
///
/// - **Marina Kitchen** lost its devices 75 minutes ago: its two brands show
///   temporarily closed (Habibi Bowls stays open at Sharjah), the managers have
///   the notice and the kitchen manager the task, and a paper order of 50
///   minutes ago is entered with the late-entry flag;
/// - **Marina, three business days ago**: its closing was submitted and the area
///   manager approved it; a paper order for that day is refused and the area
///   manager has the task to record it in the next day's closing;
/// - **Sharjah, two business days ago**: the closing was submitted with a till
///   short by AED 35.00 (the cash handover of the shortage is made), a paper
///   order of 21:00 that day was entered a day later, the area manager approved
///   it and the closing is reopened, waiting to be counted and submitted again.
///
/// Idempotent: it returns 0 when Marina has an outage already. Returns how many
/// stories it made.
async fn outage_and_closings(
    state: &AppState,
    cashier: Option<i64>,
    area: Option<&User>,
) -> Result<usize> {
    use fnb::closings::{self, SubmitAsk};
    use fnb::{Outage, paper_order};
    let db = &state.db;
    // The late entries that wait for the area manager need the default approval
    // policy of the kernel.
    wl_kernel::approvals::install(db).await?;
    let (Some(cashier), Some(marina), Some(sharjah)) = (
        cashier,
        Outlet::by_code(db, "marina").await?,
        Outlet::by_code(db, "sharjah").await?,
    ) else {
        return Ok(0);
    };
    if Outage::query()
        .where_(Outage::OUTLET_ID.eq(marina.id))
        .exists(db)
        .await?
    {
        return Ok(0);
    }
    let now = renox::db::now();
    let mut made = 0;

    // Marina, three business days ago: approved, and a refused late entry.
    let old = marina.business_date(now - TimeDelta::days(3));
    let closing = closings::ensure(db, marina.id, old).await?;
    let till = wl_kernel::sales::till_desk(state);
    till.open(
        state,
        None,
        wl_kernel::sales::OpenTill {
            till: marina.till_key(),
            location: Some(marina.code.clone()),
            legal_entity_id: marina.legal_entity_id,
            business_date: old,
            cashier_user: cashier,
            float: 20_000,
        },
    )
    .await?;
    closings::submit(
        state,
        None,
        closing.id,
        &SubmitAsk {
            counted: 20_000,
            checklist_done: true,
            cashier: Some(cashier),
        },
    )
    .await?;
    closings::approve(state, None, closing.id).await?;
    let late_at = fnb::business_day::at_local(old, "22:00").unwrap_or(now);
    // The refusal is the story: the task is made before the error.
    let refused = paper_order::enter_as(
        state,
        cashier,
        &paper_ask(
            &marina,
            "P-0099",
            8_400,
            late_at,
            "Printout found after the outage",
        ),
    )
    .await;
    println!(
        "Marina {old}: closing approved, late entry {}.",
        if refused.is_err() {
            "refused"
        } else {
            "recorded"
        }
    );
    made += 1;

    // Sharjah, two business days ago: AED 35.00 short, a late entry approved.
    let day = sharjah.business_date(now - TimeDelta::days(2));
    let closing = closings::ensure(db, sharjah.id, day).await?;
    till.open(
        state,
        None,
        wl_kernel::sales::OpenTill {
            till: sharjah.till_key(),
            location: Some(sharjah.code.clone()),
            legal_entity_id: sharjah.legal_entity_id,
            business_date: day,
            cashier_user: cashier,
            float: 30_000,
        },
    )
    .await?;
    closings::submit(
        state,
        None,
        closing.id,
        &SubmitAsk {
            counted: 26_500,
            checklist_done: true,
            cashier: Some(cashier),
        },
    )
    .await?;
    let at = fnb::business_day::at_local(day, "21:00").unwrap_or(now);
    let entered = paper_order::enter_as(
        state,
        cashier,
        &paper_ask(
            &sharjah,
            "P-0107",
            12_600,
            at,
            "Terminal was down, ticket found next morning",
        ),
    )
    .await;
    match (entered, area) {
        (
            Ok(wl_kernel::late_entries::Entered::Pending {
                approval_request_id,
                ..
            }),
            Some(area),
        ) => {
            if let Err(error) =
                wl_kernel::approvals::approve(state, approval_request_id, area, "").await
            {
                println!("The late entry of Sharjah waits for the area manager: {error}");
            }
        }
        (Ok(_), _) => {}
        (Err(error), _) => println!("The late entry of Sharjah was not made: {error}"),
    }
    made += 1;

    // Marina now: devices silent 75 minutes ago, channels paused, a paper order.
    let silent_since = now - TimeDelta::minutes(75);
    fnb::outage::open_by_devices(state, &marina, silent_since).await?;
    let before = now - TimeDelta::minutes(50);
    paper_order::enter_as(
        state,
        cashier,
        &paper_ask(&marina, "P-0108", 4_200, before, "Devices were down"),
    )
    .await?;
    made += 1;
    Ok(made)
}
