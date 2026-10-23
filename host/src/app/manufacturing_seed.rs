//! The 07 Manufacturing client's values for the shared modules (07 §2, §3.3,
//! §7.1 *Negative-flow settings*, §9; BL-MFG-11, BL-MFG-12).
//!
//! - **Document kinds** ([`register_documents`]): the product conformity
//!   certificates of a regulated item (the ECAS certificate, the Emirates
//!   Quality Mark licence, the food-contact certificate) attach to the item
//!   and block `dispatch.pick` and `dispatch.dispatch` as legal blocks with
//!   the override `never` (`⚠ open: lawyer L-19`), looking 30 days ahead
//!   (`DOC.RUL.02`); the industrial licence, the Industrial Registry and the
//!   ICV certificate only remind (`DOC.RUL.01`) and block nothing; the exit
//!   evidence of an export (a customs exit certificate, a bill of lading, an
//!   airway bill) attaches to the dispatch and blocks nothing either.
//! - **The unique customer PO number** ([`sales`]): the setting *Unique
//!   customer PO number per customer* is on (BL-MFG-06).
//!
//! Nothing here is a rule: the rules run in the modules and in the vertical
//! with these values.

use manufacturing::conformity::{DISPATCH, ECAS, EQM, FOOD_CONTACT, PICK};
use manufacturing::exports::{AIRWAY_BILL, BILL_OF_LADING, CUSTOMS_EXIT};
use wl_documents::registrations::{OTHER, TRADEMARK, VerticalSettings, register_vertical};
use wl_documents::{KindSeed, register_kind};
use wl_kernel::documents::{BlockMode, OverrideMode};
use wl_kernel::inventory::{DISPATCH_SUBJECT_TYPE, ITEM_SUBJECT_TYPE};

use super::roles::{ADMIN, PRODUCTION_MANAGER, PRODUCTION_PLANNER};

/// The client app these values are for.
pub const MANUFACTURING: &str = "manufacturing";

/// The days before a certificate expires that its holder is reminded and the
/// look-ahead of `DOC.RUL.02` starts.
pub const LOOK_AHEAD_DAYS: i64 = 30;

/// The product conformity kinds: key, English name, Arabic name.
pub const CONFORMITY: [(&str, &str, &str); 3] = [
    (ECAS, "ECAS certificate", "شهادة ECAS"),
    (
        EQM,
        "Emirates Quality Mark licence",
        "رخصة علامة الجودة الإماراتية",
    ),
    (
        FOOD_CONTACT,
        "Food-contact certificate",
        "شهادة ملامسة الأغذية",
    ),
];

/// The licences of the company: key, English name, Arabic name. They remind
/// and block nothing.
pub const COMPANY_KINDS: [(&str, &str, &str); 3] = [
    (
        "industrial_licence",
        "Industrial licence",
        "الرخصة الصناعية",
    ),
    (
        "industrial_registry",
        "Industrial Registry",
        "السجل الصناعي",
    ),
    (
        "icv_certificate",
        "ICV certificate",
        "شهادة القيمة الوطنية المضافة",
    ),
];

/// The exit evidence of an export: key, English name, Arabic name.
pub const EXIT_EVIDENCE: [(&str, &str, &str); 3] = [
    (
        CUSTOMS_EXIT,
        "Customs exit certificate",
        "شهادة الخروج الجمركية",
    ),
    (BILL_OF_LADING, "Bill of lading", "بوليصة الشحن"),
    (AIRWAY_BILL, "Airway bill", "بوليصة الشحن الجوي"),
];

/// The conformity kind `key` as wl-documents' seed: a legal block of both
/// steps of a dispatch, attached to the item.
pub fn conformity_seed(key: &str, name_en: &str, name_ar: &str) -> KindSeed {
    KindSeed::new(key, name_en)
        .name_ar(name_ar)
        .subject_type(ITEM_SUBJECT_TYPE)
        .expires()
        .numbered()
        .reminders(&[60, 30, 7])
        .look_ahead(LOOK_AHEAD_DAYS)
        .responsible_role(PRODUCTION_MANAGER)
        // Sales and the planner get the look-ahead of the orders promised
        // after the expiry (`DOC.RUL.02`).
        .also_notify(&[manufacturing::permissions::SALES_ROLE, PRODUCTION_PLANNER])
        .override_mode(OverrideMode::Never)
        .legal_block()
        .blocks(PICK, BlockMode::Block)
        .blocks(DISPATCH, BlockMode::Block)
}

/// Registers the 07 kinds with wl-documents and turns on the two
/// registration kinds with the company admin as responsible role.
pub fn register_documents(client_app: &str) {
    if client_app != MANUFACTURING {
        return;
    }
    for (key, name_en, name_ar) in CONFORMITY {
        register_kind(conformity_seed(key, name_en, name_ar));
    }
    for (key, name_en, name_ar) in COMPANY_KINDS {
        register_kind(
            KindSeed::new(key, name_en)
                .name_ar(name_ar)
                .expires()
                .numbered()
                .reminders(&[90, 30, 7])
                .responsible_role(ADMIN),
        );
    }
    for (key, name_en, name_ar) in EXIT_EVIDENCE {
        register_kind(
            KindSeed::new(key, name_en)
                .name_ar(name_ar)
                .subject_type(DISPATCH_SUBJECT_TYPE)
                .numbered()
                .responsible_role(ADMIN),
        );
    }
    register_vertical(
        MANUFACTURING,
        VerticalSettings::new()
            .enabled(&[TRADEMARK, OTHER])
            .role(TRADEMARK, ADMIN)
            .role(OTHER, ADMIN),
    );
}

/// Marks `MFG.RUL.08` compliance-critical for a manufacturing client: a run
/// that fails after its last attempt, or a daily one that does not run, alerts
/// the owner role and the company admin (BL-KRN-08).
pub fn register_critical(client_app: &str) {
    if client_app != MANUFACTURING {
        return;
    }
    wl_kernel::compliance::CriticalRule::new(manufacturing::exports::RULE)
        .daily_at(manufacturing::exports::RUN_AT)
        .register();
}

/// `module` with the 07 sales settings: the customer PO number is unique per
/// customer with its call-off number (BL-MFG-06).
pub fn sales(module: wl_sales::WlSales) -> wl_sales::WlSales {
    module.unique_customer_po(true)
}
