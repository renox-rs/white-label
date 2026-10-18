//! Expiry look-ahead (wl-documents §5.2, #48): the logs of what the nightly
//! run has reported.

pub mod logs;

pub use logs::{AffectedItem, ReminderLog};
