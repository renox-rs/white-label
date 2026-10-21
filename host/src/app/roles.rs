//! The roles and what each may do. Routes ask for a permission
//! (`require_permission`), templates with `can('users.manage')`; add a
//! permission here, give it to roles, and `define` makes the database agree
//! (it runs on every sign-up, seed and `users:admin`).

use accounting::permissions as acc;
use construction::permissions as con;
use property::permissions as prp;
use renox::anyhow::anyhow;
use renox::auth::permissions;
use renox::command::Args;
use renox::prelude::*;
use wl_attendance::permissions as attendance;
use wl_documents::permissions as documents;
use wl_hr::permissions as hr;
use wl_inventory::permissions as inventory;
use wl_invoicing::permissions as invoices;
use wl_notify::permissions as notify;
use wl_parties::permissions as parties;
use wl_payments::permissions as payments;
use wl_sales::permissions as sales;
use wl_ticketing::permissions as tickets;

/// Manages users and their roles.
pub const USERS: &str = "users.manage";
/// Reads the activity log.
pub const ACTIVITY: &str = "activity.view";
/// Reads the settings of the shared modules for this client (BL-PRP-26).
pub const SETTINGS_VIEW: &str = "settings.view";

/// Everything, except the restricted block reasons (`PTY.ACL.02`).
pub const ADMIN: &str = wl_kernel::COMPANY_ADMIN_ROLE;
/// The MLRO and their deputy: see parties, block them, and see and use the
/// restricted block reasons (wl-parties §6).
pub const COMPLIANCE: &str = "compliance";
/// The receivables role: drafts, issues and follows up invoices, requests
/// write-offs and handles disputes (wl-invoicing §3, §6).
pub const RECEIVABLES: &str = "receivables";
/// The accountant: sees invoices, and prepares, reviews and locks the VAT
/// summaries (wl-invoicing §6; `FIN.ACL.10` wants two of them), and records,
/// reverses and refunds payments (wl-payments §6), and prepares the company
/// budget (wl-invoicing `FIN.BUD.01`).
pub const ACCOUNTANT: &str = "accountant";
/// The finance manager: told when a dispute stays open (`FIN.RUL.15`), and
/// sees the invoices and receivables to follow it up (wl-invoicing §7),
/// approves an exception for a blocked party (wl-parties §6) and approves the
/// company budget (`FIN.BUD.01`).
pub const FINANCE_MANAGER: &str = "finance_manager";
/// The owner: approves the company budget and the owner drawings (a drawing
/// the owner records themselves is flagged when they are the only holder,
/// `FIN.ACL.23`), and reads budget vs actual with the drawings lines
/// (wl-invoicing `FIN.BUD.01`, `FIN.ACL.21`, `FIN.ACL.22`).
pub const OWNER: &str = "owner";
/// The maintenance coordinator: raises, triages and assigns tickets, and
/// works them (wl-ticketing §6, 01 roles).
pub const COORDINATOR: &str = "coordinator";
/// The facility manager: decides what happens to a blocked vendor's service
/// contracts, and ends or reassigns their visits (wl-ticketing §6,
/// wl-parties §5.6).
pub const FACILITY_MANAGER: &str = "facility_manager";
/// A technician: sees and works the tickets assigned to them
/// (wl-ticketing §6, 01 roles).
pub const TECHNICIAN: &str = "technician";
/// Marketing: sees and manages campaigns and adds cost entries (wl-sales §6).
pub const MARKETING: &str = "marketing";
/// The marketing manager: also approves costs over a campaign's budget and
/// reads the acquisition reports (wl-sales §6, `SAL.ACL.07`).
pub const MARKETING_MANAGER: &str = "marketing_manager";
/// The property manager: owns a set of buildings (units, leases, renewals,
/// escalations) and approves the cancellation of a lease after money was
/// received (01 §3.1, `PRP.ACL.18`).
pub const PROPERTY_MANAGER: &str = prp::PROPERTY_MANAGER_ROLE;
/// The leasing agent: drafts and submits leases, asks for a cancellation
/// (01 §3.1, `PRP.ACL.18`).
pub const LEASING_AGENT: &str = prp::LEASING_AGENT_ROLE;
/// The partner of an accounting firm: owns the client relationships, ends
/// engagements (04 §3.1).
pub const PARTNER: &str = acc::PARTNER_ROLE;
/// The engagement manager: runs a portfolio of clients, activates,
/// suspends and resumes their engagements, prepares CDD files and tax
/// profiles (04 §3.1).
pub const ENGAGEMENT_MANAGER: &str = acc::MANAGER_ROLE;
/// The tax specialist: returns and tax profiles (04 §3.1).
pub const TAX_SPECIALIST: &str = acc::TAX_SPECIALIST_ROLE;
/// Billing and admin: proposals, retainers, invoices and collections
/// (04 §3.1).
pub const BILLING: &str = acc::BILLING_ROLE;
/// HR: keeps the employee records, starts and follows onboardings, and is the
/// `hr` owner of onboarding steps (wl-hr §3, §6).
pub const HR: &str = "hr";
/// A site supervisor: marks attendance of the crews and people of their
/// location and corrects it within the window (wl-attendance §6, `HRM.ACL.01`).
pub const SUPERVISOR: &str = "supervisor";
/// The storekeeper: receives deliveries at the stores of their site and
/// records what was accepted and rejected (wl-inventory §6, `STK.ACL.04`).
pub const STOREKEEPER: &str = "storekeeper";
/// The procurement officer: sees purchase orders and approves an
/// over-receipt at the locations in their scope (wl-inventory §6,
/// `STK.ACL.04`).
pub const PROCUREMENT: &str = "procurement";
/// The procurement manager: told of a supplier review and of a request to
/// change the preferred supplier of an item, and approves it when they are
/// not the requester (wl-inventory §6, `STK.ACL.07`).
pub const PROCUREMENT_MANAGER: &str = "procurement_manager";
/// The project manager: runs a project, asks for its termination and settles
/// its final account (02 Construction §3.1, `CON.ACL.15`).
pub const PROJECT_MANAGER: &str = con::PROJECT_MANAGER_ROLE;
/// The quantity surveyor (QS): records the client's certificates and the
/// deductions on them (02 §3, `CON.ACL.14`).
pub const QUANTITY_SURVEYOR: &str = con::QS_ROLE;
/// The HSE officer: told of a midday break flag (`HRM.RUL.13`) and the
/// responsible role of the ISO certificate (02 §7.2).
pub const HSE_OFFICER: &str = "hse";
/// Everyone who signs up.
pub const MEMBER: &str = "member";

/// Each role and its permissions.
pub const ROLES: &[(&str, &[&str])] = &[
    (
        ADMIN,
        &[
            // 04 `ACC.ACL.05`: any member of staff may raise an AML concern and
            // then sees nothing of it.
            acc::AML_RAISE,
            // `aml.manage` is the MLRO's and deputy's alone, as the restricted block
            // reasons are: not even the company admin sees an AML case.
            USERS,
            ACTIVITY,
            SETTINGS_VIEW,
            wl_kernel::permissions::LEGAL_ENTITIES_VIEW,
            wl_kernel::permissions::LEGAL_ENTITIES_MANAGE,
            wl_kernel::permissions::APPROVALS_VIEW,
            wl_kernel::permissions::AUTOMATIONS_RETRY,
            wl_kernel::permissions::RECORDS_VOID,
            wl_kernel::permissions::LATE_ENTRIES_APPROVE,
            wl_kernel::permissions::INTEGRATIONS_VIEW,
            wl_kernel::permissions::INTEGRATIONS_MANAGE,
            parties::VIEW,
            parties::CREATE,
            parties::UPDATE,
            parties::BLOCK,
            parties::EXPORT,
            documents::VIEW,
            documents::UPLOAD,
            documents::REVIEW,
            documents::MANAGE,
            documents::KINDS_MANAGE,
            documents::BLOCK_OVERRIDE,
            documents::SOPS_MANAGE,
            notify::LOG_VIEW,
            notify::TEMPLATES_MANAGE,
            notify::CHANNELS_MANAGE,
            notify::RECORD_PROOF,
            invoices::VIEW,
            invoices::CREATE,
            invoices::ISSUE,
            invoices::CREDIT,
            invoices::WRITE_OFF,
            invoices::DISPUTE,
            invoices::RECEIVABLES_VIEW,
            invoices::VAT_VIEW,
            invoices::VAT_PREPARE,
            invoices::VAT_REVIEW,
            invoices::VAT_LOCK,
            invoices::SUPPLIER_BILLS_VIEW,
            invoices::SUPPLIER_BILLS_CREATE,
            invoices::SUPPLIER_BILLS_APPROVE,
            invoices::BUDGETS_VIEW,
            invoices::BUDGETS_MANAGE,
            invoices::BUDGETS_APPROVE,
            invoices::BUDGET_CATEGORIES_MANAGE,
            // The company admin approves every owner drawing, never the
            // requester (`FIN.ACL.21`).
            invoices::DRAWINGS_APPROVE,
            invoices::CASHFLOW_VIEW,
            invoices::CASHFLOW_MANAGE,
            invoices::EINVOICES_VIEW,
            invoices::EINVOICES_MANAGE,
            invoices::SUBSCRIPTIONS_VIEW,
            invoices::SUBSCRIPTIONS_MANAGE,
            invoices::PAYER_BILLS_VIEW,
            invoices::PAYER_BILLS_ISSUE,
            invoices::PAYER_BILLS_RECONCILE,
            invoices::PAYER_BILLS_WRITE_OFF,
            tickets::VIEW,
            tickets::CREATE,
            tickets::ASSIGN,
            tickets::WORK,
            tickets::CONFIDENTIAL_VIEW,
            tickets::MANAGE,
            tickets::SERVICE_CONTRACTS_VIEW,
            tickets::SERVICE_CONTRACTS_MANAGE,
            tickets::MAINTENANCE_PLANS_MANAGE,
            tickets::TASKS_VIEW,
            tickets::TASKS_WORK,
            tickets::TASKS_REVIEW,
            tickets::TASKS_MANAGE,
            payments::VIEW,
            payments::RECORD,
            payments::REVERSE,
            payments::CHARGEBACK,
            payments::LINKS,
            payments::REFUNDS_REQUEST,
            payments::REFUNDS_APPROVE,
            payments::REFUNDS_PAY,
            payments::DEPOSITS_VIEW,
            payments::DEPOSITS_MANAGE,
            payments::DEPOSITS_REFUND,
            payments::CHEQUES_VIEW,
            payments::CHEQUES_DEPOSIT,
            payments::CHEQUES_MANAGE,
            payments::BANK_RECONCILE,
            payments::CHEQUES_LEGAL,
            payments::EXPENSES_VIEW,
            payments::EXPENSES_SUBMIT,
            payments::EXPENSES_APPROVE,
            sales::CAMPAIGNS_VIEW,
            sales::CAMPAIGNS_MANAGE,
            sales::CAMPAIGNS_COSTS_MANAGE,
            sales::ACQUISITION_REPORTS_VIEW,
            sales::LEADS_VIEW,
            sales::LEADS_MANAGE,
            sales::PACKAGES_VIEW,
            sales::PACKAGES_SELL,
            sales::PACKAGES_EXTEND,
            sales::PACKAGES_ADJUST,
            sales::POS_SELL,
            sales::SALES_ORDERS_VIEW,
            sales::SALES_ORDERS_CREATE,
            sales::SALES_ORDERS_ACCEPT,
            sales::SALES_ORDERS_CANCEL,
            sales::RETURNS_REQUEST,
            sales::RETURNS_HANDLE,
            hr::EMPLOYEES_VIEW,
            hr::EMPLOYEES_MANAGE,
            hr::EMPLOYEES_EXIT_APPROVE,
            hr::ONBOARDING_VIEW,
            hr::ONBOARDING_MANAGE,
            hr::LEAVE_VIEW,
            hr::LEAVE_REQUEST,
            hr::LEAVE_APPROVE,
            attendance::VIEW,
            attendance::MARK,
            attendance::SELF_CHECK_IN,
            attendance::REVIEW,
            attendance::CORRECT,
            attendance::WORK_LOCATIONS_MANAGE,
            attendance::TIMESHEETS_SUBMIT,
            attendance::TIMESHEETS_ADJUST,
            attendance::TIMESHEETS_APPROVE,
            attendance::TIMESHEETS_APPROVE_ON_BEHALF,
            attendance::TIMESHEETS_REOPEN,
            inventory::ITEMS_VIEW,
            inventory::ITEMS_MANAGE,
            inventory::LOCATIONS_MANAGE,
            inventory::STOCK_VIEW,
            inventory::PURCHASE_ORDERS_VIEW,
            inventory::PURCHASE_ORDERS_ISSUE,
            inventory::GOODS_RECEIPTS_POST,
            inventory::GOODS_RECEIPTS_REVERSE,
            inventory::SUPPLIER_RETURNS_MANAGE,
            inventory::LANDED_COSTS_POST,
            inventory::ITEM_SUPPLIERS_MANAGE,
            inventory::ITEM_SUPPLIERS_APPROVE,
            inventory::STOCK_ISSUE,
            inventory::STOCK_TRANSFER,
            inventory::STOCK_ADJUST,
            inventory::STOCK_HOLD,
            inventory::LOTS_RELEASE,
            inventory::RECALLS_MANAGE,
            con::PROJECTS_VIEW,
            con::PROJECTS_MANAGE,
            con::PROJECTS_TERMINATE,
            con::PROJECTS_CANCEL,
            con::PROJECTS_AWARD,
            con::RETENTIONS_MANAGE,
            con::GUARANTEES_MANAGE,
            con::VARIATIONS_RECORD,
            con::VARIATIONS_SUBMIT,
            prp::UNITS_VIEW,
            prp::UNITS_MANAGE,
            prp::UNITS_RESERVE,
            prp::LEASES_VIEW,
            prp::LEASES_CREATE,
            prp::LEASES_CANCEL,
            prp::LEASES_ACTIVATE,
            prp::LEASES_MANAGE,
            prp::LEASES_FORCE_TERMINATE,
            prp::OWNER_STATEMENTS_PREPARE,
            prp::OWNER_STATEMENTS_APPROVE,
            prp::AGREEMENTS_MANAGE,
            prp::COMPLIANCE_VIEW,
            acc::ENGAGEMENTS_VIEW,
            acc::ENGAGEMENTS_MANAGE,
            acc::ENGAGEMENTS_END,
            acc::CDD_VIEW,
            acc::CDD_PREPARE,
            acc::CDD_APPROVE,
            acc::TAX_PROFILES_VIEW,
            acc::TAX_PROFILES_MANAGE,
            acc::TAX_PROFILES_APPROVE,
            acc::RETURNS_VIEW,
            acc::RETURNS_PREPARE,
            acc::RETURNS_REVIEW,
            acc::RETURNS_FILE,
            acc::WIP_WRITE_OFF,
            acc::KPI_VIEW,
            acc::KPI_MANAGE,
            acc::KPI_PUBLISH,
        ],
    ),
    (
        PROPERTY_MANAGER,
        &[
            prp::UNITS_VIEW,
            prp::UNITS_MANAGE,
            prp::UNITS_RESERVE,
            prp::LEASES_VIEW,
            prp::LEASES_CREATE,
            prp::LEASES_CANCEL,
            prp::LEASES_ACTIVATE,
            prp::LEASES_MANAGE,
            prp::OWNER_STATEMENTS_APPROVE,
            prp::AGREEMENTS_MANAGE,
            prp::COMPLIANCE_VIEW,
            parties::VIEW,
            documents::VIEW,
            // The signed tenancy contract and the Ejari certificate of a lease.
            documents::UPLOAD,
            invoices::VIEW,
            tickets::VIEW,
            tickets::CREATE,
            tickets::ASSIGN,
            payments::VIEW,
            payments::DEPOSITS_VIEW,
            payments::DEPOSITS_MANAGE,
            payments::CHEQUES_VIEW,
        ],
    ),
    (
        LEASING_AGENT,
        &[
            prp::UNITS_VIEW,
            prp::LEASES_VIEW,
            prp::LEASES_CREATE,
            prp::LEASES_CANCEL,
            parties::VIEW,
            parties::CREATE,
            parties::UPDATE,
            documents::VIEW,
            documents::UPLOAD,
            sales::LEADS_VIEW,
            sales::LEADS_MANAGE,
        ],
    ),
    (
        COMPLIANCE,
        &[
            // 04 `ACC.ACL.05`: the MLRO and deputy raise and work the AML cases
            // (`ACC.RUL.12`, `report`, `close`).
            acc::AML_RAISE,
            acc::AML_MANAGE,
            // The identity documents of the people the company deals with are
            // accepted here (the uploader never accepts their own).
            documents::VIEW,
            documents::REVIEW,
            parties::VIEW,
            parties::BLOCK,
            parties::BLOCK_REASON_VIEW,
            parties::BLOCK_EXCEPTION,
            // 04 §8 and `PTY.RUL.01`: the MLRO and deputy review the possible
            // matches (the second review of a confirmed one), run screenings
            // and upload a sanctions list by hand.
            parties::SCREENING_REVIEW,
            parties::SCREENING_RUN,
            // 04: the MLRO approves CDD files and the one-use override of
            // an expired KYC document (`DOC.ACL.02`).
            documents::BLOCK_OVERRIDE,
            acc::ENGAGEMENTS_VIEW,
            acc::CDD_VIEW,
            acc::CDD_PREPARE,
            acc::CDD_APPROVE,
            // 04 `ACC.ACL.15`: sees the returns, and the KYC flag on a
            // return filed with an expired client document.
            acc::RETURNS_VIEW,
            // 04 §3.1: the MLRO opens and works the CDD update task of an
            // expired KYC document (a staff task assigned to them,
            // `ACC.RUL.05`) and sees the deadline tasks.
            tickets::VIEW,
            tickets::WORK,
            // The MLRO's tasks about a client are confidential tickets
            // (`confidential_task`): the MLRO and the deputy see them, the
            // managers and partners do not (`ACC.ACL.05`, `ACC.ACL.14`).
            tickets::CONFIDENTIAL_VIEW,
            tickets::TASKS_VIEW,
            tickets::TASKS_WORK,
        ],
    ),
    (
        PARTNER,
        &[
            acc::AML_RAISE,
            acc::ENGAGEMENTS_VIEW,
            acc::ENGAGEMENTS_MANAGE,
            acc::ENGAGEMENTS_END,
            acc::CDD_VIEW,
            acc::TAX_PROFILES_VIEW,
            acc::TAX_PROFILES_APPROVE,
            // 04 §3.1: signs off returns, files them, and decides the
            // write-offs of unbilled time above the manager's share.
            acc::RETURNS_VIEW,
            acc::RETURNS_REVIEW,
            acc::RETURNS_FILE,
            acc::WIP_WRITE_OFF,
            // 04 §3.1 "all": reads the KPI packs and can withdraw one.
            acc::KPI_VIEW,
            acc::KPI_PUBLISH,
            // 04 `FIN.ACL.15`, `FIN.BUD.01`, `FIN.ACL.21`: the partner
            // approves write-offs, budgets and owner drawings (never their
            // own request) and reads the cash forecast.
            invoices::WRITE_OFF,
            invoices::BUDGETS_VIEW,
            invoices::BUDGETS_APPROVE,
            invoices::DRAWINGS_APPROVE,
            invoices::CASHFLOW_VIEW,
            invoices::RECEIVABLES_VIEW,
            parties::VIEW,
            documents::VIEW,
            invoices::VIEW,
            invoices::SUBSCRIPTIONS_VIEW,
            tickets::TASKS_VIEW,
        ],
    ),
    (
        ENGAGEMENT_MANAGER,
        &[
            acc::AML_RAISE,
            acc::ENGAGEMENTS_VIEW,
            acc::ENGAGEMENTS_MANAGE,
            acc::CDD_VIEW,
            acc::CDD_PREPARE,
            acc::TAX_PROFILES_VIEW,
            acc::TAX_PROFILES_MANAGE,
            // `ACC.ACL.13`: approves another person's change of a tax profile.
            acc::TAX_PROFILES_APPROVE,
            // 04 §3.1 "returns (review)": reviews and cancels returns and
            // writes off unbilled time up to the limit (`ACC.ACL.09`).
            acc::RETURNS_VIEW,
            acc::RETURNS_REVIEW,
            acc::WIP_WRITE_OFF,
            // 04 §4.5: the manager raises the client's dispute and asks for a
            // write-off.
            invoices::DISPUTE,
            invoices::WRITE_OFF,
            // 04 §7.1 *KPI pack*: the manager prepares and publishes.
            acc::KPI_VIEW,
            acc::KPI_MANAGE,
            acc::KPI_PUBLISH,
            parties::VIEW,
            parties::CREATE,
            parties::UPDATE,
            documents::VIEW,
            documents::UPLOAD,
            invoices::VIEW,
            // 04 §3.1 "engagements, tasks, returns (review)": works and
            // reviews the deadline tasks, and waives them or closes them as
            // missed (`ACC.RUL.04`, BL-TKT-09); sees the staff tasks of
            // their clients.
            tickets::VIEW,
            tickets::TASKS_VIEW,
            tickets::TASKS_WORK,
            tickets::TASKS_REVIEW,
            tickets::TASKS_MANAGE,
        ],
    ),
    (
        TAX_SPECIALIST,
        &[
            acc::AML_RAISE,
            acc::ENGAGEMENTS_VIEW,
            acc::TAX_PROFILES_VIEW,
            acc::TAX_PROFILES_MANAGE,
            // 04 §3.1 "returns (prepare, review)".
            acc::RETURNS_VIEW,
            acc::RETURNS_PREPARE,
            acc::RETURNS_REVIEW,
            acc::RETURNS_FILE,
            parties::VIEW,
            documents::VIEW,
            tickets::TASKS_VIEW,
            tickets::TASKS_WORK,
            tickets::TASKS_REVIEW,
        ],
    ),
    (
        BILLING,
        &[
            acc::AML_RAISE,
            acc::ENGAGEMENTS_VIEW,
            // 04 §3.1 billing / admin is HR: `HRM.RUL.17` tells it.
            hr::EMPLOYEES_VIEW,
            hr::EMPLOYEES_MANAGE,
            hr::ONBOARDING_VIEW,
            hr::ONBOARDING_MANAGE,
            hr::LEAVE_VIEW,
            hr::LEAVE_APPROVE,
            parties::VIEW,
            invoices::VIEW,
            // 04 `FIN.ACL.15`: billing asks for the write-off (a partner
            // approves it), disputes on the client's behalf, keeps the
            // budget and watches the cash forecast.
            invoices::WRITE_OFF,
            invoices::DISPUTE,
            invoices::BUDGETS_VIEW,
            invoices::BUDGETS_MANAGE,
            invoices::BUDGET_CATEGORIES_MANAGE,
            invoices::CASHFLOW_VIEW,
            invoices::CASHFLOW_MANAGE,
            // Supplier bills, owner drawings and the budget's actuals.
            invoices::SUPPLIER_BILLS_VIEW,
            invoices::SUPPLIER_BILLS_CREATE,
            invoices::SUPPLIER_BILLS_APPROVE,
            invoices::CREATE,
            invoices::ISSUE,
            invoices::CREDIT,
            invoices::RECEIVABLES_VIEW,
            invoices::SUBSCRIPTIONS_VIEW,
            invoices::SUBSCRIPTIONS_MANAGE,
            payments::VIEW,
            payments::RECORD,
        ],
    ),
    (
        RECEIVABLES,
        &[
            invoices::VIEW,
            invoices::CREATE,
            invoices::ISSUE,
            invoices::CREDIT,
            invoices::WRITE_OFF,
            invoices::DISPUTE,
            invoices::RECEIVABLES_VIEW,
            invoices::SUBSCRIPTIONS_VIEW,
        ],
    ),
    (
        ACCOUNTANT,
        &[
            acc::AML_RAISE,
            prp::UNITS_VIEW,
            prp::LEASES_VIEW,
            prp::LEASES_CANCEL,
            prp::OWNER_STATEMENTS_PREPARE,
            con::PROJECTS_VIEW,
            con::RETENTIONS_MANAGE,
            con::GUARANTEES_MANAGE,
            invoices::VIEW,
            invoices::CREDIT,
            invoices::VAT_VIEW,
            invoices::VAT_PREPARE,
            invoices::VAT_REVIEW,
            invoices::VAT_LOCK,
            invoices::SUPPLIER_BILLS_VIEW,
            invoices::SUPPLIER_BILLS_CREATE,
            invoices::SUPPLIER_BILLS_APPROVE,
            invoices::RECEIVABLES_VIEW,
            invoices::BUDGETS_VIEW,
            invoices::BUDGETS_MANAGE,
            invoices::BUDGET_CATEGORIES_MANAGE,
            invoices::CASHFLOW_VIEW,
            invoices::CASHFLOW_MANAGE,
            invoices::EINVOICES_VIEW,
            invoices::EINVOICES_MANAGE,
            invoices::SUBSCRIPTIONS_VIEW,
            invoices::SUBSCRIPTIONS_MANAGE,
            invoices::PAYER_BILLS_VIEW,
            invoices::PAYER_BILLS_ISSUE,
            invoices::PAYER_BILLS_RECONCILE,
            invoices::PAYER_BILLS_WRITE_OFF,
            // The accountant works the deadline tasks (04 compliance
            // calendar, wl-ticketing §4.4).
            tickets::TASKS_VIEW,
            tickets::TASKS_WORK,
            // 04: sees the engagements, prepares CDD files.
            acc::ENGAGEMENTS_VIEW,
            acc::CDD_VIEW,
            acc::CDD_PREPARE,
            acc::TAX_PROFILES_VIEW,
            // `ACC.ACL.13`: asks for a change of a tax profile.
            acc::TAX_PROFILES_MANAGE,
            // 04 §3.1 "returns (prepare)".
            acc::RETURNS_VIEW,
            acc::RETURNS_PREPARE,
            // 04 §3.1 "prepares KPI packs": imports, maps and comments.
            acc::KPI_VIEW,
            acc::KPI_MANAGE,
            payments::VIEW,
            payments::RECORD,
            payments::REVERSE,
            payments::CHARGEBACK,
            payments::LINKS,
            payments::REFUNDS_REQUEST,
            payments::REFUNDS_APPROVE,
            payments::REFUNDS_PAY,
            payments::DEPOSITS_VIEW,
            payments::DEPOSITS_MANAGE,
            payments::DEPOSITS_REFUND,
            payments::CHEQUES_VIEW,
            payments::CHEQUES_DEPOSIT,
            payments::CHEQUES_MANAGE,
            payments::BANK_RECONCILE,
            payments::CHEQUES_LEGAL,
            payments::EXPENSES_VIEW,
            payments::EXPENSES_SUBMIT,
            payments::EXPENSES_APPROVE,
        ],
    ),
    (
        FINANCE_MANAGER,
        &[
            invoices::VIEW,
            invoices::RECEIVABLES_VIEW,
            invoices::BUDGETS_VIEW,
            invoices::BUDGETS_APPROVE,
            invoices::CASHFLOW_VIEW,
            payments::EXPENSES_VIEW,
            payments::EXPENSES_APPROVE,
            parties::VIEW,
            parties::BLOCK_EXCEPTION,
        ],
    ),
    (
        OWNER,
        &[
            invoices::VIEW,
            invoices::RECEIVABLES_VIEW,
            invoices::SUPPLIER_BILLS_VIEW,
            invoices::BUDGETS_VIEW,
            invoices::BUDGETS_APPROVE,
            invoices::CASHFLOW_VIEW,
            invoices::DRAWINGS_APPROVE,
            // The second person for `waive` and `reopen` (`HRM.ACL.10`), and
            // the MD who approves the non-standard exits (`HRM.ACL.07`).
            hr::EMPLOYEES_VIEW,
            hr::EMPLOYEES_EXIT_APPROVE,
            hr::ONBOARDING_VIEW,
            hr::ONBOARDING_MANAGE,
        ],
    ),
    (
        HR,
        &[
            hr::EMPLOYEES_VIEW,
            hr::EMPLOYEES_MANAGE,
            hr::ONBOARDING_VIEW,
            hr::ONBOARDING_MANAGE,
            hr::LEAVE_VIEW,
            hr::LEAVE_APPROVE,
            attendance::VIEW,
            attendance::REVIEW,
            // HR clears a permit flag by correcting or accepting it, and is
            // never the supervisor who marked the record (`HRM.ACL.09`).
            attendance::CORRECT,
        ],
    ),
    (
        SUPERVISOR,
        &[attendance::VIEW, attendance::MARK, attendance::CORRECT],
    ),
    (
        STOREKEEPER,
        &[
            inventory::ITEMS_VIEW,
            inventory::STOCK_VIEW,
            inventory::PURCHASE_ORDERS_VIEW,
            inventory::GOODS_RECEIPTS_POST,
            inventory::STOCK_ISSUE,
            inventory::STOCK_TRANSFER,
            inventory::STOCK_ADJUST,
            inventory::STOCK_HOLD,
            inventory::LOTS_RELEASE,
        ],
    ),
    (
        PROCUREMENT,
        &[
            inventory::ITEMS_VIEW,
            inventory::STOCK_VIEW,
            inventory::PURCHASE_ORDERS_VIEW,
            inventory::PURCHASE_ORDERS_ISSUE,
            inventory::SUPPLIER_RETURNS_MANAGE,
            inventory::LANDED_COSTS_POST,
            inventory::ITEM_SUPPLIERS_MANAGE,
            inventory::RECALLS_MANAGE,
        ],
    ),
    (
        PROJECT_MANAGER,
        &[
            con::PROJECTS_VIEW,
            con::PROJECTS_MANAGE,
            con::PROJECTS_TERMINATE,
            con::PROJECTS_AWARD,
            con::IPAS_SUBMIT,
            con::IPAS_DEDUCTIONS_DECIDE,
            con::VARIATIONS_SUBMIT,
            inventory::ITEMS_VIEW,
            inventory::STOCK_VIEW,
            inventory::PURCHASE_ORDERS_VIEW,
            attendance::VIEW,
            attendance::REVIEW,
        ],
    ),
    (
        QUANTITY_SURVEYOR,
        &[
            con::PROJECTS_VIEW,
            con::IPAS_CERTIFY,
            con::IPAS_DEDUCTIONS_RECORD,
            con::VARIATIONS_RECORD,
            con::RETENTIONS_MANAGE,
        ],
    ),
    (
        PROCUREMENT_MANAGER,
        &[
            inventory::ITEMS_VIEW,
            inventory::STOCK_VIEW,
            inventory::PURCHASE_ORDERS_VIEW,
            inventory::PURCHASE_ORDERS_ISSUE,
            inventory::ITEM_SUPPLIERS_MANAGE,
            inventory::ITEM_SUPPLIERS_APPROVE,
        ],
    ),
    (
        COORDINATOR,
        &[
            prp::COMPLIANCE_VIEW,
            tickets::VIEW,
            tickets::CREATE,
            tickets::ASSIGN,
            tickets::WORK,
            tickets::SERVICE_CONTRACTS_VIEW,
        ],
    ),
    (
        FACILITY_MANAGER,
        &[
            tickets::VIEW,
            tickets::ASSIGN,
            tickets::SERVICE_CONTRACTS_VIEW,
            tickets::SERVICE_CONTRACTS_MANAGE,
        ],
    ),
    (TECHNICIAN, &[tickets::VIEW, tickets::WORK]),
    (
        MARKETING,
        &[
            sales::CAMPAIGNS_VIEW,
            sales::CAMPAIGNS_MANAGE,
            sales::LEADS_VIEW,
            sales::LEADS_MANAGE,
        ],
    ),
    (
        MARKETING_MANAGER,
        &[
            sales::CAMPAIGNS_VIEW,
            sales::CAMPAIGNS_MANAGE,
            sales::CAMPAIGNS_COSTS_MANAGE,
            sales::ACQUISITION_REPORTS_VIEW,
            sales::LEADS_VIEW,
            sales::LEADS_MANAGE,
        ],
    ),
    (HSE_OFFICER, &[attendance::VIEW]),
    (MEMBER, &[]),
];

/// Creates the roles, or brings their permissions up to date.
pub async fn define(db: &Db) -> Result {
    for (role, grants) in ROLES {
        permissions::define_role(db, role, grants).await?;
    }
    Ok(())
}

/// `users:admin <email>`: gives an existing user (see `users:invite`) the
/// admin role.
pub async fn make_admin(args: Args, state: AppState) -> Result {
    let Some(email) = args.positional().first().copied() else {
        return Err(anyhow!("usage: users:admin <email>").into());
    };
    let user = User::find_by_email(&state.db, email)
        .await?
        .ok_or_else(|| anyhow!("no user with the email {email}"))?;
    define(&state.db).await?;
    user.assign_role(&state.db, ADMIN).await?;
    println!("{} is an admin.", user.email);
    Ok(())
}

/// `users:invite <email> <name> [--role=admin,hr]`: adds a user (there is no
/// public sign-up), mails them the link to choose a password and prints it,
/// which is how the first admin of a new server gets in.
pub async fn invite(args: Args, state: AppState) -> Result {
    let positional = args.positional();
    let (Some(email), Some(name)) = (positional.first().copied(), positional.get(1).copied())
    else {
        return Err(anyhow!("usage: users:invite <email> <name> [--role=admin,hr]").into());
    };
    let roles: Vec<&str> = args
        .value("--role")
        .map(|list| {
            list.split(',')
                .map(str::trim)
                .filter(|r| !r.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if let Some(bad) = roles
        .iter()
        .find(|r| !ROLES.iter().any(|(name, _)| name == *r))
    {
        return Err(anyhow!("there is no role called {bad}").into());
    }
    let invitation = super::users::invite(&state, name, email, &roles).await?;
    println!("{} is added.", invitation.user.email);
    if invitation.mailed {
        println!("The link to choose a password was mailed. It works for an hour:");
    } else {
        println!("The mail could not be sent. Give them this link (it works for an hour):");
    }
    println!("{}", invitation.link);
    Ok(())
}
