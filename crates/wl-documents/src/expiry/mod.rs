//! Expiry look-ahead (wl-documents §5.2, #48): the logs of what the nightly
//! run has reported, and who it tells.

pub mod logs;
pub mod responsible;

pub use logs::{AffectedItem, ReminderLog};
pub use responsible::{Recipient, resolve, role_for};
