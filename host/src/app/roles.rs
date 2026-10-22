//! The roles and what each may do. Routes ask for a permission
//! (`require_permission`), templates with `can('users.manage')`; add a
//! permission here, give it to roles, and `define` makes the database agree
//! (it runs on every sign-up, seed and `users:admin`).

use accounting::permissions as acc;
use clinics::permissions as cln;
use construction::permissions as con;
use logistics::permissions as log;
use manpower::permissions as mpw;
use property::permissions as prp;
use renox::anyhow::anyhow;
use renox::auth::permissions;
use renox::command::Args;
use renox::prelude::*;
use wl_attendance::permissions as attendance;
use wl_delivery::permissions as delivery;
use wl_documents::permissions as documents;
use wl_hr::permissions as hr;
use wl_inventory::permissions as inventory;
use wl_invoicing::permissions as invoices;
use wl_notify::permissions as notify;
use wl_parties::permissions as parties;
use wl_payments::permissions as payments;
use wl_sales::permissions as sales;
use wl_scheduling::permissions as scheduling;
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
/// The clinic manager of a branch (05 §3.1): runs the branch's bookings and
/// deposits, sees its visits and insurance policies, and is told when a
/// practitioner licence expires (`CLN.RUL.01`).
pub const CLINIC_MANAGER: &str = cln::CLINIC_MANAGER_ROLE;
/// The receptionist of a branch (05 §3.1): books, checks in and records the
/// coverage outcome of an insured visit (`CLN.ACL.15`).
pub const RECEPTIONIST: &str = cln::RECEPTIONIST_ROLE;
/// The practitioner of a branch (05 §3.1): records the services delivered on
/// a visit and the consumables used (`CLN.ENC.01`).
pub const PRACTITIONER: &str = cln::PRACTITIONER_ROLE;
/// The billing coordinator (05 §3.1): prepares and follows the prior
/// authorisations and is asked to request a new one when one stops covering
/// an appointment (`CLN.RUL.07`, `CLN.RUL.09`).
pub const BILLING_COORDINATOR: &str = cln::BILLING_COORDINATOR_ROLE;
/// The call-centre agent (05 §3.1): books and reschedules for all branches
/// and follows up leads.
pub const CALL_CENTRE_AGENT: &str = cln::CALL_CENTRE_ROLE;
/// The nurse or assistant (05 §3.1): records the consumables used.
pub const NURSE: &str = cln::NURSE_ROLE;
/// HR: keeps the employee records, starts and follows onboardings, and is the
/// `hr` owner of onboarding steps (wl-hr §3, §6).
pub const HR: &str = "hr";
/// The dispatcher: puts deliveries on runs, plans and dispatches runs, and
/// manages deliveries (wl-delivery §6, 03 roles).
pub const DISPATCHER: &str = "dispatcher";
/// The operations manager of a logistics client: approves and reverses
/// deliveries, approves a COD difference (`WRK.ACL.09`), counts and recounts
/// drivers' cash, raises a later shortage against a driver and writes small
/// shortages off within the limit (wl-payments §6, 03 roles; the second
/// approver of a write-off is another holder of `cash.write_off`).
pub const OPERATIONS_MANAGER: &str = "operations_manager";
/// The documentation officer of a logistics client: works the freight jobs
/// and their customs and shipping documents, delivers, closes and reopens them
/// (03 §3.1, `LOG.JOB.01`).
pub const DOCUMENTATION_OFFICER: &str = log::DOCUMENTATION_OFFICER_ROLE;
/// The fleet manager of a logistics client: vehicles, their documents,
/// services, fuel and fines; the responsible role of the vehicle document
/// kinds and the one told when a vehicle is grounded (03 §3.1, `LOG.RUL.10`).
pub const FLEET_MANAGER: &str = log::FLEET_MANAGER_ROLE;
/// The cashier of a logistics client: counts the cash drivers hand over and
/// writes off a shortage up to the limit (03 §3.3, `FIN.ACL.05`,
/// `FIN.ACL.06`).
pub const CASHIER: &str = log::CASHIER_ROLE;
/// A driver or rider: sees today's stops of their own runs and records their
/// outcomes (wl-delivery §6, `WRK.ACL.05`).
pub const DRIVER: &str = "driver";
/// Hub staff: scan parcels in and out of a depot (03 §3.1, `LOG.HUB.01`).
pub const HUB_STAFF: &str = log::HUB_STAFF_ROLE;
/// Customer service of a logistics client: answers the client's queries and
/// flags a COD statement it contests (03 §3.1, `LOG.ACL.12`).
pub const CUSTOMER_SERVICE: &str = log::CUSTOMER_SERVICE_ROLE;
/// A client user of a logistics company: books, tracks, voids and cancels its
/// own shipments in the portal (03 §3.1, `LOG.ACL.02`, `LOG.ACL.09`).
pub const SHIPPING_CLIENT: &str = log::CLIENT_ROLE;
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
/// The account manager: owns client accounts, their contracts, requests and
/// deployments (09 Manpower §3.1); assigned per client company (§3.2).
pub const ACCOUNT_MANAGER: &str = mpw::ACCOUNT_MANAGER_ROLE;
/// The PRO (visa officer): works the visa cases of the workers and is told
/// when a step passes its target (09 Manpower `MPW.RUL.14`).
pub const PRO: &str = mpw::PRO_ROLE;
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
            // 03 has no finance manager: the company admin approves the
            // exception that lets a blocked client's remittance be paid
            // (`PTY.ACL.03`, BL-LOG-19).
            parties::BLOCK_EXCEPTION,
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
            // The MD approves a rate card version dated in the past
            // (`MPW.ACL.16`).
            invoices::RATE_CARDS_VIEW,
            invoices::RATE_CARDS_MANAGE,
            invoices::RATE_CARDS_APPROVE,
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
            payments::CASH_VIEW,
            payments::CASH_COUNT,
            payments::CASH_WRITE_OFF,
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
            sales::LOYALTY_VIEW,
            sales::LOYALTY_ENROL,
            sales::LOYALTY_REDEEM,
            sales::LOYALTY_ADJUST,
            sales::LOYALTY_MANAGE,
            sales::SEGMENTS_MANAGE,
            sales::SEGMENTS_EXPORT,
            hr::EMPLOYEES_VIEW,
            hr::EMPLOYEES_MANAGE,
            hr::EMPLOYEES_SENSITIVE_VIEW,
            hr::EMPLOYEES_EXIT_APPROVE,
            hr::PAY_ACCOUNTS_APPROVE,
            hr::ONBOARDING_VIEW,
            hr::ONBOARDING_MANAGE,
            hr::LEAVE_VIEW,
            hr::LEAVE_REQUEST,
            hr::LEAVE_APPROVE,
            hr::PAYROLL_VIEW,
            hr::PAYROLL_PREPARE,
            hr::PAYROLL_APPROVE,
            hr::ADJUSTMENTS_MANAGE,
            hr::SETTLEMENTS_PREPARE,
            hr::SETTLEMENTS_APPROVE,
            hr::SETTLEMENTS_PAY,
            delivery::VIEW,
            delivery::DISPATCH,
            delivery::COMPLETE,
            delivery::MANAGE,
            delivery::RUNS_VIEW,
            delivery::RUNS_DISPATCH,
            delivery::RUNS_DRIVE,
            delivery::POD_VIEW,
            delivery::REVERSE,
            delivery::APPROVE_COD_DIFFERENCE,
            log::SHIPMENTS_VIEW,
            log::SHIPMENTS_BOOK,
            log::SHIPMENTS_MANAGE,
            log::SHIPMENTS_SCAN,
            log::SHIPMENTS_DELIVER,
            log::SHIPMENTS_VOID,
            log::SHIPMENTS_CANCEL,
            log::SHIPMENTS_APPROVE_LOSS,
            log::SHIPMENTS_APPROVE_DISPOSAL,
            log::FREIGHT_MANAGE,
            log::JOBS_VIEW,
            log::JOBS_MANAGE,
            log::JOBS_DELIVER,
            log::JOBS_CLOSE,
            log::JOBS_BILL_CANCELLED,
            log::REMITTANCES_VIEW,
            log::REMITTANCES_PREPARE,
            log::REMITTANCES_APPROVE,
            log::REMITTANCES_PAY,
            // A change of the remittance bank account is asked here and
            // approved by a second user (`LOG.ACL.13`).
            log::REMITTANCES_REQUEST_ACCOUNT,
            log::REMITTANCES_DISPUTE,
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
            scheduling::VIEW,
            scheduling::VIEW_DETAILS,
            scheduling::MANAGE,
            scheduling::CHECKIN,
            scheduling::WAIVE_DEPOSIT,
            scheduling::RESOURCES_MANAGE,
            // 05: the visits, the insurance policies and the coverage recorded
            // before check-in (`CLN.ACL.15`).
            cln::VISITS_VIEW,
            cln::POLICIES_VIEW,
            cln::POLICIES_MANAGE,
            cln::COVERAGE_RECORD,
            cln::AUTHORISATIONS_MANAGE,
            cln::CHARGES_CAPTURE,
            cln::CHARGES_DESK,
            cln::CLAIMS_MANAGE,
            cln::CLAIMS_WRITEOFF,
            cln::MISSED_WINDOW_BILLING,
            cln::RECALLS_TRACE,
            cln::RECALLS_NOTIFY,
            cln::RECALLS_CALL,
            cln::PERMITS_VIEW,
            cln::PERMITS_MANAGE,
            cln::APPOINTMENTS_CALL,
            inventory::RECALLS_MANAGE,
            // The last prices and the supplier scores (`STK.SUP.01`).
            inventory::STOCK_COST_VIEW,
            inventory::SUPPLIER_SCORES_VIEW,
            con::PROJECTS_VIEW,
            con::PROJECTS_MANAGE,
            con::PROJECTS_TERMINATE,
            con::PROJECTS_CANCEL,
            con::PROJECTS_AWARD,
            con::RETENTIONS_MANAGE,
            con::GUARANTEES_MANAGE,
            con::VARIATIONS_RECORD,
            con::VARIATIONS_SUBMIT,
            mpw::CONTRACTS_VIEW,
            mpw::CONTRACTS_MANAGE,
            mpw::SITES_MANAGE,
            mpw::REQUESTS_VIEW,
            mpw::REQUESTS_CREATE,
            mpw::REQUESTS_MANAGE,
            mpw::DEPLOYMENTS_VIEW,
            mpw::DEPLOYMENTS_MANAGE,
            mpw::ABSENCE_MANAGE,
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
            sales::LOYALTY_VIEW,
            // Told of WPS rejections (`HRM.RUL.11`), reads the run.
            hr::PAYROLL_VIEW,
            // Pays the final settlements and records their payments.
            hr::SETTLEMENTS_PAY,
            prp::UNITS_VIEW,
            prp::LEASES_VIEW,
            prp::LEASES_CANCEL,
            prp::OWNER_STATEMENTS_PREPARE,
            con::PROJECTS_VIEW,
            con::RETENTIONS_MANAGE,
            con::GUARANTEES_MANAGE,
            invoices::VIEW,
            invoices::CREDIT,
            // 03: the accountant requests a write-off, which the operations
            // manager or the company admin approves (`FIN.ACL.15`).
            invoices::WRITE_OFF,
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
            // 03: prepares and pays COD remittances; a second accountant
            // approves them and the adjustment lines (`LOG.ACL.07`,
            // `LOG.ACL.12`).
            log::JOBS_VIEW,
            // The accountant decides whether a cost billed to a cancelled job
            // is invoiced (BL-LOG-23).
            log::JOBS_BILL_CANCELLED,
            log::REMITTANCES_VIEW,
            log::REMITTANCES_PREPARE,
            log::REMITTANCES_APPROVE,
            log::REMITTANCES_PAY,
            // A change of the remittance bank account is asked here and
            // approved by a second user (`LOG.ACL.13`).
            log::REMITTANCES_REQUEST_ACCOUNT,
            log::REMITTANCES_DISPUTE,
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
            payments::CASH_VIEW,
            payments::CASH_COUNT,
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
            payments::CASH_VIEW,
            payments::CASH_WRITE_OFF,
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
            hr::PAY_ACCOUNTS_APPROVE,
            hr::ONBOARDING_VIEW,
            hr::ONBOARDING_MANAGE,
            scheduling::VIEW,
            cln::VISITS_VIEW,
            cln::MISSED_WINDOW_BILLING,
            cln::RECALLS_TRACE,
            // The recall owner (05 §7.2): opens a recall, decides the lines of
            // a hold, and approves the advertising permit of a campaign.
            inventory::ITEMS_VIEW,
            inventory::STOCK_VIEW,
            inventory::STOCK_HOLD,
            inventory::LOTS_RELEASE,
            inventory::RECALLS_MANAGE,
            cln::PERMITS_VIEW,
            cln::PERMITS_MANAGE,
            // The owner of the clinical and infection control SOPs publishes
            // their versions (`DOC.RUL.04`, 05 §7.2).
            documents::SOPS_MANAGE,
            hr::PAYROLL_VIEW,
            hr::PAYROLL_APPROVE,
            // The MD approves the final settlements and resolves their disputes
            // (`HRM.ACL.04`).
            hr::SETTLEMENTS_APPROVE,
        ],
    ),
    (
        HR,
        &[
            hr::EMPLOYEES_VIEW,
            hr::EMPLOYEES_MANAGE,
            // Salary and pay accounts on the employee page, audited
            // (`HRM.ACL.03`).
            hr::EMPLOYEES_SENSITIVE_VIEW,
            // The absence case of a worker who has not turned up
            // (`MPW.RUL.09`, 09 Manpower).
            mpw::ABSENCE_MANAGE,
            hr::ONBOARDING_VIEW,
            hr::ONBOARDING_MANAGE,
            hr::LEAVE_VIEW,
            hr::LEAVE_APPROVE,
            // HR prepares payroll; the MD approves it (`HRM.ACL.04`).
            hr::PAYROLL_VIEW,
            hr::PAYROLL_PREPARE,
            hr::ADJUSTMENTS_MANAGE,
            // HR prepares and disputes the final settlements.
            hr::SETTLEMENTS_PREPARE,
            attendance::VIEW,
            attendance::REVIEW,
            // HR clears a permit flag by correcting or accepting it, and is
            // never the supervisor who marked the record (`HRM.ACL.09`).
            attendance::CORRECT,
        ],
    ),
    (
        SUPERVISOR,
        &[
            attendance::VIEW,
            attendance::MARK,
            attendance::CORRECT,
            // 09 §3.1: sees the deployments of their site (§3.2).
            mpw::DEPLOYMENTS_VIEW,
        ],
    ),
    (
        ACCOUNT_MANAGER,
        &[
            mpw::CONTRACTS_VIEW,
            mpw::CONTRACTS_MANAGE,
            mpw::SITES_MANAGE,
            mpw::REQUESTS_VIEW,
            mpw::REQUESTS_CREATE,
            mpw::REQUESTS_MANAGE,
            mpw::DEPLOYMENTS_VIEW,
            mpw::DEPLOYMENTS_MANAGE,
            parties::VIEW,
            documents::VIEW,
            invoices::VIEW,
            // The account manager makes the rate card versions of the
            // client contracts and records the signed acceptance
            // (`MPW.ACL.16`).
            invoices::RATE_CARDS_VIEW,
            invoices::RATE_CARDS_MANAGE,
            attendance::VIEW,
            tickets::VIEW,
            tickets::CREATE,
        ],
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
            inventory::STOCK_COST_VIEW,
            inventory::SUPPLIER_SCORES_VIEW,
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
            inventory::STOCK_COST_VIEW,
            inventory::SUPPLIER_SCORES_VIEW,
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
            cln::PERMITS_VIEW,
            sales::CAMPAIGNS_VIEW,
            sales::CAMPAIGNS_MANAGE,
            sales::SEGMENTS_MANAGE,
            sales::LEADS_VIEW,
            sales::LEADS_MANAGE,
            // Sends the campaign's broadcast (`CLN.ACL.13`, `NTF.ACL.03`).
            notify::MARKETING_SEND,
        ],
    ),
    (
        MARKETING_MANAGER,
        &[
            cln::PERMITS_VIEW,
            sales::CAMPAIGNS_VIEW,
            sales::CAMPAIGNS_MANAGE,
            sales::CAMPAIGNS_COSTS_MANAGE,
            sales::ACQUISITION_REPORTS_VIEW,
            sales::LEADS_VIEW,
            sales::LEADS_MANAGE,
            sales::SEGMENTS_MANAGE,
            sales::SEGMENTS_EXPORT,
            // The second person who approves a broadcast above the limit
            // (`NTF.ACL.03`), never the one who asked for it.
            notify::MARKETING_SEND,
        ],
    ),
    (
        CLINIC_MANAGER,
        &[
            scheduling::VIEW,
            scheduling::VIEW_DETAILS,
            scheduling::MANAGE,
            scheduling::CHECKIN,
            scheduling::WAIVE_DEPOSIT,
            scheduling::RESOURCES_MANAGE,
            cln::VISITS_VIEW,
            cln::POLICIES_VIEW,
            cln::POLICIES_MANAGE,
            cln::COVERAGE_RECORD,
            cln::AUTHORISATIONS_MANAGE,
            cln::CHARGES_CAPTURE,
            cln::CHARGES_DESK,
            cln::CLAIMS_MANAGE,
            cln::CLAIMS_WRITEOFF,
            cln::RECALLS_TRACE,
            cln::RECALLS_NOTIFY,
            cln::RECALLS_CALL,
            cln::APPOINTMENTS_CALL,
            inventory::ITEMS_VIEW,
            inventory::STOCK_VIEW,
            inventory::STOCK_HOLD,
            inventory::LOTS_RELEASE,
            // `STK.RUL.06`: told when a pending issue stays open for 24
            // hours (the holders of `stock.approve` in the branch).
            inventory::STOCK_APPROVE,
            // The owner of the front desk, billing and complaints SOPs
            // publishes their versions (`DOC.RUL.04`, 05 §7.2).
            documents::SOPS_MANAGE,
            invoices::PAYER_BILLS_VIEW,
            invoices::PAYER_BILLS_WRITE_OFF,
            // Adjusts points and decides an adjustment above 500 (`SAL.ACL.07`).
            sales::LOYALTY_VIEW,
            sales::LOYALTY_ADJUST,
            sales::LOYALTY_MANAGE,
        ],
    ),
    (
        PRACTITIONER,
        &[
            scheduling::VIEW,
            cln::VISITS_VIEW,
            cln::CHARGES_CAPTURE,
            cln::CHARGES_DESK,
        ],
    ),
    (
        BILLING_COORDINATOR,
        &[
            scheduling::VIEW,
            cln::VISITS_VIEW,
            cln::POLICIES_VIEW,
            cln::POLICIES_MANAGE,
            cln::AUTHORISATIONS_MANAGE,
            cln::CLAIMS_MANAGE,
            invoices::PAYER_BILLS_VIEW,
            invoices::PAYER_BILLS_ISSUE,
            invoices::PAYER_BILLS_RECONCILE,
        ],
    ),
    (
        RECEPTIONIST,
        &[
            scheduling::VIEW,
            scheduling::MANAGE,
            scheduling::CHECKIN,
            cln::VISITS_VIEW,
            cln::POLICIES_VIEW,
            cln::POLICIES_MANAGE,
            cln::COVERAGE_RECORD,
            cln::CHARGES_DESK,
            cln::RECALLS_CALL,
            cln::APPOINTMENTS_CALL,
            // The desk enrols members, rings up counter sales and redeems points with the patient's code (`SAL.LOY.01`).
            sales::LOYALTY_VIEW,
            sales::LOYALTY_ENROL,
            sales::LOYALTY_REDEEM,
            sales::POS_SELL,
        ],
    ),
    (
        CALL_CENTRE_AGENT,
        &[
            scheduling::VIEW,
            scheduling::MANAGE,
            cln::VISITS_VIEW,
            cln::APPOINTMENTS_CALL,
            sales::LEADS_VIEW,
            sales::LEADS_MANAGE,
        ],
    ),
    (
        NURSE,
        &[scheduling::VIEW, cln::VISITS_VIEW, cln::CHARGES_DESK],
    ),
    (
        DISPATCHER,
        &[
            delivery::VIEW,
            delivery::DISPATCH,
            delivery::MANAGE,
            delivery::RUNS_VIEW,
            delivery::RUNS_DISPATCH,
            delivery::POD_VIEW,
            // The dispatcher asks to reverse a delivery; the operations
            // manager or the company admin approves (`WRK.ACL.10`).
            delivery::REVERSE,
            log::SHIPMENTS_VIEW,
            log::SHIPMENTS_BOOK,
            log::SHIPMENTS_MANAGE,
            log::SHIPMENTS_VOID,
            log::SHIPMENTS_CANCEL,
            log::FREIGHT_MANAGE,
        ],
    ),
    (
        OPERATIONS_MANAGER,
        &[
            // The late e-POD of a driver beyond 24 hours is approved here
            // (`PLT.ACL.10`), and the digest of voids and late entries goes
            // to the managers who hold it (`PLT.RUL.06`).
            wl_kernel::permissions::LATE_ENTRIES_APPROVE,
            // 03 has no HR role: the operations manager owns the onboarding
            // checklists of the depot, and `HRM.RUL.17` tells it of a step
            // that is 5 days overdue (`HRM.ONB.01`).
            hr::ONBOARDING_VIEW,
            hr::ONBOARDING_MANAGE,
            delivery::VIEW,
            delivery::DISPATCH,
            delivery::MANAGE,
            delivery::RUNS_VIEW,
            delivery::RUNS_DISPATCH,
            delivery::POD_VIEW,
            delivery::REVERSE,
            delivery::APPROVE_COD_DIFFERENCE,
            payments::VIEW,
            payments::CASH_VIEW,
            payments::CASH_COUNT,
            payments::CASH_WRITE_OFF,
            log::REMITTANCES_VIEW,
            log::SHIPMENTS_VIEW,
            log::SHIPMENTS_BOOK,
            log::SHIPMENTS_MANAGE,
            log::SHIPMENTS_VOID,
            log::SHIPMENTS_CANCEL,
            log::SHIPMENTS_APPROVE_LOSS,
            log::SHIPMENTS_APPROVE_DISPOSAL,
            log::FREIGHT_MANAGE,
            log::JOBS_VIEW,
        ],
    ),
    (
        FLEET_MANAGER,
        &[
            delivery::RUNS_VIEW,
            documents::VIEW,
            documents::UPLOAD,
            log::SHIPMENTS_VIEW,
        ],
    ),
    (
        CASHIER,
        &[
            delivery::RUNS_VIEW,
            payments::VIEW,
            payments::CASH_VIEW,
            payments::CASH_COUNT,
            payments::CASH_WRITE_OFF,
        ],
    ),
    (
        DOCUMENTATION_OFFICER,
        &[
            log::SHIPMENTS_VIEW,
            log::FREIGHT_MANAGE,
            log::JOBS_VIEW,
            log::JOBS_MANAGE,
            log::JOBS_DELIVER,
            log::JOBS_CLOSE,
            // The documentation officer issues the supplementary invoice of a
            // reopened job (BL-LOG-22) and sees the invoices of its jobs
            // (`LOG.ACL.11`).
            invoices::VIEW,
            invoices::ISSUE,
        ],
    ),
    (
        DRIVER,
        &[
            delivery::RUNS_DRIVE,
            delivery::COMPLETE,
            log::SHIPMENTS_SCAN,
            log::SHIPMENTS_DELIVER,
        ],
    ),
    (HUB_STAFF, &[log::SHIPMENTS_VIEW, log::SHIPMENTS_SCAN]),
    (
        CUSTOMER_SERVICE,
        &[
            log::SHIPMENTS_VIEW,
            log::REMITTANCES_VIEW,
            log::REMITTANCES_DISPUTE,
            tickets::VIEW,
            tickets::CREATE,
        ],
    ),
    (
        SHIPPING_CLIENT,
        &[
            log::SHIPMENTS_BOOK,
            log::SHIPMENTS_VOID,
            log::SHIPMENTS_CANCEL,
            // The client asks to change its remittance bank account in the
            // portal (`LOG.ACL.13`).
            log::REMITTANCES_REQUEST_ACCOUNT,
            // It disputes a paid statement in the portal (`LOG.ACL.12`).
            log::REMITTANCES_DISPUTE,
        ],
    ),
    (HSE_OFFICER, &[attendance::VIEW]),
    (PRO, &[hr::EMPLOYEES_VIEW, hr::VISAS_MANAGE]),
    (MEMBER, &[]),
];

/// What a vertical grants a role on top of [`ROLES`] for this deployment only
/// (09: HR and the accountant see the confidential labour cases and grievances,
/// which the accountant of 04 must not). Set once when the app is built.
static EXTRA: std::sync::RwLock<Vec<(&'static str, &'static str)>> =
    std::sync::RwLock::new(Vec::new());

/// Forgets the extra grants of the deployment built before. Each deployment
/// starts from here (`app_for_tuned`), so the grants of one vertical never reach
/// the app of another that is built later in the same process.
pub fn reset_extra() {
    EXTRA.write().unwrap_or_else(|e| e.into_inner()).clear();
}

/// Grants `role` the `extra` permissions for this deployment, besides its row of
/// [`ROLES`]; [`define`] keeps them, and creates a role that only a vertical
/// grants (09's `md`). A repeat changes nothing.
pub fn grant_extra(role: &'static str, extra: &[&'static str]) {
    let mut found = EXTRA.write().unwrap_or_else(|e| e.into_inner());
    for permission in extra {
        if !found.contains(&(role, *permission)) {
            found.push((role, permission));
        }
    }
}

/// What `role` holds in this deployment: its row of [`ROLES`] and the extra
/// grants of the vertical ([`grant_extra`]).
pub fn grants_of(role: &str) -> Vec<&'static str> {
    let mut grants: Vec<&'static str> = ROLES
        .iter()
        .find(|(name, _)| *name == role)
        .map(|(_, grants)| grants.to_vec())
        .unwrap_or_default();
    for (name, permission) in EXTRA.read().unwrap_or_else(|e| e.into_inner()).iter() {
        if *name == role && !grants.contains(permission) {
            grants.push(permission);
        }
    }
    grants
}

/// Every role of this deployment: those of [`ROLES`], then the ones only the
/// vertical grants ([`grant_extra`]).
pub fn role_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = ROLES.iter().map(|(name, _)| *name).collect();
    for (name, _) in EXTRA.read().unwrap_or_else(|e| e.into_inner()).iter() {
        if !names.contains(name) {
            names.push(name);
        }
    }
    names
}

/// Creates the roles, or brings their permissions up to date.
pub async fn define(db: &Db) -> Result {
    for role in role_names() {
        permissions::define_role(db, role, &grants_of(role)).await?;
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
        .find(|r| !role_names().iter().any(|n| *n == **r))
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
