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

The app owns `src/texts.rs`, the English texts (`clinics.*`). A client changes or translates them
in `resources/lang/{locale}.json`.
