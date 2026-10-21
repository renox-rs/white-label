//! The shared-mechanism settings of the 03 Logistics client on the two
//! settings pages (`/settings/compliance`, BL-LOG-25; `/settings/negative-flow`,
//! BL-LOG-26), next to the Property rows of [`super::compliance_settings`] and
//! [`super::negative_flow`], which show them for a Property client.
//!
//! One row per line of the issues' settings tables. A value is read back from
//! where it is in force: the document kinds from the registry wl-documents
//! installs from, the notification types and the compliance-critical rules
//! from the kernel registries, the connections from the integrations registry,
//! the numbers from the module builders ([`super::logistics_seed`]) called with
//! the same constants ([`logistics::defaults`]). The last columns say what the
//! value is set by and the spec reference; the two decisions still open with
//! the lawyer are flagged (`L-19`, `L-20`).

use logistics::defaults::{self, KINDS};
use logistics::notices;
use logistics::rates::aed;
use wl_kernel::integrations;

use super::compliance_settings::{Row, kind_text, type_text};
use super::logistics_seed::{self, LOGISTICS};
use super::negative_flow::{self, Row as MoneyRow};
use super::roles::{ADMIN, OPERATIONS_MANAGER};

/// The row's value is this client's seed.
const SEED: &str = "Logistics seed (BL-LOG-25)";
/// The row's value is the module's own default.
const MODULE: &str = "default of the module";
/// The row's value is set by the host for this client.
const HOST: &str = "set by the host for this client";

fn row(number: &str, group: &str, setting: &str, value: String, source: &str, reference: &str) -> Row {
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

/// The providers registered for a guarded transition, in words.
fn providers(guarded: &str) -> String {
    let subjects = wl_kernel::documents::block_subjects(guarded).is_some();
    let future = wl_kernel::documents::future_work(guarded).is_some();
    format!(
        "{guarded}: BlockSubjects {}, FutureWork {}",
        if subjects { "registered" } else { "missing" },
        if future { "registered" } else { "missing" },
    )
}

/// The connection `key` as registered: its owner role and fallback.
fn connection_text(key: &str) -> String {
    match integrations::get(key) {
        None => format!("{key}: not registered"),
        Some(def) => format!(
            "{key}: owner {}; screens {}; fallback: {}",
            def.owner_role_name(),
            def.screen_keys().join(", "),
            def.fallback_en()
        ),
    }
}

/// The 03 client's document, rule, integration, notice, sales and staff
/// settings (BL-LOG-25), numbered as the issue's table.
pub fn compliance_rows(owner_role: &str) -> Vec<Row> {
    let sales = logistics_seed::sales(wl_sales::WlSales::new());
    let sales = sales.config();
    let hr = logistics_seed::hr(wl_hr::WlHr::new(), owner_role);
    let hr = hr.config();
    let critical: Vec<String> = defaults::CRITICAL_RULES
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
    let kind = |key: &str| {
        KINDS
            .iter()
            .find(|k| k.key == key)
            .map(|k| format!("{}: {}", k.name_en, kind_text(k.key)))
            .unwrap_or_default()
    };
    let roles: Vec<String> = KINDS
        .iter()
        .map(|k| format!("{}: {}", k.key, k.role))
        .collect();
    let groups: Vec<String> = [
        notices::CLIENT_NOTICE,
        notices::ACTION_REQUEST_DISPATCHER,
        notices::ACTION_REQUEST_DOCUMENTATION,
        notices::ACCOUNT_CONFIRM,
        notices::STATEMENT_NOTICE,
    ]
    .iter()
    .map(|key| type_text(key))
    .collect();
    let channels: Vec<String> = defaults::CAMPAIGN_CHANNELS
        .iter()
        .map(|(name, kind)| format!("{name} (recorded as {kind})"))
        .collect();
    let onboarding: Vec<String> = defaults::ONBOARDING
        .iter()
        .map(|o| format!("{} ({} steps)", o.role, o.steps.len()))
        .collect();
    let sops: Vec<String> = defaults::SOPS
        .iter()
        .map(|s| format!("{}: owner {}, readers {}", s.title, s.owner_role, s.readers.join(", ")))
        .collect();
    vec![
        open(
            row("1", "Documents", "Kind Driving licence: blocks",
                kind("driving_licence"), SEED, "§9 Document blocks"),
            "L-19 (whether these are legal blocks in the strict sense)",
        ),
        open(
            row("2", "Documents", "Kind RTA rider permit and qualification certificate: blocks",
                kind("rider_permit"), SEED, "§9 Document blocks"),
            "L-19",
        ),
        open(
            row("3", "Documents", "Kinds Mulkiya, vehicle insurance, Asateel or other vehicle permit: blocks",
                [kind("mulkiya"), kind("vehicle_insurance"), kind("vehicle_permit")].join(" | "),
                SEED, "§9 Document blocks"),
            "L-19",
        ),
        open(
            row("4", "Documents", "Override mode of the three blocking kinds",
                "never: no override is offered anywhere and the mode cannot be changed in the settings (every entry is locked)".to_owned(),
                SEED, "§9 Document blocks"),
            "L-19 (until answered, never)",
        ),
        row("5", "Documents", "Responsible role per kind", roles.join("; "),
            SEED, "§9 Document blocks"),
        row("6", "Notices", "Notification type groups", groups.join(" | ")
            + " | no 03 type is statutory",
            SEED, "§9 Notification types; wl-notify §5.4"),
        row("7", "Documents", "BlockSubjects and FutureWork",
            format!("{}; {}; a run maps to its driver (employee) and its vehicle (asset), a delivery to the run's driver; FutureWork lists the planned runs of the driver or the vehicle",
                providers(defaults::ASSIGN_GUARDED), providers(defaults::DISPATCH_GUARDED)),
            "BlockSubjects and FutureWork: wl-delivery", "§9 Document blocks"),
        row("8", "Documents", "Qualification kinds (HRM.QUA.01)",
            "driving licence; RTA rider permit and certificate (riders): the document kinds above attach to the employee".to_owned(),
            SEED, "§5"),
        row("9", "Rules", "Compliance-critical rules", critical.join("; "),
            "marked by the host for this client (LOG.RUL.10 is also watched for a missed nightly run at 00:30)", "§7.2"),
        row("10", "Rules", "Owner role (PLT.RUL.04)",
            format!("{owner_role}; the company admin ({ADMIN}) is alerted as well, once when it is the same role"),
            "client.toml [roles] owner, else the company admin", "§7.2"),
        row("11", "Integrations", "Connections with a manual fallback",
            format!("{} | e-invoicing ASP (registered by wl-invoicing for an entity with a go-live date) | Mirsal and ATLP are not connections: their outage is a freight job hold",
                connection_text(defaults::SMS_GATEWAY)),
            SEED, "§8 Integration down"),
        row("12", "Integrations", "SMS gateway fallback",
            "tracking SMS queued; the dispatcher gets a task listing the recipients on today's runs to call; the clients of those shipments are told by e-mail".to_owned(),
            "logistics::sms_fallback", "§8"),
        row("13", "Integrations", "E-invoicing ASP fallback",
            "invoices issued and sent as PDF; transmissions queued; FIN.RUL.16 raises a task after 24 hours".to_owned(),
            MODULE, "§8"),
        row("14", "E-invoicing", "EInvoiceScope",
            format!("an invoice for international goods transport by air (air waybill) issued within {} months of the entity's go-live is not transmitted; sea and road freight are", defaults::EINVOICE_EXCLUSION_MONTHS),
            SEED, "§9 E-invoicing"),
        row("15", "Late entry", "Late entry of a driver's e-POD (PLT.ACL.10)",
            format!("business time = the stop time; flagged with a reason after M = {} minutes; an operations manager approves after N = {} hours ({})",
                defaults::LATE_ENTRY_FLAG_MINUTES, defaults::LATE_ENTRY_APPROVAL_HOURS, defaults::EPOD_LATE_ENTRY),
            "limits seeded by `logistics:install`, editable per entity", "§4.1, wl-kernel"),
        row("16", "Late entry", "Digest of voids and late entries (PLT.RUL.06)",
            format!("on, daily at 07:00, to the holders of late_entries.approve: the {OPERATIONS_MANAGER} of each depot and the company admin"),
            MODULE, "§7.2"),
        row("17", "Sales", "Lead sources used (SAL.CRM.02)",
            format!("{}; recorded on the lead and carried to the client party when it converts; walk-in and aggregator are turned off; a source requires a campaign: {}",
                defaults::LEAD_SOURCES.join(", "),
                if sales.campaign_required_sources.is_empty() { "none".to_owned() } else { sales.campaign_required_sources.join(", ") }),
            SEED, "§5; §7.2 settings"),
        row("18", "Sales", "Campaign channels (SAL.CRM.02, SAL.ACL.07)",
            format!("{}; converted value: the client's first invoice; a cost entry over the budget needs a manager: {}",
                channels.join(", "), on(sales.over_budget_approval)),
            "channels: the module's fixed list; approval: Logistics seed", "§5; §7.2 settings"),
        row("19", "Staff", "Onboarding checklists (HRM.ONB.01, HRM.RUL.16, HRM.RUL.17, HRM.ACL.10)",
            format!("{}; started when the employee record is created in onboarding; tasks to the operations manager, the line manager (the {}) and the employee; the employee's own steps only by them; completion blocks nothing (run dispatch stays guarded by the driving licence); HRM.RUL.17 reminds on the first run after the due date, then every 2 days, and tells the operations manager and the line manager after 5 days",
                onboarding.join("; "), hr.manager_role),
            "checklists: Logistics seed; reminder days: default of the module", "§5; §7.2 settings"),
        row("20", "Staff", "SOP library (HRM.ONB.01, DOC.RUL.04, HRM.ACL.10)",
            format!("{}; DOC.RUL.04 asks each reader to acknowledge within 7 days, reminds every 3 days and tells the reader's manager and the SOP owner role after 7 days overdue; only the reader acknowledges; a new version asks again",
                sops.join("; ")),
            "SOPs and readers: Logistics seed; days: default of the module", "§5; §7.2 settings"),
        row("21", "Registrations", "Registration and certification kinds (DOC.EXP.01, DOC.RUL.01)",
            format!("Trademark registration ({} days) and Other certification ({} days), subject legal entity, responsible role {ADMIN}, no blocks",
                days(&defaults::TRADEMARK_REMINDERS), days(&defaults::OTHER_CERTIFICATION_REMINDERS)),
            "default of the module, on for 03", "§5; §7.2 settings"),
    ]
}

/// The 03 client's delivery, cash, money and access settings (BL-LOG-26),
/// numbered as the issue's table.
pub fn money_rows(entities: &[String]) -> Vec<MoneyRow> {
    let invoicing = negative_flow::invoicing_for(LOGISTICS, entities);
    let invoicing = invoicing.config();
    let payments = negative_flow::payments_for(LOGISTICS);
    let payments = payments.config();
    let delivery = negative_flow::delivery_for(LOGISTICS);
    let delivery = delivery.settings();
    let parties = super::accounting_seed::parties_for(LOGISTICS);
    let absorbers: Vec<&str> = delivery
        .cod_absorbers()
        .iter()
        .map(|absorber| absorber.as_str())
        .collect();
    let thresholds = if entities.is_empty() {
        "none".to_owned()
    } else {
        entities
            .iter()
            .map(|code| format!("{code} {}", aed(invoicing.cash_threshold_of(code))))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let abort: Vec<String> = delivery.abort_reasons();
    vec![
        money("1", "Delivery", "Partial acceptance (source type shipment)",
            format!("source default {}; a shipment with several parcels asks for it per delivery, a single-parcel shipment never offers a refused quantity",
                on(delivery.partial_acceptance(logistics::SUBJECT_TYPE))),
            HOST),
        money("2", "Delivery", "Failure reasons (shipment)",
            format!("the module's list plus {}; run aborted and cancelled by client do not count against the maximum attempts; wrong address puts the shipment on hold (address issue)",
                defaults::FAILURE_REASONS.join(", ")),
            "seeded by wl-delivery (`wl:install`)"),
        money("3", "Delivery", "Run abort reasons", abort.join(", "), HOST),
        money("4", "Delivery", "Hand-over between drivers",
            format!("the receiving driver scans each parcel: on; counts the cash: {}", on(delivery.hand_over_counts_cash())),
            HOST),
        money("5", "Delivery", "COD difference tolerance (WRK.ACL.09)",
            format!("{}; a short payment is allowed with approval: {}", aed(delivery.cod_tolerance()), on(delivery.cod_approval())),
            HOST),
        money("6", "Delivery", "COD difference approver",
            format!("{OPERATIONS_MANAGER} (deliveries.approve_cod_difference); the driver cannot approve"),
            "role permissions"),
        money("7", "Delivery", "COD absorbers offered",
            format!("{}; customer hidden; no preselected default", absorbers.join(", ")),
            HOST),
        money("8", "Delivery", "COD approval timeout and fallback",
            format!("{} minutes; then only the failed attempt", delivery.cod_approval_minutes()),
            HOST),
        money("9", "Delivery", "Reversal and manual lost approval (WRK.ACL.10)",
            format!("on; approver {OPERATIONS_MANAGER} or the company admin; the requester cannot approve and the driver of the run can neither request nor approve"),
            "role permissions; logistics reversal and loss"),
        money("10", "Delivery", "Untraced days before lost (WRK.RUL.17)",
            delivery.untraced_days().to_string(), HOST),
        money("11", "Cash", "Cash holders", "drivers: per person, per run".to_owned(), "wl-payments mapping"),
        money("12", "Cash", "Cash counted by another user (FIN.ACL.05)",
            "on; counter cashier or operations manager, never the driver who hands it over".to_owned(),
            "default of the module; roles"),
        money("13", "Cash", "Cash shortage write-off limit (FIN.ACL.06)",
            format!("{}: up to it the cashier writes off; above it {} approves", aed(payments.cash_write_off_limit), payments.cash_manager_role),
            HOST),
        money("14", "Cash", "Shortage recovery by salary deduction (FIN.ACL.19)",
            format!("{} (open: lawyer L-20); a setting, never decided in code", on(payments.cash_salary_deduction)),
            HOST),
        money("15", "Cash", "Handover escalation (FIN.RUL.22)",
            "replaced for drivers by LOG.RUL.13 (N = 12 h, M = 48 h)".to_owned(),
            HOST),
        money("16", "Receivables", "Invoice write-off (FIN.ACL.15)",
            format!("limit {}; approvers {}; the requester cannot approve", aed(defaults::INVOICE_WRITE_OFF_LIMIT), invoicing.write_off_approvers.join(" or ")),
            HOST),
        money("17", "Receivables", "Write-off notice to the client",
            if invoicing.write_off_notice { "sent" } else { "not sent" }.to_owned(),
            HOST),
        money("18", "Receivables", "Bad-debt relief proposal (FIN.RUL.14)",
            format!("{} after {} months", on(invoicing.bad_debt_relief), invoicing.bad_debt_relief_months.get()),
            HOST),
        money("19", "Receivables", "Invoice dispute from the portal",
            format!("client users may raise it: {}; outcomes upheld or credited; escalation FIN.RUL.15 after {} days to {}",
                on(invoicing.portal_disputes), invoicing.dispute_escalation_days, invoicing.dispute_escalation_role),
            HOST),
        money("20", "Receivables", "Bounce fee on a returned cheque",
            payments.bounce_fee.as_ref().map_or_else(
                || "off".to_owned(),
                |fee| format!("{} {}", fee.charge_type, aed(fee.amount)),
            ),
            "default of the module (open: lawyer L-22)"),
        money("21", "Access", "Blocked-client exception approver (PTY.ACL.03)",
            format!("{}; the requester cannot approve", parties.approver()),
            HOST),
        money("22", "Access", "Sanctions screening", "off".to_owned(), "default of the module"),
        money("23", "Access", "Portal after contract end (POR.ACL.01)",
            format!("read-only, closes after {} days (logistics::portal)", defaults::PORTAL_CLOSES_AFTER_DAYS),
            "Logistics seed"),
        money("24", "Cash-flow", "Forecast alert (FIN.CFL.01, FIN.RUL.23)",
            format!("{} weeks from {:?}; alert below {}; {} at {}; COD remittances counted as payments on their schedule (logistics.cod_remittances); due dates moved by days late: {}; disputed amounts counted: {}",
                invoicing.cash_forecast_weeks, invoicing.cash_week_start, thresholds,
                invoicing.cash_alert_roles.join(" and "), invoicing.cash_alert_time,
                on(invoicing.cash_days_late), on(invoicing.cash_disputed_counted)),
            HOST),
        money("25", "Budget", "Budget categories and approval (FIN.BUD.01, FIN.RUL.24)",
            format!("categories owned by the accountant; one budget per legal entity and fiscal year, approved by the company admin; warns at {}% of a month's line: the category's owner role and {}",
                invoicing.budget_variance_percent, invoicing.budget_variance_role),
            "categories: Logistics seed; approval: default of the module"),
        money("26", "Budget", "Owner drawings (FIN.ACL.21)",
            format!("every drawing approved by the company admin, never the requester; {} told when none else can approve", invoicing.drawings_notify_role),
            HOST),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compliance_page_lists_every_line_of_the_issue_table_once() {
        let rows = compliance_rows("operations_manager");
        let numbers: Vec<String> = rows.iter().map(|r| r.number.clone()).collect();
        let expected: Vec<String> = (1..=21).map(|n| n.to_string()).collect();
        assert_eq!(numbers, expected);
        assert!(
            rows.iter()
                .all(|r| !r.value.is_empty() && !r.source.is_empty() && !r.reference.is_empty())
        );
    }

    #[test]
    fn only_the_three_blocking_rows_wait_on_the_lawyer() {
        let rows = compliance_rows("operations_manager");
        let open: Vec<&str> = rows
            .iter()
            .filter(|r| r.open.contains("L-19"))
            .map(|r| r.number.as_str())
            .collect();
        assert_eq!(open, ["1", "2", "3", "4"]);
    }

    #[test]
    fn the_money_page_lists_every_line_of_the_issue_table_once() {
        let rows = money_rows(&["NEC".to_owned()]);
        let numbers: Vec<String> = rows.iter().map(|r| r.number.clone()).collect();
        let expected: Vec<String> = (1..=26).map(|n| n.to_string()).collect();
        assert_eq!(numbers, expected);
    }
}
