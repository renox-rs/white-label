//! The list screens of the shared modules named by #529 (documents, parties,
//! ticketing) and #530 (notify, scheduling, kernel): each template puts its
//! `renox::grid` directly in the page, after the header, with nothing but
//! dialogs after it, so the grid fills the screen on a desktop. The screens
//! whose rows come from code (block effects, the settings tables, the
//! approvals inbox) fill it with one pane instead.

mod grid_support;

use grid_support::grid_is_a_child_of_the_page;

/// Every grid screen of the two issues, under `crates/`.
const GRIDS: [&str; 21] = [
    "wl-documents/views/wl-documents/review_queue.html",
    "wl-documents/views/wl-documents/sops.html",
    "wl-documents/views/wl-documents/overrides.html",
    "wl-documents/views/wl-documents/acknowledgements.html",
    "wl-documents/views/wl-documents/kinds.html",
    "wl-documents/views/wl-documents/flags.html",
    "wl-parties/views/wl-parties/review_tasks.html",
    "wl-parties/views/wl-parties/screening_tasks.html",
    "wl-parties/views/wl-parties/compliance_tasks.html",
    "wl-ticketing/views/wl-ticketing/compliance_flags.html",
    "wl-ticketing/views/wl-ticketing/deadline_tasks.html",
    "wl-ticketing/views/wl-ticketing/categories.html",
    "wl-notify/views/wl-notify/log.html",
    "wl-notify/views/wl-notify/types.html",
    "wl-notify/views/wl-notify/broadcasts.html",
    "wl-scheduling/views/wl-scheduling/bookings.html",
    "wl-scheduling/views/wl-scheduling/waiting.html",
    "wl-scheduling/views/wl-scheduling/conflicts.html",
    "wl-kernel/views/wl-kernel/integrations.html",
    "wl-kernel/views/wl-kernel/compliance_alerts.html",
    "wl-kernel/views/wl-kernel/legal_entities.html",
];

#[test]
fn every_shared_module_list_is_a_grid_that_fills_the_page() {
    for template in GRIDS {
        grid_is_a_child_of_the_page(&format!("../crates/{template}"));
    }
}

/// The lists of rows built in code fill the screen with one pane that
/// scrolls by itself, and are not wrapped in a stack or a card.
#[test]
fn the_lists_built_in_code_fill_the_page_with_one_pane() {
    let root = env!("CARGO_MANIFEST_DIR");
    for path in [
        format!("{root}/../crates/wl-parties/views/wl-parties/block_effects.html"),
        format!("{root}/resources/views/settings/compliance.html"),
        format!("{root}/resources/views/settings/negative_flow.html"),
        format!("{root}/resources/views/approvals/index.html"),
    ] {
        let source = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{path}"));
        assert!(
            source.contains("<section class=\"rx-pane rx-fill\">"),
            "{path} fills the page"
        );
        assert!(
            !source.contains("<rx-stack>\n    <rx-page-header"),
            "{path}"
        );
        assert!(!source.contains(" card>"), "{path} has no card table");
        assert!(source.contains("name=\"q\""), "{path} has a search");
    }
}
