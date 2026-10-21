//! The IPAs of the demo and what happens to each of them: the table the
//! story of the projects reads (`super::seed_ipas`).

/// What the project manager does with a deduction line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decide {
    /// Accepts it.
    Accept,
    /// Contests it.
    Contest,
    /// Tries to accept it: above AED 25,000 the MD is asked and the line stays
    /// undecided until then.
    AskMd,
    /// Leaves it undecided.
    Wait,
}

/// A deduction the client takes off a certificate.
#[derive(Debug, Clone, Copy)]
pub struct Ded {
    /// `back_charge`, `contra_charge`, `liquidated_damages` or `other`.
    pub kind: &'static str,
    /// The client's reference.
    pub reference: &'static str,
    /// The amount excluding VAT, in dirhams.
    pub amount: i64,
    /// What the project manager does with it.
    pub decide: Decide,
}

/// The client's payment of an invoice.
#[derive(Debug, Clone, Copy)]
pub struct Pay {
    /// Days after the invoice.
    pub after: i64,
    /// The share of the open balance paid, in percent.
    pub percent: i64,
}

/// One IPA of the demo and what happens to it.
#[derive(Debug, Clone, Copy)]
pub struct Cycle {
    /// The project.
    pub code: &'static str,
    /// The number of the IPA in the project.
    pub number: i64,
    /// The cut-off day the draft is made on, days from today.
    pub cutoff: i64,
    /// The cumulative work claimed, in basis points of the contract value.
    pub claimed_bp: i64,
    /// Days after the cut-off that it is submitted.
    pub submit: i64,
    /// The number of the pending IPA it replaces.
    pub replaces: Option<i64>,
    /// Days after the submission that the client certifies, and the certified
    /// share of the claim, in percent.
    pub certify: Option<(i64, i64)>,
    /// The deductions on the certificate.
    pub deductions: &'static [Ded],
    /// Days after the certificate that receivables invoices it.
    pub invoice: Option<i64>,
    /// The client's payment.
    pub pay: Option<Pay>,
}

const NONE: &[Ded] = &[];

const fn paid(after: i64, percent: i64) -> Option<Pay> {
    Some(Pay { after, percent })
}

#[allow(clippy::too_many_arguments)]
const fn cycle(
    code: &'static str,
    number: i64,
    cutoff: i64,
    claimed_bp: i64,
    submit: i64,
    certify: Option<(i64, i64)>,
    invoice: Option<i64>,
    pay: Option<Pay>,
) -> Cycle {
    Cycle {
        code,
        number,
        cutoff,
        claimed_bp,
        submit,
        replaces: None,
        certify,
        deductions: NONE,
        invoice,
        pay,
    }
}

/// The IPAs of the cut-offs of the office fit-out that was completed.
pub const COMPLETED: &[Cycle] = &[
    cycle(
        "OFC-06",
        1,
        -210,
        1_800,
        3,
        Some((14, 95)),
        Some(2),
        paid(20, 100),
    ),
    cycle(
        "OFC-06",
        2,
        -179,
        5_000,
        3,
        Some((14, 96)),
        Some(2),
        paid(20, 100),
    ),
    cycle(
        "OFC-06",
        3,
        -149,
        9_600,
        3,
        Some((14, 97)),
        Some(2),
        paid(25, 100),
    ),
];

/// The IPAs of the cut-off of day -118.
pub const JUNE: &[Cycle] = &[
    cycle(
        "TRM-07",
        1,
        -118,
        2_200,
        2,
        Some((12, 95)),
        Some(2),
        paid(25, 100),
    ),
    cycle(
        "TWR-01",
        1,
        -118,
        1_400,
        3,
        Some((16, 94)),
        Some(2),
        paid(25, 100),
    ),
    cycle(
        "MALL-02",
        1,
        -118,
        1_000,
        3,
        Some((12, 100)),
        Some(2),
        paid(20, 100),
    ),
];

/// The IPAs of the cut-off of day -88.
pub const JULY: &[Cycle] = &[
    Cycle {
        deductions: &[
            Ded {
                kind: "liquidated_damages",
                reference: "LD-TRM-002",
                amount: 12_000,
                decide: Decide::Accept,
            },
            Ded {
                kind: "contra_charge",
                reference: "CC-TRM-011",
                amount: 24_000,
                decide: Decide::Contest,
            },
        ],
        ..cycle("TRM-07", 2, -88, 5_000, 2, Some((14, 96)), Some(2), None)
    },
    Cycle {
        deductions: &[
            Ded {
                kind: "back_charge",
                reference: "BC-TWR-017",
                amount: 8_500,
                decide: Decide::Accept,
            },
            Ded {
                kind: "liquidated_damages",
                reference: "LD-TWR-004",
                amount: 20_000,
                decide: Decide::Accept,
            },
            Ded {
                kind: "contra_charge",
                reference: "CC-TWR-009",
                amount: 6_000,
                decide: Decide::Contest,
            },
        ],
        ..cycle(
            "TWR-01",
            2,
            -88,
            3_300,
            3,
            Some((18, 96)),
            Some(2),
            paid(30, 55),
        )
    },
    cycle(
        "MALL-02",
        2,
        -88,
        3_000,
        3,
        Some((14, 95)),
        Some(2),
        paid(30, 100),
    ),
    // The client pays this one by cheque, which bounces.
    cycle("CLN-03", 1, -88, 1_100, 3, Some((15, 95)), Some(2), None),
];

/// The IPAs of the cut-off of day -57.
pub const AUGUST: &[Cycle] = &[
    // Never certified: the next IPA replaces it.
    cycle("TWR-01", 3, -57, 5_100, 2, None, None, None),
    // Certified, and invoiced late: the invoice is issued and not yet due.
    cycle("MALL-02", 3, -57, 5_300, 3, Some((18, 96)), Some(32), None),
    cycle("CLN-03", 2, -57, 2_600, 3, Some((16, 95)), Some(2), None),
    cycle(
        "RST-04",
        1,
        -57,
        1_500,
        3,
        Some((12, 96)),
        Some(2),
        paid(20, 100),
    ),
];

/// The IPAs of the cut-off of day -26.
pub const SEPTEMBER: &[Cycle] = &[
    Cycle {
        replaces: Some(3),
        deductions: &[
            Ded {
                kind: "liquidated_damages",
                reference: "LD-TWR-011",
                amount: 30_000,
                decide: Decide::AskMd,
            },
            Ded {
                kind: "back_charge",
                reference: "BC-TWR-022",
                amount: 9_000,
                decide: Decide::Wait,
            },
        ],
        ..cycle("TWR-01", 4, -26, 5_700, 2, None, None, None)
    },
    cycle("MALL-02", 4, -26, 6_800, 17, None, None, None),
    cycle("CLN-03", 3, -26, 3_700, 4, None, None, None),
    cycle("RST-04", 2, -26, 3_600, 16, None, None, None),
];

/// The final account of the terminated showroom: the draft of the cut-off of
/// day -57 became it when the project was terminated.
pub const FINAL_ACCOUNT: Cycle = cycle("TRM-07", 3, -57, 6_400, 17, Some((15, 96)), None, None);

/// The BOQ of a project: the share of the contract value (basis points), the
/// number, the description, the unit and the rate in dirhams.
pub const BOQ: [(i64, &str, &str, &str, i64); 5] = [
    (3_000, "1.1", "Gypsum partitions and ceilings", "m2", 180),
    (
        2_500,
        "2.1",
        "Joinery, doors and built-in units",
        "nr",
        2_500,
    ),
    (2_000, "3.1", "Flooring and wall finishes", "m2", 220),
    (1_000, "4.1", "Painting and decorative finishes", "m2", 38),
    (1_500, "5.1", "MEP first and second fix", "m", 95),
];
