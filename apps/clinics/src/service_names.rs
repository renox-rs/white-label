//! The names of the services in words.
//!
//! A booking, a charge line and a tariff keep the service's code
//! (`dental_checkup`); a person reads its name (*Dental check-up*), in the
//! page's language. A code with no service type behind it (a consumable's
//! SKU, an old code) is shown as it is.

use std::collections::BTreeMap;

use renox::prelude::*;
use wl_kernel::scheduling::service_names;

/// The names of the services by code, in `locale` (what the scheduling module
/// provides; none when no module does, so a code shows as it is).
///
/// # Errors
///
/// The database's errors.
pub async fn names(state: &AppState, locale: &str) -> Result<BTreeMap<String, String>> {
    service_names(state).names(state, locale).await
}

/// The name of `code` in `names`, else the code itself.
#[must_use]
pub fn name_of(names: &BTreeMap<String, String>, code: &str) -> String {
    names.get(code).cloned().unwrap_or_else(|| code.to_owned())
}

/// The name of `code` with the code the forms take (`Dental X-ray
/// (dental_xray)`), else the code alone.
#[must_use]
pub fn name_with_code(names: &BTreeMap<String, String>, code: &str) -> String {
    match names.get(code) {
        Some(name) => format!("{name} ({code})"),
        None => code.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_without_a_service_type_is_shown_as_it_is() {
        let names = BTreeMap::from([("follow_up".to_owned(), "Follow-up visit".to_owned())]);
        assert_eq!(name_of(&names, "follow_up"), "Follow-up visit");
        assert_eq!(name_of(&names, "GLOVES-M"), "GLOVES-M");
        assert_eq!(
            name_with_code(&names, "follow_up"),
            "Follow-up visit (follow_up)"
        );
        assert_eq!(name_with_code(&names, "GLOVES-M"), "GLOVES-M");
    }
}
