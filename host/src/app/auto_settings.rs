//! The 08 Auto client's values for the shared sales, stock, money, document
//! and notification mechanisms (08 §7.1 settings; BL-AUT-15, BL-AUT-18).
//!
//! The mechanisms live in the shared modules; this file turns the 08 numbers
//! into the owning modules' own calls.

use wl_sales::WlSales;

/// The order channels whose lines without stock are back-ordered in 08
/// (`BL-AUT-15`, wl-sales §7 *Back-orders allowed per channel*): the counter,
/// the phone and the sales reps. The portal stays off.
pub const BACK_ORDER_CHANNELS: &[&str] = &["counter", "phone", "sales_rep"];

/// The sales module with the Auto client's values: back-orders allowed on the
/// counter, phone and sales rep channels.
pub fn sales(module: WlSales) -> WlSales {
    module.back_orders_for(BACK_ORDER_CHANNELS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aut_15_back_orders_are_on_for_counter_phone_and_sales_rep_only() {
        let module = sales(WlSales::new());
        let config = module.config();
        for channel in ["counter", "phone", "sales_rep"] {
            assert!(config.back_orders_allowed(channel), "{channel}");
        }
        assert!(!config.back_orders_allowed("portal"));
        assert!(!WlSales::new().config().back_orders_allowed("counter"));
    }
}
