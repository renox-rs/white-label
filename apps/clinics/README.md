# clinics

The vertical app for private outpatient clinics
([spec](../../docs/verticals/05-clinics.md)): the appointments that come from
wl-scheduling bookings, the visit a check-in opens, the patients' insurance
policies and what the front desk records before the patient is let in, and the
practitioner licences that guard the bookings. The app reaches the other
modules only through the contracts of `wl-kernel` (the scheduling events and
the check-in guard, the qualification guard, the notifier, the task creator),
never through their crates. Clinical records stay in the clinic's EMR.

The host mounts it when `client.toml` names `app = "clinics"`:

```
use renox::prelude::*;

# let _ =
App::new().module(clinics::Clinics)
# ;
```

What is here so far:

| Part | Spec | Module |
|---|---|---|
| Permissions (`visits.view`, `insurance_policies.*`, `coverage.record`) and the role names the rules use (owner, clinic manager, receptionist) | §3.1, §3.3 | `permissions` |
| The appointment record: the clinic's side of a wl-scheduling booking (patient, service code, visit date, the policy it was booked with), made from `BookingBooked`, moved by `BookingRescheduled`, closed by `BookingCancelled` | §6 *Appointment* | `appointment`, `appointments` |
| The visit a check-in opens (`clinics.visit`, `VIS-0007`) and its machine (checked in, in progress, completed, cancelled): every transition is the system's, run by the listeners on `BookingStarted`, `BookingCompleted` and `BookingCancelled` after the check-in; a cancellation before the check-in opens none | §6 *Visit*, §7.1 *Appointment* | `visit`, `visit_machine`, `visits` |
| The insurance policy (patient, payer, plan, member id, dates, the card's valid-to) and the eligibility result recorded per policy and day (positive or negative, the payer's reference); a policy valid to the visit day is valid all day, an expired card fails a policy still in force; an appointment is booked with a policy of its patient (an *insured* visit) until the patient is checked in; the policies page records one | §6 *Insurance policy*, `CLN.INS.02` | `policy`, `policies`, `appointments`, `pages::policies` |
| `CLN.RUL.01`: when the licence or the branch privileges of a practitioner expire (`QualificationExpired`, from wl-hr's `HRM.RUL.10`), the clinic managers of the branch and the owner / medical director each get one message naming the practitioner, the branch and the number of appointments affected, once per expiry; no second scan, no cancellation. The block on new bookings, check-in and start is wl-scheduling's through `required_qualifications`, and the booked appointments are put in conflict by `SCH.RUL.05`. A run that fails five times alerts the owner; `CLN.RUL.01` and `DOC.RUL.03` are compliance-critical, owned by the owner role | §4.5, §7.2 `CLN.RUL.01` | `licences`, `seed` |
| The coverage outcome of an insured visit (`CLN.ACL.15`): on the appointment page the receptionist records the eligibility result and one outcome, *insured* (set by itself by a positive result for a policy and card valid on the visit day), *re-split* to another valid policy of the patient with a positive result today, or *self-pay* after the patient signs the self-pay consent showing the estimated price (the self-pay price list on the visit day; "no self-pay price for ..." otherwise); `check_in` is refused with "coverage outcome required" until both are set (the kernel's check-in guard); the data moves to the visit at the check-in and is discarded when the appointment is cancelled or rescheduled, booked with another policy, or not checked in by 23:59 of its day; `coverage_not_confirmed` is the 05 cancellation reason (business, deposit refunded in full); before `start` the outcome may change to *insured* when the eligibility turns positive (the consent is kept, marked superseded), after `start` it is fixed; a service not on a self-pay consent needs an updated one (`consent::ensure_covered`, `consent::sign_updated`) | §3.3 `CLN.ACL.15`, §4.2, §7.1 *Appointment* `check_in`, §9 | `coverage`, `consent`, `pages::appointments`, `pages::visits`, `reasons` |

The app owns `src/texts.rs`, the English texts (`clinics.*`). A client changes or translates them
in `resources/lang/{locale}.json`.
