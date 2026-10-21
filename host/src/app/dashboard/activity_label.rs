//! The words of the dashboard's "recent activity" list. The audit log keeps
//! an action as a key (`auth.login`, `inventory.item_supplier_added`); a
//! person reads a sentence. The texts live in `resources/lang/en.json` under
//! `dashboard.activity.action.<key>`; an action nobody wrote a text for is
//! made readable from its key ("Inventory item supplier added") instead of
//! being shown raw.

use renox::prelude::*;

/// What `action` says, in the language of `lang`.
pub fn label(lang: &Lang, action: &str) -> String {
    let key = format!("dashboard.activity.action.{action}");
    let text = lang.t(&key, &[]);
    if text == key { humanize(action) } else { text }
}

/// `inventory.item_supplier_added` as "Inventory item supplier added".
fn humanize(action: &str) -> String {
    let words = action.replace(['.', '_'], " ");
    let words = words.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::humanize;

    #[test]
    fn a_key_reads_as_a_sentence() {
        assert_eq!(
            humanize("inventory.item_supplier_added"),
            "Inventory item supplier added"
        );
        assert_eq!(humanize("auth.login"), "Auth login");
        assert_eq!(humanize("single"), "Single");
        assert_eq!(humanize(""), "");
        assert_eq!(humanize("a..b__c"), "A b c");
    }
}
