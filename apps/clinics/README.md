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

The app owns `src/texts.rs`, the English texts (`clinics.*`). A client changes or translates them
in `resources/lang/{locale}.json`.
