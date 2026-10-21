//! The vertical's static tables (05 §7.2 *Settings for the shared negative
//! flows*, §4.5). The host turns them into the document kinds and the service
//! types it installs (`host/src/app/clinics_seed.rs`); the app depends on no
//! crate that owns those tables.

/// The practitioner licence (DHA, DoH, Sharjah Health Authority or MOHAP):
/// a document kind of the practitioner and a qualification of wl-hr
/// (`HRM.QUA.01`).
pub const LICENCE_KIND: &str = "practitioner_licence";

/// The practitioner's privileges at one branch (`HRM.QUA.01`).
pub const PRIVILEGES_KIND: &str = "branch_privileges";

/// The qualification kinds every service type of the clinic requires of the
/// person who delivers it, as wl-scheduling stores them
/// (`required_qualifications`, comma separated; `SCH.ACL.04`,
/// `SCH.RUL.05`).
pub const REQUIRED_QUALIFICATIONS: &str = "practitioner_licence,branch_privileges";

/// A service type the clinic starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceSeed {
    /// The stable code (`consultation`).
    pub code: &'static str,
    /// The name in English.
    pub name_en: &'static str,
    /// The name in Arabic.
    pub name_ar: &'static str,
    /// How long it takes.
    pub minutes: i64,
}

/// The service types the host installs where the database has none of that
/// code (05 §4.1): each checks the patient in, tracks the visit and needs the
/// licence and the privileges ([`REQUIRED_QUALIFICATIONS`]). Prices, deposits
/// and the self-pay price list are the clinic's own (`SAL.CAT.01`).
pub const SERVICE_TYPES: &[ServiceSeed] = &[
    ServiceSeed {
        code: "consultation",
        name_en: "Consultation",
        name_ar: "استشارة",
        minutes: 30,
    },
    ServiceSeed {
        code: "follow_up",
        name_en: "Follow-up visit",
        name_ar: "زيارة متابعة",
        minutes: 20,
    },
    ServiceSeed {
        code: "procedure",
        name_en: "Procedure",
        name_ar: "إجراء",
        minutes: 60,
    },
];

/// The days before the expiry of a licence or of privileges that `DOC.RUL.01`
/// reminds (180 days are for the facility licence, not for a practitioner).
pub const REMINDER_DAYS: &[i64] = &[90, 60, 30, 7];

/// The days ahead `DOC.RUL.02` lists the bookings a lapsing licence affects
/// (the same 30 days as `SCH.RUL.05`).
pub const LOOK_AHEAD_DAYS: i64 = 30;

// ---- Documents (05 §7.2 *Responsible role per document kind*, §5) ----

/// The facility licence of the clinic (DHA, DoH, Sharjah Health Authority or
/// MOHAP): a document of the legal entity, renewed by the owner / medical
/// director, compliance-critical (`DOC.RUL.03`: the owner is told when it
/// expires).
pub const FACILITY_LICENCE_KIND: &str = "facility_licence";

/// The days before its expiry that `DOC.RUL.01` reminds of the facility
/// licence: 180 days, since a renewal takes months, then the usual steps.
pub const FACILITY_REMINDER_DAYS: &[i64] = &[180, 90, 30, 7];

/// The patient's Emirates ID: a document of the patient, kept by the
/// receptionist; the patient (or the guardian) is told as the party when it
/// is about to expire. The image needs `patients.sensitive.view`
/// (`CLN.ACL.12`).
pub const EMIRATES_ID_KIND: &str = "patient_emirates_id";

/// The days before its expiry that the patient is reminded of the Emirates
/// ID.
pub const EMIRATES_ID_REMINDER_DAYS: &[i64] = &[90, 30, 7];

// ---- Scheduling (05 §7.2 settings) ----

/// Hours before an appointment's start at which an open conflict is
/// escalated to the clinic manager (`SCH.RUL.06`, H).
pub const ESCALATION_HOURS: i64 = 2;

/// Minutes after the start of an appointment before the no-show rule marks
/// it (`SCH.RUL.02`, M).
pub const NO_SHOW_GRACE_MINUTES: i64 = 15;

/// Hours before an appointment at which a reminder goes out (`SCH.RUL.01`,
/// N = 24, 2).
pub const REMINDER_HOURS: &[i64] = &[24, 2];

/// The booking deposit of a service type the clinic seeds: none. A clinic
/// enables it per service type (05 §13 question 8).
pub const BOOKING_DEPOSIT_FILS: i64 = 0;

/// Hours before the start inside which a patient's cancellation forfeits the
/// deposit, for a service type that has one.
pub const DEPOSIT_FORFEIT_HOURS: i64 = 24;

// ---- Notifications (05 §7.2, wl-notify §7) ----

/// A broadcast to more than this many patients needs a second approver
/// (`NTF.ACL.03`).
pub const BROADCAST_APPROVAL_ABOVE: i64 = 200;

/// The most marketing messages one patient gets in 7 days (`NTF.RUL.02`).
pub const MARKETING_CAP: i64 = 2;

/// The marketing send window, minutes after midnight Asia/Dubai: 07:00 to
/// 21:00 (`NTF.RUL.02`).
pub const SEND_WINDOW_MINUTES: (i64, i64) = (7 * 60, 21 * 60);

// ---- Sales and loyalty (05 §7.2, wl-sales §7) ----

/// The lead sources a lead or a first sale must name a campaign for
/// (`SAL.CRM.02`).
pub const CAMPAIGN_REQUIRED_SOURCES: &[&str] = &["paid_ads", "social"];

/// The lead sources 05 uses: the nine of the module, all of them.
pub const LEAD_SOURCES: &[&str] = &[
    "referral",
    "walk_in",
    "website",
    "social",
    "paid_ads",
    "aggregator",
    "event",
    "partner",
    "other",
];

/// The days before a loyalty lot expires that its member is reminded
/// (`SAL.RUL.08`).
pub const LOYALTY_REMINDER_DAYS: i64 = 30;

/// The points above which a manual adjustment needs a manager
/// (`SAL.ACL.07`).
pub const LOYALTY_ADJUSTMENT_LIMIT: i64 = 500;

/// The loyalty programme 05 seeds: 1 point per AED 10 paid by the patient,
/// 100 points an AED 10 discount (10 fils a point), valid 12 months. Whether
/// health authority rules limit rewards on medical services is
/// `⚠ open: lawyer (L-38)`; the seed earns on self-pay services until
/// answered. The VAT of the discounted supply is `⚠ open: accountant (A-15)`.
pub const LOYALTY_POINTS: i64 = 1;
/// The dirhams that earn [`LOYALTY_POINTS`].
pub const LOYALTY_EARN_PER_AED: i64 = 10;
/// What one point is worth, in fils.
pub const LOYALTY_POINT_VALUE_FILS: i64 = 10;
/// The fewest points that can be redeemed: 100 points, AED 10.
pub const LOYALTY_MIN_REDEEM: i64 = 100;
/// The months points are valid after they are earned.
pub const LOYALTY_VALIDITY_MONTHS: i64 = 12;
/// The name of the programme.
pub const LOYALTY_NAME: &str = "Rewards";

/// The days of no visit after which a patient is in the lapsed segment: six
/// months.
pub const LAPSED_SEGMENT_DAYS: i64 = 180;
/// The name of the lapsed segment.
pub const LAPSED_SEGMENT_NAME: &str = "No visit in 6 months";

// ---- Connections (05 §8 *When a connection is down*) ----

/// The key of the claims channel (eClaimLink / DHPO, Shafafiya, the MOHAP
/// channel). The MVP records submissions made through the payer's portal; an
/// integrated channel (`CLN.CLM.02`, later) reports to this key.
pub const CLAIMS_CHANNEL: &str = "clinics.claims_channel";

/// What the front desk does while the SMS connection is down.
pub const SMS_FALLBACK_EN: &str = "Phone the patients on the call list of each branch: the appointments of the next 24 hours, with a called tick for each.";
/// The same, in Arabic.
pub const SMS_FALLBACK_AR: &str =
    "اتصل بالمرضى من قائمة كل فرع: مواعيد الساعات الأربع والعشرين القادمة، مع علامة اتصال لكل موعد.";
/// What the front desk does while the WhatsApp connection is down.
pub const WHATSAPP_FALLBACK_EN: &str = SMS_FALLBACK_EN;
/// The same, in Arabic.
pub const WHATSAPP_FALLBACK_AR: &str = SMS_FALLBACK_AR;
/// What the desk does while the payment gateway is down.
pub const GATEWAY_FALLBACK_EN: &str =
    "Take the deposit or the payment on the card terminal or in cash and record it.";
/// The same, in Arabic.
pub const GATEWAY_FALLBACK_AR: &str = "استلم العربون أو الدفعة عبر جهاز البطاقات أو نقدًا وسجّلها.";
/// What billing does while the claims channel is down.
pub const CLAIMS_FALLBACK_EN: &str = "Submit through the payer portal and record the reference.";
/// The same, in Arabic.
pub const CLAIMS_FALLBACK_AR: &str = "قدّم عبر بوابة شركة التأمين وسجّل الرقم المرجعي.";
/// What staff do while e-mail is down.
pub const EMAIL_FALLBACK_EN: &str = "Receipts and invoices stay visible in the portal; give the patient a printed copy at the desk.";
/// The same, in Arabic.
pub const EMAIL_FALLBACK_AR: &str =
    "تبقى الإيصالات والفواتير ظاهرة في البوابة؛ سلّم المريض نسخة مطبوعة عند الاستقبال.";

// ---- Onboarding checklists (HRM.ONB.01) ----

/// One step of an onboarding checklist: title, owner (`hr`, `manager`,
/// `employee` or `role:<key>`), due days after the start and kind (`task`,
/// `sops` or `checklist:<template>`).
pub type StepSpec = (&'static str, &'static str, i64, &'static str);

/// An onboarding checklist for one job role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnboardingSpec {
    /// The template's name.
    pub name: &'static str,
    /// The job role it applies to.
    pub role: &'static str,
    /// The steps, in order.
    pub steps: &'static [StepSpec],
}

/// The first step of every checklist: the clinic manager sets up the account
/// and the access (tasks to the clinic manager, the line manager and the
/// employee; the employee records only their own steps, `HRM.ACL.10`).
const ACCOUNT_STEP: StepSpec = (
    "System account and access",
    "role:clinic_manager",
    1,
    "task",
);
/// The line manager's welcome.
const WELCOME_STEP: StepSpec = (
    "Welcome, branch tour and introductions",
    "manager",
    1,
    "task",
);
/// The employee uploads what they hold.
const DOCUMENTS_STEP: StepSpec = (
    "Upload your Emirates ID and your documents",
    "employee",
    3,
    "task",
);
/// The employee reads the SOPs and acknowledges them.
const SOPS_STEP: StepSpec = ("SOPs acknowledged", "employee", 7, "sops");

/// The onboarding checklists 05 seeds: receptionist, call-centre agent, nurse
/// or assistant, practitioner, billing coordinator and storekeeper. Each
/// ends with the SOPs; completion blocks nothing (the role is active before
/// the checklist is complete, and bookings stay guarded by the licences,
/// `SCH.ACL.04`). The checklist steps are `WRK.CHK.01` templates with photos
/// and a signature.
pub const ONBOARDING: [OnboardingSpec; 6] = [
    OnboardingSpec {
        name: "Receptionist onboarding",
        role: "receptionist",
        steps: &[
            ACCOUNT_STEP,
            WELCOME_STEP,
            (
                "Front desk, check-in and payments",
                "manager",
                3,
                "checklist:receptionist_induction",
            ),
            DOCUMENTS_STEP,
            SOPS_STEP,
        ],
    },
    OnboardingSpec {
        name: "Call-centre agent onboarding",
        role: "call_centre_agent",
        steps: &[
            ACCOUNT_STEP,
            WELCOME_STEP,
            (
                "Booking, leads and patient contacts",
                "manager",
                3,
                "checklist:call_centre_induction",
            ),
            DOCUMENTS_STEP,
            SOPS_STEP,
        ],
    },
    OnboardingSpec {
        name: "Nurse and assistant onboarding",
        role: "nurse",
        steps: &[
            ACCOUNT_STEP,
            WELCOME_STEP,
            (
                "Rooms, consumables and consent forms",
                "manager",
                3,
                "checklist:nurse_induction",
            ),
            DOCUMENTS_STEP,
            SOPS_STEP,
        ],
    },
    OnboardingSpec {
        name: "Practitioner onboarding",
        role: "practitioner",
        steps: &[
            ACCOUNT_STEP,
            WELCOME_STEP,
            (
                "Licence and branch privileges recorded",
                "role:clinic_manager",
                1,
                "task",
            ),
            (
                "Schedule, charge capture and consent forms",
                "manager",
                3,
                "checklist:practitioner_induction",
            ),
            DOCUMENTS_STEP,
            SOPS_STEP,
        ],
    },
    OnboardingSpec {
        name: "Billing coordinator onboarding",
        role: "billing_coordinator",
        steps: &[
            ACCOUNT_STEP,
            WELCOME_STEP,
            (
                "Eligibility, authorisations and claims",
                "manager",
                3,
                "checklist:billing_induction",
            ),
            DOCUMENTS_STEP,
            SOPS_STEP,
        ],
    },
    OnboardingSpec {
        name: "Storekeeper onboarding",
        role: "storekeeper",
        steps: &[
            ACCOUNT_STEP,
            WELCOME_STEP,
            (
                "Receiving, lots and issuing consumables",
                "manager",
                3,
                "checklist:storekeeper_induction",
            ),
            DOCUMENTS_STEP,
            SOPS_STEP,
        ],
    },
];

/// An SOP of the library with its owner role and required readers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SopSpec {
    /// The code.
    pub code: &'static str,
    /// The English title.
    pub title: &'static str,
    /// The category.
    pub category: &'static str,
    /// The role whose holders accept and review it.
    pub owner_role: &'static str,
    /// The roles that must read and acknowledge it.
    pub readers: &'static [&'static str],
}

/// The SOP library 05 seeds: the clinic manager owns the front desk, billing
/// and complaints procedures; the owner / medical director owns infection
/// control and the clinical ones. `DOC.RUL.04` asks each reader to
/// acknowledge within 7 days, reminds every 3 days and tells the reader's
/// manager and the owner role of the SOP after 7 days overdue (the module's
/// defaults); a new version asks for a new acknowledgement.
pub const SOPS: [SopSpec; 5] = [
    SopSpec {
        code: "SOP-CLN-FRONT-DESK",
        title: "Front desk: registration, check-in and payments",
        category: "front_desk",
        owner_role: crate::permissions::CLINIC_MANAGER_ROLE,
        readers: &[
            crate::permissions::RECEPTIONIST_ROLE,
            crate::permissions::CALL_CENTRE_ROLE,
            crate::permissions::CLINIC_MANAGER_ROLE,
        ],
    },
    SopSpec {
        code: "SOP-CLN-BILLING",
        title: "Billing, claims and payer follow-up",
        category: "billing",
        owner_role: crate::permissions::CLINIC_MANAGER_ROLE,
        readers: &[
            crate::permissions::BILLING_COORDINATOR_ROLE,
            crate::permissions::RECEPTIONIST_ROLE,
            crate::permissions::CLINIC_MANAGER_ROLE,
        ],
    },
    SopSpec {
        code: "SOP-CLN-COMPLAINTS",
        title: "Patient complaints",
        category: "complaints",
        owner_role: crate::permissions::CLINIC_MANAGER_ROLE,
        readers: &[
            crate::permissions::RECEPTIONIST_ROLE,
            crate::permissions::CALL_CENTRE_ROLE,
            crate::permissions::NURSE_ROLE,
            crate::permissions::CLINIC_MANAGER_ROLE,
        ],
    },
    SopSpec {
        code: "SOP-CLN-INFECTION-CONTROL",
        title: "Infection control",
        category: "infection_control",
        owner_role: crate::permissions::OWNER_ROLE,
        readers: &[
            crate::permissions::PRACTITIONER_ROLE,
            crate::permissions::NURSE_ROLE,
            crate::permissions::RECEPTIONIST_ROLE,
            crate::permissions::STOREKEEPER_ROLE,
            crate::permissions::CLINIC_MANAGER_ROLE,
        ],
    },
    SopSpec {
        code: "SOP-CLN-CLINICAL",
        title: "Clinical procedures and consent",
        category: "clinical",
        owner_role: crate::permissions::OWNER_ROLE,
        readers: &[
            crate::permissions::PRACTITIONER_ROLE,
            crate::permissions::NURSE_ROLE,
        ],
    },
];

/// The compliance-critical rules 05 lists (§7.2): the owner / medical
/// director is alerted through `PLT.RUL.04` when one fails or misses its
/// schedule, and the client cannot switch that off.
pub const CRITICAL_RULES: [&str; 4] = ["CLN.RUL.01", "SCH.RUL.05", "CLN.RUL.03", "DOC.RUL.03"];

// ---- money and stock (BL-CLN-16, 05 section 7.2 settings) -----------------

/// The write-off limit of a payer agreement, in fils: AED 1,000 (`FIN.ACL.14`).
/// A write-off above it is the owner's to approve.
pub const CLAIM_WRITE_OFF_LIMIT: i64 = 100_000;

/// Days after a claim is submitted when `FIN.RUL.13` follows up a claim with
/// no remittance (N = 45).
pub const PAYER_FOLLOW_UP_DAYS: i64 = 45;

/// Days a billing dispute may stay open before `FIN.RUL.15` escalates it.
pub const DISPUTE_ESCALATION_DAYS: u32 = 14;

/// The projected balance below which `FIN.RUL.23` alerts, in fils: AED
/// 50,000, in any of the next 13 weeks.
pub const CASH_THRESHOLD: i64 = 5_000_000;

/// The weeks the cash-flow forecast looks ahead (`FIN.CFL.01`).
pub const CASH_FORECAST_WEEKS: u32 = 13;

/// When the daily cash-flow check runs (`FIN.RUL.23`), Asia/Dubai.
pub const CASH_ALERT_TIME: &str = "06:00";

/// The share of a month's budget line at which `FIN.RUL.24` warns, in percent.
pub const BUDGET_WARNING_PERCENT: u32 = 100;

/// Minutes after an online payment became `pending` when the gateway is asked
/// for its final state (`FIN.RUL.19`).
pub const PENDING_TIMEOUT_MINUTES: i64 = 30;

/// Hours after a gateway refund failed before it is retried (`FIN.RUL.21`).
pub const REFUND_RETRY_HOURS: i64 = 24;

/// The retries of one failed refund: once.
pub const REFUND_MAX_RETRIES: i64 = 1;

/// Days before the evidence deadline of a chargeback that the accountant is
/// reminded (`FIN.RUL.20`).
pub const CHARGEBACK_REMINDER_DAYS: i64 = 2;

/// Hours a pending issue stays open before the clinic manager is told and the
/// storekeeper gets the task (`STK.RUL.06`).
pub const PENDING_ISSUE_ALERT_HOURS: i64 = 24;

/// Hours between two reminders of an open stock hold (`STK.RUL.07`).
pub const HOLD_REMINDER_HOURS: i64 = 24;

/// The reason of a stock hold on a suspect consumable (complaint, broken cold
/// chain).
pub const SUSPECT_HOLD_REASON: &str = "quality";

/// The hold reasons whose release needs a second holder of `lots.release`
/// (`STK.ACL.05`).
pub const SECOND_PERSON_HOLD_REASONS: &[&str] = &["recall"];

/// Cash holders (`FIN.CSH.01`, 05 section 7.2): each receptionist, per person
/// and per day at the branch: one handover per receptionist, branch and day.
/// The handover is wl-payments' `BL-PAY-17` (#110), not built yet; these are
/// the values it will read.
pub const CASH_HOLDER_MODE: &str = "staff";

/// The period of a handover: a day (Asia/Dubai).
pub const CASH_HANDOVER_PERIOD: &str = "day";

/// The write-off limit of a cash shortage, in fils: AED 0, every shortage is
/// approved (`FIN.ACL.06`).
pub const CASH_SHORTAGE_WRITE_OFF_LIMIT: i64 = 0;

/// Whether a shortage may be recovered by salary deduction (`FIN.ACL.19`):
/// off `⚠ open: lawyer (L-20)`.
pub const SALARY_RECOVERY: bool = false;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_service_types_have_unique_codes() {
        let mut codes: Vec<&str> = SERVICE_TYPES.iter().map(|s| s.code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), SERVICE_TYPES.len());
    }

    #[test]
    fn the_money_values_are_the_05_ones() {
        assert_eq!(CLAIM_WRITE_OFF_LIMIT, 100_000);
        assert_eq!(CASH_THRESHOLD, 5_000_000);
        assert_eq!(CASH_SHORTAGE_WRITE_OFF_LIMIT, 0);
        assert_eq!(SECOND_PERSON_HOLD_REASONS, ["recall"]);
    }

    #[test]
    fn every_service_requires_the_licence_and_the_privileges() {
        assert_eq!(
            REQUIRED_QUALIFICATIONS.split(',').collect::<Vec<_>>(),
            [LICENCE_KIND, PRIVILEGES_KIND]
        );
    }
}
