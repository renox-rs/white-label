//! The roles and what each may do. Routes ask for a permission
//! (`require_permission`), templates with `can('users.manage')`; add a
//! permission here, give it to roles, and `define` makes the database agree
//! (it runs on every sign-up, seed and `users:admin`).

use accounting::permissions as acc;
use property::permissions as prp;
use renox::anyhow::anyhow;
use renox::auth::permissions;
use renox::command::Args;
use renox::prelude::*;
use wl_documents::permissions as documents;
use wl_hr::permissions as hr;
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
/// Everyone who signs up.
pub const MEMBER: &str = "member";

/// Each role and its permissions.
pub const ROLES: &[(&str, &[&str])] = &[
    (
        ADMIN,
        &[
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
            hr::EMPLOYEES_VIEW,
            hr::EMPLOYEES_MANAGE,
            hr::ONBOARDING_VIEW,
            hr::ONBOARDING_MANAGE,
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
            parties::VIEW,
            parties::BLOCK,
            parties::BLOCK_REASON_VIEW,
            parties::BLOCK_EXCEPTION,
            // 04: the MLRO approves CDD files and the one-use override of
            // an expired KYC document (`DOC.ACL.02`).
            documents::VIEW,
            documents::BLOCK_OVERRIDE,
            acc::ENGAGEMENTS_VIEW,
            acc::CDD_VIEW,
            acc::CDD_PREPARE,
            acc::CDD_APPROVE,
        ],
    ),
    (
        PARTNER,
        &[
            acc::ENGAGEMENTS_VIEW,
            acc::ENGAGEMENTS_MANAGE,
            acc::ENGAGEMENTS_END,
            acc::CDD_VIEW,
            acc::TAX_PROFILES_VIEW,
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
            acc::ENGAGEMENTS_VIEW,
            acc::ENGAGEMENTS_MANAGE,
            acc::CDD_VIEW,
            acc::CDD_PREPARE,
            acc::TAX_PROFILES_VIEW,
            acc::TAX_PROFILES_MANAGE,
            parties::VIEW,
            parties::CREATE,
            parties::UPDATE,
            documents::VIEW,
            documents::UPLOAD,
            invoices::VIEW,
            tickets::TASKS_VIEW,
            tickets::TASKS_WORK,
            tickets::TASKS_REVIEW,
        ],
    ),
    (
        TAX_SPECIALIST,
        &[
            acc::ENGAGEMENTS_VIEW,
            acc::TAX_PROFILES_VIEW,
            acc::TAX_PROFILES_MANAGE,
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
            acc::ENGAGEMENTS_VIEW,
            parties::VIEW,
            invoices::VIEW,
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
            prp::UNITS_VIEW,
            prp::LEASES_VIEW,
            prp::LEASES_CANCEL,
            prp::OWNER_STATEMENTS_PREPARE,
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
            // The accountant works the deadline tasks (04 compliance
            // calendar, wl-ticketing §4.4).
            tickets::TASKS_VIEW,
            tickets::TASKS_WORK,
            // 04: sees the engagements, prepares CDD files.
            acc::ENGAGEMENTS_VIEW,
            acc::CDD_VIEW,
            acc::CDD_PREPARE,
            acc::TAX_PROFILES_VIEW,
            payments::VIEW,
            payments::RECORD,
            payments::REVERSE,
            payments::CHARGEBACK,
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
            // The second person for `waive` and `reopen` (`HRM.ACL.10`).
            hr::EMPLOYEES_VIEW,
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
    (MEMBER, &[]),
];

/// Creates the roles, or brings their permissions up to date.
pub async fn define(db: &Db) -> Result {
    for (role, grants) in ROLES {
        permissions::define_role(db, role, grants).await?;
    }
    Ok(())
}

/// `users:admin <email>`: the first admin of a new server, after they
/// signed up.
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
