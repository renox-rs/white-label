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
| The enrolment (student, guardian and payer are parties of wl-parties, by id; a course seat is taken with `seats::take` in the transaction that inserts it and held 48 hours; a package or a membership holds none) and its machine: `activate` (terms accepted, discount approved), `lapse` (guard slot of BL-EDU-01), `freeze` (each cause stored, 30 days a year for the student's own, a freeze on a frozen enrolment only adds its cause), `unfreeze` (only when no cause is left), `complete`, `withdraw` (the refund the policy gives stored, instalments not yet invoiced cancelled, seat released, a membership cancelled) and `transfer` (guard slot of BL-EDU-02); the transitions a user or the system may run come as a pair (`freeze`, `freeze_system`). The enrolment pages: the list, the page with its actions and the new enrolment form with the party picker and the batch select | §4.1, §6 *Enrolment*, §7.1 *Enrolment*, `EDU.ENR.01` | `enrolment`, `enrolment_machine`, `enrolments`, `pages::enrolments` |
| The refund calculation `EDU.REF.01` as a pure function (before the start: paid less the registration fee; after it: paid less the sessions held, the admin fee and the registration fee; nothing after 50% of the sessions; the amount owed when used sessions and fees exceed what was paid; a package at the single-session price; a membership left to its subscription) | §4.3, §7.1 *Enrolment* `withdraw`, `EDU.REF.01` | `refund` |
| The fees of an enrolment through the kernel contracts: the first invoice and the instalment plan (`Invoicer`), the package sold (`PackageSales`, so `PackageLedger::consume` and `reverse` find it), the membership's subscription started on activation (`Subscriptions`), and the daily run `edtech:instalments` that invoices the instalments that fall due | §4.1 step 7, §4.3, `FIN.BIL.01`, `FIN.SUB.01` | `fees` |
| The certificate and its machine: `issue` (the criteria of `EDU.ACL.06` met, the guard slot `document_valid` of BL-EDU-07; the number from a gapless series, never reused), `reissue` (the old one superseded, a linked draft with the corrections, a recorded attestation copied only when confirmed) and `revoke` (`certificates.revoke`); a draft is made when a course enrolment completes. The pages: the list, the certificate with its actions, and the public verification page `/verify/certificate/{code}` that needs no login | §4.4, §7.1 *Certificate*, `EDU.CRT.01` | `certificate`, `certificate_machine`, `certificates`, `pages::certificates` |
| The tutor assignments (batch or session, tutor resource and employee, valid from and to) kept as history behind the tutor columns, the change of a batch's tutor from a date (the new tutor free and eligible on every session moved, all or nothing) with its page action, and the slot `tutors::tutor_eligible` where BL-EDU-08 puts the guard | §7.1 *Batch* and *Session* `change_tutor`, `EDU.ACL.14` | `tutor_assignment`, `tutors` |
| The labels the documents and bills of other modules read (`BAT-0007`, `ENR-0007`, `CRT-0150`) for the batch, session, enrolment, certificate and course subjects, so that a document kind's `blocks` entry on `batch.open` or `certificate.issue` can name them | §9, wl-documents `register_subject_label` | `lib` |
