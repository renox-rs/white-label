//! The shared compliance, document and notice settings of the Property client
//! (01 Property §4.5, §7.2, §9; BL-PRP-27), on the page
//! `/settings/compliance`, next to the money settings of
//! [`super::negative_flow`].
//!
//! One row per line of the issue's settings table (1 to 22, with 6a and 15a).
//! A value is read back from where it is in force: the document kinds from the
//! registry wl-documents installs from, the notification types from the kernel
//! registry, the compliance-critical rules from the kernel registry, the
//! numbers from the same constants ([`property::seed`]) the module builders
//! are called with. The last columns say what the value is **set by** (this
//! client's seed or the module's own default) and the **spec reference**, and
//! the two decisions still open with the lawyer and the authority are flagged
//! (`L-19`, `AU-08`).

use property::notices;
use property::seed::{
    self, CATEGORIES, CERTIFY, CIVIL_DEFENCE_APPROVAL, COMPLIANCE_CATEGORIES, CRITICAL_RULES,
    KINDS, LEAD_SOURCES, ONBOARDING, REASSIGN_REASONS, SOPS,
};
use renox::nav::NavLink;
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::notify::notification_type;

use super::property_seed::{hr_for, sales_for};
use super::roles::SETTINGS_VIEW;

/// The row's value is this client's seed.
const SEED: &str = "Property seed (BL-PRP-27)";
/// The row's value is the module's own default.
const MODULE: &str = "default of the module";

/// One setting of the page.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The number in the issue's table (`6a`, `15a` for a sub-row).
    pub number: String,
    /// The group: owner alerts, documents, ticketing, notices, late entry,
    /// sales, staff, registrations.
    pub group: String,
    /// The setting.
    pub setting: String,
    /// Its value for this client.
    pub value: String,
    /// Where it is set: this client's seed or the module's own default.
    pub source: String,
    /// The spec reference.
    pub reference: String,
    /// The open item the value waits on, if any.
    pub open: String,
}

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

impl Row {
    fn open(mut self, item: &str) -> Row {
        self.open = item.to_owned();
        self
    }
}

fn names(keys: &[&str]) -> String {
    keys.join(", ")
}

/// The document kind `key` as registered, in words.
pub(crate) fn kind_text(key: &str) -> String {
    let Some(kind) = wl_documents::kinds::kind_seeds()
        .into_iter()
        .find(|k| k.key == key)
    else {
        return "not registered (the documents module is off)".to_owned();
    };
    let blocks: Vec<String> = kind
        .blocks
        .iter()
        .map(|b| {
            let mode = if b.mode == wl_kernel::documents::BlockMode::Block {
                "block"
            } else {
                "flag"
            };
            match &b.condition {
                Some(c) => format!(
                    "{} ({mode}; {} in {})",
                    b.guarded,
                    c.field,
                    c.values.join(", ")
                ),
                None => format!("{} ({mode})", b.guarded),
            }
        })
        .collect();
    let mut text = vec![
        format!("expiry {}", if kind.has_expiry { "yes" } else { "no" }),
        format!(
            "blocks: {}",
            if blocks.is_empty() {
                "none".to_owned()
            } else {
                blocks.join("; ")
            }
        ),
        format!("override {}", kind.override_mode.as_str()),
    ];
    if kind.legal_block {
        text.push("legal block".to_owned());
    }
    if kind.compliance_critical {
        text.push("compliance-critical".to_owned());
    }
    if kind.look_ahead_days > 0 {
        text.push(format!("look-ahead {} days", kind.look_ahead_days));
    }
    if !kind.reminder_offsets.is_empty() {
        let days: Vec<String> = kind.reminder_offsets.iter().map(i64::to_string).collect();
        text.push(format!("reminders at {} days", days.join("/")));
    }
    if let Some(role) = &kind.responsible_role {
        text.push(format!("responsible role {role}"));
    }
    if !kind.also_notify.is_empty() {
        text.push(format!("also tells {}", kind.also_notify.join(", ")));
    }
    text.join("; ")
}

/// The notification type `key` as registered: its group, whether it is
/// statutory and its give-up reactions.
pub(crate) fn type_text(key: &str) -> String {
    match notification_type(key) {
        None => format!("{key}: not registered"),
        Some(def) => {
            let reactions: Vec<&str> = def.give_up.iter().map(|r| r.as_str()).collect();
            let role = def
                .staff_role
                .as_deref()
                .map_or(String::new(), |r| format!(" to {r}"));
            format!(
                "{key}: group {}{}; on give-up {}{role}",
                def.group,
                if def.statutory { " (statutory)" } else { "" },
                reactions.join(" + "),
            )
        }
    }
}

/// What the client's configuration says, read back from the modules' own
/// registries.
pub fn rows(client_app: &str, owner_role: &str) -> Vec<Row> {
    let sales = sales_for(client_app);
    let sales = sales.config();
    let hr = hr_for(client_app, owner_role);
    let hr = hr.config();
    let critical: Vec<String> = CRITICAL_RULES
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
    let providers = |guarded: &str| {
        let subjects = wl_kernel::documents::block_subjects(guarded).is_some();
        let future = wl_kernel::documents::future_work(guarded).is_some();
        format!(
            "{guarded}: BlockSubjects {}, FutureWork {}",
            if subjects { "registered" } else { "missing" },
            if future { "registered" } else { "missing" },
        )
    };
    let kind_roles: Vec<String> = KINDS
        .iter()
        .map(|k| format!("{}: {}", k.key, k.role))
        .collect();
    let categories: Vec<String> = CATEGORIES
        .iter()
        .map(|c| format!("{} ({})", c.key, c.name_en))
        .collect();
    let onboarding: Vec<String> = ONBOARDING
        .iter()
        .map(|o| format!("{} ({} steps)", o.role, o.steps.len()))
        .collect();
    let sops: Vec<String> = SOPS
        .iter()
        .map(|s| format!("{} — owner {}", s.title, s.owner_role))
        .collect();
    let channels: Vec<String> = seed::CAMPAIGN_CHANNELS
        .iter()
        .map(|(name, kind)| format!("{name} (recorded as {kind})"))
        .collect();
    let certificate_texts: Vec<String> = seed::KINDS
        .iter()
        .filter(|k| k.key == seed::BUILDING_SAFETY_CERTIFICATE || k.key == seed::LIFT_CERTIFICATE)
        .map(|k| format!("{}: {}", k.name_en, kind_text(k.key)))
        .collect();
    vec![
        row("1", "Owner alerts", "Owner role (PLT.RUL.04)",
            format!("{owner_role}; the company admin is also alerted, once when it is the same role"),
            "client.toml [roles] owner, else the company admin", "§7.2 Compliance-critical rules and owner alerts"),
        row("2", "Owner alerts", "Compliance-critical rules", critical.join("; "),
            "marked by the modules and by Property (DOC.RUL.01 to 03 have no missed-run watch)", "§7.2 rules table"),
        row("3", "Documents", "Document kind Civil Defence approval (subject: vendor party)",
            kind_text(CIVIL_DEFENCE_APPROVAL), SEED, "§7.2 Document guard hooks; §4.5")
            .open("L-19 (whether the law forbids the work outright); AU-08 (other emirates: the same kind and categories are seeded)"),
        row("4", "Documents", "Providers registered for the Civil Defence approval",
            format!("{}; {}", providers("ticket.resolve"), providers("service_contract.activate")),
            "BlockSubjects: wl-ticketing; FutureWork: Property host", "§7.2 Document guard hooks"),
        row("5", "Documents", "Who is told on Civil Defence approval expiry (DOC.RUL.03)",
            "the coordinator and the property manager (the kind's responsible role and also-notify) and the company admin (the owner role, because it is compliance-critical); the contract gets the vendor document flag, open visits are flagged, buildings get vendor_document_expired".to_owned(),
            SEED, "§4.5 The vendor's Civil Defence approval expires"),
        row("6", "Documents", "Compliance certificate kinds (subject: building)",
            format!("{} | on expiry the building is flagged certificate_expired; the entry on {CERTIFY} is a flag that nothing runs, so leasing is never blocked", certificate_texts.join(" | ")),
            SEED, "§4.5 A compliance certificate expires; §7.2 DOC.RUL.01"),
        row("6a", "Documents", "Responsible role per document kind", kind_roles.join("; "),
            SEED, "§7.2 Document kinds and responsible roles"),
        row("7", "Ticketing", "Compliance-critical categories", names(&COMPLIANCE_CATEGORIES),
            SEED, "§7.2 WRK.RUL.14; wl-ticketing §7"),
        row("8", "Ticketing", "WRK.RUL.12 contract expiring",
            format!("N = {} days (the contract's renewal notice days); the coordinator and the property manager are told", seed::CONTRACT_EXPIRING_DAYS),
            MODULE, "§7.2"),
        row("9", "Ticketing", "WRK.RUL.14 planned visit missed",
            format!("N = {} days; for fire and life safety and lifts the property manager is also told and the building flagged visit_missed", seed::VISIT_MISSED_DAYS),
            SEED, "§7.2; §4.5 A planned visit is missed"),
        row("10", "Ticketing", "WRK.RUL.18 contract ended",
            format!("task due {} working day; flag no_contract for fire and life safety and lifts; the property manager and the company admin are told", seed::CONTRACT_ENDED_TASK_DAYS),
            MODULE, "§7.2; §4.5"),
        row("11", "Ticketing", "WRK.RUL.19 vendor no-show",
            format!("M = {} hours; reassignment offered to the coordinator; for fire and life safety and lifts the property manager is also told; repeated no-shows on the contract page", seed::NO_SHOW_HOURS),
            SEED, "§7.2; §4.2 The vendor does not turn up"),
        row("12", "Ticketing", "Reassignment reasons", names(&REASSIGN_REASONS), MODULE, "§7.1 Ticket assign"),
        row("13", "Ticketing", "Service contract approval (PRP.ACL.14)",
            format!("above {} a year a property manager approves; above {} the owner's approval is recorded. Not enforced yet: a service contract has no annual value in wl-ticketing", property::money_text(seed::SERVICE_CONTRACT_MANAGER_ABOVE), property::money_text(seed::SERVICE_CONTRACT_OWNER_ABOVE)),
            "not built (limits kept in property::seed)", "§3.3 PRP.ACL.14; §7.1 Service contract"),
        row("14", "Ticketing", "Ticket categories seeded by 01", categories.join("; ") + "; billing dispute is the platform's own (with invoicing)",
            SEED, "§7.1 Lease Legal case flag; §4.1; §4.4"),
        row("15", "Notices", "Statutory notice types",
            [notices::NOTICE_OF_CHANGED_TERMS, notices::LANDLORD_NOTICE, notices::PAYMENT_DEMAND]
                .iter().map(|k| type_text(k)).collect::<Vec<_>>().join(" | ")
                + " | the formal notice on a bounced cheque is a wl-payments type. Proof: courier receipt, registered mail receipt, signed hand-delivery note or a call note",
            SEED, "§7.2 Statutory notices; wl-notify §5.4"),
        row("15a", "Notices", "Notification types and their groups",
            [notices::CONTRACT_FOR_SIGNATURE, notices::RENEWAL_WINDOW, notices::NOTICE_WITHDRAWN, notices::PAYEE_CHANGED,
             notices::OWNER_STATEMENT, notices::OWNER_STATEMENT_REVISED, notices::OWNER_STATEMENT_DISPUTE_OUTCOME]
                .iter().map(|k| type_text(k)).collect::<Vec<_>>().join(" | ")
                + " | staff-only types: reaction none",
            SEED, "§7.2 Notification types; wl-notify §5.4"),
        row("16", "Late entry", "Late field updates on tickets",
            "the work time stays the business time (property.ticket_update); flagged above 30 minutes with a reason, manager approval above 24 hours".to_owned(),
            MODULE, "§7.2 Ticket; PLT.ACL.10"),
        row("17", "Integrations", "Ejari / Tawtheeq",
            "manual in the MVP; PRP.RUL.02 (N = 14) and PRP.RUL.13 (N = 7) chase the number and the cancellation; no integration connection to monitor".to_owned(),
            SEED, "§7.2 Lease; §8"),
        row("18", "Sales", "Lead sources used (SAL.CRM.02)",
            format!("{}; recorded on the enquiry and carried to the tenant party when it converts; aggregator is turned off; a source requires a campaign: {}",
                names(&LEAD_SOURCES), if sales.campaign_required_sources.is_empty() { "none".to_owned() } else { sales.campaign_required_sources.join(", ") }),
            SEED, "§5; §7.2 settings"),
        row("19", "Sales", "Campaign channels (SAL.CRM.02, SAL.ACL.07)",
            format!("{}; converted value: the first lease's contract value; a cost entry over the budget needs a manager: {}",
                channels.join(", "), if sales.over_budget_approval { "on" } else { "off" }),
            "channels: the module's fixed list; approval: Property seed", "§5; §7.2 settings"),
        row("20", "Staff", "Onboarding checklists (HRM.ONB.01, HRM.RUL.16, HRM.RUL.17, HRM.ACL.10)",
            format!("{}; started when the employee record is created in onboarding; the line manager is the {} of the building; completion blocks nothing; HRM.RUL.17 reminds on the first run after the due date, then every 2 days, and tells the admin and the line manager after 5 days",
                onboarding.join("; "), hr.manager_role),
            "checklists: Property seed; reminder days: default of the module", "§5; §7.2 settings"),
        row("21", "Staff", "SOP library (HRM.ONB.01, DOC.RUL.04, HRM.ACL.10)",
            format!("{}; DOC.RUL.04 asks each reader to acknowledge within 7 days, reminds every 3 days and tells the reader's manager and the SOP owner role after 7 days overdue; a new version asks again",
                sops.join("; ")),
            "SOPs and readers: Property seed; days: default of the module", "§5; §7.2 settings"),
        row("22", "Registrations", "Registration and certification kinds (DOC.EXP.01, DOC.RUL.01)",
            format!("Trademark registration ({} days) and Other certification ({} days), subject legal entity, responsible role {owner_role} (the owner role), no blocks: {} | {}",
                days(&seed::TRADEMARK_REMINDERS), days(&seed::OTHER_CERTIFICATION_REMINDERS),
                kind_text("trademark_registration"), kind_text("other_certification")),
            "default of the module, on for 01", "§5; §7.2 settings"),
    ]
}

fn days(list: &[i64]) -> String {
    list.iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join("/")
}

/// The Property client's compliance settings and the page that shows them.
#[derive(Debug, Clone)]
pub struct ComplianceSettings {
    client_app: String,
    owner_role: String,
}

impl ComplianceSettings {
    /// The settings of the client running `client_app` whose owner role is
    /// `owner_role`.
    pub fn new(client_app: &str, owner_role: &str) -> Self {
        ComplianceSettings {
            client_app: client_app.to_owned(),
            owner_role: owner_role.to_owned(),
        }
    }
}

impl Module for ComplianceSettings {
    fn name(&self) -> &'static str {
        "compliance_settings"
    }

    fn register(&self, app: &mut Registry) {
        app.provide(self.clone());
        app.nav(
            "app",
            NavLink::route("settings.compliance", "nav.compliance_settings")
                .icon("shield-check")
                .group("nav.admin")
                .order(13)
                .permission(SETTINGS_VIEW),
        );
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .get("/settings/compliance", show)
            .name("settings.compliance")
            .require_permission(SETTINGS_VIEW)
            .require_verified()
    }
}

async fn show(State(state): State<AppState>) -> Result<Response> {
    let settings = state
        .provided::<ComplianceSettings>()
        .ok_or(Error::NotFound)?;
    // The 03 client has its own table (BL-LOG-25).
    let rows = if settings.client_app == super::logistics_seed::LOGISTICS {
        super::logistics_settings::compliance_rows(&settings.owner_role)
    } else if settings.client_app == super::edtech_seed::EDTECH {
        // The 10 client has its own table too (BL-EDU-17).
        super::edtech_settings::rows(&settings.owner_role)
    } else {
        rows(&settings.client_app, &settings.owner_role)
    };
    Ok(view("settings/compliance.html", context! { rows }).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_lists_every_line_of_the_issue_table_once() {
        let rows = rows("property", "admin");
        let numbers: Vec<&str> = rows.iter().map(|r| r.number.as_str()).collect();
        let mut expected: Vec<String> = (1..=22).map(|n| n.to_string()).collect();
        expected.insert(6, "6a".to_owned());
        expected.insert(16, "15a".to_owned());
        assert_eq!(numbers, expected);
        assert!(
            rows.iter()
                .all(|r| !r.value.is_empty() && !r.source.is_empty() && !r.reference.is_empty())
        );
    }

    #[test]
    fn only_the_civil_defence_row_waits_on_the_lawyer_and_the_authority() {
        let rows = rows("property", "admin");
        let open: Vec<&str> = rows
            .iter()
            .filter(|r| r.open.contains("L-19"))
            .map(|r| r.number.as_str())
            .collect();
        assert_eq!(open, ["3"]);
    }
}
