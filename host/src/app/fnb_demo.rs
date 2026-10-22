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
use wl_delivery::Deliveries;
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
    self, ACCOUNTANT, AREA_MANAGER, CASHIER, DRIVER, HEAD_CHEF, KITCHEN_MANAGER, LINE_COOK,
    MEMBER, MENU_MANAGER, OPERATIONS_DIRECTOR, STOREKEEPER,
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
        label: "Head chef",
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
    for login in LOGINS {
        let user = match User::find_by_email(db, login.email).await? {
            Some(user) => user,
            None => {
                let mut user = User::register(db, login.name, login.email, PASSWORD).await?;
                user.email_verified_at = Some(renox::db::now());
                user.save(db).await?;
                user.assign_role(db, login.role).await?;
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
    }
    let made = orders(&state, &customers).await?;
    println!("{made} demo orders made for the Zaatar & Co. kitchen.");
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
        if FnbOrder::of_sales_order(&state.db, sales.id).await?.is_some() {
            continue;
        }
        if let Some(order) = fnb::placing::open(
            state,
            SalesOrderId(sales.id),
            FnbOrderStatus::Accepted,
        )
        .await?
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
}

const STORIES: [Story; 12] = [
    Story {
        key: "FNBD-1",
        channel: "website",
        customer: 0,
        mode: "online",
        lines: &[("shawarma-plate", 2)],
        sales: SalesOrderStatus::PendingAcceptance,
        status: FnbOrderStatus::Placed,
        placed_hours_ago: 0,
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
    let prepaid = story.mode == PAYMENT_ONLINE;
    let mut order = FnbOrder {
        number: sales.number.clone(),
        sales_order_id: sales.id,
        party_id: party.0,
        brand_id: brand,
        outlet_id: outlet.map(|outlet| outlet.id),
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
        accepted_at: (story.status != FnbOrderStatus::Placed).then_some(placed),
        status: story.status,
        ..FnbOrder::default()
    };
    match story.key {
        // The customer was not home (customer side).
        "FNBD-5" => failed(&mut order, "not_home", reason::CUSTOMER_UNREACHABLE, side::CUSTOMER, party::CUSTOMER),
        // The run was aborted (outlet side).
        "FNBD-6" => failed(&mut order, "run_aborted", reason::RUN_ABORTED, side::OUTLET, party::BUSINESS),
        // Closed after a customer-side failure: nothing refunded, the cashier
        // may ask for a refund within 7 days.
        "FNBD-7" => {
            failed(&mut order, "not_home", reason::CUSTOMER_UNREACHABLE, side::CUSTOMER, party::CUSTOMER);
            closed(&mut order, decision::NO_REFUND, 0, 48);
        }
        // A cash order failed and closed 40 days ago, and one failed now.
        "FNBD-8" => {
            failed(&mut order, "no_cash", reason::NO_CASH, side::CUSTOMER, party::CUSTOMER);
            closed(&mut order, decision::NOT_APPLICABLE, 0, 24 * 40 - 1);
        }
        "FNBD-9" => failed(&mut order, "refused", reason::REFUSED, side::CUSTOMER, party::CUSTOMER),
        "FNBD-4" => order.completed_at = Some(placed + TimeDelta::minutes(40)),
        // Cancelled by the customer from the order page, refunded in full.
        "FNBD-11" => cancelled(&mut order, cancel_reason::CUSTOMER_REQUEST, party::CUSTOMER, true, 5),
        // The aggregator cancelled it: no refund, the aggregator pays.
        "FNBD-12" => cancelled(&mut order, cancel_reason::AGGREGATOR_CANCELLED, party::AGGREGATOR, false, 6),
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
    order.close_reason = Some(
        fnb::cancel::reason_text(why, "")
    );
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

/// Asks wl-delivery for the delivery of a `ready` order, as the kitchen does
/// when the last ticket is bumped: a dispatcher puts it on a run.
async fn ask_delivery(state: &AppState, order: &FnbOrder) -> Result {
    let request = DeliveryRequest::new(order.subject(), &order.recipient_name)
        .address(&order.address, &order.area)
        .phone(&order.recipient_phone)
        .perishable()
        .partial_acceptance(false)
        .promise(order.placed_at.unwrap_or_else(renox::db::now), 45)
        .collect(if order.is_cash_on_delivery() { order.total } else { 0 })
        .item(DeliveryItem::new(&format!("Order {}", order.number), 1_000));
    let delivery = Deliveries::request(state, request).await?;
    FnbOrder::query()
        .where_(FnbOrder::ID.eq(order.id))
        .update(&state.db, &[("delivery_id", &Some(delivery.id))])
        .await?;
    Ok(())
}
