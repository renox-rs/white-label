//! The vertical's permissions (05 Clinics §3). Roles get them in
//! `host/src/app/roles.rs`; the role names the rules use are the consts at
//! the end. Bookings, check-in and the licence checks keep wl-scheduling's
//! own permissions (`bookings.*`).

/// See the clinic's appointments, visits and insurance policies in scope.
pub const VISITS_VIEW: &str = "visits.view";
/// See and record a patient's insurance policies and the eligibility results
/// (`CLN.INS.02`).
pub const POLICIES_MANAGE: &str = "insurance_policies.manage";
/// See a patient's insurance policies (never the card image, which needs
/// `patients.sensitive.view`, `CLN.ACL.12`).
pub const POLICIES_VIEW: &str = "insurance_policies.view";
/// Record, before check-in, the eligibility result and the coverage outcome
/// of an insured visit, and have the patient sign the self-pay consent
/// (`CLN.ACL.15`: the receptionist's).
pub const COVERAGE_RECORD: &str = "coverage.record";

/// Prepare prior authorisations, send them to the payer, record the payer's
/// answer and keep the payer's list of services that need one
/// (`CLN.INS.03`: the billing coordinator's).
pub const AUTHORISATIONS_MANAGE: &str = "insurance.manage";

/// Record the services delivered on a visit, mark the planned procedure not
/// suitable and bill the visit (`CLN.ENC.01`: the practitioner's).
pub const CHARGES_CAPTURE: &str = "charges.capture";
/// Record the consumables issued to a visit, set the work done to *none*,
/// record the patient's acceptance of the work done and choose to refund the
/// deposit in full (the front desk's and the nurse's).
pub const CHARGES_DESK: &str = "charges.desk";

/// Prepare, validate and submit the clinic's claims to the payers, record the
/// channel's rejection and the payer's remittance and recoupment, and send a
/// claim again (`CLN.CLM.01`, `CLN.ACL.09`: the billing coordinator's).
pub const CLAIMS_MANAGE: &str = "claims.manage";

/// The owner / medical director: told when a practitioner licence expires
/// (`CLN.RUL.01`) and alerted when a compliance-critical rule fails.
pub const OWNER_ROLE: &str = "owner";
/// The clinic manager of a branch: the responsible role of the practitioner
/// licence and branch privileges, told when one expires.
pub const CLINIC_MANAGER_ROLE: &str = "clinic_manager";
/// The receptionist of a branch: books, checks in, records the coverage
/// outcome.
pub const RECEPTIONIST_ROLE: &str = "receptionist";
/// The billing coordinator: prepares and follows the prior authorisations,
/// and is asked to request a new one when one stops covering an appointment
/// (`CLN.RUL.07`, `CLN.RUL.09`).
pub const BILLING_COORDINATOR_ROLE: &str = "billing_coordinator";
/// The practitioner (doctor, dentist, therapist): sees the own visits and
/// records the charges of the services delivered (`CLN.ENC.01`).
pub const PRACTITIONER_ROLE: &str = "practitioner";
