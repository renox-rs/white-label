//! The prepaid package (wl-sales §3 *Package*, §4.4, §5.5; base for
//! BL-SAL-16): credits of sessions, hours or items bought by a holder. Its
//! status moves only through the machine in [`crate::package_machine`]; the
//! credits move only with a [`PackageMovement`].
//!
//! Decisions of this base, which BL-SAL-16 needs and the rest of the
//! lifecycle builds on:
//!
//! - There is no catalogue in this crate yet, so a package carries the code
//!   and name of the item as it was sold. The sale of the package and its
//!   invoice are made by the caller (a counter sale, a vertical):
//!   [`crate::packages::sell`] records the package as `pending_payment`, and
//!   the caller links the invoice it drafted for it.
//! - `refund` (the unused balance paid back through wl-payments with a credit
//!   note) is not built here: it belongs to a later issue.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::sales::PackageId;
use wl_kernel::{PartyId, Subject};

/// Where a package is (§4.4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PackageStatus {
    /// Sold, not yet paid.
    #[default]
    PendingPayment,
    /// Paid (or the first instalment paid); credits usable.
    Active,
    /// All credits used.
    Completed,
    /// Validity ended with credits left; can be extended.
    Expired,
    /// Unused balance refunded.
    Refunded,
    /// Never paid.
    Cancelled,
}

/// A prepaid package.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_packages",
    state_machine,
    index(holder_party_id, item_code, status),
    index(invoice_id),
    index(valid_to)
)]
pub struct Package {
    /// The primary key.
    pub id: i64,
    /// Who uses the credits.
    pub holder_party_id: i64,
    /// Who pays, when not the holder (a guardian).
    pub payer_party_id: Option<i64>,
    /// The code of the catalogue item it was sold for.
    pub item_code: String,
    /// The name of the item as sold.
    pub item_name: String,
    /// `session`, `hour` or `item`.
    pub unit: String,
    /// The credits it holds.
    pub credits_total: i64,
    /// The credits used so far.
    pub credits_used: i64,
    /// The price paid, in fils.
    pub price_paid: i64,
    /// The ISO currency code.
    pub currency: String,
    /// The first day of validity, set when it activates.
    pub valid_from: Option<NaiveDate>,
    /// The last day of validity, set when it activates.
    pub valid_to: Option<NaiveDate>,
    /// The months it is valid for after activation.
    pub validity_months: i64,
    /// How many times it was extended.
    pub extensions: i64,
    /// The invoice it is paid with.
    pub invoice_id: Option<i64>,
    /// The staff user who sold it.
    pub sold_by: Option<i64>,
    /// When it activated.
    pub activated_at: Option<DateTime>,
    /// The balance of its invoice reopened by a reversed payment, in fils;
    /// zero when none is open (BL-SAL-16). The package keeps its state.
    pub reopened_balance: i64,
    /// When the balance was reopened.
    pub reopened_at: Option<DateTime>,
    /// The seller's "collect before next use" task.
    pub collect_ticket_id: Option<i64>,
    /// Where it is; changed only by the machine.
    pub status: PackageStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Package {
    /// The package as an id.
    pub fn package_id(&self) -> PackageId {
        PackageId(self.id)
    }

    /// The package as a kernel subject.
    pub fn subject(&self) -> Subject {
        self.package_id().subject()
    }

    /// The holder.
    pub fn holder(&self) -> PartyId {
        PartyId(self.holder_party_id)
    }

    /// The credits left.
    pub fn remaining(&self) -> i64 {
        self.credits_total - self.credits_used
    }

    /// Whether a reopened invoice balance is still unpaid.
    pub fn has_unpaid_balance(&self) -> bool {
        self.reopened_balance > 0
    }

    /// Whether its validity ended before `today`.
    pub fn is_past_validity(&self, today: NaiveDate) -> bool {
        self.valid_to.is_some_and(|end| end < today)
    }
}

/// What a movement does to the balance.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MovementKind {
    /// Credits used by a source.
    #[default]
    Consume,
    /// Credits given back.
    Reverse,
    /// The end date moved.
    Extend,
    /// A manual correction.
    Adjust,
    /// Unused credits refunded.
    Refund,
}

/// A change to the credit balance, with the record that caused it.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_package_movements",
    index(package_id),
    index(source_type, source_id)
)]
pub struct PackageMovement {
    /// The primary key.
    pub id: i64,
    /// The package.
    #[model(references = "wl_sales_packages")]
    pub package_id: i64,
    /// What it did.
    pub kind: MovementKind,
    /// The credits, always positive; the kind gives the direction.
    pub quantity: i64,
    /// The type of the record that caused it (a booking, an attendance).
    pub source_type: Option<String>,
    /// The id of the record that caused it.
    pub source_id: Option<i64>,
    /// The user who did it, if a user did.
    pub user_id: Option<i64>,
    /// The reason or note.
    pub note: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PackageMovement {
    /// The record that caused it.
    pub fn source(&self) -> Option<Subject> {
        match (&self.source_type, self.source_id) {
            (Some(kind), Some(id)) => Some(Subject::new(kind, id)),
            _ => None,
        }
    }
}

impl Package {
    /// The movements of the package, oldest first.
    pub async fn movements(&self, db: &Db) -> Result<Vec<PackageMovement>> {
        PackageMovement::query()
            .where_(PackageMovement::PACKAGE_ID.eq(self.id))
            .order_by(PackageMovement::ID)
            .get(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_remaining_credits_are_total_less_used() {
        let package = Package {
            credits_total: 10,
            credits_used: 4,
            ..Package::default()
        };
        assert_eq!(package.remaining(), 6);
    }

    #[test]
    fn a_reopened_balance_is_unpaid_until_it_is_cleared() {
        let mut package = Package::default();
        assert!(!package.has_unpaid_balance());
        package.reopened_balance = 30_000;
        assert!(package.has_unpaid_balance());
    }

    #[test]
    fn validity_ends_after_the_last_day() {
        let day = |d| NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
        let package = Package {
            valid_to: Some(day(10)),
            ..Package::default()
        };
        assert!(!package.is_past_validity(day(10)));
        assert!(package.is_past_validity(day(11)));
        assert!(!Package::default().is_past_validity(day(11)));
    }

    #[test]
    fn statuses_are_stored_in_snake_case() {
        let names: Vec<_> = PackageStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            names,
            [
                "pending_payment",
                "active",
                "completed",
                "expired",
                "refunded",
                "cancelled"
            ]
        );
    }
}
