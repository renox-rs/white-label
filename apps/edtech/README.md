# edtech

The vertical app for training and tutoring centres
([spec](../../docs/verticals/10-edtech.md)): courses and programmes, the
batches that run them, the sessions generated from a batch's weekly plan,
enrolments with their seats and fees, attendance and certificates. The app
reaches the other modules only through the contracts of `wl-kernel` (the
scheduling events and availability, the qualification and document guards, the
notifier, the task creator, the invoicer), never through their crates.

The host mounts it when `client.toml` names `app = "edtech"`:

```
use renox::prelude::*;

# let _ =
App::new().module(edtech::EdTech)
# ;
```

What is here so far:

| Part | Spec | Module |
|---|---|---|
| Permissions (`courses.*`, `batches.*`, `sessions.*`) and the role names the rules use (owner, centre manager, counsellor, academic coordinator, tutor, accountant) | §3.1, §3.3 | `permissions` |
| The starter tables the host installs: the document kinds (centre permit, course approval, police clearance, tutor permit, student ID, guardian consent), the service types (trial class, placement test, one-to-one) and the ticket categories | §7.1 *Booking*, §9 | `seed` |
| The cancellation reason codes 10 adds to wl-scheduling (`tutor_unavailable`, `tutor_ineligible`) | §7.1 *Booking* | `reasons` |
