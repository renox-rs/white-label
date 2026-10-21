//! What the Manpower client keeps in the shared modules (09 Manpower §7.2,
//! wl-ticketing §3): the ticket categories the vertical opens tickets in. The
//! host stores them where the database lacks them (`manpower:install`); the
//! vertical only names them.

/// The category of a complaint about the conduct of a worker (BL-MPW-14,
/// `WRK.TKT.01`).
pub const CONDUCT_CATEGORY: &str = "conduct";

/// The category of an absence case, opened by `MPW.RUL.09` for HR
/// (BL-MPW-17).
pub const ABSENCE_CATEGORY: &str = "absence";

/// The role whose holders look after absence cases.
pub const ABSENCE_ROLE: &str = crate::permissions::HR_ROLE;

/// A ticket category the vertical seeds.
#[derive(Debug, Clone, Copy)]
pub struct CategorySpec {
    /// The fixed key the vertical opens tickets with.
    pub key: &'static str,
    /// The English name.
    pub name_en: &'static str,
    /// The Arabic name.
    pub name_ar: &'static str,
    /// The role whose holders look after its tickets.
    pub assignee_role: Option<&'static str>,
}

/// The ticket categories of 09.
pub const CATEGORIES: [CategorySpec; 2] = [
    CategorySpec {
        key: CONDUCT_CATEGORY,
        name_en: "Conduct complaint",
        name_ar: "شكوى سلوك",
        assignee_role: None,
    },
    CategorySpec {
        key: ABSENCE_CATEGORY,
        name_en: "Absence case",
        name_ar: "حالة غياب",
        assignee_role: Some(ABSENCE_ROLE),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_categories_have_distinct_keys() {
        assert_ne!(CATEGORIES[0].key, CATEGORIES[1].key);
        assert!(CATEGORIES.iter().any(|c| c.key == ABSENCE_CATEGORY));
    }
}
