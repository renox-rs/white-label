//! The vertical's permissions (10 EdTech §3). Roles get them in
//! `host/src/app/roles.rs`; the role names the rules use are the consts at
//! the end. Bookings of trials, placement tests and one-to-one slots keep
//! wl-scheduling's own permissions (`bookings.*`).

/// See the courses and programmes.
pub const COURSES_VIEW: &str = "courses.view";
/// Define courses and programmes, and record their regulator approval
/// (`EDU.CRS.01`, `EDU.CRS.02`).
pub const COURSES_MANAGE: &str = "courses.manage";
/// See the batches in scope and their seats.
pub const BATCHES_VIEW: &str = "batches.view";
/// Plan a batch, open it, change its tutor, cancel it (`EDU.BAT.01`); the
/// coordinator also requests a termination (`EDU.ACL.15`).
pub const BATCHES_MANAGE: &str = "batches.manage";
/// Approve the termination of a running batch (`EDU.ACL.15`: the centre
/// manager, never the user who requested it).
pub const BATCHES_APPROVE: &str = "batches.approve";
/// Take a seat above the batch capacity, up to the room's limit
/// (`EDU.ACL.17`: the centre manager's alone).
pub const SEATS_EXCEED: &str = "batches.exceed_capacity";
/// See the sessions in scope.
pub const SESSIONS_VIEW: &str = "sessions.view";
/// Teach: submit the attendance of the sessions of the batches assigned
/// (`EDU.ACL.03`, `EDU.ACL.04`).
pub const SESSIONS_TEACH: &str = "sessions.teach";

/// See the enrolments in scope.
pub const ENROLMENTS_VIEW: &str = "enrolments.view";
/// Enrol a student, activate, freeze, unfreeze and withdraw an enrolment
/// (`EDU.ENR.01`, `enrolments.manage` of §7.1).
pub const ENROLMENTS_MANAGE: &str = "enrolments.manage";
/// Approve a discount above the limit (the centre manager's).
pub const ENROLMENTS_APPROVE: &str = "enrolments.approve";
/// Decide the refund of a withdrawal (`FIN.ACL.04`: the centre manager and the
/// accountant, never the user who asked for it).
pub const ENROLMENTS_REFUND_APPROVE: &str = "enrolments.approve_refund";
/// See the certificates in scope.
pub const CERTIFICATES_VIEW: &str = "certificates.view";
/// Issue and reissue a certificate (`certificates.issue` of §7.1).
pub const CERTIFICATES_ISSUE: &str = "certificates.issue";
/// Revoke a certificate (`certificates.revoke`: the company admin).
pub const CERTIFICATES_REVOKE: &str = "certificates.revoke";

/// Record a payment (wl-payments' own permission, named here because a module
/// does not depend on another): the counsellor takes the fee at the desk
/// from the enrolment's page ([`crate::desk`]).
pub const PAYMENTS_RECORD: &str = "payments.record";

/// The owner of the centre: told when a centre permit or a course approval
/// expires (`EDU.RUL.13`) and alerted when a compliance-critical rule fails.
pub const OWNER_ROLE: &str = "owner";
/// The centre manager of a campus: runs the timetable, approves what needs
/// approval and renews the centre permit.
pub const CENTRE_MANAGER_ROLE: &str = "centre_manager";
/// The admissions counsellor (the front desk): enquiries, trial classes,
/// enrolments and the fee collected at the desk.
pub const COUNSELLOR_ROLE: &str = "counsellor";
/// The academic coordinator: courses, batches, tutor and room allocation;
/// the responsible role of the course approval and of the tutor documents.
pub const COORDINATOR_ROLE: &str = "academic_coordinator";
/// The tutor or trainer: teaches the sessions of the batches assigned and
/// marks attendance.
pub const TUTOR_ROLE: &str = "tutor";
/// The accountant: invoices, refunds and tutor pay (the host's own role).
pub const ACCOUNTANT_ROLE: &str = "accountant";
