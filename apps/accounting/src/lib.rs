#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod activation;
pub mod calendar;
pub mod cdd;
pub mod cdd_files;
pub mod cdd_machine;
pub mod client_notices;
mod db;
pub mod deadline_notices;
pub mod documents;
pub mod engagement;
pub mod engagement_machine;
pub mod engagements;
mod events;
pub mod kyc_updates;
pub mod pages;
pub mod permissions;
pub mod portal;
pub mod reasons;
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
pub mod tax_return;
mod texts;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use cdd::{CddFile, CddParty, CddStatus, RiskRating, ScreenedRole};
pub use engagement::{Engagement, EngagementStatus, FeeBasis, NewEngagement};
pub use tax_profile::{TaxProfile, VatPeriod};
pub use tax_return::{Return, ReturnSignature, ReturnStatus, ReturnType};

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
        "accounting/portal_return.html",
        include_str!("../views/accounting/portal_return.html"),
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
            .view_data::<pages::portal_returns::PortalReturnPage>();
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
            .model::<client_notices::ClientNotice>()
            .model::<deadline_notices::DeadlineNotice>()
            .model::<request_lists::RequestList>()
            .model::<Return>()
            .model::<ReturnSignature>();
        engagement_machine::register();
        return_machine::register();
        return_effects::register_types();
        cdd_machine::register();
        // `engagement.activate` checks the client's KYC documents.
        documents::register();
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
