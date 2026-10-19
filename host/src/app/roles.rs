//! The roles and what each may do. Routes ask for a permission
//! (`require_permission`), templates with `can('users.manage')`; add a
//! permission here, give it to roles, and `define` makes the database agree
//! (it runs on every sign-up, seed and `users:admin`).

use renox::anyhow::anyhow;
use renox::auth::permissions;
use renox::command::Args;
use renox::prelude::*;
use wl_documents::permissions as documents;
use wl_invoicing::permissions as invoices;
use wl_notify::permissions as notify;
use wl_parties::permissions as parties;
use wl_payments::permissions as payments;
use wl_ticketing::permissions as tickets;

/// Manages users and their roles.
pub const USERS: &str = "users.manage";
/// Reads the activity log.
pub const ACTIVITY: &str = "activity.view";

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
/// reverses and refunds payments (wl-payments §6).
pub const ACCOUNTANT: &str = "accountant";
/// The finance manager: told when a dispute stays open (`FIN.RUL.15`), and
/// sees the invoices and receivables to follow it up (wl-invoicing §7), and
/// approves an exception for a blocked party (wl-parties §6).
pub const FINANCE_MANAGER: &str = "finance_manager";
/// The maintenance coordinator: raises, triages and assigns tickets, and
/// works them (wl-ticketing §6, 01 roles).
pub const COORDINATOR: &str = "coordinator";
/// A technician: sees and works the tickets assigned to them
/// (wl-ticketing §6, 01 roles).
pub const TECHNICIAN: &str = "technician";
/// Everyone who signs up.
pub const MEMBER: &str = "member";

/// Each role and its permissions.
pub const ROLES: &[(&str, &[&str])] = &[
    (
        ADMIN,
        &[
            USERS,
            ACTIVITY,
            wl_kernel::permissions::LEGAL_ENTITIES_VIEW,
            wl_kernel::permissions::LEGAL_ENTITIES_MANAGE,
            wl_kernel::permissions::APPROVALS_VIEW,
            wl_kernel::permissions::AUTOMATIONS_RETRY,
            wl_kernel::permissions::RECORDS_VOID,
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
            tickets::VIEW,
            tickets::CREATE,
            tickets::ASSIGN,
            tickets::WORK,
            tickets::CONFIDENTIAL_VIEW,
            tickets::MANAGE,
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
        ],
    ),
    (
        COMPLIANCE,
        &[
            parties::VIEW,
            parties::BLOCK,
            parties::BLOCK_REASON_VIEW,
            parties::BLOCK_EXCEPTION,
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
        ],
    ),
    (
        ACCOUNTANT,
        &[
            invoices::VIEW,
            invoices::CREDIT,
            invoices::VAT_VIEW,
            invoices::VAT_PREPARE,
            invoices::VAT_REVIEW,
            invoices::VAT_LOCK,
            invoices::RECEIVABLES_VIEW,
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
        ],
    ),
    (
        FINANCE_MANAGER,
        &[
            invoices::VIEW,
            invoices::RECEIVABLES_VIEW,
            parties::VIEW,
            parties::BLOCK_EXCEPTION,
        ],
    ),
    (
        COORDINATOR,
        &[
            tickets::VIEW,
            tickets::CREATE,
            tickets::ASSIGN,
            tickets::WORK,
        ],
    ),
    (TECHNICIAN, &[tickets::VIEW, tickets::WORK]),
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
