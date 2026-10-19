//! The shared negative-flow settings of money for the Property client (01
//! Property §3.3, §4.1, §4.2, §4.4, §7.1, §7.2, §9; BL-PRP-26).
//!
//! The shared modules (`wl-invoicing`, `wl-payments`) keep their settings as
//! builder calls on the module (their `config()`); this file is the one place
//! where the Property client's values are set ([`invoicing_for`],
//! [`payments_for`]) and the page `/settings/negative-flow` ([`NegativeFlow`])
//! reads the same configuration back, so a value shown is a value in force.
//! Nothing here is a rule: the rules (`FIN.RUL.15`, `FIN.RUL.18`,
//! `FIN.RUL.23`, `FIN.RUL.24`, e-invoicing) run in the modules with these
//! values. Where a value is the module's own default, or is set by another
//! issue's seed (the ticket categories, BL-PRP-27), the page says where.
//!
//! Left to the client on purpose: the bounce fee (`⚠ open: lawyer L-22`, VAT
//! `⚠ open: accountant A-05`: none until set), the e-invoicing go-live date of
//! each entity (`WlInvoicing::einvoicing`, 1 January 2027 for revenue of AED 50
//! million or more, 1 July 2027 for the others) and the ASP adapter.

use renox::chrono::Weekday;
use renox::nav::NavLink;
use renox::prelude::*;
use serde::Serialize;
use wl_invoicing::DuplicateKey::{ContractPeriodAmount, SupplierInvoiceNumber};
use wl_invoicing::WlInvoicing;
use wl_payments::WlPayments;

use super::roles::{ACCOUNTANT, ADMIN, PROPERTY_MANAGER, SETTINGS_VIEW};

/// Days a dispute may stay open before it is escalated (`FIN.RUL.15`).
pub const DISPUTE_ESCALATION_DAYS: u32 = 14;
/// Days the tenant may dispute published deposit deductions.
pub const DEDUCTIONS_WINDOW_DAYS: i64 = 14;
/// Days staff have to answer a deductions dispute.
pub const DEDUCTIONS_RESPONSE_DAYS: i64 = 7;
/// The most presentations of one cheque.
pub const CHEQUE_PRESENTATIONS: i64 = 2;
/// Days after a bounce when the tenant and the property manager are reminded.
pub const BOUNCE_REMIND_DAYS: i64 = 3;
/// Days after a bounce when the accountant and the property manager decide.
pub const BOUNCE_DECIDE_DAYS: i64 = 14;
/// The projected balance below which `FIN.RUL.23` alerts, in fils (AED 50,000).
pub const CASH_THRESHOLD: i64 = 5_000_000;

/// The invoicing module with the Property client's settings: duplicate vendor
/// bills held by two keys (`PRP.ACL.22`), a write-off approved by a property
/// manager other than the requester (`FIN.ACL.15`), the tenant's dispute in
/// the portal, the dispute escalation to the property manager after
/// [`DISPUTE_ESCALATION_DAYS`], and the cash-flow alert (`FIN.RUL.23`) for the
/// company admin and the accountant with the due dates moved by each tenant's
/// average days late and the disputed amounts left out. `entities` are the
/// codes of the client's legal entities: each gets the [`CASH_THRESHOLD`].
pub fn invoicing_for(client_app: &str, entities: &[String]) -> WlInvoicing {
    let module = WlInvoicing::new();
    match client_app {
        "property" => {
            let mut module = module
                .duplicate_keys(&[SupplierInvoiceNumber, ContractPeriodAmount])
                .write_off_approvers(&[PROPERTY_MANAGER, ADMIN])
                .portal_disputes(true)
                .dispute_escalation_days(DISPUTE_ESCALATION_DAYS)
                .dispute_escalation_role(PROPERTY_MANAGER)
                .cash_week_start(Weekday::Mon)
                .cash_days_late(true)
                .cash_disputed_counted(false)
                .cash_alert_roles(&[ADMIN, ACCOUNTANT]);
            for code in entities {
                module = module.cash_threshold(code, CASH_THRESHOLD);
            }
            module
        }
        _ => module,
    }
}

/// The payments module with the Property client's settings: the tenant may
/// dispute published deposit deductions for [`DEDUCTIONS_WINDOW_DAYS`] days
/// (staff answer within [`DEDUCTIONS_RESPONSE_DAYS`]), a cheque is presented
/// twice at most, and a bounced cheque is chased after [`BOUNCE_REMIND_DAYS`]
/// and [`BOUNCE_DECIDE_DAYS`] days. No bounce fee is set.
pub fn payments_for(client_app: &str) -> WlPayments {
    let module = WlPayments::new();
    match client_app {
        "property" => module
            .deductions_dispute(DEDUCTIONS_WINDOW_DAYS, DEDUCTIONS_RESPONSE_DAYS)
            .cheque_presentations(CHEQUE_PRESENTATIONS)
            .cheque_chase(BOUNCE_REMIND_DAYS, BOUNCE_DECIDE_DAYS),
        _ => module,
    }
}

/// One setting of the page.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The number in the spec table (`7a` for a sub-row).
    pub number: String,
    /// The group: invoices, cheques, payments, deposits, supplier bills,
    /// party blocked, cash-flow and budget.
    pub group: String,
    /// The setting.
    pub setting: String,
    /// Its value for this client.
    pub value: String,
    /// Where it is set.
    pub source: String,
}

fn on(flag: bool) -> &'static str {
    if flag { "on" } else { "off" }
}

/// What the client's configuration says, read back from the modules' own
/// settings.
pub fn rows(client_app: &str, entities: &[String]) -> Vec<Row> {
    let invoicing = invoicing_for(client_app, entities);
    let invoicing = invoicing.config();
    let payments = payments_for(client_app);
    let payments = payments.config();
    let host = "set by the host for this client";
    let module = "default of the module";
    let category = "ticket category (the Property seed)";
    let make = |number: &str, group: &str, setting: &str, value: String, source: &str| Row {
        number: number.to_owned(),
        group: group.to_owned(),
        setting: setting.to_owned(),
        value,
        source: source.to_owned(),
    };
    let einvoice = if invoicing.einvoice.is_empty() {
        "no entity has a go-live date yet; invoices to a recipient in scope queue while the ASP is down".to_owned()
    } else {
        invoicing
            .einvoice
            .iter()
            .map(|e| format!("{} from {}", e.entity_code, e.go_live))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let bounce_fee = payments.bounce_fee.as_ref().map_or_else(
        || "none until the client sets one".to_owned(),
        |fee| format!("{} {}", fee.charge_type, aed(fee.amount)),
    );
    let thresholds = if entities.is_empty() {
        "none".to_owned()
    } else {
        entities
            .iter()
            .map(|code| format!("{code} {}", aed(invoicing.cash_threshold_of(code))))
            .collect::<Vec<_>>()
            .join(", ")
    };
    vec![
        make("1", "Invoices", "Write-off approvers (limit AED 0: always an approval, never the requester)", invoicing.write_off_approvers.join(", "), host),
        make("2", "Invoices", "Invoice dispute by the party in the portal", on(invoicing.portal_disputes).to_owned(), host),
        make("3", "Invoices", "Dispute escalation (FIN.RUL.15)", format!("{} days, then {}", invoicing.dispute_escalation_days, invoicing.dispute_escalation_role), host),
        make("4", "Invoices", "Automations paused by a dispute", "FIN.RUL.01 reminders and PRP.RUL.10".to_owned(), module),
        make("5", "Invoices", "Bad-debt relief (FIN.RUL.14)", format!("{} after {} months; residential rent is exempt", on(invoicing.bad_debt_relief), invoicing.bad_debt_relief_months.get()), module),
        make("6", "Invoices", "E-invoicing go-live", einvoice, "per client (WlInvoicing::einvoicing)"),
        make("7", "Invoices", "Chargeable ticket invoiced on", "close (confirm or close), not resolve".to_owned(), category),
        make("7a", "Invoices", "Drafts issued automatically", "on for the ticket source, reviewed for the others".to_owned(), "wl-invoicing source setting"),
        make("8", "Cheques", "Bounce fee", bounce_fee, "per client (open: L-22, A-05)"),
        make("9", "Cheques", "Maximum presentations of one cheque", payments.cheque_max_presentations.to_string(), host),
        make("10", "Cheques", "Chase after a bounce (FIN.RUL.18)", format!("{} days, then the decision task at {} days", payments.cheque_reminder_days, payments.cheque_decision_days), host),
        make("11", "Cheques", "On bounce (FIN.RUL.04)", "the tenant and the property manager are told; the accountant has a task".to_owned(), module),
        make("12", "Cheques", "Legal follow-up of a bounced cheque", "civil case; the lease gets a legal case flag".to_owned(), "Property legal case flag"),
        make("13", "Cheques", "Void", "allowed while received (entered in error)".to_owned(), module),
        make("14", "Payments", "Payment void", "with a reason, approved by another user".to_owned(), module),
        make("15", "Payments", "Reversing a confirmed bank match", "a property manager only".to_owned(), "Property permission PRP.ACL.16"),
        make("16", "Payments", "Late entry of cheque deposits", "allowed; business time is the deposit date".to_owned(), module),
        make("17", "Deposits", "Deductions dispute by the tenant", format!("{}; window {} days, response {} days", on(payments.deductions_dispute), payments.dispute_window_days, payments.dispute_response_days), host),
        make("18", "Deposits", "Deduction approval (PRP.ACL.05)", format!("a property manager; above {} the finance manager or the owner too", aed(property::deposit_forfeit::SECOND_APPROVAL_ABOVE)), "Property"),
        make("19", "Deposits", "Refund separation of duties (FIN.ACL.04)", "on".to_owned(), module),
        make("20", "Supplier bills", "Duplicate check keys (PRP.ACL.22)", invoicing.duplicate_keys.iter().map(|k| format!("{k:?}")).collect::<Vec<_>>().join(", "), host),
        make("20a", "Supplier bills", "Bill after the contract end", "each line needs a service date; work after the end goes to exception".to_owned(), module),
        make("21", "Party blocked", "Block effect on leases", "continue; decided by the property manager".to_owned(), "block effect kinds"),
        make("22", "Party blocked", "Block effect on service contracts", "review; decided by the property manager".to_owned(), "block effect kinds"),
        make("23", "Party blocked", "Money out stopped for a blocked party", "deposit refund, owner payout, vendor payment; exception through PTY.ACL.03".to_owned(), module),
        make("24", "Party blocked", "Money in from a blocked party", "continues".to_owned(), module),
        make("25", "Cash-flow", "Forecast alert (FIN.RUL.23)", format!("{} weeks from {:?}; alert below {}; {} at {}; due dates moved by days late: {}; disputed amounts counted: {}", invoicing.cash_forecast_weeks, invoicing.cash_week_start, thresholds, invoicing.cash_alert_roles.join(" and "), invoicing.cash_alert_time, on(invoicing.cash_days_late), on(invoicing.cash_disputed_counted)), host),
        make("26", "Budget", "Budget categories and approval (FIN.RUL.24)", format!("warns at {}% of a month's line: the category's owner role and {}", invoicing.budget_variance_percent, invoicing.budget_variance_role), module),
        make("27", "Budget", "Owner drawings (FIN.ACL.21)", format!("every drawing approved by the company admin, never the requester; payouts to landlords are not drawings; {} told when none else can approve", invoicing.drawings_notify_role), module),
    ]
}

/// `fils` as `AED 50,000.00`.
pub fn aed(fils: i64) -> String {
    property::money_text(fils)
}

/// The Property client's values and the page that shows them.
#[derive(Debug, Clone)]
pub struct NegativeFlow {
    client_app: String,
    entities: Vec<String>,
}

impl NegativeFlow {
    /// The settings of the client running `client_app` with the legal
    /// entities `entities` (their codes).
    pub fn new(client_app: &str, entities: Vec<String>) -> Self {
        NegativeFlow {
            client_app: client_app.to_owned(),
            entities,
        }
    }
}

impl Module for NegativeFlow {
    fn name(&self) -> &'static str {
        "negative_flow"
    }

    fn register(&self, app: &mut Registry) {
        app.provide(self.clone());
        app.nav(
            "app",
            NavLink::route("settings.negative_flow", "nav.negative_flow")
                .icon("sliders-horizontal")
                .group("nav.admin")
                .order(12)
                .permission(SETTINGS_VIEW),
        );
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .get("/settings/negative-flow", show)
            .name("settings.negative_flow")
            .require_permission(SETTINGS_VIEW)
            .require_verified()
    }
}

async fn show(State(state): State<AppState>) -> Result<Response> {
    let settings = state.provided::<NegativeFlow>().ok_or(Error::NotFound)?;
    let rows = rows(&settings.client_app, &settings.entities);
    Ok(view("settings/negative_flow.html", context! { rows }).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entities() -> Vec<String> {
        vec!["PGP".to_owned(), "PGFZ".to_owned()]
    }

    #[test]
    fn the_property_client_has_the_seeded_money_values() {
        let invoicing = invoicing_for("property", &entities());
        let config = invoicing.config();
        assert!(config.portal_disputes);
        assert_eq!(config.duplicate_keys, [SupplierInvoiceNumber, ContractPeriodAmount]);
        assert_eq!(config.dispute_escalation_days, 14);
        assert_eq!(config.dispute_escalation_role, PROPERTY_MANAGER);
        assert_eq!(config.write_off_approvers, [PROPERTY_MANAGER, ADMIN]);
        assert_eq!(config.cash_threshold_of("PGP"), 5_000_000);
        assert_eq!(config.cash_threshold_of("PGFZ"), 5_000_000);
        assert_eq!(config.cash_alert_roles, [ADMIN, ACCOUNTANT]);
        assert!(config.cash_days_late);
        assert!(!config.cash_disputed_counted);
        assert_eq!(config.cash_forecast_weeks, 13);
        assert_eq!(config.cash_week_start, Weekday::Mon);
        assert_eq!(config.cash_alert_time, "06:00");

        let payments = payments_for("property");
        let config = payments.config();
        assert!(config.deductions_dispute);
        assert_eq!((config.dispute_window_days, config.dispute_response_days), (14, 7));
        assert_eq!(config.cheque_max_presentations, 2);
        assert_eq!((config.cheque_reminder_days, config.cheque_decision_days), (3, 14));
        assert!(config.bounce_fee.is_none(), "the seed sets no bounce fee");
    }

    #[test]
    fn another_client_keeps_the_module_defaults() {
        assert!(!invoicing_for("other", &entities()).config().portal_disputes);
        assert!(!payments_for("other").config().deductions_dispute);
    }

    #[test]
    fn the_page_lists_every_item_of_the_spec_once() {
        let rows = rows("property", &entities());
        let numbers: Vec<&str> = rows.iter().map(|r| r.number.as_str()).collect();
        let mut expected: Vec<String> = (1..=27).map(|n| n.to_string()).collect();
        expected.insert(7, "7a".to_owned());
        expected.insert(21, "20a".to_owned());
        assert_eq!(numbers, expected);
        let bounce = rows.iter().find(|r| r.number == "8").expect("item 8");
        assert!(bounce.value.starts_with("none"));
    }
}
