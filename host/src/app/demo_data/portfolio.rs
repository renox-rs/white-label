//! The buildings and units of the demo company (DEMO-DATA.md, Property): two
//! residential towers in Jumeirah Village, a mixed-use building in Business
//! Bay, a warehouse block in Al Quoz, and, to show Tawtheeq and the other
//! emirates, a residence in Abu Dhabi and one in Sharjah. Communities are
//! generic; no real building, developer or plot appears.

use property::{NewBuilding, NewUnit, UnitType, UnitUse};

/// One building.
pub struct BuildingSpec {
    /// The short code.
    pub code: &'static str,
    /// The English name.
    pub en: &'static str,
    /// The Arabic name.
    pub ar: &'static str,
    /// The emirate key.
    pub emirate: &'static str,
    /// The community.
    pub community: &'static str,
    /// The plot number (a demo one).
    pub plot: &'static str,
    /// The premise number (DEWA, a demo one).
    pub premise: &'static str,
}

/// The buildings, in the order they are created.
pub const BUILDINGS: &[BuildingSpec] = &[
    BuildingSpec {
        code: "JVT-A",
        en: "Palm Grove Tower A",
        ar: "برج بالم جروف أ",
        emirate: "dubai",
        community: "Jumeirah Village Circle",
        plot: "DEMO-611-0101",
        premise: "DEMO-2650101",
    },
    BuildingSpec {
        code: "JVT-B",
        en: "Palm Grove Tower B",
        ar: "برج بالم جروف ب",
        emirate: "dubai",
        community: "Jumeirah Village Circle",
        plot: "DEMO-611-0102",
        premise: "DEMO-2650102",
    },
    BuildingSpec {
        code: "BBY",
        en: "Palm Grove Business Bay Plaza",
        ar: "بالم جروف بلازا الخليج التجاري",
        emirate: "dubai",
        community: "Business Bay",
        plot: "DEMO-345-0201",
        premise: "DEMO-3450201",
    },
    BuildingSpec {
        code: "AQZ",
        en: "Al Quoz Warehouse Block 3",
        ar: "مجمع مستودعات القوز 3",
        emirate: "dubai",
        community: "Al Quoz Industrial 3",
        plot: "DEMO-368-0301",
        premise: "DEMO-3680301",
    },
    BuildingSpec {
        code: "ADR",
        en: "Palm Grove Reem Residences",
        ar: "مساكن بالم جروف الريم",
        emirate: "abu_dhabi",
        community: "Al Reem Island",
        plot: "DEMO-AD-0401",
        premise: "DEMO-AD-4010",
    },
    BuildingSpec {
        code: "SHJ",
        en: "Palm Grove Majaz Heights",
        ar: "مرتفعات بالم جروف المجاز",
        emirate: "sharjah",
        community: "Al Majaz",
        plot: "DEMO-SH-0501",
        premise: "DEMO-SH-5010",
    },
];

impl BuildingSpec {
    /// The building to create.
    pub fn new_building(&self) -> NewBuilding {
        NewBuilding {
            code: self.code.to_owned(),
            name_en: self.en.to_owned(),
            name_ar: Some(self.ar.to_owned()),
            emirate: self.emirate.to_owned(),
            community: Some(self.community.to_owned()),
            plot_number: Some(self.plot.to_owned()),
            premise_number: Some(self.premise.to_owned()),
        }
    }
}

/// One unit.
pub struct UnitSpec {
    /// The building's code.
    pub building: &'static str,
    /// The unit number.
    pub number: String,
    /// The kind of space.
    pub unit_type: UnitType,
    /// Residential or commercial.
    pub usage: UnitUse,
    /// Bedrooms, for homes.
    pub bedrooms: Option<i64>,
    /// The size in square feet.
    pub area: i64,
    /// The asking rent a year, in dirhams.
    pub rent: i64,
}

impl UnitSpec {
    /// The unit to create in building `building_id`.
    pub fn new_unit(&self, building_id: i64) -> NewUnit {
        NewUnit {
            building_id,
            floor_id: None,
            number: self.number.clone(),
            unit_type: self.unit_type,
            usage: self.usage,
            area_sqft_milli: Some(self.area * 1000),
            bedrooms: self.bedrooms,
            asking_rent: Some(self.rent * 100),
            notes: None,
        }
    }
}

fn home(building: &'static str, number: String, bedrooms: i64, area: i64, rent: i64) -> UnitSpec {
    UnitSpec {
        building,
        number,
        unit_type: UnitType::Apartment,
        usage: UnitUse::Residential,
        bedrooms: Some(bedrooms),
        area,
        rent,
    }
}

fn commercial(
    building: &'static str,
    number: String,
    unit_type: UnitType,
    area: i64,
    rent: i64,
) -> UnitSpec {
    UnitSpec {
        building,
        number,
        unit_type,
        usage: UnitUse::Commercial,
        bedrooms: None,
        area,
        rent,
    }
}

/// Every unit of the demo, 62 of them: rents are plausible market levels,
/// off the round numbers by the floor and a small step by position.
pub fn units() -> Vec<UnitSpec> {
    let mut all = Vec::new();
    // Tower A: 7 floors, a one- and a two-bedroom each.
    for floor in 1..=7_i64 {
        all.push(home(
            "JVT-A",
            format!("{floor}01"),
            1,
            760 + floor * 6,
            54_000 + floor * 1_750 + (floor % 3) * 250,
        ));
        all.push(home(
            "JVT-A",
            format!("{floor}02"),
            2,
            1_180 + floor * 8,
            81_500 + floor * 2_100 + (floor % 3) * 300,
        ));
    }
    // Tower B: 6 floors, a studio and a one-bedroom each.
    for floor in 1..=6_i64 {
        all.push(home(
            "JVT-B",
            format!("{floor}01"),
            0,
            430 + floor * 4,
            38_500 + floor * 1_300 + (floor % 2) * 400,
        ));
        all.push(home(
            "JVT-B",
            format!("{floor}02"),
            1,
            790 + floor * 5,
            56_000 + floor * 1_650 + (floor % 2) * 350,
        ));
    }
    // Business Bay: shops on the ground floor, offices, and four homes on top.
    for (number, rent) in [("G01", 236_500), ("G02", 258_000)] {
        all.push(commercial(
            "BBY",
            number.to_owned(),
            UnitType::Shop,
            1_150,
            rent,
        ));
    }
    for floor in 3..=6_i64 {
        for side in 1..=2_i64 {
            all.push(commercial(
                "BBY",
                format!("{floor}0{side}"),
                UnitType::Office,
                1_020 + side * 160,
                94_500 + floor * 6_200 + side * 11_500,
            ));
        }
    }
    for (number, rent) in [("801", 112_500), ("802", 118_900)] {
        all.push(home("BBY", number.to_owned(), 2, 1_240, rent));
    }
    // Al Quoz: eight warehouse bays.
    for bay in 1..=8_i64 {
        all.push(commercial(
            "AQZ",
            format!("W{bay}"),
            UnitType::Warehouse,
            4_800 + bay * 350,
            176_000 + bay * 9_250,
        ));
    }
    // Abu Dhabi: Reem Island, one- and two-bedrooms.
    for floor in 1..=4_i64 {
        all.push(home(
            "ADR",
            format!("{floor}01"),
            1,
            820 + floor * 5,
            63_500 + floor * 1_900,
        ));
        all.push(home(
            "ADR",
            format!("{floor}02"),
            2,
            1_260 + floor * 6,
            104_500 + floor * 2_400,
        ));
    }
    // Sharjah: Al Majaz, two-bedrooms.
    for floor in 1..=4_i64 {
        all.push(home(
            "SHJ",
            format!("{floor}01"),
            2,
            1_090 + floor * 4,
            38_500 + floor * 1_100,
        ));
        all.push(home(
            "SHJ",
            format!("{floor}02"),
            3,
            1_480 + floor * 5,
            51_500 + floor * 1_400,
        ));
    }
    all
}
