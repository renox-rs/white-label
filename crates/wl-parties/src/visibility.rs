//! Who sees which block reason (`PTY.ACL.02`, ACCESS-CONTROL `SEN` / `FLD`;
//! no tipping off).
//!
//! A holder of `parties.block_reason.view` sees and uses every reason code
//! and every Block record. Everyone else sees a restricted code as the
//! neutral `on_hold` (**On hold**), cannot choose one in the block dialog
//! and cannot read a restricted Block record. Non-restricted codes and
//! records are shown to everyone who can see the party.

use renox::prelude::*;
use wl_kernel::NEUTRAL_BLOCK_REASON;

use crate::block_record::BlockRecord;
use crate::permissions;
use crate::reasons::BlockReasons;
use crate::texts;

/// A package text by key without the `wl_parties.` prefix.
type Text<'a> = &'a dyn Fn(&str) -> String;

/// Whether `user` may see and use restricted reason codes and Block
/// records (`parties.block_reason.view`).
pub fn sees_restricted(user: &User) -> bool {
    user.has_permission(permissions::BLOCK_REASON_VIEW)
}

/// The code a user sees for `code`: the code itself, or `on_hold` when it is
/// restricted and the user does not hold `parties.block_reason.view`.
pub fn display_code(code: &str, restricted: bool, sees_restricted: bool) -> &str {
    if sees_restricted {
        code
    } else {
        wl_kernel::parties::display_block_reason(code, restricted)
    }
}

/// Whether a user may read `record`: a restricted one only with
/// `parties.block_reason.view`.
pub fn can_read(record: &BlockRecord, sees_restricted: bool) -> bool {
    sees_restricted || !record.restricted
}

/// The records a user may read, in their order; restricted ones are left
/// out for users without `parties.block_reason.view`.
pub fn visible_records(records: Vec<BlockRecord>, sees_restricted: bool) -> Vec<BlockRecord> {
    records
        .into_iter()
        .filter(|record| can_read(record, sees_restricted))
        .collect()
}

/// The label of a reason code in `lang`: the text `wl_parties.reason.<code>`,
/// or the code itself when there is none (a vertical's code without a text).
pub fn reason_label(lang: &Lang, code: &str) -> String {
    reason_label_with(&texts_in(lang), code)
}

/// [`reason_label`] in English, for places without a request language (the
/// block guard).
pub(crate) fn reason_label_en(code: &str) -> String {
    reason_label_with(
        &|key: &str| texts::english(&format!("wl_parties.{key}")),
        code,
    )
}

/// What a blocked party shows for its display code in `lang`: **On hold**
/// for `on_hold`, else "On hold — <reason>" (e.g. "On hold — Non-payment").
/// Give it only a display code (the party's `hold_reason`, or
/// [`display_code`]), never a restricted code for a user who may not see it.
pub fn hold_label(lang: &Lang, display_code: &str) -> String {
    hold_label_with(&texts_in(lang), display_code)
}

/// The reasons a user may choose in the block dialog, as `(code, label)`:
/// every reason with `parties.block_reason.view`, else the non-restricted
/// ones.
pub fn offered_reasons(
    reasons: &BlockReasons,
    lang: &Lang,
    sees_restricted: bool,
) -> Vec<(String, String)> {
    offered_reasons_with(reasons, &texts_in(lang), sees_restricted)
}

/// The package texts in `lang`, by key without the prefix.
fn texts_in(lang: &Lang) -> impl Fn(&str) -> String + '_ {
    move |key: &str| texts::t(lang, &format!("wl_parties.{key}"))
}

/// [`reason_label`] with the texts of `t`.
fn reason_label_with(t: Text<'_>, code: &str) -> String {
    let key = format!("reason.{code}");
    let label = t(&key);
    if label == key || label == format!("wl_parties.{key}") {
        code.to_owned()
    } else {
        label
    }
}

/// [`hold_label`] with the texts of `t`.
fn hold_label_with(t: Text<'_>, display_code: &str) -> String {
    let on_hold = t("status.blocked");
    if display_code.trim().is_empty() || display_code == NEUTRAL_BLOCK_REASON {
        on_hold
    } else {
        format!("{on_hold} — {}", reason_label_with(t, display_code))
    }
}

/// [`offered_reasons`] with the texts of `t`.
fn offered_reasons_with(
    reasons: &BlockReasons,
    t: Text<'_>,
    sees_restricted: bool,
) -> Vec<(String, String)> {
    reasons
        .offered(sees_restricted)
        .map(|reason| (reason.code.clone(), reason_label_with(t, &reason.code)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn english(key: &str) -> String {
        texts::english(&format!("wl_parties.{key}"))
    }

    fn record(code: &str, restricted: bool) -> BlockRecord {
        BlockRecord::open(1, code, restricted, "note", Some(2))
    }

    #[test]
    fn a_restricted_code_is_on_hold_without_the_permission() {
        assert_eq!(display_code("aml_concern", true, false), "on_hold");
        assert_eq!(display_code("aml_concern", true, true), "aml_concern");
        assert_eq!(display_code("non_payment", false, false), "non_payment");
        assert_eq!(display_code("non_payment", false, true), "non_payment");
    }

    #[test]
    fn restricted_records_are_left_out_without_the_permission() {
        let records = vec![record("non_payment", false), record("sanctions", true)];
        let seen = visible_records(records.clone(), false);
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].reason_code, "non_payment");
        assert_eq!(visible_records(records, true).len(), 2);
        assert!(!can_read(&record("aml_concern", true), false));
        assert!(can_read(&record("aml_concern", true), true));
    }

    #[test]
    fn a_blocked_party_shows_on_hold_and_the_reason() {
        assert_eq!(
            hold_label_with(&english, "non_payment"),
            "On hold — Non-payment"
        );
        assert_eq!(hold_label_with(&english, "on_hold"), "On hold");
        assert_eq!(hold_label_with(&english, ""), "On hold");
    }

    #[test]
    fn a_code_without_a_text_is_shown_as_the_code() {
        assert_eq!(
            reason_label_with(&english, "unpaid_deposit"),
            "unpaid_deposit"
        );
        assert_eq!(
            reason_label_with(&english, "legal_dispute"),
            "Legal dispute"
        );
    }

    #[test]
    fn the_dialog_offers_restricted_codes_only_with_the_permission() {
        let reasons = BlockReasons::default();
        let codes = |sees| {
            offered_reasons_with(&reasons, &english, sees)
                .into_iter()
                .map(|(code, _)| code)
                .collect::<Vec<_>>()
        };
        let plain = codes(false);
        assert!(plain.contains(&"on_hold".to_owned()));
        assert!(plain.contains(&"non_payment".to_owned()));
        assert!(!plain.contains(&"sanctions".to_owned()));
        assert!(!plain.contains(&"aml_concern".to_owned()));
        let full = codes(true);
        assert!(full.contains(&"sanctions".to_owned()));
        assert!(full.contains(&"aml_concern".to_owned()));
    }
}
