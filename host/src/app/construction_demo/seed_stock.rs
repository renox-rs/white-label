//! Procurement and stock: the items and the stores of the projects, the
//! suppliers of each item, purchase orders in every state the module can put
//! them in, goods receipts posted by the storekeeper (one with goods
//! rejected, which makes a return to the supplier and a draft debit note), a
//! reorder suggestion, a request to change the preferred supplier and the
//! supplier scores.
//!
//! Purchase orders and purchase requests have no screen that creates them
//! yet, so they are inserted in their first state (`draft`, and `submitted`
//! for the request that waits for the MD) with the module's models; from
//! there on everything goes through the routes: `issue` by procurement,
//! `post` by the storekeeper.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_inventory::goods_receipt::{DraftLine, DraftReceipt, create_draft};
use wl_inventory::{
    Item, Location, PurchaseOrder, PurchaseOrderLine, PurchaseOrderStatus, PurchaseRequest,
    PurchaseRequestStatus, RejectedHandling,
};
use wl_kernel::PartyId;

use super::at;
use super::people::{self, email_of};
use super::roles::{ADMIN, PROCUREMENT, STOREKEEPER};
use super::{Seed, day, text, upto};

/// An item of the stores: code, name, Arabic name, unit, reorder point and
/// maximum (in units), the preferred supplier and the alternative.
#[derive(Debug, Clone, Copy)]
pub struct ItemPlan {
    /// The code.
    pub code: &'static str,
    /// The English name.
    pub en: &'static str,
    /// The Arabic name.
    pub ar: &'static str,
    /// The base unit.
    pub unit: &'static str,
    /// Reordered at or below this many units.
    pub point: i64,
    /// Reordered up to this many units.
    pub max: i64,
    /// The preferred supplier.
    pub preferred: &'static str,
    /// The alternative supplier, if any.
    pub alternative: Option<&'static str>,
    /// The lead time of the preferred supplier, in days.
    pub lead_days: i64,
}

/// The items.
pub const ITEMS: &[ItemPlan] = &[
    ItemPlan {
        code: "GYP-12",
        en: "Gypsum board 12.5 mm, 1200 x 2400",
        ar: "لوح جبس 12.5 ملم",
        unit: "sheet",
        point: 200,
        max: 800,
        preferred: people::GYPSUM,
        alternative: None,
        lead_days: 5,
    },
    ItemPlan {
        code: "STUD-70",
        en: "Metal stud 70 mm, 3 m",
        ar: "عمود معدني 70 ملم",
        unit: "pcs",
        point: 300,
        max: 1_200,
        preferred: people::GYPSUM,
        alternative: None,
        lead_days: 5,
    },
    ItemPlan {
        code: "PLY-18",
        en: "Plywood 18 mm, 1220 x 2440",
        ar: "خشب رقائقي 18 ملم",
        unit: "sheet",
        point: 40,
        max: 200,
        preferred: people::GYPSUM,
        alternative: None,
        lead_days: 7,
    },
    ItemPlan {
        code: "TILE-60",
        en: "Porcelain tile 60 x 60 cm",
        ar: "بلاط بورسلين 60 × 60",
        unit: "m2",
        point: 100,
        max: 600,
        preferred: people::GYPSUM,
        alternative: None,
        lead_days: 10,
    },
    ItemPlan {
        code: "PAINT-20",
        en: "Interior emulsion paint, 20 L",
        ar: "دهان داخلي 20 لتر",
        unit: "pail",
        point: 30,
        max: 150,
        preferred: people::PAINTS,
        alternative: Some(people::GYPSUM),
        lead_days: 3,
    },
    ItemPlan {
        code: "CABLE-25",
        en: "Power cable 2.5 mm2, 100 m roll",
        ar: "كابل كهرباء 2.5 ملم",
        unit: "roll",
        point: 10,
        max: 60,
        preferred: people::ELECTRICAL,
        alternative: Some(people::MEP),
        lead_days: 4,
    },
    ItemPlan {
        code: "MCB-20",
        en: "Miniature circuit breaker 20 A",
        ar: "قاطع كهربائي 20 أمبير",
        unit: "pcs",
        point: 200,
        max: 600,
        preferred: people::ELECTRICAL,
        alternative: Some(people::MEP),
        lead_days: 4,
    },
    ItemPlan {
        code: "LED-600",
        en: "LED panel light 600 x 600",
        ar: "إضاءة لوحة LED 60 × 60",
        unit: "pcs",
        point: 150,
        max: 500,
        preferred: people::ELECTRICAL,
        alternative: Some(people::MEP),
        lead_days: 6,
    },
    ItemPlan {
        code: "PVC-110",
        en: "PVC drainage pipe 110 mm, 6 m",
        ar: "ماسورة صرف PVC 110 ملم",
        unit: "pcs",
        point: 20,
        max: 120,
        preferred: people::MEP,
        alternative: Some(people::GYPSUM),
        lead_days: 5,
    },
    ItemPlan {
        code: "ALU-WIN",
        en: "Aluminium window frame, powder coated",
        ar: "إطار نافذة ألمنيوم",
        unit: "pcs",
        point: 0,
        max: 0,
        preferred: people::GLASS,
        alternative: None,
        lead_days: 14,
    },
];

/// What arrives on a day: the position of the order line, the quantity that
/// came, the quantity accepted, and why the rest was rejected.
type Arrival = (usize, i64, i64);

/// A receipt of a purchase order.
#[derive(Debug, Clone, Copy)]
pub struct Receipt {
    /// The day, days from today.
    pub day: i64,
    /// The lines: the position in the order (from 1), what came, what was
    /// accepted (units).
    pub lines: &'static [Arrival],
    /// Whether the rejected goods are credited and refused at the door.
    pub refused_at_door: bool,
}

/// A purchase order of the demo.
#[derive(Debug, Clone, Copy)]
pub struct Order {
    /// The key.
    pub key: &'static str,
    /// The supplier.
    pub supplier: &'static str,
    /// The project whose store receives.
    pub project: &'static str,
    /// The lines: item code, units, price in fils per unit.
    pub lines: &'static [(&'static str, i64, i64)],
    /// The day it is written, days from today.
    pub written: i64,
    /// Whether procurement issues it.
    pub issue: bool,
    /// The receipts.
    pub receipts: &'static [Receipt],
}

/// The orders.
pub const ORDERS: &[Order] = &[
    Order {
        key: "P1",
        supplier: people::GYPSUM,
        project: "TWR-01",
        lines: &[("GYP-12", 600, 1_750), ("STUD-70", 900, 920)],
        written: -136,
        issue: true,
        receipts: &[
            Receipt {
                day: -128,
                lines: &[(1, 300, 300), (2, 450, 450)],
                refused_at_door: false,
            },
            Receipt {
                day: -115,
                lines: &[(1, 200, 190), (2, 450, 450)],
                refused_at_door: true,
            },
            Receipt {
                day: -100,
                lines: &[(1, 110, 110)],
                refused_at_door: false,
            },
        ],
    },
    Order {
        key: "P2",
        supplier: people::GYPSUM,
        project: "MALL-02",
        lines: &[
            ("GYP-12", 300, 1_750),
            ("PLY-18", 120, 8_900),
            ("TILE-60", 400, 5_400),
        ],
        written: -82,
        issue: true,
        receipts: &[
            Receipt {
                day: -75,
                lines: &[(1, 300, 300), (2, 60, 60)],
                refused_at_door: false,
            },
            // Twelve tiles came broken and wait aside for the supplier to
            // collect them.
            Receipt {
                day: -58,
                lines: &[(2, 60, 60), (3, 212, 200)],
                refused_at_door: false,
            },
            Receipt {
                day: -41,
                lines: &[(3, 200, 200)],
                refused_at_door: false,
            },
        ],
    },
    Order {
        key: "P3",
        supplier: people::ELECTRICAL,
        project: "TWR-01",
        lines: &[
            ("CABLE-25", 40, 31_000),
            ("MCB-20", 300, 1_850),
            ("LED-600", 250, 6_400),
        ],
        written: -70,
        issue: true,
        receipts: &[
            Receipt {
                day: -64,
                lines: &[(1, 20, 20), (2, 300, 300), (3, 100, 100)],
                refused_at_door: false,
            },
            Receipt {
                day: -50,
                lines: &[(1, 10, 10)],
                refused_at_door: false,
            },
        ],
    },
    Order {
        key: "P4",
        supplier: people::ELECTRICAL,
        project: "MALL-02",
        lines: &[
            ("CABLE-25", 20, 31_000),
            ("MCB-20", 120, 1_850),
            ("LED-600", 160, 6_400),
        ],
        written: -40,
        issue: true,
        receipts: &[Receipt {
            day: -34,
            lines: &[(2, 120, 120), (3, 60, 60)],
            refused_at_door: false,
        }],
    },
    Order {
        key: "P5",
        supplier: people::PAINTS,
        project: "CLN-03",
        lines: &[("PAINT-20", 150, 17_500)],
        written: -30,
        issue: true,
        receipts: &[],
    },
    Order {
        key: "P6",
        supplier: people::MEP,
        project: "RST-04",
        lines: &[("PVC-110", 80, 6_800)],
        written: -20,
        issue: true,
        receipts: &[Receipt {
            day: -12,
            lines: &[(1, 80, 80)],
            refused_at_door: false,
        }],
    },
    // The registration of this subcontractor expired: the issue waits for the
    // project manager's approval and the order stays a draft.
    Order {
        key: "P7",
        supplier: people::GLASS,
        project: "CLN-03",
        lines: &[("ALU-WIN", 24, 98_000)],
        written: -6,
        issue: true,
        receipts: &[],
    },
    // Above the MD's limit of AED 50,000: it stays a draft.
    Order {
        key: "P8",
        supplier: people::GYPSUM,
        project: "TWR-01",
        lines: &[("GYP-12", 4_000, 1_750), ("STUD-70", 2_000, 920)],
        written: -3,
        issue: false,
        receipts: &[],
    },
    Order {
        key: "P9",
        supplier: people::PAINTS,
        project: "VIL-05",
        lines: &[("PAINT-20", 40, 17_500)],
        written: -2,
        issue: false,
        receipts: &[],
    },
];

/// `date` plus `days`, never later than today.
fn on(offset: i64) -> NaiveDate {
    upto(day(offset))
}

impl Seed {
    /// The store of project `code` (made when the project is awarded).
    pub(crate) async fn store_of(&self, code: &str) -> Result<Location> {
        Location::query()
            .where_(Location::CODE.eq(format!("STORE-{code}").as_str()))
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no store for {code}").into())
    }

    /// The stores, items and suppliers of the items, then the orders.
    pub(crate) async fn stock(&mut self) -> Result {
        let procurement = email_of(PROCUREMENT);
        // The items, with their suppliers. The first supplier of an item is
        // set directly; a later change of the preferred one is a request.
        at!(self, day(-200), {
            for plan in ITEMS {
                let mut item = Item::new(plan.code, plan.en, plan.unit)
                    .reordered(plan.point * 1_000, plan.max * 1_000);
                item.name_ar = Some(plan.ar.to_owned());
                item.save(self.db()).await?;
                self.items.insert(plan.code, item.id);
                self.report.add("items", 1);
                let lead = plan.lead_days.to_string();
                let preferred = self.party(plan.preferred).id.to_string();
                self.driver
                    .post(
                        procurement,
                        &format!("/items/{}/suppliers/add", item.id),
                        &[
                            ("supplier", preferred.as_str()),
                            ("role", "preferred"),
                            ("lead_days", lead.as_str()),
                            ("unit", plan.unit),
                            ("code", plan.code),
                        ],
                    )
                    .await?;
                if let Some(alternative) = plan.alternative {
                    let alternative = self.party(alternative).id.to_string();
                    self.driver
                        .post(
                            procurement,
                            &format!("/items/{}/suppliers/add", item.id),
                            &[
                                ("supplier", alternative.as_str()),
                                ("role", "alternative"),
                                ("rank", "1"),
                                ("lead_days", "10"),
                            ],
                        )
                        .await?;
                }
            }
            Ok::<(), renox::Error>(())
        })?;
        for order in ORDERS {
            self.order(order).await?;
        }
        Ok(())
    }

    /// One purchase order, from the draft to its receipts.
    async fn order(&mut self, plan: &Order) -> Result {
        let procurement = email_of(PROCUREMENT);
        let storekeeper = email_of(STOREKEEPER);
        let store = self.store_of(plan.project).await?;
        let supplier = PartyId(self.party(plan.supplier).id);
        let written = on(plan.written);
        let order = at!(self, written, {
            let mut order = PurchaseOrder::new(supplier, store.id);
            // There is no screen that writes a purchase order yet: it is made
            // in its first state, the draft.
            let mut tx = self.db().begin().await?;
            order
                .insert_in_state(
                    &mut tx,
                    PurchaseOrderStatus::Draft,
                    "Written by procurement",
                )
                .await?;
            tx.commit().await?;
            for (position, (code, units, price)) in (1..).zip(plan.lines) {
                let line = PurchaseOrderLine::new(
                    order.id,
                    position,
                    self.items[code],
                    units * 1_000,
                    *price,
                );
                let mut line = line;
                line.save(self.db()).await?;
            }
            Ok::<PurchaseOrder, renox::Error>(order)
        })?;
        self.report.add("purchase orders", 1);
        if !plan.issue {
            return Ok(());
        }
        let issued = on(plan.written + 1);
        at!(self, issued, {
            self.driver
                .post(
                    procurement,
                    &format!("/purchase-orders/{}/issue", order.id),
                    &[],
                )
                .await
        })?;
        let lines = wl_inventory::purchase_order::lines_of(self.db(), order.id).await?;
        let storekeeper_id = self.user_id(storekeeper).await?;
        for receipt in plan.receipts {
            let when = on(receipt.day);
            let draft = DraftReceipt {
                purchase_order_id: order.id,
                location_id: store.id,
                delivery_note: Some(format!("DN-{}-{}", plan.key, -receipt.day)),
                lines: receipt
                    .lines
                    .iter()
                    .map(|(position, came, accepted)| {
                        let line = &lines[position - 1];
                        let mut draft = DraftLine::on(line.id, came * 1_000, accepted * 1_000);
                        if came > accepted {
                            let handling = if receipt.refused_at_door {
                                RejectedHandling::RefusedAtDoor
                            } else {
                                RejectedHandling::KeptForCollection
                            };
                            draft = draft
                                .rejected_for("damaged", handling)
                                .evidence("PHOTO-DAMAGED-BOARDS-01")
                                .credit();
                        }
                        draft
                    })
                    .collect(),
            };
            at!(self, when, {
                let made = create_draft(self.db(), Some(storekeeper_id), draft).await?;
                self.driver
                    .post(
                        storekeeper,
                        &format!("/goods-receipts/{}/post", made.id),
                        &[],
                    )
                    .await
            })?;
            self.report.add("goods receipts", 1);
        }
        Ok(())
    }

    /// The material request above the MD's limit, the reorder suggestions,
    /// the request to change a preferred supplier, the supplier scores.
    pub(crate) async fn stock_rules(&mut self) -> Result {
        let procurement = email_of(PROCUREMENT);
        let admin = email_of(ADMIN);
        let today = crate::app::demo_data::real_today();
        // The preferred supplier of the cable is questioned: procurement asks
        // to move it to the subcontractor, who is also cheaper; it waits.
        let cable = self.items["CABLE-25"].to_string();
        let mep = self.party(people::MEP).id.to_string();
        at!(self, day(-9), {
            self.driver
                .post(
                    procurement,
                    &format!("/items/{cable}/preferred-requests"),
                    &[("supplier", mep.as_str())],
                )
                .await
        })?;
        // The PVC pipe's preferred supplier was changed, and the MD approved.
        let pvc = self.items["PVC-110"].to_string();
        let gypsum = self.party(people::GYPSUM).id.to_string();
        at!(self, day(-14), {
            self.driver
                .post(
                    procurement,
                    &format!("/items/{pvc}/preferred-requests"),
                    &[("supplier", gypsum.as_str())],
                )
                .await
        })?;
        let request = wl_inventory::PreferredSupplierRequest::query()
            .where_(wl_inventory::PreferredSupplierRequest::ITEM_ID.eq(self.items["PVC-110"]))
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no preferred request"))?;
        at!(self, day(-12), {
            self.driver
                .post(
                    admin,
                    &format!("/preferred-requests/{}/approve", request.id),
                    &[],
                )
                .await
        })?;
        // A material request above AED 50,000 waits for the MD. The request
        // has no submit screen yet, so it is made in the state it waits in.
        let store = self.store_of("TWR-01").await?;
        let requester = self.user_id(email_of(STOREKEEPER)).await?;
        at!(self, day(-3), {
            let mut request = PurchaseRequest::new(store.id);
            request.cost_object = Some("construction.cost_code:gypsum".to_owned());
            request.needed_by = Some(day(10));
            request.requester_id = Some(requester);
            request.status = PurchaseRequestStatus::Submitted;
            let mut tx = self.db().begin().await?;
            request
                .insert_in_state(
                    &mut tx,
                    PurchaseRequestStatus::Submitted,
                    "Submitted by the storekeeper; above the MD's limit",
                )
                .await?;
            tx.commit().await?;
            Ok::<(), renox::Error>(())
        })?;
        // The daily rules of stock: the reorder suggestions, the supplier
        // scores and their reviews.
        at!(self, today, {
            wl_inventory::reorder::run(&self.state).await?;
            wl_inventory::supplier_score::run(&self.state).await
        })?;
        let _ = text(today);
        Ok(())
    }
}
