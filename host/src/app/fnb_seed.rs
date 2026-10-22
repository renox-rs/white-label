//! The 06 F&B client's values for the shared sales, delivery and access
//! mechanisms (06 §3.3, §4.2, §7.1, §7.2; BL-FNB-01 to BL-FNB-03 and the
//! module issues #112, #126, #163, #166, #238 that name them).
//!
//! The mechanisms live in the shared modules; the vertical reaches them only
//! through `wl-kernel`. This file turns the 06 values into:
//!
//! - the sales module's settings ([`sales`]): the outcomes of a cancelled
//!   started line are *return to stock* and *scrap*, the limits of the
//!   cancellation and the refund (`SAL.ACL.01`, `SAL.ACL.05`, `SAL.ACL.06`,
//!   AED 50), the direct site waits for the outlet to accept
//!   (`pending_acceptance`), the aggregators' external numbers are unique;
//! - the delivery module's settings ([`delivery`]): the perishable policy for
//!   own-rider deliveries is *return* after a failed visit and *discard* after
//!   an abort (`WRK.RUL.16`), a wastage above AED 200 needs a second person
//!   (`STK.ACL.02`), the delivery promise is 45 minutes (BL-DLV-10), cooked
//!   food is not partly accepted, a cash difference is absorbed by the
//!   customer, the business or the rider (`WRK.ACL.09`), and the kitchen
//!   manager gets the dispatcher's tasks;
//! - the failure reasons 06 adds to the rider's list ([`install`]): *rider
//!   issue*, *refused: the order was late* and *refused: wrong item*;
//! - the roles' extra grants of this client ([`grant_roles`]): the cashier,
//!   the storekeeper and the accountant of the shared roles table get their
//!   06 permissions here only;
//! - the seed ([`FnbSeed`], command `fnb:install`), which adds what the
//!   database lacks and keeps what the client changed.
//!
//! Nothing here is a rule: the rules run in the modules with these values.

use fnb::permissions as fnb_perm;
use renox::prelude::*;
use wl_delivery::reasons::FailureReason;
use wl_delivery::{PerishableAction, WlDelivery};
use wl_kernel::delivery::CodAbsorber;
use wl_sales::WlSales;

use super::roles::{self, ACCOUNTANT, CASHIER, KITCHEN_MANAGER, STOREKEEPER};

/// The client app these values are for.
pub const FNB: &str = "fnb";

/// The source type of an F&B order: the key of the perishable policy and of the
/// delivery promise.
pub const ORDER_SOURCE: &str = fnb::SUBJECT_TYPE;

/// The delivery promise of every brand until a brand sets its own, in minutes.
pub const PROMISE_MINUTES: i64 = 45;

/// A wastage above this value needs a second person (`STK.ACL.02`, AED 200).
pub const WASTAGE_APPROVAL_FILS: i64 = 20_000;

/// A cash difference up to this is accepted without an approver (AED 5,
/// `WRK.ACL.09`).
pub const COD_TOLERANCE: i64 = 500;

/// A cancellation or a refund above this needs a manager (`SAL.ACL.01`,
/// `SAL.ACL.06`, AED 50).
pub const APPROVAL_LIMIT: i64 = 5_000;

/// The channels the aggregators' tablets and reports enter orders through
/// (never their real names on a screen, DEMO-DATA.md §06).
pub const AGGREGATOR_CHANNELS: [&str; 3] = ["aggregator_a", "aggregator_b", "aggregator_c"];

/// The F&B module with the 06 values (the defaults of the spec; open item L-33
/// keeps a customer-side failure unrefunded until the lawyer answers).
pub fn module() -> fnb::Fnb {
    fnb::Fnb::new()
}

/// The sales module with the 06 client's settings (06 §7.1, §3.3).
pub fn sales(module: WlSales) -> WlSales {
    module
        .over_budget_approval(true)
        .campaign_required_for(&[])
        .cancel_outcomes(&["return_to_stock", "scrap"])
        .cancel_approval_limit(APPROVAL_LIMIT)
        .refund_approval_limit(APPROVAL_LIMIT)
        .cancellation_fee_vat("standard")
        .order_acceptance_channels(&["website"])
        .aggregator_channels(&AGGREGATOR_CHANNELS)
        .unique_external_order_number(true)
        .order_manager_role(KITCHEN_MANAGER)
}

/// The delivery module with the 06 client's settings (06 §4.2, §7.1;
/// `WRK.RUL.16`, `WRK.ACL.09`, BL-DLV-10).
pub fn delivery(module: WlDelivery) -> WlDelivery {
    module
        .perishable_policy(
            ORDER_SOURCE,
            PerishableAction::Return,
            PerishableAction::Discard,
        )
        .discard_approval_limit(WASTAGE_APPROVAL_FILS)
        .promise_minutes(ORDER_SOURCE, PROMISE_MINUTES)
        .partial_acceptance(ORDER_SOURCE, false)
        .cod_tolerance(COD_TOLERANCE)
        .cod_absorbers(&[
            CodAbsorber::Customer,
            CodAbsorber::Business,
            CodAbsorber::Driver,
        ])
        .dispatcher_role(KITCHEN_MANAGER)
}

/// The delivery module for a client: the 06 values for F&B, the module's own
/// for any other.
pub fn delivery_for(client_app: &str, module: WlDelivery) -> WlDelivery {
    if client_app == FNB { delivery(module) } else { module }
}

/// What the extra grants of the F&B client are: the permissions of the roles
/// the shared table gives other duties in other verticals.
pub fn grant_roles(client_app: &str) {
    if client_app != FNB {
        return;
    }
    use wl_delivery::permissions as delivery;
    use wl_payments::permissions as payments;
    use wl_sales::permissions as sales;
    // The cashier enters aggregator and phone orders, rings up counter sales
    // and asks for a refund the kitchen manager approves (`FNB.ACL.17`); the
    // cashier of 03 counts cash, the cashier of 06 keeps the till.
    roles::grant_extra(
        CASHIER,
        &[
            fnb_perm::ORDERS_VIEW,
            fnb_perm::ORDERS_REFUND_REQUEST,
            // Cancels an order, or a line, and amends a direct order after
            // calling the customer (`SAL.ACL.01`, `SAL.ACL.05`: a manager
            // approves above AED 50 or for a started line).
            fnb_perm::ORDERS_CANCEL,
            fnb_perm::ORDERS_AMEND,
            sales::SALES_ORDERS_VIEW,
            sales::SALES_ORDERS_CREATE,
            sales::SALES_ORDERS_CANCEL,
            sales::SALES_ORDERS_AMEND,
            sales::RETURNS_REQUEST,
            sales::RETURNS_HANDLE,
            sales::POS_SELL,
        ],
    );
    // The accountant reconciles and reads the orders.
    roles::grant_extra(
        ACCOUNTANT,
        &[
            fnb_perm::ORDERS_VIEW,
            sales::SALES_ORDERS_VIEW,
            payments::VIEW,
            delivery::VIEW,
        ],
    );
    // The storekeeper sees the orders whose food they stock.
    roles::grant_extra(STOREKEEPER, &[fnb_perm::ORDERS_VIEW]);
}

/// Adds the failure reasons 06 adds to wl-delivery's list, where the database
/// lacks them. Returns how many it added.
///
/// # Errors
///
/// The database's errors.
pub async fn install(db: &Db) -> Result<usize> {
    let mut added = 0;
    for (code, name_en, name_ar, counts, allows_reattempt) in fnb::failed::FAILURE_REASONS {
        if FailureReason::query()
            .where_(FailureReason::CODE.eq(code))
            .first(db)
            .await?
            .is_some()
        {
            continue;
        }
        let mut reason = FailureReason {
            code: code.to_owned(),
            name_en: name_en.to_owned(),
            name_ar: name_ar.to_owned(),
            counts,
            allows_reattempt,
            active: true,
            ..FailureReason::default()
        };
        reason.insert(db).await?;
        added += 1;
    }
    Ok(added)
}

/// The module that seeds the 06 client's values: the seeder runs after the
/// shared modules' own (it is mounted after them), and the command
/// `fnb:install` runs the same install.
pub struct FnbSeed;

impl Module for FnbSeed {
    fn name(&self) -> &'static str {
        "fnb_seed"
    }

    fn register(&self, app: &mut Registry) {
        app.command(
            "fnb:install",
            "Add the failure reasons the F&B client adds to the rider's list, where the database lacks them",
            |_args, state: AppState| async move {
                let added = install(&state.db).await?;
                println!("{added} failure reasons added.");
                Ok(())
            },
        );
        app.seeder(|state: AppState| async move { install(&state.db).await.map(|_| ()) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_failure_reasons_are_not_reattempted_and_the_rider_issue_is_not_counted() {
        for (code, _, _, counts, allows) in fnb::failed::FAILURE_REASONS {
            assert!(!allows, "{code} is never reattempted (cooked food)");
            assert_eq!(counts, code != "rider_issue", "{code}");
        }
    }
}
