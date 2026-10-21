//! The idempotent seeder with the §7 defaults: the failure reasons (not
//! home, refused, refused to pay, wrong address, no cash, access denied, and
//! the three that do not count) and the refusal reasons (damaged, wrong
//! item, not ordered, short-dated, temperature, other) and the cash difference
//! reasons (recipient short of cash, disputes the amount, agreed discount, no
//! change).
//!
//! [`install`] adds the rows the database lacks and keeps every row that is
//! there, so a client's changes survive a re-run.

use renox::prelude::*;

use crate::reasons::{CodReason, FailureReason, RefusalReason};

/// The failure reasons as (code, English name, Arabic name, counts, allows a
/// reattempt).
pub const FAILURE_REASONS: [(&str, &str, &str, bool, bool); 9] = [
    ("not_home", "Not home", "غير موجود", true, true),
    ("refused", "Refused", "رفض الاستلام", true, true),
    ("refused_to_pay", "Refused to pay", "رفض الدفع", true, true),
    ("wrong_address", "Wrong address", "عنوان خاطئ", true, true),
    ("no_cash", "No cash", "لا يوجد نقد", true, true),
    ("access_denied", "Access denied", "الدخول ممنوع", true, true),
    (
        "not_attempted",
        "Not attempted",
        "لم تتم المحاولة",
        false,
        true,
    ),
    ("run_aborted", "Run aborted", "أُلغيت الرحلة", false, true),
    ("reversed", "Reversed", "تم التراجع", false, true),
];

/// The refusal reasons as (code, English name, Arabic name).
pub const REFUSAL_REASONS: [(&str, &str, &str); 6] = [
    ("damaged", "Damaged", "تالف"),
    ("wrong_item", "Wrong item", "صنف خاطئ"),
    ("not_ordered", "Not ordered", "لم يُطلب"),
    ("short_dated", "Short-dated", "قريب الانتهاء"),
    ("temperature", "Temperature", "درجة الحرارة"),
    ("other", "Other", "أخرى"),
];

/// The cash difference reasons as (code, English name, Arabic name, default
/// absorbed by).
pub const COD_REASONS: [(&str, &str, &str, &str); 4] = [
    (
        "recipient_short",
        "Recipient short of cash",
        "نقص النقد لدى المستلم",
        "customer",
    ),
    (
        "disputes_amount",
        "Disputes the amount",
        "يعترض على المبلغ",
        "customer",
    ),
    ("agreed_discount", "Agreed discount", "خصم متفق عليه", "business"),
    ("no_change", "No change", "لا يوجد فكة", "driver"),
];

/// How many rows [`install`] added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Installed {
    /// Failure reasons added.
    pub failure_reasons: usize,
    /// Refusal reasons added.
    pub refusal_reasons: usize,
    /// Cash difference reasons added.
    pub cod_reasons: usize,
}

impl Installed {
    /// All rows added.
    pub fn total(self) -> usize {
        self.failure_reasons + self.refusal_reasons + self.cod_reasons
    }
}

/// Adds the defaults the database lacks.
///
/// # Errors
///
/// Database errors.
pub async fn install(db: &Db) -> Result<Installed> {
    let mut installed = Installed::default();
    for (code, name_en, name_ar, counts, allows_reattempt) in FAILURE_REASONS {
        let found = FailureReason::query()
            .where_(FailureReason::CODE.eq(code))
            .first(db)
            .await?;
        if found.is_none() {
            let mut row = FailureReason {
                code: code.to_owned(),
                name_en: name_en.to_owned(),
                name_ar: name_ar.to_owned(),
                counts,
                allows_reattempt,
                active: true,
                ..FailureReason::default()
            };
            row.insert(db).await?;
            installed.failure_reasons += 1;
        }
    }
    for (code, name_en, name_ar) in REFUSAL_REASONS {
        let found = RefusalReason::query()
            .where_(RefusalReason::CODE.eq(code))
            .first(db)
            .await?;
        if found.is_none() {
            let mut row = RefusalReason {
                code: code.to_owned(),
                name_en: name_en.to_owned(),
                name_ar: name_ar.to_owned(),
                active: true,
                ..RefusalReason::default()
            };
            row.insert(db).await?;
            installed.refusal_reasons += 1;
        }
    }
    for (code, name_en, name_ar, absorbed_by) in COD_REASONS {
        let found = CodReason::query()
            .where_(CodReason::CODE.eq(code))
            .first(db)
            .await?;
        if found.is_none() {
            let mut row = CodReason {
                code: code.to_owned(),
                name_en: name_en.to_owned(),
                name_ar: name_ar.to_owned(),
                default_absorbed_by: absorbed_by.to_owned(),
                active: true,
                ..CodReason::default()
            };
            row.insert(db).await?;
            installed.cod_reasons += 1;
        }
    }
    Ok(installed)
}
