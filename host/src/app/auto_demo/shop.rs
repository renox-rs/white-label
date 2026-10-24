//! The shop and the workshop: the parts counters and the workshop's store, the
//! parts bought from the suppliers and received (purchase orders issued by the
//! purchaser, goods receipts posted by the storekeeper), the bay of the
//! workshop, the storage fee switch and the insurers with their payer
//! agreements and rate cards.
//!
//! The Deira counter is the first store made: it is the store the sales orders
//! reserve from (`selling_location` takes the first store). The workshop's
//! store is scoped to the workshop, so the parts of a job card come from it.

use auto::insurers::{self, NewInsurer};
use auto::labour_menu::{self, NewOperation};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_inventory::goods_receipt::{DraftLine, DraftReceipt, create_draft};
use wl_inventory::{Item, Location, PurchaseOrder, PurchaseOrderLine, PurchaseOrderStatus};
use wl_invoicing::rate_card::Unit;
use wl_invoicing::rate_cards::{self, LineInput};
use wl_invoicing::{PayerAgreement, VatRecipient};
use wl_kernel::documents::today;
use wl_kernel::{LegalEntity, Subject};
use wl_scheduling::{AvailabilityRule, Resource, ResourceType, ServiceType, resource};

use super::super::demo_data::{at, day};
use super::{ADMIN_LOGIN, CARLA, KARIM, PIA, Seed, WALEED};

/// The workshop of the bookings and of the store.
pub(crate) fn workshop() -> Subject {
    Subject::new("scheduling.location", 1)
}

/// An item: code, English name, Arabic name, unit, reorder point, maximum.
const ITEMS: &[(&str, &str, &str, &str, i64, i64)] = &[
    ("BRK-PAD", "Brake pad set", "طقم فحمات فرامل", "pcs", 20, 80),
    ("OIL-F", "Oil filter", "فلتر زيت", "pcs", 20, 100),
    ("AIR-F", "Air filter", "فلتر هواء", "pcs", 10, 60),
    ("BAT-70", "Battery 70Ah", "بطارية 70 أمبير", "pcs", 4, 16),
    ("ALT-9", "Alternator", "دينمو شحن", "pcs", 2, 8),
    (
        "TIMING-KIT",
        "Timing belt kit",
        "طقم سير الكاتينة",
        "pcs",
        4,
        16,
    ),
    ("CLUTCH-K", "Clutch kit", "طقم دبرياج", "pcs", 2, 8),
    (
        "SPARK-4",
        "Spark plug set (4)",
        "طقم بوجيهات",
        "pcs",
        10,
        40,
    ),
];

/// A purchase order: key, day written, supplier, store, lines (code, units,
/// unit cost in fils), the day of its receipt (none: still on its way).
struct Buy {
    key: &'static str,
    written: i64,
    supplier: &'static str,
    store: &'static str,
    lines: &'static [(&'static str, i64, i64)],
    received: Option<i64>,
}

const BUYS: &[Buy] = &[
    Buy {
        key: "P1",
        written: -150,
        supplier: "gulf_spares",
        store: "deira",
        lines: &[
            ("BRK-PAD", 60, 8_000),
            ("OIL-F", 80, 3_500),
            ("AIR-F", 40, 4_500),
            ("BAT-70", 12, 31_000),
            ("ALT-9", 6, 45_000),
            ("TIMING-KIT", 6, 18_000),
            ("SPARK-4", 30, 6_000),
        ],
        received: Some(-148),
    },
    Buy {
        key: "P2",
        written: -120,
        supplier: "brake_parts",
        store: "deira",
        lines: &[("BRK-PAD", 40, 7_800)],
        received: Some(-118),
    },
    Buy {
        key: "P3",
        written: -100,
        supplier: "gulf_spares",
        store: "workshop",
        lines: &[
            ("BRK-PAD", 24, 8_000),
            ("OIL-F", 30, 3_500),
            ("AIR-F", 12, 4_500),
            ("BAT-70", 4, 31_000),
            ("ALT-9", 3, 45_000),
            ("SPARK-4", 12, 6_000),
        ],
        received: Some(-98),
    },
    Buy {
        key: "P4",
        written: -60,
        supplier: "oil_house",
        store: "sharjah",
        lines: &[("OIL-F", 30, 3_400), ("AIR-F", 15, 4_400)],
        received: Some(-58),
    },
    Buy {
        key: "P5",
        written: -28,
        supplier: "gulf_spares",
        store: "deira",
        lines: &[("CLUTCH-K", 5, 52_000)],
        received: Some(-20),
    },
    Buy {
        key: "P6",
        written: -3,
        supplier: "gulf_spares",
        store: "deira",
        lines: &[("TIMING-KIT", 12, 18_000)],
        received: None,
    },
];

impl Seed {
    /// The store with this key (`deira`, `workshop`, `sharjah`).
    pub(crate) fn store(&self, key: &str) -> &Location {
        self.stores
            .get(key)
            .unwrap_or_else(|| panic!("the demo store {key}"))
    }

    /// Stores, items, suppliers' orders and receipts, the bay, the switches.
    pub(crate) async fn shop(&mut self) -> Result {
        let mut stores = Vec::new();
        let mut items = Vec::new();
        at!(self, day(-180), {
            // The Deira counter first: sales orders reserve from it.
            for (key, code, name, scope) in [
                ("deira", "DEIRA", "Deira parts counter", None),
                ("workshop", "WORKSHOP-1", "Workshop 1", Some(workshop())),
                ("sharjah", "SHARJAH", "Sharjah parts counter", None),
            ] {
                let mut store = Location::new(code, name);
                if let Some(scope) = &scope {
                    store = store.in_scope(scope);
                }
                store.save(self.db()).await?;
                stores.push((key, store));
            }
            for (code, en, ar, unit, point, max) in ITEMS {
                let mut item = Item::new(code, en, unit).reordered(point * 1_000, max * 1_000);
                item.name_ar = Some((*ar).to_owned());
                item.save(self.db()).await?;
                items.push((*code, item));
            }
            auto::settings::set(self.db(), auto::settings::STORAGE_FEE_ENABLED, "1", None).await?;
            // The workshop's bay is open every day of the week, so a booking
            // does not depend on the weekday of the seed.
            let mut bay = Resource::new("Bay 1", ResourceType::Bay).at(&workshop());
            bay.save(self.db()).await?;
            for weekday in 1..=7 {
                AvailabilityRule::new(bay.id, weekday, 0, 1440)
                    .save(self.db())
                    .await?;
            }
            for service in ServiceType::query().get(self.db()).await? {
                resource::offer(self.db(), service.id, bay.id).await?;
            }
            Ok::<(), renox::Error>(())
        })?;
        self.stores.extend(stores);
        self.items.extend(items);
        for buy in BUYS {
            self.buy(buy).await?;
        }
        Ok(())
    }

    /// One purchase order, from the draft to its receipt.
    async fn buy(&self, buy: &Buy) -> Result {
        let supplier = self.party(buy.supplier);
        let store = self.store(buy.store).clone();
        let order = at!(self, day(buy.written), {
            let mut order = PurchaseOrder::new(supplier, store.id);
            // There is no screen that writes a purchase order yet: it is made
            // in its first state, the draft; issuing is the purchaser's page.
            let mut tx = self.db().begin().await?;
            order
                .insert_in_state(
                    &mut tx,
                    PurchaseOrderStatus::Draft,
                    "Written by the purchaser",
                )
                .await?;
            tx.commit().await?;
            for (position, (code, units, cost)) in (1..).zip(buy.lines) {
                let mut line = PurchaseOrderLine::new(
                    order.id,
                    position,
                    self.items[code].id,
                    units * 1_000,
                    *cost,
                );
                line.save(self.db()).await?;
            }
            self.driver
                .post(PIA, &format!("/purchase-orders/{}/issue", order.id), &[])
                .await?;
            Ok::<PurchaseOrder, renox::Error>(order)
        })?;
        let Some(received) = buy.received else {
            return Ok(());
        };
        let lines = wl_inventory::purchase_order::lines_of(self.db(), order.id).await?;
        let keeper = self.user(KARIM).id;
        at!(self, day(received), {
            let draft = DraftReceipt {
                purchase_order_id: order.id,
                location_id: store.id,
                delivery_note: Some(format!("DN-{}", buy.key)),
                lines: lines
                    .iter()
                    .map(|line| DraftLine::on(line.id, line.quantity_milli, line.quantity_milli))
                    .collect(),
            };
            let made = create_draft(self.db(), Some(keeper), draft).await?;
            self.driver
                .post(KARIM, &format!("/goods-receipts/{}/post", made.id), &[])
                .await?;
            Ok::<(), renox::Error>(())
        })
    }

    /// The labour menu of the workshop: the flat-rate jobs of every day and the
    /// hourly work, kept by the workshop manager before any job card is
    /// priced from it.
    pub(crate) async fn labour_menu(&mut self) -> Result {
        let waleed = self.who(WALEED).clone();
        at!(self, day(-180), {
            for (code, name_en, name_ar, flat) in [
                (
                    "OIL-CHG",
                    "Oil and filter change",
                    "تغيير الزيت والفلتر",
                    Some((500, 18_000)),
                ),
                (
                    "BRK-FRT",
                    "Front brake pads",
                    "تبديل فحمات الفرامل الأمامية",
                    Some((1_500, 22_500)),
                ),
                (
                    "SVC-60K",
                    "Periodic service at 60,000 km",
                    "الصيانة الدورية عند 60,000 كم",
                    Some((3_000, 45_000)),
                ),
                ("DIAG", "Diagnostics", "فحص الأعطال", None),
            ] {
                let mut new = match flat {
                    Some((hours, price)) => NewOperation::flat(code, name_en, hours, price),
                    None => NewOperation::hourly(code, name_en, 15_000),
                };
                new.name_ar = Some(name_ar.to_owned());
                labour_menu::create(&self.state, &waleed, new).await?;
            }
            Ok::<(), renox::Error>(())
        })
    }

    /// The two insurers, each with its payer agreement, its claim e-mail and
    /// its rate card (accepted by the administrator).
    pub(crate) async fn insurers(&mut self) -> Result {
        let carla = self.who(CARLA).clone();
        let admin = self.who(ADMIN_LOGIN).clone();
        let entity = LegalEntity::default_entity(self.db()).await?;
        let mut made = Vec::new();
        at!(self, day(-175), {
            for (key, party, name, email, storage) in [
                (
                    "falcon",
                    "falcon",
                    "Falcon motor",
                    "claims@falconmotor.example",
                    Some(5_000),
                ),
                (
                    "crescent",
                    "crescent",
                    "Crescent Gulf motor",
                    "motor@crescentgulf.example",
                    None,
                ),
            ] {
                let payer = self.party(party);
                let mut agreement = PayerAgreement {
                    legal_entity_id: entity.id,
                    payer_party_id: payer.0,
                    name: name.to_owned(),
                    active: true,
                    vat_recipient: VatRecipient::Payer,
                    first_submission_days: 30,
                    resubmission_days: 30,
                    resubmission_limit: 2,
                    write_off_limit: 100_000,
                    ..PayerAgreement::default()
                };
                agreement.insert(self.db()).await?;
                let mut new = NewInsurer::new(payer.0)
                    .agreement(agreement.id)
                    .email(email)
                    .labour_rate(12_000);
                if let Some(rate) = storage {
                    new = new.storage_rate(rate);
                }
                let insurer = insurers::register(&self.state, &carla, new).await?;
                let card =
                    rate_cards::create_card(self.db(), None, payer, &insurer.subject()).await?;
                let version = rate_cards::new_version(
                    self.db(),
                    None,
                    card.id,
                    today() - renox::chrono::TimeDelta::days(30),
                    false,
                )
                .await?;
                rate_cards::add_line(
                    self.db(),
                    version.id,
                    LineInput::new("body_repair", "standard", Unit::Hour, 12_000),
                )
                .await?;
                rate_cards::accept(&self.state, &admin, version.id, Some("signed-rate-card"))
                    .await?;
                made.push((key, insurer));
            }
            Ok::<(), renox::Error>(())
        })?;
        self.insurers.extend(made);
        Ok(())
    }
}
