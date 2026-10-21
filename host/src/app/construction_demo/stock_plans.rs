//! The items and the purchase orders of the demo, the tables the stock part
//! of the story reads (`super::seed_stock`).

use super::people;

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
