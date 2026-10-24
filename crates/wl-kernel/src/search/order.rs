//! The order `search:rebuild` builds the types in: a type after the types
//! it declared with [`SearchableBuilder::after`](super::SearchableBuilder::after)
//! (an invoice after the parties, whose names its documents read), else
//! alphabetically by subject type.

use std::collections::{BTreeMap, BTreeSet};

/// The subject types of `types` (each with the types it comes after), in
/// the order to build them: every type after the types it names, ties
/// alphabetical. A named type that is not in `types` is ignored; a cycle
/// (which a module should not declare) is broken by taking the
/// alphabetically first type still waiting, so every type is built once.
pub fn rebuild_order(types: &[(String, Vec<String>)]) -> Vec<String> {
    let known: BTreeSet<&str> = types.iter().map(|(name, _)| name.as_str()).collect();
    let mut waiting: BTreeMap<&str, BTreeSet<&str>> = types
        .iter()
        .map(|(name, after)| {
            let before: BTreeSet<&str> = after
                .iter()
                .map(String::as_str)
                .filter(|other| known.contains(other) && *other != name.as_str())
                .collect();
            (name.as_str(), before)
        })
        .collect();
    let mut order: Vec<String> = Vec::with_capacity(waiting.len());
    while !waiting.is_empty() {
        let next = waiting
            .iter()
            .find(|(_, before)| before.is_empty())
            .map(|(name, _)| *name)
            // A cycle: the first type still waiting goes anyway.
            .unwrap_or_else(|| *waiting.keys().next().expect("not empty"));
        waiting.remove(next);
        for before in waiting.values_mut() {
            before.remove(next);
        }
        order.push(next.to_owned());
    }
    order
}

#[cfg(test)]
mod tests {
    use super::rebuild_order;

    fn types(list: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
        list.iter()
            .map(|(name, after)| {
                (
                    (*name).to_owned(),
                    after.iter().map(|a| (*a).to_owned()).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn parties_come_before_the_types_that_read_their_names() {
        let order = rebuild_order(&types(&[
            ("invoicing.invoice", &["parties.party"]),
            ("invoicing.credit_note", &["invoicing.invoice", "parties.party"]),
            ("parties.party", &[]),
            ("property.lease", &["parties.party", "property.unit"]),
            ("property.unit", &["property.building"]),
            ("property.building", &[]),
            ("ticketing.ticket", &["parties.party", "property.unit"]),
        ]));
        let at = |name: &str| order.iter().position(|t| t == name).unwrap();
        assert!(at("parties.party") < at("invoicing.invoice"), "{order:?}");
        assert!(at("invoicing.invoice") < at("invoicing.credit_note"), "{order:?}");
        assert!(at("property.building") < at("property.unit"), "{order:?}");
        assert!(at("property.unit") < at("property.lease"), "{order:?}");
        assert!(at("property.unit") < at("ticketing.ticket"), "{order:?}");
        assert_eq!(order.len(), 7);
    }

    #[test]
    fn without_dependencies_the_order_is_alphabetical() {
        let order = rebuild_order(&types(&[("b.b", &[]), ("a.a", &[]), ("c.c", &["x.unknown"])]));
        assert_eq!(order, ["a.a", "b.b", "c.c"]);
    }

    #[test]
    fn a_cycle_still_builds_every_type_once() {
        let order = rebuild_order(&types(&[("a.a", &["b.b"]), ("b.b", &["a.a"]), ("c.c", &[])]));
        assert_eq!(order, ["c.c", "a.a", "b.b"]);
    }
}
