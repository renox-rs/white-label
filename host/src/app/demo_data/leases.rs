//! The leases of the demo company: who rents which unit, from when, for how
//! much, and what has happened to the lease and its money since. The order
//! matters: a unit gets its history first (the lease that ended or was
//! renewed), then its current lease.

use super::seed_documents::Papers;

/// What became of a lease (the last thing the demo does to it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fate {
    /// Still being prepared.
    Draft,
    /// Submitted, waiting for signatures and the deposit.
    Pending,
    /// In force.
    Active,
    /// Cancelled before it started, with nothing received.
    Cancelled,
    /// Cheques and the deposit were received; the leasing agent asked to
    /// cancel and a property manager has not decided.
    CancelAsked,
    /// The same, and a property manager approved: refunds and returns ran.
    CancelApproved,
    /// In force, and in the renewal window (the daily rule opened it).
    Window,
    /// In the window, with the property manager's decision.
    Decided(&'static str),
    /// In the window, deciding to renew on the same terms, with the renewal
    /// prepared and sent for signature.
    RenewalSent,
    /// Ended without renewal; the tenant holds over.
    Expired,
    /// The tenant (or the landlord) gave notice.
    Notice {
        /// A tenant's notice (else the landlord's).
        by_tenant: bool,
        /// The planned move-out, in days from today.
        move_out: i64,
        /// The early termination penalty in dirhams.
        penalty: i64,
    },
    /// Ended and moved out.
    Terminated {
        /// The day the notice was given, in days from today.
        notice: i64,
        /// The move-out day, in days from today.
        move_out: i64,
        /// The inspection found defects that need work.
        defects: bool,
    },
    /// Renewed by a second version, which is in force.
    Renewed,
    /// Amended by a second version, which is in force.
    Amended {
        /// The day the change applies from, in days from today.
        effective: i64,
        /// The annual rent of the amendment in dirhams.
        rent: i64,
    },
}

/// What happened to the money of a lease, besides the instalments being
/// invoiced on their due dates and paid by cheque.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Quirk {
    /// Nothing: every past cheque clears.
    None,
    /// The cheque of this instalment (from 1) bounced and is still owed.
    Bounced(usize),
    /// It bounced, and the tenant paid by transfer: the cheque is replaced.
    BouncedReplaced(usize),
    /// It bounced and was presented again; it cleared.
    BouncedCleared(usize),
    /// It bounced and was handed to a legal case.
    BouncedLegal(usize),
    /// There is no cheque for the last instalments due: this many invoices
    /// are overdue.
    Unpaid(usize),
    /// The tenant paid the first instalment by transfer and half of the
    /// second.
    PartPaid,
    /// The tenant disputes the invoice of this instalment.
    Disputed(usize),
    /// The invoice of this instalment was issued twice as large by mistake and
    /// is credited in full.
    Credited(usize),
    /// The invoice of this instalment is deemed uncollectable and written off.
    WrittenOff(usize),
    /// Every instalment is paid by cheque, the first by bank transfer.
    FirstByTransfer,
}

/// One lease of the demo.
#[derive(Debug, Clone, Copy)]
pub struct Plan {
    /// Its name in the demo, for the steps that refer to it.
    pub key: &'static str,
    /// The building code and the unit number.
    pub unit: (&'static str, &'static str),
    /// The tenant's English name.
    pub tenant: &'static str,
    /// The first day, in days from today.
    pub start: i64,
    /// The length of the term in months.
    pub months: u32,
    /// The annual rent in dirhams; the unit's asking rent when 0.
    pub rent: i64,
    /// The rent instalments.
    pub instalments: u32,
    /// The security deposit in dirhams; 0 for none.
    pub deposit: i64,
    /// What becomes of it.
    pub fate: Fate,
    /// What happens to its money.
    pub quirk: Quirk,
    /// The tenant's papers.
    pub papers: Papers,
}

const fn plan(
    key: &'static str,
    unit: (&'static str, &'static str),
    tenant: &'static str,
    start: i64,
    instalments: u32,
    deposit: i64,
    fate: Fate,
) -> Plan {
    Plan {
        key,
        unit,
        tenant,
        start,
        months: 12,
        rent: 0,
        instalments,
        deposit,
        fate,
        quirk: Quirk::None,
        papers: Papers::Full,
    }
}

impl Plan {
    const fn quirk(mut self, quirk: Quirk) -> Self {
        self.quirk = quirk;
        self
    }

    const fn papers(mut self, papers: Papers) -> Self {
        self.papers = papers;
        self
    }
}

use Fate::{
    Active, Amended, CancelApproved, CancelAsked, Cancelled, Decided, Draft, Expired, Notice,
    Pending, RenewalSent, Renewed, Terminated, Window,
};
use Quirk::{
    Bounced, BouncedCleared, BouncedLegal, BouncedReplaced, Credited, Disputed, FirstByTransfer,
    PartPaid, Unpaid, WrittenOff,
};

/// The leases of the demo, in the order they are made.
pub const PLANS: &[Plan] = &[
    // Tower A.
    plan(
        "a101",
        ("JVT-A", "101"),
        "Omar Faisal Al Shamsi",
        -200,
        4,
        2_800,
        Active,
    )
    .quirk(Bounced(2)),
    plan(
        "a102",
        ("JVT-A", "102"),
        "Priya Raghunathan",
        -120,
        12,
        4_100,
        Active,
    )
    .quirk(BouncedReplaced(2)),
    plan(
        "a201",
        ("JVT-A", "201"),
        "Arjun Menon",
        -305,
        4,
        3_100,
        Window,
    ),
    plan(
        "a202",
        ("JVT-A", "202"),
        "Sneha Kulkarni",
        -335,
        2,
        4_200,
        Decided("changed_terms"),
    ),
    plan(
        "a301",
        ("JVT-A", "301"),
        "Imran Qureshi",
        -275,
        6,
        3_500,
        RenewalSent,
    ),
    plan(
        "a302",
        ("JVT-A", "302"),
        "Zainab Ahmed Malik",
        -250,
        4,
        4_800,
        Notice {
            by_tenant: true,
            move_out: 45,
            penalty: 4_000,
        },
    )
    .quirk(Unpaid(1)),
    plan(
        "a401",
        ("JVT-A", "401"),
        "Jose Miguel Santos",
        -60,
        4,
        3_200,
        Active,
    )
    .papers(Papers::IdExpiresIn(9)),
    plan(
        "a402",
        ("JVT-A", "402"),
        "Angelica Reyes",
        -25,
        1,
        5_000,
        Active,
    )
    .papers(Papers::IdExpiresIn(17)),
    plan(
        "a501",
        ("JVT-A", "501"),
        "Daniel Whitaker",
        -600,
        4,
        3_800,
        Renewed,
    ),
    plan(
        "a502",
        ("JVT-A", "502"),
        "Elena Petrova",
        -150,
        4,
        4_600,
        Amended {
            effective: 12,
            rent: 90_000,
        },
    ),
    plan(
        "a601",
        ("JVT-A", "601"),
        "Thomas Okafor",
        -377,
        2,
        3_300,
        Expired,
    )
    .quirk(WrittenOff(2)),
    plan(
        "a702",
        ("JVT-A", "702"),
        "Fatuma Ali Hassan",
        20,
        4,
        4_700,
        Draft,
    ),
    // Tower B.
    plan(
        "b101",
        ("JVT-B", "101"),
        "Camille Laurent",
        -92,
        4,
        1_950,
        Active,
    )
    .quirk(PartPaid),
    plan(
        "b102",
        ("JVT-B", "102"),
        "Lukas Meier",
        -210,
        12,
        2_800,
        Active,
    )
    .quirk(Unpaid(3))
    .papers(Papers::IdExpiresIn(26)),
    plan(
        "b201",
        ("JVT-B", "201"),
        "Hassan Jaber Al Rumaithi",
        -520,
        4,
        2_900,
        Terminated {
            notice: -210,
            move_out: -158,
            defects: true,
        },
    ),
    plan(
        "b202",
        ("JVT-B", "202"),
        "Khalid Saeed Al Mansoori",
        -560,
        2,
        3_000,
        Terminated {
            notice: -250,
            move_out: -200,
            defects: false,
        },
    ),
    plan(
        "b301",
        ("JVT-B", "301"),
        "Salma Tarek Mansour",
        10,
        2,
        0,
        Pending,
    ),
    plan(
        "b302",
        ("JVT-B", "302"),
        "Nikhil Deshpande",
        25,
        4,
        3_400,
        Draft,
    )
    .papers(Papers::NoPassport),
    plan(
        "b401",
        ("JVT-B", "401"),
        "Marwa Saleh Al Ameri",
        30,
        4,
        3_600,
        Draft,
    )
    .papers(Papers::VisaExpired(40)),
    plan(
        "b402",
        ("JVT-B", "402"),
        "Maria Teresa Dizon",
        -150,
        4,
        3_600,
        Active,
    )
    .quirk(Disputed(2)),
    plan(
        "b501",
        ("JVT-B", "501"),
        "Fatima Yousef Haddad",
        15,
        4,
        0,
        Cancelled,
    ),
    plan(
        "b502",
        ("JVT-B", "502"),
        "Aisha Mohammed Al Blooshi",
        18,
        4,
        4_100,
        CancelAsked,
    ),
    plan(
        "b601",
        ("JVT-B", "601"),
        "Yousef Ibrahim Al Hammadi",
        12,
        4,
        4_300,
        CancelApproved,
    ),
    // Business Bay.
    plan(
        "bbg1",
        ("BBY", "G01"),
        "Desert Rose Cafe LLC",
        -250,
        4,
        23_650,
        Active,
    )
    .quirk(BouncedLegal(2)),
    plan(
        "bbg2",
        ("BBY", "G02"),
        "Northgate Trading FZCO",
        -100,
        4,
        25_800,
        Active,
    )
    .quirk(FirstByTransfer),
    plan(
        "bb301",
        ("BBY", "301"),
        "Skyline Dental Clinic LLC",
        -180,
        6,
        11_000,
        Active,
    )
    .quirk(BouncedReplaced(3)),
    plan(
        "bb302",
        ("BBY", "302"),
        "Pearl Tech Solutions LLC",
        -330,
        4,
        9_900,
        Window,
    )
    .quirk(Credited(2)),
    plan(
        "bb401",
        ("BBY", "401"),
        "Meridian Logistics LLC",
        15,
        4,
        12_000,
        Draft,
    ),
    plan(
        "bb402",
        ("BBY", "402"),
        "Falcon Eye Security Services LLC",
        -45,
        12,
        11_500,
        Active,
    ),
    plan(
        "bb501",
        ("BBY", "501"),
        "Bluewave Logistics FZE",
        -385,
        2,
        12_400,
        Expired,
    )
    .quirk(WrittenOff(2)),
    plan(
        "bb801",
        ("BBY", "801"),
        "Rania Khalil Nasser",
        -75,
        4,
        5_625,
        Active,
    ),
    plan(
        "bb802",
        ("BBY", "802"),
        "Noura Salem Al Mazrouei",
        -480,
        4,
        5_900,
        Renewed,
    ),
    // Al Quoz.
    plan(
        "aq1",
        ("AQZ", "W1"),
        "Meridian Logistics LLC",
        -220,
        4,
        18_500,
        Active,
    )
    .quirk(BouncedLegal(3)),
    plan(
        "aq2",
        ("AQZ", "W2"),
        "Northgate Trading FZCO",
        -40,
        2,
        19_500,
        Active,
    ),
    plan(
        "sj401",
        ("SHJ", "401"),
        "Pavel Novak",
        -420,
        2,
        2_500,
        Terminated {
            notice: -240,
            move_out: -200,
            defects: true,
        },
    ),
    plan(
        "aq3",
        ("AQZ", "W3"),
        "Pearl Tech Solutions LLC",
        -450,
        4,
        20_400,
        Terminated {
            notice: -130,
            move_out: -88,
            defects: true,
        },
    ),
    // Abu Dhabi.
    plan(
        "ad101",
        ("ADR", "101"),
        "Mahmoud Adel Farouk",
        -180,
        4,
        3_300,
        Active,
    )
    .quirk(BouncedCleared(2)),
    plan(
        "ad102",
        ("ADR", "102"),
        "Khalid Saeed Al Mansoori",
        -60,
        4,
        5_400,
        Active,
    ),
    plan(
        "ad201",
        ("ADR", "201"),
        "Hassan Jaber Al Rumaithi",
        -30,
        2,
        5_750,
        Active,
    ),
    // Sharjah.
    plan(
        "sj402",
        ("SHJ", "402"),
        "Dina Qasim Al Zahrani",
        14,
        4,
        2_750,
        CancelApproved,
    ),
    plan(
        "sj101",
        ("SHJ", "101"),
        "Ibrahim Saleh Al Kaabi",
        -170,
        4,
        1_960,
        Active,
    )
    .quirk(Credited(2)),
    plan("sj102", ("SHJ", "102"), "Sunita Rao", -95, 4, 2_650, Active).quirk(PartPaid),
    plan(
        "sj201",
        ("SHJ", "201"),
        "Ahmed Nabil Khalifa",
        -20,
        2,
        2_000,
        Active,
    ),
    plan(
        "sj202",
        ("SHJ", "202"),
        "Grace Mwangi",
        -230,
        4,
        2_700,
        Notice {
            by_tenant: false,
            move_out: 60,
            penalty: 0,
        },
    ),
    plan(
        "sj302",
        ("SHJ", "302"),
        "Dina Qasim Al Zahrani",
        -260,
        4,
        2_750,
        Window,
    ),
];
