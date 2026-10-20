#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod activation;
pub mod aml;
pub mod aml_cases;
pub mod aml_machine;
pub mod calendar;
pub mod cdd;
pub mod cdd_files;
pub mod cdd_machine;
pub mod client_notices;
pub mod corrections;
mod db;
pub mod deadline_notices;
pub mod documents;
pub mod ending;
pub mod engagement;
pub mod engagement_machine;
pub mod engagements;
mod events;
pub mod exits;
pub mod filing;
pub mod kpi;
pub mod kpi_machine;
pub mod kpi_packs;
pub mod kyc_updates;
pub mod letters;
pub mod pages;
pub mod permissions;
pub mod portal;
pub mod reasons;
pub mod regeneration;
pub mod request_lists;
pub mod retainer;
pub mod return_effects;
pub mod return_machine;
pub mod returns;
pub mod screening;
pub mod seed;
pub mod suspension;
pub mod tasks;
pub mod tax_profile;
pub mod tax_profile_change;
pub mod tax_return;
mod texts;
pub mod wip;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use aml::{AmlCase, AmlDecision, AmlKind, AmlMatch, AmlStatus};
pub use cdd::{CddFile, CddParty, CddStatus, RiskRating, ScreenedRole};
pub use engagement::{Engagement, EngagementStatus, FeeBasis, NewEngagement};
pub use kpi::{KpiFigure, KpiMapping, KpiPack, KpiStatus};
pub use letters::EngagementLetter;
pub use tax_profile::{TaxProfile, VatPeriod};
pub use tax_return::{Return, ReturnSignature, ReturnStatus, ReturnType};
pub use wip::{WipWriteOff, WriteOffStatus};

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory (`resources/views/accounting/engagement.html`).
const VIEWS: &[(&str, &str)] = &[
    (
        "accounting/engagements.html",
        include_str!("../views/accounting/engagements.html"),
    ),
    (
        "accounting/engagement.html",
        include_str!("../views/accounting/engagement.html"),
    ),
    (
        "accounting/cdd_file.html",
        include_str!("../views/accounting/cdd_file.html"),
    ),
    (
        "accounting/deadline_notice.html",
        include_str!("../views/accounting/deadline_notice.html"),
    ),
    (
        "accounting/returns.html",
        include_str!("../views/accounting/returns.html"),
    ),
    (
        "accounting/return.html",
        include_str!("../views/accounting/return.html"),
    ),
    (
        "accounting/tax_profile.html",
        include_str!("../views/accounting/tax_profile.html"),
    ),
    (
        "accounting/portal_return.html",
        include_str!("../views/accounting/portal_return.html"),
    ),
    (
        "accounting/aml_cases.html",
        include_str!("../views/accounting/aml_cases.html"),
    ),
    (
        "accounting/aml_case.html",
        include_str!("../views/accounting/aml_case.html"),
    ),
    (
        "accounting/portal_letter.html",
        include_str!("../views/accounting/portal_letter.html"),
    ),
    (
        "accounting/kpi_packs.html",
        include_str!("../views/accounting/kpi_packs.html"),
    ),
    (
        "accounting/kpi_pack.html",
        include_str!("../views/accounting/kpi_pack.html"),
    ),
    (
        "accounting/portal_kpi_pack.html",
        include_str!("../views/accounting/portal_kpi_pack.html"),
    ),
];

/// Dirhams as text from fils: `AED 25,000.00`.
pub fn money_text(fils: i64) -> String {
    let sign = if fils < 0 { "-" } else { "" };
    let magnitude = fils.unsigned_abs();
    let whole = (magnitude / 100).to_string();
    let mut grouped = String::new();
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("{sign}AED {grouped}.{:02}", magnitude % 100)
}

/// The Accounting module. Add it to the app with
/// `.module(accounting::Accounting)`.
pub struct Accounting;

impl Module for Accounting {
    fn name(&self) -> &'static str {
        "accounting"
    }

    fn routes(&self) -> Routes {
        pages::routes()
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.view_data::<pages::engagements::IndexPage>()
            .view_data::<pages::engagements::ShowPage>()
            .view_data::<pages::cdd::CddPage>()
            .view_data::<pages::deadline_notices::NoticePage>()
            .view_data::<pages::returns::IndexPage>()
            .view_data::<pages::returns::ShowPage>()
            .view_data::<pages::tax_profiles::ShowPage>()
            .view_data::<pages::portal_returns::PortalReturnPage>()
            .view_data::<pages::letters::PortalLetterPage>()
            .view_data::<pages::aml::IndexPage>()
            .view_data::<pages::aml::ShowPage>()
            .view_data::<pages::kpi::IndexPage>()
            .view_data::<pages::kpi::ShowPage>()
            .view_data::<pages::kpi::PortalPage>();
        app.templates(|env| {
            for (name, source) in VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
        app.model::<Engagement>()
            .model::<CddFile>()
            .model::<CddParty>()
            .model::<TaxProfile>()
            .model::<tax_profile_change::TaxProfileChange>()
            .model::<client_notices::ClientNotice>()
            .model::<deadline_notices::DeadlineNotice>()
            .model::<request_lists::RequestList>()
            .model::<AmlCase>()
            .model::<AmlMatch>()
            .model::<letters::EngagementLetter>()
            .model::<Return>()
            .model::<ReturnSignature>()
            .model::<WipWriteOff>()
            .model::<KpiPack>()
            .model::<KpiFigure>()
            .model::<KpiMapping>();
        engagement_machine::register();
        return_machine::register();
        // `ACC.ACL.13`: a tax profile change waits for another manager.
        tax_profile_change::register();
        wip::register();
        kpi_machine::register();
        ending::register(app);
        return_effects::register_types();
        cdd_machine::register();
        // The AML case: the MLRO's alone (`ACC.ACL.05`), BL-ACC-10.
        aml_machine::register();
        aml_cases::register();
        // `engagement.activate` checks the client's KYC documents.
        documents::register();
        // `ACC.RUL.05` and `DOC.RUL.03` are compliance-critical too (BL-ACC-23).
        seed::register_critical_rules();
        events::register(app);
        // An expired KYC document of a client being served opens a CDD
        // update task (BL-ACC-12).
        kyc_updates::register(app);
        // `ACC.RUL.04` (wl-ticketing's `mark_overdue` run under the 04 id)
        // is compliance-critical, owned by the partner (BL-ACC-13).
        tasks::register();
        // The neutral templates the MLRO sends (`ACC.ACL.14`).
        client_notices::register_types();
        // `ACC.RUL.09`: the statutory deadline notice on suspend or end, its
        // daily follow-up, compliance-critical (BL-ACC-04).
        deadline_notices::register(app);
        // `ACC.RUL.02` and `POR.RUL.01`: the request lists of the periods,
        // skipped while an engagement is suspended (BL-ACC-03, BL-ACC-05).
        request_lists::register(app);
        // `ACC.RUL.11`: the unsigned engagement letter is chased and, at 30
        // days, the manager decides (BL-ACC-02).
        letters::register(app);
        // `ACC.RUL.06`, the client notices of a published or withdrawn KPI
        // pack and the privacy incident task (BL-ACC-20, BL-ACC-21).
        kpi_packs::register(app);
        // A voluntary disclosure whose task was lost after the commit is
        // opened by the daily catch-up (BL-ACC-18).
        corrections::register(app);
        // In the app group of the staff sidebar; the label is the host's
        // `nav.engagements` text.
        app.nav(
            "app",
            NavLink::route("accounting.engagements", "nav.engagements")
                .icon("handshake")
                .group("nav.app")
                .order(10)
                .permission(permissions::ENGAGEMENTS_VIEW),
        );
        // The MLRO's cases: only their holders see the link (`aml.manage`).
        app.nav(
            "app",
            NavLink::route("accounting.aml_cases", "nav.aml_cases")
                .icon("shield-alert")
                .group("nav.app")
                .order(12)
                .permission(permissions::AML_MANAGE),
        );
        app.nav(
            "app",
            NavLink::route("accounting.kpi_packs", "nav.kpi_packs")
                .icon("chart-line")
                .group("nav.app")
                .order(13)
                .permission(permissions::KPI_VIEW),
        );
        app.nav(
            "app",
            NavLink::route("accounting.returns", "nav.returns")
                .icon("file-text")
                .group("nav.app")
                .order(11)
                .permission(permissions::RETURNS_VIEW),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_reads_as_dirhams_with_grouped_thousands() {
        assert_eq!(money_text(2_500_000), "AED 25,000.00");
        assert_eq!(money_text(5), "AED 0.05");
        assert_eq!(money_text(-123_456_789), "-AED 1,234,567.89");
        assert_eq!(money_text(0), "AED 0.00");
    }
}
