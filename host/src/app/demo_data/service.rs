//! The maintenance side of the demo: the vendors' contracts and papers, and
//! the tickets of tenants, buildings and planned visits.

/// A vendor's contract (AMC).
pub struct ContractPlan {
    /// The vendor's English name.
    pub vendor: &'static str,
    /// The title.
    pub title: &'static str,
    /// The first day, in days from today.
    pub start: i64,
    /// The last day, in days from today.
    pub end: i64,
    /// What it covers: the category and the buildings.
    pub category: &'static str,
    /// The buildings.
    pub buildings: &'static [&'static str],
    /// Where it stands.
    pub standing: ContractStanding,
}

/// Where a contract is at the end of the seed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractStanding {
    /// Not signed yet.
    Draft,
    /// In force.
    Active,
    /// In its renewal notice window.
    Expiring,
    /// Ended without renewal.
    Expired,
    /// Ended early by the facility manager.
    Terminated,
}

/// The contracts.
pub const CONTRACTS: &[ContractPlan] = &[
    ContractPlan {
        vendor: "Skyhigh Lifts and Escalators LLC",
        title: "Lifts annual maintenance, Towers and Business Bay",
        start: -315,
        end: 50,
        category: "lifts",
        buildings: &["JVT-A", "JVT-B", "BBY"],
        standing: ContractStanding::Expiring,
    },
    ContractPlan {
        vendor: "Safeguard Fire Systems LLC",
        title: "Fire and life safety annual maintenance",
        start: -200,
        end: 165,
        category: "fire_life_safety",
        buildings: &["JVT-A", "JVT-B", "BBY", "AQZ", "ADR", "SHJ"],
        standing: ContractStanding::Active,
    },
    ContractPlan {
        vendor: "Coolbreeze AC Services LLC",
        title: "Chillers and AC annual maintenance, Towers",
        start: -100,
        end: 265,
        category: "ac_hvac",
        buildings: &["JVT-A", "JVT-B"],
        standing: ContractStanding::Active,
    },
    ContractPlan {
        vendor: "BrightClean Facility Services LLC",
        title: "Common area cleaning, Business Bay and Tower A",
        start: -420,
        end: -55,
        category: "cleaning",
        buildings: &["BBY", "JVT-A"],
        standing: ContractStanding::Expired,
    },
    ContractPlan {
        vendor: "Pestguard Gulf LLC",
        title: "Pest control, Business Bay and Al Quoz",
        start: -250,
        end: 115,
        category: "pest_control",
        buildings: &["BBY", "AQZ"],
        standing: ContractStanding::Terminated,
    },
    ContractPlan {
        vendor: "Spark Electrical Contracting LLC",
        title: "Electrical maintenance, Tower B",
        start: 15,
        end: 380,
        category: "electrical",
        buildings: &["JVT-B"],
        standing: ContractStanding::Draft,
    },
    ContractPlan {
        vendor: "BrightClean Facility Services LLC",
        title: "Common area cleaning, Abu Dhabi and Sharjah",
        start: 30,
        end: 395,
        category: "cleaning",
        buildings: &["ADR", "SHJ"],
        standing: ContractStanding::Draft,
    },
];

/// Who works on a ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Worker {
    /// Nobody yet.
    Nobody,
    /// A vendor (English name).
    Vendor(&'static str),
    /// The in-house technician.
    Technician,
}

/// Where a ticket stands at the end of the seed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// Raised.
    Open,
    /// With its worker.
    Assigned,
    /// Work started.
    InProgress,
    /// Paused for this reason.
    OnHold(&'static str),
    /// Work done, waiting for the sign-off.
    Resolved,
    /// Done and closed by staff.
    Closed,
    /// Closed long ago (the system closed it).
    Done,
    /// Cancelled for this reason.
    Cancelled(&'static str),
    /// A repair charged to the tenant: charge lines, done, closed, invoiced.
    Charged,
}

/// One ticket.
#[derive(Debug, Clone, Copy)]
pub struct TicketPlan {
    /// The building and, for a unit's ticket, the unit.
    pub place: (&'static str, Option<&'static str>),
    /// The category.
    pub category: &'static str,
    /// The title.
    pub title: &'static str,
    /// The tenant who asked, if one did.
    pub tenant: Option<&'static str>,
    /// Who works on it.
    pub worker: Worker,
    /// Days ago it was raised.
    pub age: i64,
    /// Where it stands.
    pub standing: Standing,
}

const fn ticket(
    place: (&'static str, Option<&'static str>),
    category: &'static str,
    title: &'static str,
    tenant: Option<&'static str>,
    worker: Worker,
    age: i64,
    standing: Standing,
) -> TicketPlan {
    TicketPlan {
        place,
        category,
        title,
        tenant,
        worker,
        age,
        standing,
    }
}

use Standing::{Assigned, Cancelled, Charged, Closed, Done, InProgress, OnHold, Open, Resolved};
use Worker::{Nobody, Technician, Vendor};

const COOL: &str = "Coolbreeze AC Services LLC";
const PLUMB: &str = "Al Noor Trading LLC";
const SPARK: &str = "Spark Electrical Contracting LLC";
const PEST: &str = "Pestguard Gulf LLC";
const CLEAN: &str = "BrightClean Facility Services LLC";
const FIRE: &str = "Safeguard Fire Systems LLC";
const LIFT: &str = "Skyhigh Lifts and Escalators LLC";

/// The tickets, oldest first.
pub const TICKETS: &[TicketPlan] = &[
    // Done long ago.
    ticket(
        ("JVT-A", Some("101")),
        "ac_hvac",
        "AC not cooling in the living room",
        Some("Omar Faisal Al Shamsi"),
        Vendor(COOL),
        430,
        Done,
    ),
    ticket(
        ("JVT-A", Some("102")),
        "plumbing",
        "Kitchen sink blocked",
        Some("Priya Raghunathan"),
        Vendor(PLUMB),
        410,
        Done,
    ),
    ticket(
        ("JVT-B", Some("102")),
        "electrical",
        "Sockets in the bedroom have no power",
        Some("Lukas Meier"),
        Vendor(SPARK),
        395,
        Done,
    ),
    ticket(
        ("BBY", Some("G01")),
        "pest_control",
        "Cockroaches in the kitchen storage",
        Some("Desert Rose Cafe LLC"),
        Vendor(PEST),
        380,
        Done,
    ),
    ticket(
        ("JVT-A", None),
        "lifts",
        "Lift 2 stuck between floors",
        None,
        Vendor(LIFT),
        372,
        Done,
    ),
    ticket(
        ("ADR", Some("101")),
        "ac_hvac",
        "AC leaking water onto the balcony",
        Some("Mahmoud Adel Farouk"),
        Vendor(COOL),
        351,
        Done,
    ),
    ticket(
        ("JVT-B", Some("402")),
        "plumbing",
        "Bathroom leak from the ceiling",
        Some("Maria Teresa Dizon"),
        Vendor(PLUMB),
        330,
        Done,
    ),
    ticket(
        ("BBY", None),
        "fire_life_safety",
        "Quarterly fire alarm test, Business Bay",
        None,
        Vendor(FIRE),
        318,
        Done,
    ),
    ticket(
        ("SHJ", Some("101")),
        "general",
        "Front door lock stuck",
        Some("Ibrahim Saleh Al Kaabi"),
        Technician,
        301,
        Done,
    ),
    ticket(
        ("AQZ", Some("W1")),
        "electrical",
        "Warehouse lighting circuit tripping",
        Some("Meridian Logistics LLC"),
        Vendor(SPARK),
        287,
        Done,
    ),
    ticket(
        ("JVT-A", Some("401")),
        "ac_hvac",
        "Thermostat not responding",
        Some("Jose Miguel Santos"),
        Vendor(COOL),
        266,
        Done,
    ),
    ticket(
        ("JVT-A", None),
        "cleaning",
        "Lobby deep clean before the audit",
        None,
        Vendor(CLEAN),
        249,
        Done,
    ),
    ticket(
        ("BBY", Some("301")),
        "ac_hvac",
        "Office AC too noisy",
        Some("Skyline Dental Clinic LLC"),
        Vendor(COOL),
        233,
        Done,
    ),
    ticket(
        ("JVT-B", None),
        "fire_life_safety",
        "Quarterly fire alarm test, Tower B",
        None,
        Vendor(FIRE),
        221,
        Done,
    ),
    ticket(
        ("JVT-A", Some("501")),
        "plumbing",
        "Water heater not heating",
        Some("Daniel Whitaker"),
        Vendor(PLUMB),
        207,
        Done,
    ),
    ticket(
        ("ADR", Some("102")),
        "general",
        "Intercom handset not working",
        Some("Khalid Saeed Al Mansoori"),
        Technician,
        190,
        Done,
    ),
    ticket(
        ("SHJ", Some("102")),
        "ac_hvac",
        "AC remote lost, unit will not switch off",
        Some("Sunita Rao"),
        Vendor(COOL),
        176,
        Done,
    ),
    ticket(
        ("JVT-A", None),
        "lifts",
        "Quarterly lift service",
        None,
        Vendor(LIFT),
        160,
        Done,
    ),
    ticket(
        ("BBY", Some("G02")),
        "security",
        "Shop shutter lock jammed",
        Some("Northgate Trading FZCO"),
        Technician,
        148,
        Done,
    ),
    ticket(
        ("JVT-A", Some("202")),
        "electrical",
        "Corridor light flickering",
        Some("Sneha Kulkarni"),
        Vendor(SPARK),
        133,
        Done,
    ),
    ticket(
        ("JVT-B", Some("101")),
        "pest_control",
        "Ants in the kitchen",
        Some("Camille Laurent"),
        Vendor(PEST),
        119,
        Done,
    ),
    ticket(
        ("BBY", None),
        "lifts",
        "Quarterly lift service, Business Bay",
        None,
        Vendor(LIFT),
        104,
        Done,
    ),
    ticket(
        ("JVT-A", Some("301")),
        "plumbing",
        "Leak under the basin",
        Some("Imran Qureshi"),
        Vendor(PLUMB),
        92,
        Done,
    ),
    ticket(
        ("AQZ", None),
        "fire_life_safety",
        "Fire extinguishers refilled",
        None,
        Vendor(FIRE),
        81,
        Done,
    ),
    ticket(
        ("ADR", Some("201")),
        "ac_hvac",
        "AC filters need cleaning",
        Some("Hassan Jaber Al Rumaithi"),
        Vendor(COOL),
        68,
        Done,
    ),
    // Closed by staff after the tenant confirmed.
    ticket(
        ("JVT-A", Some("102")),
        "electrical",
        "Replace burnt-out bathroom downlight",
        Some("Priya Raghunathan"),
        Vendor(SPARK),
        47,
        Closed,
    ),
    ticket(
        ("JVT-B", Some("402")),
        "general",
        "Mailbox lock replaced",
        Some("Maria Teresa Dizon"),
        Technician,
        41,
        Closed,
    ),
    ticket(
        ("BBY", Some("801")),
        "plumbing",
        "Washing machine tap leaking",
        Some("Rania Khalil Nasser"),
        Vendor(PLUMB),
        33,
        Closed,
    ),
    ticket(
        ("SHJ", Some("201")),
        "ac_hvac",
        "AC not cooling after move-in",
        Some("Ahmed Nabil Khalifa"),
        Vendor(COOL),
        12,
        Closed,
    ),
    // Repairs charged to the tenant.
    ticket(
        ("JVT-A", Some("401")),
        "tenant_repair",
        "Broken window handle (tenant damage)",
        Some("Jose Miguel Santos"),
        Technician,
        24,
        Charged,
    ),
    ticket(
        ("JVT-B", Some("102")),
        "tenant_repair",
        "Replace kitchen cabinet door damaged by tenant",
        Some("Lukas Meier"),
        Technician,
        15,
        Charged,
    ),
    // Cancelled.
    ticket(
        ("JVT-A", Some("201")),
        "plumbing",
        "Reported leak, found to be condensation",
        Some("Arjun Menon"),
        Nobody,
        22,
        Cancelled("Duplicate of the AC ticket; no leak found"),
    ),
    ticket(
        ("BBY", Some("402")),
        "general",
        "Request to repaint the meeting room",
        Some("Falcon Eye Security Services LLC"),
        Nobody,
        9,
        Cancelled("Tenant fit-out, not the landlord's work"),
    ),
    // Resolved, waiting for the tenant.
    ticket(
        ("JVT-A", Some("102")),
        "ac_hvac",
        "AC makes a rattling noise",
        Some("Priya Raghunathan"),
        Vendor(COOL),
        6,
        Resolved,
    ),
    ticket(
        ("ADR", Some("101")),
        "plumbing",
        "Shower mixer dripping",
        Some("Mahmoud Adel Farouk"),
        Vendor(PLUMB),
        4,
        Resolved,
    ),
    // On hold.
    ticket(
        ("JVT-B", Some("502")),
        "electrical",
        "Replace the intercom power supply",
        Some("Aisha Mohammed Al Blooshi"),
        Vendor(SPARK),
        11,
        OnHold("parts"),
    ),
    ticket(
        ("BBY", Some("G01")),
        "ac_hvac",
        "Cafe split unit needs a new compressor",
        Some("Desert Rose Cafe LLC"),
        Vendor(COOL),
        8,
        OnHold("approval"),
    ),
    // Work started; the first two are past their resolution time.
    ticket(
        ("JVT-A", None),
        "lifts",
        "Lift 1 door closes too slowly",
        None,
        Vendor(LIFT),
        7,
        InProgress,
    ),
    ticket(
        ("BBY", Some("301")),
        "plumbing",
        "Pantry sink leaking onto the floor below",
        Some("Skyline Dental Clinic LLC"),
        Vendor(PLUMB),
        5,
        InProgress,
    ),
    ticket(
        ("JVT-B", Some("202")),
        "general",
        "Balcony door will not lock",
        Some("Hassan Jaber Al Rumaithi"),
        Technician,
        1,
        InProgress,
    ),
    // Assigned, waiting for the visit.
    ticket(
        ("SHJ", Some("102")),
        "electrical",
        "Replace faulty circuit breaker",
        Some("Sunita Rao"),
        Vendor(SPARK),
        3,
        Assigned,
    ),
    ticket(
        ("AQZ", Some("W2")),
        "security",
        "Gate access card reader not working",
        Some("Northgate Trading FZCO"),
        Technician,
        2,
        Assigned,
    ),
    ticket(
        ("JVT-A", Some("401")),
        "pest_control",
        "Pest control visit requested",
        Some("Jose Miguel Santos"),
        Vendor(PEST),
        1,
        Assigned,
    ),
    // New today.
    ticket(
        ("JVT-A", Some("402")),
        "ac_hvac",
        "AC not cooling, new tenant",
        Some("Angelica Reyes"),
        Nobody,
        0,
        Open,
    ),
    ticket(
        ("JVT-B", Some("101")),
        "general",
        "Garbage chute blocked on floor 1",
        Some("Camille Laurent"),
        Nobody,
        0,
        Open,
    ),
    ticket(
        ("ADR", Some("102")),
        "plumbing",
        "No hot water in the master bathroom",
        Some("Khalid Saeed Al Mansoori"),
        Nobody,
        0,
        Open,
    ),
];
