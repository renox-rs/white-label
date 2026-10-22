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

/// Close a claim that missed the payer's first-submission window, or that is
/// partly paid or denied, by writing it off or moving it to the patient
/// (`CLN.ACL.16`, `FIN.ACL.14`: the clinic manager's; above AED 1,000 the
/// owner approves).
pub const CLAIMS_WRITEOFF: &str = "claims.writeoff";
/// Allow, on a payer contract, billing the patient for a payer share the
/// clinic did not submit in time (`CLN.ACL.16`: the owner's alone).
pub const MISSED_WINDOW_BILLING: &str = "contracts.missed_window_billing";

/// See the trace of a recalled lot: the patients who got it, by name, visit
/// day and branch (health data, every view audited, `CLN.ACL.01`): the clinic
/// manager's and the medical director's.
pub const RECALLS_TRACE: &str = "recalls.trace";
/// Send the notices of a recall to the patients (`STK.ACL.05`: the clinic
/// manager's).
pub const RECALLS_NOTIFY: &str = "recalls.notify";
/// The recall owner's permission, wl-inventory's `recalls.manage` (the trace
/// page shows the *Waive* button to those who hold it; the route checks it).
pub const RECALLS_MANAGE: &str = "recalls.manage";
/// Record the outcome of the call to a patient whose recall notice could not
/// be delivered (the front desk's, in the branch of the visit).
pub const RECALLS_CALL: &str = "recalls.call";

/// See the advertising permits recorded on campaigns (`CLN.ACL.13`): the
/// marketing coordinator's, so a campaign that cannot be activated shows why.
pub const PERMITS_VIEW: &str = "advertising_permits.view";
/// Record, approve and change the regulator's advertising permit number and
/// validity of a campaign that promotes services (`CLN.ACL.13`: the medical
/// director's).
pub const PERMITS_MANAGE: &str = "advertising_permits.manage";

/// See, per branch, the appointments of the next 24 hours to call when the
/// SMS or WhatsApp connection is down, and tick each as called (05 section 8:
/// the front desk's).
pub const APPOINTMENTS_CALL: &str = "appointments.call";

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
/// The call-centre agent: books and reschedules for all branches and follows
/// up leads (05 §3.1).
pub const CALL_CENTRE_ROLE: &str = "call_centre_agent";
/// The nurse or assistant: prepares rooms, records the consumables used and
/// helps with consent forms (05 §3.1).
pub const NURSE_ROLE: &str = "nurse";
/// The storekeeper of a branch store: receives and issues the consumables
/// (05 §3.1).
pub const STOREKEEPER_ROLE: &str = "storekeeper";
