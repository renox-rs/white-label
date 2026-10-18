//! The SOP library (wl-documents §3 *SOP*, #52).
//!
//! An [`Sop`] is its own record; its versions and translations are ordinary
//! documents of the [`KIND_SOP`] and [`KIND_SOP_TRANSLATION`] kinds attached
//! to it as subject [`SUBJECT_TYPE`]. Its status is derived, never stored
//! ([`SopStatus`]).

pub mod model;
pub mod readers;
pub mod review;
pub mod service;
pub mod translations;
pub mod versions;

use crate::kinds::{KindSeed, register_kind};

pub use model::{Sop, SopReader, SopStatus, SopTranslation};
pub use service::{SopInput, Sops};

/// The SOP's subject type for documents.
pub const SUBJECT_TYPE: &str = "documents.sop";

/// The key of the SOP version kind.
pub const KIND_SOP: &str = "sop";

/// The key of the SOP translation kind.
pub const KIND_SOP_TRANSLATION: &str = "sop_translation";

/// Days before the review date the owner role is reminded (kind offset).
pub const REVIEW_REMINDER_DAYS: i64 = 30;

/// Whether `kind` is one of the two SOP kinds.
pub fn is_sop_kind(kind: &str) -> bool {
    kind == KIND_SOP || kind == KIND_SOP_TRANSLATION
}

/// The seeds of the two SOP kinds: no blocks, override `never`, review on.
/// The version has the review date as expiry and one reminder; the
/// translation has neither.
pub fn kind_seeds() -> [KindSeed; 2] {
    use wl_kernel::documents::OverrideMode;
    [
        KindSeed::new(KIND_SOP, "SOP")
            .name_ar("إجراء تشغيلي معياري")
            .subject_type(SUBJECT_TYPE)
            .expires()
            .reminders(&[REVIEW_REMINDER_DAYS])
            .file_types(&["pdf", "docx"])
            .override_mode(OverrideMode::Never),
        KindSeed::new(KIND_SOP_TRANSLATION, "SOP translation")
            .name_ar("ترجمة إجراء تشغيلي معياري")
            .subject_type(SUBJECT_TYPE)
            .file_types(&["pdf", "docx"])
            .override_mode(OverrideMode::Never),
    ]
}

/// Registers the SOP kinds so `documents:install` and the seeder store them.
pub fn register() {
    for seed in kind_seeds() {
        register_kind(seed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_kinds_have_no_blocks_and_never_override() {
        let [sop, translation] = kind_seeds();
        assert_eq!(sop.key, "sop");
        assert!(sop.has_expiry && !sop.has_number);
        assert_eq!(sop.reminder_offsets, vec![30]);
        assert_eq!(sop.allowed_file_types, vec!["pdf", "docx"]);
        assert!(!translation.has_expiry);
        assert!(translation.reminder_offsets.is_empty());
        for seed in [&sop, &translation] {
            assert!(seed.blocks.is_empty());
            assert!(!seed.legal_block && !seed.compliance_critical);
            assert_eq!(seed.subject_types, vec![SUBJECT_TYPE]);
            assert_eq!(
                seed.override_mode,
                wl_kernel::documents::OverrideMode::Never
            );
            assert!(seed.needs_review);
        }
    }

    #[test]
    fn only_the_two_kinds_are_sop_kinds() {
        assert!(is_sop_kind("sop") && is_sop_kind("sop_translation"));
        assert!(!is_sop_kind("residence_visa"));
    }
}
