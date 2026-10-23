//! The plant of the demo company: the lines, the stores, the items and their
//! opening stock, the lots, the bills of materials and who holds which role
//! where.

use manufacturing::permissions::{line_scope, plant_scope};
use manufacturing::{Line, Plant};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_inventory::item::{ItemType, value_of};
use wl_inventory::{Item, Location, LocationType, Lot, LotStatus, StockMovement};
use wl_kernel::Subject;

use super::{
    MANAGER_EMAIL, OPERATOR_EMAIL, PLANNER_EMAIL, QC_EMAIL, STORE_EMAIL, SUPERVISOR_EMAIL, Seed,
    day, fixture,
};
use crate::app::roles::{
    LINE_OPERATOR, PRODUCTION_MANAGER, PRODUCTION_PLANNER, QC_INSPECTOR, SUPERVISOR,
};

/// An item of the catalogue: code, name, unit, kind, lot-tracked, cost of one
/// unit in fils.
type ItemRow = (
    &'static str,
    &'static str,
    &'static str,
    ItemType,
    bool,
    i64,
);

/// The raw materials, then the finished goods.
const ITEMS: &[ItemRow] = &[
    (
        "KRAFT-LINER",
        "Kraft liner paper",
        "kg",
        ItemType::Component,
        true,
        380,
    ),
    (
        "FLUTING-MED",
        "Corrugating medium",
        "kg",
        ItemType::Component,
        true,
        320,
    ),
    (
        "SLEEVE-FILM",
        "Sleeve film, 40 micron",
        "kg",
        ItemType::Component,
        true,
        1_200,
    ),
    (
        "INK-BLUE",
        "Blue flexo ink",
        "kg",
        ItemType::Component,
        true,
        4_500,
    ),
    (
        "STRAP-1000",
        "Pallet strapping, 1,000 m",
        "pcs",
        ItemType::Component,
        false,
        2_500,
    ),
    (
        "BOX-FOOD",
        "Food-grade box 40x30x20",
        "pcs",
        ItemType::Finished,
        true,
        180,
    ),
    (
        "BOX-RTL",
        "Retail display box",
        "pcs",
        ItemType::Finished,
        true,
        140,
    ),
    (
        "SLV-EPH",
        "Printed sleeve, Emirates Print House",
        "pcs",
        ItemType::Finished,
        true,
        5,
    ),
    (
        "BOX-EXP",
        "Export carton, export grade",
        "pcs",
        ItemType::Finished,
        true,
        450,
    ),
];

impl Seed {
    /// The plant with its lines and stores.
    pub(crate) async fn floor(&mut self) -> Result {
        let plant = Plant::create(
            self.db(),
            Plant {
                name: "Sharjah Industrial Area plant".into(),
                active: true,
                ..Plant::default()
            },
        )
        .await?;
        self.plant = plant.id;
        let make = |code: &str, name: &str, kind: LocationType| {
            let mut location = Location::new(code, name);
            location.location_type = kind;
            location
        };
        let mut rm = make("RM-STORE", "Raw material store", LocationType::CentralStore);
        rm.save(self.db()).await?;
        let mut fg = make("FG-STORE", "Finished goods store", LocationType::Warehouse);
        fg.save(self.db()).await?;
        let mut quarantine = make("QUARANTINE", "Quarantine store", LocationType::Quarantine);
        quarantine.save(self.db()).await?;
        (self.rm, self.fg, self.quarantine) = (rm.id, fg.id, quarantine.id);
        for (code, name, rate) in [
            ("L1", "Line 1, corrugator and converting", 12_000),
            ("L2", "Line 2, flexo printing and sleeves", 9_000),
        ] {
            let line = Line::create(
                self.db(),
                Line {
                    plant_id: plant.id,
                    code: code.into(),
                    name: name.into(),
                    hourly_rate: rate,
                    store_location_id: Some(rm.id),
                    finished_location_id: Some(fg.id),
                    active: true,
                    ..Line::default()
                },
            )
            .await?;
            self.lines.push(line.id);
        }
        for (code, name, unit, kind, tracked, cost) in ITEMS {
            let mut item = Item::new(code, name, unit);
            item.item_type = *kind;
            item.lot_tracked = *tracked;
            item.average_cost = *cost;
            item.save(self.db()).await?;
            self.items.insert(code, item.id);
        }
        Ok(())
    }

    /// The roles held for the plant or a line only.
    pub(crate) async fn assign_scoped_roles(&self) -> Result {
        let plant = plant_scope(self.plant);
        for (email, role, scope) in [
            (MANAGER_EMAIL, PRODUCTION_MANAGER, plant.clone()),
            (PLANNER_EMAIL, PRODUCTION_PLANNER, plant.clone()),
            (QC_EMAIL, QC_INSPECTOR, plant.clone()),
            (SUPERVISOR_EMAIL, SUPERVISOR, line_scope(self.lines[0])),
            (OPERATOR_EMAIL, LINE_OPERATOR, line_scope(self.lines[0])),
        ] {
            let user = self.user(email).await?;
            user.assign_role_in(self.db(), role, &scope).await?;
        }
        // The supervisor also keeps line 2.
        let supervisor = self.user(SUPERVISOR_EMAIL).await?;
        supervisor
            .assign_role_in(self.db(), SUPERVISOR, &line_scope(self.lines[1]))
            .await?;
        Ok(())
    }

    fn lot_status(number: &str) -> LotStatus {
        if number == "KL-2318" {
            LotStatus::Quarantine
        } else {
            LotStatus::Released
        }
    }

    async fn receive(
        &self,
        code: &str,
        lot: Option<i64>,
        location: i64,
        units: i64,
        source: i64,
    ) -> Result {
        let item = Item::find_or_404(self.db(), self.items[code]).await?;
        let quantity = units * 1_000;
        let by = self.user(STORE_EMAIL).await?.id;
        StockMovement::receive(
            item.id,
            lot,
            location,
            quantity,
            item.average_cost,
            value_of(quantity, item.average_cost),
            &Subject::new("inventory.goods_receipt", source),
            Some(by),
        )
        .save(self.db())
        .await?;
        Ok(())
    }

    async fn lot(
        &mut self,
        code: &str,
        number: &'static str,
        supplier: &str,
        expires: Option<i64>,
        status: LotStatus,
    ) -> Result<i64> {
        let lot = fixture!(
            self.db(),
            Lot {
                item_id: self.items[code],
                number: number.to_owned(),
                supplier_lot: Some(supplier.to_owned()),
                expiry_date: expires.map(day),
                status,
                ..Lot::default()
            },
            status
        );
        self.lots.insert(number, lot.id);
        Ok(lot.id)
    }

    /// The opening stock: raw materials in lots (one in quarantine for
    /// incoming inspection, one that its supplier will recall), finished
    /// goods in the finished goods store.
    pub(crate) async fn stock(&mut self) -> Result {
        let rm = self.rm;
        let mut source = 700;
        for (code, number, supplier, expires, units, location) in [
            (
                "KRAFT-LINER",
                "KL-2291",
                "MONDI-77412",
                Some(300),
                3_000,
                rm,
            ),
            (
                "KRAFT-LINER",
                "KL-2305",
                "MONDI-77590",
                Some(320),
                4_000,
                rm,
            ),
            (
                "KRAFT-LINER",
                "KL-2318",
                "MONDI-77631",
                Some(330),
                2_000,
                self.quarantine,
            ),
            (
                "FLUTING-MED",
                "FM-118",
                "SMURFIT-5521",
                Some(400),
                2_000,
                rm,
            ),
            ("SLEEVE-FILM", "SF-9031", "JINDAL-2210", Some(240), 900, rm),
            ("INK-BLUE", "IK-4410", "SUN-CHEM-8801", Some(25), 60, rm),
        ] {
            let status = Self::lot_status(number);
            let lot = self.lot(code, number, supplier, expires, status).await?;
            source += 1;
            self.receive(code, Some(lot), location, units, source)
                .await?;
        }
        self.receive("STRAP-1000", None, rm, 400, 790).await?;
        let fg = self.fg;
        for (code, number, expires, units) in [
            ("BOX-FOOD", "F-BOX-0901", None, 3_000),
            ("BOX-RTL", "F-RTL-0901", None, 2_000),
            ("BOX-EXP", "F-EXP-0902", None, 400),
        ] {
            let lot = self
                .lot(code, number, "", expires, LotStatus::Released)
                .await?;
            source += 1;
            self.receive(code, Some(lot), fg, units, source).await?;
        }
        Ok(())
    }

    /// The released bills of materials (kept by the production manager).
    pub(crate) async fn boms(&mut self) -> Result {
        for (code, hours, components) in [
            (
                "BOX-FOOD",
                "0.01",
                [("KRAFT-LINER", "0.35"), ("FLUTING-MED", "0.15")].as_slice(),
            ),
            ("BOX-RTL", "0.01", [("KRAFT-LINER", "0.25")].as_slice()),
            (
                "SLV-EPH",
                "0.01",
                [("SLEEVE-FILM", "0.004"), ("INK-BLUE", "0.001")].as_slice(),
            ),
        ] {
            let made = self
                .driver
                .post(
                    MANAGER_EMAIL,
                    "/boms",
                    &[("item", &self.items[code].to_string()), ("hours", hours)],
                )
                .await?;
            let id = made
                .redirected_id()
                .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the BOM of {code}"))?;
            for (component, quantity) in components {
                self.driver
                    .post(
                        MANAGER_EMAIL,
                        &format!("/boms/{id}/lines"),
                        &[
                            ("item", &self.items[component].to_string()),
                            ("quantity", quantity),
                        ],
                    )
                    .await?;
            }
            self.driver
                .post(MANAGER_EMAIL, &format!("/boms/{id}/release"), &[])
                .await?;
            self.boms.insert(code, id);
        }
        Ok(())
    }
}
