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
| The course (programme, level, hours, delivery, regulator approval) and its page | §6 *Course*, `EDU.CRS.01`, `EDU.CRS.02` | `course`, `courses`, `pages::courses` |
| The batch (capacity, dates, weekly pattern, tutor and room) and its machine: `open` (the plan is complete and gives sessions), `change_tutor`, `start` (system, on the first session date), `complete` (system), and `cancel` and `terminate` declared with their reason and the guard slots of BL-EDU-13 and BL-EDU-12 | §6 *Batch*, §7.1 *Batch*, `EDU.BAT.01` | `batch`, `batch_machine`, `batches`, `pages::batches` |
| The plan: the sessions the weekly pattern gives between two dates, around the public holidays of the calendar | §4.2 step 3 | `plan` |
| The seats of a batch: `available`, `take` (one statement, refused when full; only the room limit goes over the capacity) and `release` | `EDU.ACL.17` | `seats` |
| The session (batch, date, time, tutor, room, make-up) and its machine: `reschedule`, `change_tutor`, `submit_attendance` (the tutor, once it started), `correct_attendance` (declared with its reason and the slot of BL-EDU-14) and `cancel` (with a reason and an optional make-up session). The sessions of a batch are generated in the effect of its `open`, around the public holidays, after the tutor and the room are checked free for every date; the batch completes when its last session is held or cancelled | §6 *Session*, §7.1 *Session*, `EDU.BAT.01`, `EDU.ATT.01` | `session`, `session_machine`, `sessions`, `pages::sessions` |
| The sessions as wl-scheduling blocks of their tutor and room (`AvailabilitySource`, source `session`, reference `edtech.session:7`), so a trial or one-to-one slot is never booked over a class | wl-scheduling §6, §8 | `sessions::SessionBlocks` |
