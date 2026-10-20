//! The company's own books: what it pays its suppliers, its budget, the
//! expense claims of staff, the bank statements and the opening cash balances.

/// How a supplier bill ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BillFate {
    /// Entered, not submitted.
    Draft,
    /// Submitted, matched, waiting for approval.
    Matched,
    /// Approved, not paid.
    Approved,
    /// Approved and part of it paid.
    PartPaid,
    /// Approved and paid.
    Paid,
    /// Entered wrongly and voided.
    Voided,
    /// Entered twice: held as a possible duplicate.
    Duplicate,
}

/// One supplier bill.
pub struct BillPlan {
    /// The supplier.
    pub supplier: &'static str,
    /// The supplier's invoice number.
    pub number: &'static str,
    /// Days ago the bill is dated.
    pub age: i64,
    /// The lines: description, price in dirhams, VAT category.
    pub lines: &'static [(&'static str, i64, &'static str)],
    /// The legal entity code that is billed.
    pub entity: &'static str,
    /// What became of it.
    pub fate: BillFate,
}

use BillFate::{Approved, Draft, Duplicate, Matched, Paid, PartPaid, Voided};

/// The bills, oldest first.
pub const BILLS: &[BillPlan] = &[
    BillPlan { supplier: "Skyhigh Lifts and Escalators LLC", number: "SKY-2026-0318", age: 188, lines: &[("Lift maintenance, quarter 1", 12_600, "standard")], entity: "PGP", fate: Paid },
    BillPlan { supplier: "Coolbreeze AC Services LLC", number: "CB-7741", age: 171, lines: &[("Chiller service, Tower A", 8_450, "standard"), ("Spare parts", 1_960, "standard")], entity: "PGP", fate: Paid },
    BillPlan { supplier: "Safeguard Fire Systems LLC", number: "SFG-1190", age: 160, lines: &[("Fire alarm inspection, quarter 1", 9_800, "standard")], entity: "PGP", fate: Paid },
    BillPlan { supplier: "BrightClean Facility Services LLC", number: "BC-2026-044", age: 150, lines: &[("Common area cleaning, March", 14_250, "standard")], entity: "PGFZ", fate: Paid },
    BillPlan { supplier: "Pestguard Gulf LLC", number: "PG-3302", age: 143, lines: &[("Pest control, quarter 1", 3_600, "standard")], entity: "PGFZ", fate: Paid },
    BillPlan { supplier: "Al Noor Trading LLC", number: "ANT-0912", age: 121, lines: &[("Plumbing call-outs, March to May", 4_870, "standard")], entity: "PGP", fate: Paid },
    BillPlan { supplier: "Skyhigh Lifts and Escalators LLC", number: "SKY-2026-0644", age: 96, lines: &[("Lift maintenance, quarter 2", 12_600, "standard")], entity: "PGP", fate: Paid },
    BillPlan { supplier: "Safeguard Fire Systems LLC", number: "SFG-1288", age: 88, lines: &[("Fire alarm inspection, quarter 2", 9_800, "standard"), ("Extinguisher refills", 2_340, "standard")], entity: "PGP", fate: PartPaid },
    BillPlan { supplier: "BrightClean Facility Services LLC", number: "BC-2026-091", age: 62, lines: &[("Common area cleaning, August", 14_250, "standard")], entity: "PGFZ", fate: PartPaid },
    BillPlan { supplier: "Coolbreeze AC Services LLC", number: "CB-7902", age: 49, lines: &[("Chiller service, Tower B", 8_450, "standard")], entity: "PGP", fate: Approved },
    BillPlan { supplier: "Spark Electrical Contracting LLC", number: "SPK-558", age: 41, lines: &[("Corridor lighting replacement", 6_320, "standard")], entity: "PGP", fate: Approved },
    BillPlan { supplier: "Gulf Build Materials LLC", number: "GBM-20418", age: 35, lines: &[("Paint and sealant for move-out repairs", 2_915, "standard"), ("Export packaging", 400, "zero_rated")], entity: "PGP", fate: Approved },
    BillPlan { supplier: "Skyhigh Lifts and Escalators LLC", number: "SKY-2026-0987", age: 14, lines: &[("Lift maintenance, quarter 3", 12_600, "standard")], entity: "PGP", fate: Matched },
    BillPlan { supplier: "Pestguard Gulf LLC", number: "PG-3411", age: 12, lines: &[("Pest control, quarter 3", 3_600, "standard")], entity: "PGFZ", fate: Matched },
    BillPlan { supplier: "Pestguard Gulf LLC", number: "pg-3411 ", age: 10, lines: &[("Pest control, quarter 3", 3_600, "standard")], entity: "PGFZ", fate: Duplicate },
    BillPlan { supplier: "Al Noor Trading LLC", number: "ANT-1044", age: 9, lines: &[("Plumbing call-outs, June to August", 5_240, "standard")], entity: "PGP", fate: Matched },
    BillPlan { supplier: "Skyhigh Lifts and Escalators LLC", number: "sky-2026-0644 ", age: 8, lines: &[("Lift maintenance, quarter 2", 12_600, "standard")], entity: "PGP", fate: Duplicate },
    BillPlan { supplier: "Spark Electrical Contracting LLC", number: "SPK-571", age: 7, lines: &[("Wrong building: lighting, Tower A", 6_320, "standard")], entity: "PGP", fate: Voided },
    BillPlan { supplier: "Coolbreeze AC Services LLC", number: "CB-7961", age: 6, lines: &[("Emergency AC repair, Business Bay", 3_180, "standard")], entity: "PGP", fate: Voided },
    BillPlan { supplier: "Safeguard Fire Systems LLC", number: "SFG-1342", age: 3, lines: &[("Fire alarm inspection, quarter 3", 9_800, "standard")], entity: "PGP", fate: Draft },
    BillPlan { supplier: "BrightClean Facility Services LLC", number: "BC-2026-103", age: 1, lines: &[("Common area cleaning, September", 14_250, "standard")], entity: "PGFZ", fate: Draft },
];

/// The monthly budget of an entity, per category key, in dirhams.
pub const BUDGET: &[(&str, i64)] = &[
    ("sales_services", 520_000),
    ("other_income", 12_000),
    ("cost_of_sales", 38_000),
    ("salaries_staff", 188_000),
    ("rent_utilities", 31_500),
    ("marketing", 12_500),
    ("transport_fuel", 4_800),
    ("professional_fees", 9_500),
    ("bank_charges", 2_100),
    ("other_expense", 6_400),
];

/// A bank statement.
pub struct StatementPlan {
    /// The bank account label.
    pub account: &'static str,
    /// The file name.
    pub file: &'static str,
    /// `csv`, or a format nothing reads.
    pub format: &'static str,
    /// The entity code.
    pub entity: &'static str,
    /// The period, in days from today.
    pub period: (i64, i64),
    /// The closing balance in dirhams.
    pub closing: i64,
    /// The last step: `completed`, `in_review`, `failed` or `reverted`.
    pub end: &'static str,
}

/// The statements.
pub const STATEMENTS: &[StatementPlan] = &[
    StatementPlan { account: "Gulf Demo Bank AE00 0000 0000 0000 0001 234", file: "statement-august.csv", format: "csv", entity: "PGP", period: (-80, -50), closing: 1_284_500, end: "completed" },
    StatementPlan { account: "Gulf Demo Bank AE00 0000 0000 0000 0001 234", file: "statement-september.csv", format: "csv", entity: "PGP", period: (-49, -20), closing: 1_391_260, end: "completed" },
    StatementPlan { account: "Gulf Demo Bank AE00 0000 0000 0000 0001 234", file: "statement-october.csv", format: "csv", entity: "PGP", period: (-19, 0), closing: 1_452_880, end: "in_review" },
    StatementPlan { account: "Emirates Demo Bank AE00 0000 0000 0000 0005 678", file: "facilities-october.csv", format: "csv", entity: "PGFZ", period: (-19, 0), closing: 318_450, end: "in_review" },
    StatementPlan { account: "Gulf Demo Bank AE00 0000 0000 0000 0001 234", file: "statement-july.xlsx", format: "excel", entity: "PGP", period: (-110, -81), closing: 1_190_000, end: "failed" },
    StatementPlan { account: "Emirates Demo Bank AE00 0000 0000 0000 0005 678", file: "facilities-september.csv", format: "csv", entity: "PGFZ", period: (-49, -20), closing: 305_900, end: "reverted" },
    StatementPlan { account: "Emirates Demo Bank AE00 0000 0000 0000 0005 678", file: "facilities-august.csv", format: "csv", entity: "PGFZ", period: (-80, -50), closing: 298_120, end: "reverted" },
    StatementPlan { account: "Emirates Demo Bank AE00 0000 0000 0000 0005 678", file: "facilities-july.csv", format: "excel", entity: "PGFZ", period: (-110, -81), closing: 290_400, end: "failed" },
];
