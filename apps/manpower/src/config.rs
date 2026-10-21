//! The settings of the vertical (09 Manpower §7.2). They are client defaults
//! kept in code; the settings page comes with BL-MPW-21.

/// N of `MPW.RUL.10` (§7.2): an invoice overdue more than this many days
/// (today minus the due date, in Asia/Dubai) puts the client contract on
/// credit hold.
pub const DEFAULT_HOLD_OVERDUE_DAYS: i64 = 30;

/// N of `MPW.RUL.11` (§7.2): the first notice before a client contract's end
/// date, in days.
pub const DEFAULT_END_NOTICE_FIRST_DAYS: i64 = 60;

/// M of `MPW.RUL.11` (§7.2): the second notice, in days before the end date.
pub const DEFAULT_END_NOTICE_SECOND_DAYS: i64 = 30;

/// N of `MPW.ACL.07` (§3.3): the document and licence guard of a deployment
/// looks this many days ahead of the day of the transition (or up to the end
/// date of the deployment, if that is earlier), so a worker is not deployed on
/// a visa that expires next week.
pub const DEFAULT_DOCUMENT_LOOK_AHEAD_DAYS: i64 = 30;

/// N of `MPW.RUL.13` (§7.2): the client and the account manager are told that
/// cover is not found this many hours before the cover period starts (the
/// replacement request's start date and shift start time).
pub const DEFAULT_COVER_NOTICE_HOURS: i64 = 24;
