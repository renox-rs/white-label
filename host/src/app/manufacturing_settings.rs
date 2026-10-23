//! The shared-mechanism settings of the 07 Manufacturing client on the two
//! settings pages (`/settings/compliance`, `/settings/negative-flow`,
//! BL-MFG-14), next to the other clients' tables.
//!
//! One row per line of the issue's settings table. A value is read back from
//! where it is in force: the document kinds from the registry wl-documents
//! installs from, the notification types and the compliance-critical rules
//! from the kernel registries, the numbers from the module builders
//! ([`super::manufacturing_seed`]) called with the same constants. The last
//! columns say what the value is set by and the spec reference; the decisions
//! still open are flagged (`L-19`, `A-04`).

use manufacturing::conformity::{ECAS, EQM, FOOD_CONTACT};

use super::compliance_settings::{Row, kind_text, type_text};
use super::manufacturing_seed::{self as seed, MANUFACTURING};
use super::negative_flow::{self, Row as MoneyRow, aed};
use super::roles::{ADMIN, OWNER, PRODUCTION_MANAGER, PRODUCTION_PLANNER, SALES_MANAGER};

/// The row's value is this client's seed.
const SEED: &str = "Manufacturing seed (BL-MFG-14)";
/// The row's value is the module's own default.
const MODULE: &str = "default of the module";
/// The row's value is set by the host for this client.
const HOST: &str = "set by the host for this client";

fn row(
    number: &str,
    group: &str,
    setting: &str,
    value: String,
    source: &str,
    reference: &str,
) -> Row {
    Row {
        number: number.to_owned(),
        group: group.to_owned(),
        setting: setting.to_owned(),
        value,
        source: source.to_owned(),
        reference: reference.to_owned(),
        open: String::new(),
    }
}

fn open(mut row: Row, item: &str) -> Row {
    row.open = item.to_owned();
    row
}

fn money(number: &str, group: &str, setting: &str, value: String, source: &str) -> MoneyRow {
    MoneyRow {
        number: number.to_owned(),
        group: group.to_owned(),
        setting: setting.to_owned(),
        value,
        source: source.to_owned(),
    }
}

fn on(flag: bool) -> &'static str {
    if flag { "on" } else { "off" }
}

fn days(list: &[i64]) -> String {
    list.iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join("/")
}

/// The 07 client's document, rule, notice, late-entry, sales and staff
/// settings, numbered in the order of the issue's table.
pub fn compliance_rows(owner_role: &str) -> Vec<Row> {
    let sales = seed::sales(wl_sales::WlSales::new());
    let sales = sales.config();
    let hr = seed::hr(wl_hr::WlHr::new());
    let hr = hr.config();
    let critical: Vec<String> = ["MFG.RUL.08", "DOC.RUL.03"]
        .iter()
        .map(|rule| {
            let state = if wl_kernel::compliance::is_critical(rule) {
                "marked"
            } else {
                "not registered"
            };
            format!("{rule} ({state})")
        })
        .collect();
    let conformity: Vec<String> = [ECAS, EQM, FOOD_CONTACT]
        .iter()
        .map(|key| format!("{key}: {}", kind_text(key)))
        .collect();
    let licences: Vec<String> = seed::COMPANY_KINDS
        .iter()
        .map(|(key, ..)| format!("{key}: {}", kind_text(key)))
        .collect();
    let channels: Vec<String> = seed::CAMPAIGN_CHANNELS
        .iter()
        .map(|(name, kind)| format!("{name} (recorded as {kind})"))
        .collect();
    let onboarding: Vec<String> = seed::ONBOARDING
        .iter()
        .map(|o| format!("{} ({} steps)", o.role, o.steps.len()))
        .collect();
    let sops: Vec<String> = seed::SOPS
        .iter()
        .map(|s| {
            format!(
                "{}: owner {}, readers {}",
                s.title,
                s.owner_role,
                s.readers.join(", ")
            )
        })
        .collect();
    vec![
        open(
            row("1", "Documents", "Kinds ECAS, EQM, food-contact (subject item): blocks",
                conformity.join(" | "), SEED, "§7.1 Document blocks"),
            "L-19 (whether these are legal blocks in the strict sense)",
        ),
        open(
            row("2", "Documents", "Override mode of the three conformity kinds",
                "never: no override is offered anywhere and the mode cannot be changed in the settings (every entry is locked); any one valid kind among the item's accepted kinds passes".to_owned(),
                SEED, "§7.1 Document blocks"),
            "L-19 (until answered, never)",
        ),
        row("3", "Documents", "Industrial licence, Industrial Registry, ICV certificate",
            format!("{}; reminders only (DOC.RUL.01), no blocks; the ICV certificate reminds at {} days", licences.join(" | "), days(&seed::ICV_REMINDERS)),
            SEED, "§7.1 Document blocks; §13 q7"),
        row("4", "Documents", "Responsible role per kind",
            format!("conformity certificates: the QC inspectors of each plant that holds stock of the item or has an open work order for it, else of every plant; industrial licence, Industrial Registry, ICV certificate: {OWNER}; export exit evidence: accountant"),
            SEED, "§7.1 Responsible role per document kind"),
        row("5", "Notices", "Notification types",
            format!("{} | {} | no 07 type is statutory", type_text(seed::RECALL_NOTICE), type_text(manufacturing::notices::NOTICE)),
            "Manufacturing seed; wl-notify §5.4", "§7.1 Notification types"),
        row("6", "Rules", "Compliance-critical rules", critical.join("; "),
            "marked by the host for this client", "§7.1 Compliance-critical rules"),
        row("7", "Rules", "Owner role (PLT.RUL.04)",
            format!("{owner_role}; the company admin ({ADMIN}) is alerted as well, once when it is the same role"),
            "client.toml [roles] owner, else the company admin", "§7.1 Owner role"),
        row("8", "Integrations", "Line tablets down",
            format!("paper job cards: the supervisor enters the output at the end of the shift with its business time ({} on the work order page); more than {} hours late needs the production manager", "Made at", seed::LATE_ENTRY_APPROVAL_HOURS),
            HOST, "§7.1 Integrations; §8"),
        row("9", "Integrations", "E-invoicing ASP and e-mail down",
            "invoices queued and sent on recovery (PLT.RUL.05); documents downloaded from the record and sent by hand".to_owned(),
            MODULE, "§7.1 Integrations"),
        row("10", "Late entry", "Late entry of a production report (PLT.ACL.10)",
            format!("business time = when the output was made; flagged with a reason after M = {} minutes; the {PRODUCTION_MANAGER} approves after N = {} hours ({})",
                seed::LATE_ENTRY_FLAG_MINUTES, seed::LATE_ENTRY_APPROVAL_HOURS, manufacturing::late_report::SUBJECT_TYPE),
            "limits seeded by `manufacturing:install`, editable per entity", "§7.1 Late entry"),
        row("11", "Late entry", "Digest of voids and late entries (PLT.RUL.06)",
            format!("on, daily at {} ({}), to the holders of late_entries.approve",
                wl_kernel::digest::TASK_AT, wl_kernel::digest::TASK_ZONE),
            MODULE, "§7.2"),
        row("12", "Sales", "Lead sources used (SAL.CRM.02)",
            format!("{}; enquiries and RFQs are leads; a source requires a campaign: {}",
                seed::LEAD_SOURCES.join(", "),
                if sales.campaign_required_sources.is_empty() { "none".to_owned() } else { sales.campaign_required_sources.join(", ") }),
            SEED, "§5; §7.1 settings"),
        row("13", "Sales", "Campaign channels (SAL.CRM.02, SAL.ACL.07)",
            format!("{}; converted value: the customer's first sales order; a cost entry over the budget needs a manager: {}",
                channels.join(", "), on(sales.over_budget_approval)),
            "channels: the module's fixed list; approval: Manufacturing seed", "§5; §7.1 settings"),
        row("14", "Staff", "Onboarding checklists (HRM.ONB.01, HRM.RUL.16, HRM.RUL.17, HRM.ACL.10)",
            format!("{}; started when the employee record is created in onboarding; tasks to the {PRODUCTION_MANAGER}, the line manager (the {}) and the employee; the employee's own steps only by them; completion blocks nothing; HRM.RUL.17 reminds on the first run after the due date, then every {} days, and tells the {PRODUCTION_MANAGER} and the line supervisor after {} days",
                onboarding.join("; "), hr.manager_role,
                wl_hr::reminders::REMIND_EVERY, wl_hr::reminders::ESCALATE_AFTER),
            "checklists: Manufacturing seed; reminder days: default of the module", "§5; §7.1 settings"),
        row("15", "Staff", "SOP library (HRM.ONB.01, DOC.RUL.04, HRM.ACL.10)",
            format!("{}; DOC.RUL.04 asks each reader to acknowledge within {} days, reminds every {} days and tells the reader's manager and the SOP owner role after {} days overdue; only the reader acknowledges; a new version asks again",
                sops.join("; "),
                wl_documents::sops::service::DEFAULT_ACK_DUE_DAYS,
                wl_documents::acknowledgements::run::REMIND_EVERY,
                wl_documents::acknowledgements::run::ESCALATE_AFTER),
            "SOPs and readers: Manufacturing seed; days: default of the module", "§5; §7.1 settings"),
        row("16", "Registrations", "Registration and certification kinds (DOC.EXP.01, DOC.RUL.01)",
            format!("Halal certificate, ISO certificate and Other certification ({} days), responsible role qc_inspector; Trademark registration ({} days), responsible role {OWNER}; no blocks",
                days(&[90, 30, 7]), days(&[180, 90, 30])),
            "default of the module, on for 07", "§5; §7.1 settings"),
    ]
}

/// The 07 client's sales, receivables, stock, cash and budget settings,
/// numbered in the order of the issue's table.
pub fn money_rows(entities: &[String]) -> Vec<MoneyRow> {
    let invoicing = negative_flow::invoicing_for(MANUFACTURING, entities);
    let invoicing = invoicing.config();
    let payments = negative_flow::payments_for(MANUFACTURING);
    let payments = payments.config();
    let sales = seed::sales(wl_sales::WlSales::new());
    let sales = sales.config();
    let stock = seed::inventory(wl_inventory::WlInventory::new());
    let stock = stock.config();
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
        money("1", "Void", "Void allowed",
            "production reports before the work order is completed (approved by the line supervisor or the production manager); goods receipts not yet billed (reverse); draft dispatches (cancel). Never: invoices, delivery notes, certificates already sent (credit note or new version instead)".to_owned(),
            "kernel void registry"),
        money("2", "Sales", "Cancellation outcomes offered",
            format!("{}; the proposal follows the item's customer-specific flag (scrap if on, finish to stock if off); sales may change it per line",
                sales.cancel_outcomes.join(", ")),
            HOST),
        money("3", "Sales", "Cancellation fee (SAL.ACL.05)",
            format!("{}% default, settable per customer; VAT category of the fee line: {} (open: accountant A-04); applies to sales, approver {PRODUCTION_MANAGER}",
                sales.cancellation_fee_bps / 100, sales.cancellation_fee_vat),
            HOST),
        money("4", "Sales", "Unique customer PO number per customer",
            on(sales.unique_customer_po).to_owned(), HOST),
        money("5", "Sales", "Discount limit (SAL.ACL.02)",
            format!("{}%; above it {SALES_MANAGER} decides", sales.manual_discount_limit_bps / 100), HOST),
        money("6", "Sales", "Overdue threshold (SAL.ACL.03)",
            format!("overdue more than {} days holds the next order on account", sales.credit_grace_days), HOST),
        money("7", "Sales", "Credit re-check at dispatch",
            on(sales.credit_recheck_at_dispatch).to_owned(), HOST),
        money("8", "Sales", "Bounced cheque hold (SAL.RUL.05)",
            format!("{}: every open order on account of the customer", on(sales.bounced_cheque_hold_group)), HOST),
        money("9", "Sales", "Partial acceptance at proof of delivery",
            format!("on; the order is fulfilled and the invoice drafted for the accepted quantities: {}", on(sales.invoice_on_delivery)), HOST),
        money("10", "Party blocked", "Effect of a block",
            format!("sales orders not yet dispatched: pause (decider {SALES_MANAGER}); work orders: continue (the {PRODUCTION_PLANNER} may unlink them to stock); money in: continue"),
            "kernel block registry; Manufacturing"),
        money("11", "Receivables", "Invoice write-off (FIN.ACL.15)",
            format!("limit AED 0.00 (always approval); approvers {}; the requester cannot approve", invoicing.write_off_approvers.join(" or ")), HOST),
        money("12", "Receivables", "Bad-debt relief proposal (FIN.RUL.14)",
            format!("{} after {} months", on(invoicing.bad_debt_relief), invoicing.bad_debt_relief_months.get()), HOST),
        money("13", "Receivables", "Bounced cheque (FIN.RUL.18)",
            format!("bounce fee {}; reminders at {} and {} days; at most {} presentations",
                payments.bounce_fee.as_ref().map_or_else(|| "off".to_owned(), |fee| format!("{} {}", fee.charge_type, aed(fee.amount))),
                payments.cheque_reminder_days, payments.cheque_decision_days, payments.cheque_max_presentations),
            HOST),
        money("14", "Receivables", "Invoice disputes",
            format!("by the accountant and the customer buyer in the portal: {}; escalation FIN.RUL.15 after {} days to {}",
                on(invoicing.portal_disputes), invoicing.dispute_escalation_days, invoicing.dispute_escalation_role),
            HOST),
        money("15", "Stock", "Wastage approval (STK.ACL.02)",
            format!("above {}", aed(stock.wastage_approval_fils)), HOST),
        money("16", "Stock", "Return to supplier approval (STK.ACL.06)",
            format!("above {}; procurement, storekeeper", aed(stock.return_approval_fils)), HOST),
        money("17", "Stock", "Recall release by a second person (STK.ACL.05)",
            on(stock.release_needs_second_person("recall")).to_owned(), HOST),
        money("18", "Stock", "Negative stock policy",
            "block in every location: a work order issue below zero becomes a pending issue (STK.RUL.06), a transfer below zero is refused".to_owned(),
            "default of the module"),
        money("19", "Stock", "Open stock hold reminder (STK.RUL.07)",
            format!("{} hours", stock.hold_reminder_hours), HOST),
        money("20", "Stock", "Return chase (STK.RUL.08)",
            format!("{} days", stock.return_chase_days), HOST),
        money("21", "Stock", "Recall openers",
            format!("recalls.manage: {OWNER}, {PRODUCTION_MANAGER}"), "role permissions"),
        money("22", "Stock", "Supplier scorecard (STK.SUP.01, STK.RUL.09, STK.ACL.07)",
            format!("score 0-100 over {} months (at least {} receipts); below {} opens a review task for the procurement officer; changing the preferred supplier needs a second person: {}",
                stock.score.window_months, stock.score.min_receipts, stock.score.threshold, on(stock.preferred_change_needs_approval)),
            HOST),
        money("23", "Cash-flow", "Forecast alert (FIN.CFL.01, FIN.RUL.23)",
            format!("{} weeks from {:?}; alert below {}; {} at {}; due dates moved by days late: {}; disputed amounts counted: {}",
                invoicing.cash_forecast_weeks, invoicing.cash_week_start, thresholds,
                invoicing.cash_alert_roles.join(" and "), invoicing.cash_alert_time,
                on(invoicing.cash_days_late), on(invoicing.cash_disputed_counted)),
            HOST),
        money("24", "Budget", "Budget categories and approval (FIN.BUD.01, FIN.RUL.24)",
            format!("categories owned by the accountant; one budget per legal entity and fiscal year, approved by {OWNER}; warns at {}% of a month's line: the category's owner role and {}",
                invoicing.budget_variance_percent, invoicing.budget_variance_role),
            HOST),
        money("25", "Budget", "Owner drawings (FIN.ACL.21)",
            format!("every drawing approved by {OWNER}, never the requester; {} told when none else can approve",
                invoicing.drawings_notify_role),
            HOST),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compliance_page_lists_every_line_once() {
        let rows = compliance_rows("owner");
        let numbers: Vec<String> = rows.iter().map(|r| r.number.clone()).collect();
        let expected: Vec<String> = (1..=16).map(|n| n.to_string()).collect();
        assert_eq!(numbers, expected);
        assert!(
            rows.iter()
                .all(|r| !r.value.is_empty() && !r.source.is_empty() && !r.reference.is_empty())
        );
    }

    #[test]
    fn only_the_conformity_blocks_wait_on_the_lawyer() {
        let open: Vec<String> = compliance_rows("owner")
            .into_iter()
            .filter(|r| r.open.contains("L-19"))
            .map(|r| r.number)
            .collect();
        assert_eq!(open, ["1", "2"]);
    }

    #[test]
    fn the_money_page_lists_every_line_once() {
        let rows = money_rows(&["AWP".to_owned()]);
        let numbers: Vec<String> = rows.iter().map(|r| r.number.clone()).collect();
        let expected: Vec<String> = (1..=25).map(|n| n.to_string()).collect();
        assert_eq!(numbers, expected);
    }
}
