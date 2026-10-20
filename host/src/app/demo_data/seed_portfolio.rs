//! The buildings and units.

use property::{Building, Unit, create_building, create_unit};
use renox::prelude::*;

use super::Seed;
use super::portfolio::{BUILDINGS, units};

impl Seed<'_> {
    /// The charge catalogue of the client, which has no default: the early
    /// termination fee, damages and recharged costs, standard-rated for the
    /// demo (the VAT treatment is the client's accountant's decision, open
    /// item A-06 of the spec).
    pub(crate) async fn charge_types(&mut self) -> Result {
        use property::charge_type::{EARLY_TERMINATION_FEE, configure};
        use wl_kernel::VatCategory::Standard;
        for (code, name) in [
            (EARLY_TERMINATION_FEE, "Early termination fee"),
            (property::deposit_forfeit::DAMAGES, "Damages"),
            (
                property::statement_payout::RECHARGED_COSTS,
                "Recharged costs",
            ),
        ] {
            configure(self.db(), code, name, Standard).await?;
        }
        Ok(())
    }

    /// The buildings and units, `available`; the later steps move them.
    pub(crate) async fn portfolio(&mut self) -> Result {
        let (mut buildings, mut made_units) = (0, 0);
        for spec in BUILDINGS {
            let building = match Building::query()
                .where_(Building::CODE.eq(spec.code))
                .first(self.db())
                .await?
            {
                Some(building) => building,
                None => {
                    buildings += 1;
                    create_building(self.db(), spec.new_building()).await?
                }
            };
            self.buildings.insert(spec.code.to_owned(), building);
        }
        for spec in units() {
            let building = self.building(spec.building).clone();
            let unit = match Unit::query()
                .where_(Unit::BUILDING_ID.eq(building.id))
                .where_(Unit::NUMBER.eq(spec.number.as_str()))
                .first(self.db())
                .await?
            {
                Some(unit) => unit,
                None => {
                    made_units += 1;
                    create_unit(self.db(), spec.new_unit(building.id)).await?
                }
            };
            self.units
                .insert((spec.building.to_owned(), spec.number.clone()), unit);
        }
        self.report.add("buildings", buildings);
        self.report.add("units", made_units);
        Ok(())
    }

    /// The building with `code`.
    pub(crate) fn building(&self, code: &str) -> &Building {
        self.buildings
            .get(code)
            .unwrap_or_else(|| panic!("the demo building {code}"))
    }

    /// The unit `number` of building `code`.
    pub(crate) fn unit(&self, code: &str, number: &str) -> &Unit {
        self.units
            .get(&(code.to_owned(), number.to_owned()))
            .unwrap_or_else(|| panic!("the demo unit {code} {number}"))
    }
}
