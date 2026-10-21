# Epic: wl-notify — negative flows

> Status: draft · Last updated: 2026-10-16 · Spec version: same commit as this file

What happens when a message gives up: after all retries and fallbacks, `give_up` emits
`MessageGaveUp` with full detail and `NTF.RUL.01` runs the notification type's give-up reaction,
so no module is left assuming a message arrived. The reactions are a staff task, an alternative
recipient, marking the contact point invalid, and the statutory-notice task for notices with
legal effect. Review item X-20 (and the 01 and 04 statutory notices that depend on it).

BL-NTF-07 to BL-NTF-09 (label `area:sme-gap`, added 2026-10-16) add marketing broadcasts for
the retention campaigns of `SAL.LOY.01` (gap G4 of the
[SME problem map](../../reviews/2026-10-16-sme-problem-map.md) §3). They are not negative
flows; they follow the same template. BL-NTF-09 and the negative paths in BL-NTF-07 come from
the gap review (GR-G4).

| ID | Title | Priority | Size | Milestone | Depends on |
|---|---|---|---|---|---|
| BL-NTF-01 | Run the notification type's give-up reaction when a message gives up | medium | m | Build 1 | Base: wl-notify Message lifecycle |
| BL-NTF-02 | Open a staff task when a message could not be delivered | medium | s | Build 1 | BL-NTF-01 |
| BL-NTF-03 | Send the message to another contact of the same party | medium | s | Build 1 | BL-NTF-01, BL-NTF-02 |
| BL-NTF-04 | Mark a contact point invalid after a hard failure | medium | s | Build 1 | BL-NTF-01 |
| BL-NTF-05 | Hold the deadline of a statutory notice until proof of delivery | high | m | Build 1 | BL-NTF-01, BL-NTF-02 |
| BL-NTF-06 | Seed the default give-up reaction per type group | medium | s | Build 1 | BL-NTF-01 to BL-NTF-05 |
| BL-NTF-07 | Send a marketing broadcast to a segment, with approval above a size (`NTF.ACL.03`) | medium | m | Launch 05 | BL-SAL-19; Base: wl-notify Message lifecycle; Base: wl-parties consents; Base: wl-kernel approvals |
| BL-NTF-08 | Apply the marketing send window and frequency cap (`NTF.RUL.02`) | medium | s | Launch 05 | BL-NTF-07 |
| BL-NTF-09 | Expire unapproved broadcasts, drop stale marketing messages and stop a broadcast (`NTF.RUL.03`) | medium | s | Launch 05 | BL-NTF-07; BL-NTF-08 |
| BL-NTF-10 | Send stored documents as e-mail attachments through the kernel notifier | high | s | Build 1 | Base: wl-notify Message lifecycle |

---

### BL-NTF-01 — wl-notify: run the notification type's give-up reaction when a message gives up

**Labels:** `module:wl-notify` `area:negative-flow` `priority:medium` `size:m` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: wl-notify Message lifecycle (`send`, `fail`, `retry`, `fall_back`) · **Blocks:** BL-NTF-02, BL-NTF-03, BL-NTF-04, BL-NTF-05, BL-NTF-06, BL-ACC-24, BL-CLN-15
**Review items:** X-20

#### Goal
Each notification type declares whether it is statutory, which give-up reaction it uses and which
staff role is told. When the last message of a send ends `abandoned` through `give_up`,
`MessageGaveUp` is emitted with the channels tried and errors, and `NTF.RUL.01` runs the type's
reaction. Requester modules may also listen, but the default reaction always runs.

#### Surfaces
- **Back office:** the notification type admin shows the statutory flag, the give-up
  reaction and the staff role told for each type.
- **System:** `give_up` emits `MessageGaveUp` with the channels tried and errors, and
  `NTF.RUL.01` runs the type's reaction once per message.

#### Files to read
- `docs/modules/wl-notify.md` §3 (*Notification type*), §4.1 *Message* (`give_up`, `fall_back`),
  §5.4 (first paragraph and reaction table), §6 (event `MessageGaveUp`, `NTF.RUL.01`), §7
- `docs/WORKFLOW.md` §8 (pattern "Message gave up")

#### Terms
- **Give-up:** the message failed and no retry or fallback channel is left.
- **Give-up reaction:** one or more of `staff_task`, `alternative_recipient`,
  `mark_contact_invalid`, `statutory_notice`, `none`.
- **Statutory type:** a notice with legal or contractual effect (deadline notice, notice of
  changed terms); set by the vertical.
- **Requester module:** the module that asked for the message through `Notifier::notify`.

#### Spec excerpt (authoritative for this issue)

*Notification type* fields added: statutory (yes/no: the message is a notice with legal or
contractual effect), give-up reaction (§5.4), staff role told on give-up.

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `fall_back` | failed | (new message on next channel) → abandoned | system | next channel in the type's order is allowed for the recipient | a new message is queued on the next channel; this one ends `abandoned` |
| `give_up` | failed | abandoned | system | no retries or fallbacks left | event `MessageGaveUp`; the type's give-up reaction runs (`NTF.RUL.01`, §5.4) |

§5.4: "A message that ends `abandoned` through `give_up` was not delivered on any allowed
channel. The requester must not assume it was. `wl-notify` emits `MessageGaveUp` and runs the
notification type's give-up reaction (`NTF.RUL.01`). A requester module can listen to the event
for its own follow-up (e.g. pause a reminder sequence), but the default reaction always runs, so
no module is left without one. `NTF.RUL.01` is not compliance-critical, even for statutory types
(their `statutory_notice` task is the safeguard): a failed run is retried with backoff like any
automation and shows as `AutomationFailed`."

| Event | When | Payload |
|---|---|---|
| `MessageGaveUp` | a message ends `abandoned` after all retries and fallbacks | message id, type, subject, recipient, channels tried, last error per channel, statutory (yes/no), reaction applied |

| ID | Rule | When | Then | Parameter (default) |
|---|---|---|---|---|
| `NTF.RUL.01` | Give-up reaction | a message ends `abandoned` through `give_up` | run the notification type's give-up reaction (§5.4): staff task, alternative recipient, mark the contact point invalid, or the statutory-notice task | task due 1 working day; on |

Settings: give-up reaction per type, statutory flag (the module's and vertical's defaults);
give-up task due 1 working day.

#### Edge cases decided
- **Fallback chain:** messages that end `abandoned` through `fall_back` run no reaction; only the
  final message of the chain (through `give_up`) does. "Channels tried" covers the whole chain.
- **Combined reactions** run in this order: `mark_contact_invalid` first, then the others, so an
  alternative recipient never reuses the invalid contact.
- **Idempotent:** one reaction run per message id (rule occurrence key).
- **`suppressed` messages** (no consent, opt-out, no contact point) are not give-ups and run no
  reaction.
- **Reaction failure:** the rule run retries with backoff like any automation. `NTF.RUL.01`
  is not compliance-critical, even for statutory types; a failed run shows as `AutomationFailed`.

#### Must not
- Must not mark a give-up message as delivered or sent in any report.
- Must not skip the default reaction because a requester module also listens.

#### Done when
- [ ] Given type `invoice.reminder` with channels email → SMS, when email fails permanently and
      SMS fails 3 times, then the SMS message ends `abandoned` through `give_up`, the email
      message ends `abandoned` through `fall_back`, and one `MessageGaveUp` lists both channels
      and their last errors.
- [ ] The type's configured reaction runs once; rerunning the rule does nothing new.
- [ ] A `suppressed` message runs no reaction.
- [ ] The notification type admin shows statutory, reaction and staff role per type.

#### Out of scope
Each reaction (BL-NTF-02 to BL-NTF-05); default table (BL-NTF-06).

---

### BL-NTF-02 — wl-notify: open a staff task when a message could not be delivered

**Labels:** `module:wl-notify` `area:negative-flow` `priority:medium` `size:s` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-NTF-01 · Base: wl-ticketing Ticket lifecycle (`TaskCreator`), wl-kernel business calendar · **Blocks:** BL-NTF-03, BL-NTF-05, BL-ACC-24, BL-CLN-14, BL-CLN-15, BL-NTF-06
**Review items:** X-20

#### Goal
The `staff_task` reaction gives the type's staff role in the subject's scope a task "Could not
reach {recipient} about {subject}", with the channels tried and the errors, due in 1 working day.
For urgent types the role also gets an in-app alert.

#### Surfaces
- **Back office:** the type's staff role in the subject's scope (else the company admin)
  gets a task "Could not reach {recipient} about {subject}" with masked address, channels
  and errors, due in 1 working day, and an in-app alert for urgent types.
- **System:** a second give-up for the same subject and recipient is added as a comment to
  the open task.

#### Files to read
- `docs/modules/wl-notify.md` §5.4 (reaction `staff_task`; row "Urgent types of any module")
- `docs/modules/wl-kernel.md` §7 (weekend Saturday–Sunday, Asia/Dubai), §2 (business calendar)

#### Terms
- **Working day:** a day that is not a weekend day (Saturday, Sunday) or a public holiday in the
  kernel business calendar for the scope.
- **Task:** a ticket created through `TaskCreator`.
- **Urgent type:** a notification type whose `urgency` field is `urgent` (*Notification type*
  entity, `wl-notify.md` §3: urgency `normal` / `urgent`), set by the module in `register_type`
  or by the vertical. There is no fixed list; the field decides.

#### Spec excerpt (authoritative for this issue)

Entity *Notification type* (§3): "… category (`transactional`, `reminder`, `security`,
`marketing`), urgency (`normal`, `urgent`), … staff role told on give-up …".

| Reaction | What happens |
|---|---|
| `staff_task` | A task (`wl-ticketing`) for the type's staff role in the subject's scope: "Could not reach {recipient} about {subject}", with the channels tried and the errors. Due in 1 working day. With no holder of the role in the scope, the task goes to the company admin. When a give-up task for the same subject and recipient is still open, the new give-up is added to it as a comment, not as a second task |

| Type group (requester) | Default reaction |
|---|---|
| Urgent types of any module | `staff_task` + in-app alert to the role |

§5.4, precedence: "Each type gets its default from exactly one row, chosen in this order:
(1) a type marked statutory takes the *Statutory notices* row, whatever its group or
urgency; (2) OTP and one-time security codes take their row, and urgency adds nothing (no
staff task ever); (3) otherwise the row of the group the module names in `register_type`.
Urgency is not a group: for a type with urgency `urgent`, the *Urgent types* row adds
`staff_task` (last, if the group's reaction does not already end with it) and an in-app
alert to the type's staff role to the group's reaction. A type with urgency `urgent` and no
group takes the *Urgent types* row alone; a type with neither a group nor urgency takes
`staff_task`."

#### Edge cases decided
- **Due time:** give-up time + 1 working day at the same clock time, Asia/Dubai. Give-up on Friday
  16:00 → due Monday 16:00.
- **No holder of the role in scope:** the task goes to the company admin.
- **Content:** recipient display name and masked address (last 4 digits or first letter of the
  mailbox), channels, errors; no message body (it may contain amounts or health data).
- **Same subject and recipient** with an open give-up task: the new
  give-up is added as a comment, no second task.

#### Must not
- Must not copy the message body into the task.

#### Done when
- [ ] Given a give-up on Thursday 2026-11-05 10:00 for type `ticket.update` (staff role:
      maintenance coordinator) about ticket TKT-2026-0042 in "Tower A", the coordinator of Tower A
      has a task "Could not reach Sara M. about TKT-2026-0042" listing email and SMS errors, due
      Friday 2026-11-06 10:00.
- [ ] A give-up on Friday 2026-11-06 16:00 is due Monday 2026-11-09 16:00.
- [ ] Given type `ticket.emergency` registered with urgency `urgent` and staff role maintenance
      coordinator, a give-up for it gives the coordinator of Tower A the task and an in-app
      alert; the same give-up for `ticket.update` (urgency `normal`) gives the task and no
      alert.
- [ ] A second give-up for the same subject and recipient adds a comment to the open task.

#### Out of scope
Other reactions (BL-NTF-03 to BL-NTF-05).

---

### BL-NTF-03 — wl-notify: send the message to another contact of the same party

**Labels:** `module:wl-notify` `area:negative-flow` `priority:medium` `size:s` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-NTF-01, BL-NTF-02 · **Blocks:** BL-ACC-24, BL-NTF-06
**Review items:** X-20

#### Goal
The `alternative_recipient` reaction queues the same message to another contact of the same party
with the same purpose (the company's other finance contact, the second guardian). If there is
none, it falls back to `staff_task`.

#### Surfaces
- **System:** the recipient rule is re-run without the contacts already tried, the same
  message is queued once to the primary of the rest, and `staff_task` runs once for the
  chain when there is none or it also gives up.

#### Files to read
- `docs/modules/wl-notify.md` §5.1 step 2 (recipient resolution), §5.4 (`alternative_recipient`,
  rows for invoices and guardian messages)
- `docs/modules/wl-parties.md` §3 (*Relationship*: `contact_of`, `guardian_of`)

#### Terms
- **Same purpose:** resolved by the same recipient rule as the original (e.g. party contacts with
  portal role `finance_contact`, or guardians through `guardian_of`).

#### Spec excerpt (authoritative for this issue)

| Reaction | What happens |
|---|---|
| `alternative_recipient` | The same message is queued to another contact of the same party with the same purpose (e.g. the second guardian, the company's other finance contact), then `staff_task` if there is none. One alternative only: the recipient rule is re-run without the contacts already tried, `invalid` or suppressed, and the primary of the rest is chosen; if the alternative also gives up, `staff_task` runs once for the chain |

| Type group (requester) | Default reaction |
|---|---|
| Invoices, statements, payment reminders (`wl-invoicing`, `wl-payments`) | `mark_contact_invalid` + `alternative_recipient`; then `staff_task` to the collector role |
| Guardian messages (10, 05 dependants) | `alternative_recipient` (other guardian), then `staff_task` |

#### Edge cases decided
- **Which contact:** the recipient rule is re-run; contacts already tried, `invalid` or suppressed
  are excluded; the primary of the remaining ones is chosen. One alternative
  only.
- **The alternative also gives up:** no further alternative; `staff_task` runs (once for the
  chain).
- **Consent and quiet hours** apply to the alternative as to any message.
- **Language and channel** are chosen for the new recipient (their preference).

#### Must not
- Must not send to a contact outside the same party (or its guardians for a dependant).
- Must not loop through alternatives indefinitely.

#### Done when
- [ ] Given company "Blue Dunes LLC" with finance contacts Ali (primary) and Mona, when the
      overdue notice to Ali gives up, the same notice is queued to Mona.
- [ ] If Mona's message also gives up, the collector role gets one task.
- [ ] Given a student with one guardian only, a give-up creates a task at once.

#### Out of scope
Marking the first contact invalid (BL-NTF-04).

---

### BL-NTF-04 — wl-notify: mark a contact point invalid after a hard failure

**Labels:** `module:wl-notify` `area:negative-flow` `priority:medium` `size:s` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-NTF-01 · **Blocks:** BL-ACC-24, BL-NTF-06
**Review items:** X-20

#### Goal
On a hard bounce, an invalid number or an unknown WhatsApp account, the contact point is marked
`invalid`, the party's record shows "contact needs updating", and staff who open the party are
asked to fix it. This reaction is always combined with another one for non-marketing types.

#### Surfaces
- **Back office:** the party page shows "contact needs updating" and staff who open the
  party are asked to fix it; editing or verifying the address sets it valid; saving a
  non-marketing type with only `mark_contact_invalid` is refused.
- **System:** a hard failure marks the contact point `invalid` and emits
  `ContactPointInvalidated`; soft failures leave it `valid`.

#### Files to read
- `docs/modules/wl-notify.md` §3 (*Contact point* status fields), §5.4 (`mark_contact_invalid`),
  §6 (event `ContactPointInvalidated`)

#### Terms
- **Hard failure:** an error that will not succeed on retry: email hard bounce, number not
  valid, number not on WhatsApp.

#### Spec excerpt (authoritative for this issue)

*Contact point* fields added: status (`valid` / `invalid`), invalid since, invalid reason.

| Reaction | What happens |
|---|---|
| `mark_contact_invalid` | On a hard bounce, invalid number or unknown WhatsApp account, the contact point is marked `invalid` and the party's record shows "contact needs updating"; staff who open the party are asked to fix it. Editing or verifying the address sets it `valid` again and clears invalid since and reason. Always combined with another reaction for non-marketing types |

| Event | When | Payload |
|---|---|---|
| `ContactPointInvalidated` | a contact point is marked `invalid` | contact point, party, channel, reason |

#### Edge cases decided
- **Soft failures** (timeout, provider down, mailbox full): the contact stays `valid`.
- **Becoming valid again:** editing the address, or verifying it, sets
  `valid` and clears invalid since and reason.
- **Configured alone on a non-marketing type:** the configuration is refused; `staff_task` must be
  added.
- **The contact point is the only primary:** it stays primary but `invalid`; resolvers skip it.
- **Suppression** for hard bounces (base behaviour of `fail`) still applies.

#### Must not
- Must not delete the contact point.
- Must not mark a contact invalid on a soft failure.

#### Done when
- [ ] Given email `ali@bluedunes.ae` returns a hard bounce on a give-up, the contact point is
      `invalid` with reason `hard_bounce`, `ContactPointInvalidated` is emitted, and the party
      page shows "contact needs updating".
- [ ] A timeout error leaves it `valid`.
- [ ] Editing the address to `ali.r@bluedunes.ae` sets it `valid`.
- [ ] Saving a non-marketing type with only `mark_contact_invalid` is refused.

#### Out of scope
Opt-outs (base §5.2).

---

### BL-NTF-05 — wl-notify: hold the deadline of a statutory notice until proof of delivery

**Labels:** `module:wl-notify` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-NTF-01, BL-NTF-02 · **Blocks:** BL-ACC-04, BL-ACC-24, BL-NTF-06, BL-PRP-03, BL-PRP-09, BL-PRP-14, BL-PRP-27
**Review items:** X-20, 01-N5-b, 01-N3-b, 04-N5-c

#### Goal
A notice with legal effect (notice of changed terms, formal payment demand, deadline notice)
that could not be delivered must not count as given. The `statutory_notice` reaction gives the
responsible staff a task to deliver it by another means and record the proof. Until proof is
recorded, the subject shows "notice not delivered" and any deadline that depends on the notice is
not started.

#### Surfaces
- **Back office:** the responsible staff get a task to deliver the notice by another means;
  the subject (e.g. the lease) shows "notice not delivered" until they record proof (method,
  delivery date, note, attachment) in a form offering only the vertical's methods.
- **System:** `Notifier::notice_status(subject, type)` returns `delivered`, `proof_recorded`
  or `not_delivered`, so requesters start deadlines only from a real delivery date.

#### Files to read
- `docs/modules/wl-notify.md` §3 (statutory flag), §5.4 (`statutory_notice`, row "Statutory
  notices"), §6 (delivery log queries)
- `docs/verticals/01-property.md` §7.2 (`PRP.RUL.09`, `PRP.RUL.10`) as consumers

#### Terms
- **Proof of delivery:** what shows the notice reached the party: courier receipt, registered
  mail receipt, signed hand-delivery note, or a note of a call.
- **Dependent deadline:** a date the requester computes from the notice (e.g. demand period of
  30 days from delivery).

#### Spec excerpt (authoritative for this issue)

| Reaction | What happens |
|---|---|
| `statutory_notice` | For statutory types: `staff_task` to the responsible staff, due in 1 working day, to deliver the notice by another means (courier, registered mail, hand delivery, a call with a note) and record the proof on the subject. Until proof is recorded, the subject shows "notice not delivered", and any deadline that depends on the notice being given is not treated as started |

| Type group (requester) | Default reaction |
|---|---|
| Statutory notices (04 deadline and document-request notices to clients; 01 notices of changed terms, a landlord's notice to end a lease, formal payment demands, the formal notice on a bounced cheque; other final demands) | `mark_contact_invalid` + `statutory_notice` |

"The verticals mark their statutory types and may change a type's reaction. Which notices are
statutory, and what proof of delivery counts for them, is set by the vertical."

Service: delivery log queries ("Was the overdue notice delivered?");
`Notifier::notice_status(subject, type)` returns `delivered` (with the provider delivery time),
`proof_recorded` (with the delivery date entered in the proof) or `not_delivered`. Requesters
start a deadline from the delivery time or the proof's delivery date, never from the time the
proof was typed.

#### Edge cases decided
- **Notice status service:** `Notifier::notice_status(subject, type)`
  returns `delivered` (with the provider delivery time), `proof_recorded` (with the delivery date
  entered in the proof) or `not_delivered`. Requesters start their deadline from the delivery
  time or the proof's delivery date, never from the time the proof was typed.
- **Proof record:** method (courier, registered mail, hand delivery, call), delivery date, by,
  note, attachment (required except for a call); stored on the subject through `DocumentStore`.
- **Proof methods accepted** are set per type by the vertical; the form offers only those.
- **Proof date before the give-up:** refused (the notice was not delivered then).
- **Several statutory notices** on one subject: each has its own status.

#### Must not
- Must not let a requester treat a given-up statutory notice as delivered.
- Must not start a dependent deadline from the date the proof was entered.

#### Done when
- [ ] Given a statutory `lease.changed_terms_notice` to tenant "Sara M." that gives up on
      2026-09-01, the property manager has a task due 2026-09-02, the lease shows "notice not
      delivered", and `notice_status` returns `not_delivered`.
- [ ] When the manager records hand delivery on 2026-09-03 with a signed note, the banner clears
      and `notice_status` returns `proof_recorded` with date 2026-09-03.
- [ ] A proof without an attachment for method courier is refused.
- [ ] A delivered statutory notice returns `delivered` with the provider time and creates no task.

#### Out of scope
Which types are statutory and how each vertical computes its deadline (vertical epics, e.g.
`Epic 01 Property: PRP.RUL.09`).

---

### BL-NTF-06 — wl-notify: seed the default give-up reaction per type group

**Labels:** `module:wl-notify` `area:negative-flow` `priority:medium` `size:s` `surface:portal` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-NTF-01, BL-NTF-02, BL-NTF-03, BL-NTF-04, BL-NTF-05 · **Blocks:** BL-ACC-24, BL-EDU-17, BL-LOG-25, BL-MPW-20
**Review items:** X-20

#### Goal
Every notification type registered by a module gets a default give-up reaction from its group,
so no type is left with none. Verticals may change a type's reaction and mark statutory types.
OTP codes never create staff work: the user sees the error at once and may pick another channel.

#### Surfaces
- **Portal:** a user whose OTP SMS gives up sees "code could not be sent; try email" on the
  login screen at once and may choose another channel.
- **System:** every registered type gets its default reaction from one row of the table
  (statutory first, then OTP, then its group, urgency adding `staff_task` and an alert); OTP
  never creates a task.

#### Files to read
- `docs/modules/wl-notify.md` §5.4 (default table and last paragraph), §6 (`register_type`)

#### Terms
- **Type group:** the row of the default table a type belongs to, named by the module in
  `register_type` (invoices, bookings, portal and signatures, guardian, security notices to an
  employee, OTP, marketing and tracking). *Statutory* comes from the type's statutory flag and
  *urgent* from its urgency field, not from the group (see precedence below).

#### Spec excerpt (authoritative for this issue)

| Type group (requester) | Default reaction |
|---|---|
| Statutory notices (04 deadline and document-request notices to clients; 01 notices of changed terms, a landlord's notice to end a lease, formal payment demands, the formal notice on a bounced cheque; other final demands) | `mark_contact_invalid` + `statutory_notice` |
| Invoices, statements, payment reminders (`wl-invoicing`, `wl-payments`) | `mark_contact_invalid` + `alternative_recipient`; then `staff_task` to the collector role |
| Booking confirmations and reminders (`wl-scheduling`) | `mark_contact_invalid`; the booking shows "reminder not delivered" to the front desk; `staff_task` only for urgent types |
| Portal invitations, document requests, signature requests (`wl-portal`, `wl-documents`) | `mark_contact_invalid` + `staff_task` to the request's owner |
| Guardian messages (10, 05 dependants) | `alternative_recipient` (other guardian), then `staff_task` |
| Urgent types of any module | `staff_task` + in-app alert to the role |
| Security notices to an employee (`wl-hr` pay account change SMS, §5.7 there) | `mark_contact_invalid` + `staff_task` to HR (`employees.manage` in the employee's scope) to reach the worker by another way; no `alternative_recipient` (only the number on record before the change may be used). wl-hr keeps the change in cooling until HR records the worker reached (wl-hr §4 *Pay account*) |
| OTP and one-time security codes | `none` to staff; the user sees the error on screen at once and may choose another channel |
| Marketing, delivery tracking | `mark_contact_invalid` (marketing) or `none` |

§5.4, precedence: "Each type gets its default from exactly one row, chosen in this order:
(1) a type marked statutory takes the *Statutory notices* row, whatever its group or
urgency; (2) OTP and one-time security codes take their row, and urgency adds nothing (no
staff task ever); (3) otherwise the row of the group the module names in `register_type`.
Urgency is not a group: for a type with urgency `urgent`, the *Urgent types* row adds
`staff_task` (last, if the group's reaction does not already end with it) and an in-app
alert to the type's staff role to the group's reaction. A type with urgency `urgent` and no
group takes the *Urgent types* row alone; a type with neither a group nor urgency takes
`staff_task`."

#### Edge cases decided
- **Type registered without a group** (and not urgent): `staff_task`, the safest reaction.
- **Booking "reminder not delivered":** wl-notify exposes it through the delivery log query; the
  scheduling module shows it (`Epic wl-scheduling`).
- **Vertical override** replaces the whole reaction set of a type; the BL-NTF-04 combination rule
  still applies.
- **OTP failure** is shown synchronously to the requesting screen within the same request.
- **Pay account change SMS** (wl-hr): group "security notices to an employee", not "OTP and
  one-time security codes": the give-up creates an HR task and no alternative recipient.

#### Must not
- Must not create staff tasks for OTP give-ups.

#### Done when
- [ ] A new type registered by wl-invoicing in group "invoices" has `mark_contact_invalid` +
      `alternative_recipient` with collector role, without manual setup.
- [ ] 01's configuration marks `lease.changed_terms_notice` statutory and it gets
      `mark_contact_invalid` + `statutory_notice`.
- [ ] An OTP SMS that gives up shows "code could not be sent; try email" on the login screen and
      creates no task.
- [ ] A type without a group gets `staff_task`.
- [ ] An urgent guardian type (group guardian, urgency `urgent`) gets `alternative_recipient`,
      then `staff_task` with an in-app alert: one task, not two.
- [ ] An urgent booking reminder gets `mark_contact_invalid` + "reminder not delivered" +
      `staff_task` with an in-app alert; a normal booking reminder gets no task.
- [ ] A type marked statutory and urgent gets only `mark_contact_invalid` + `statutory_notice`.
- [ ] An OTP type marked urgent still creates no task.
- [ ] When wl-hr's pay account change SMS gives up, HR in the employee's scope gets a task, no
      other contact is messaged, and `MessageGaveUp` is emitted for wl-hr.

#### Out of scope
How the booking page shows "reminder not delivered" (`Epic wl-scheduling`).

---

### BL-NTF-07 — wl-notify: send a marketing broadcast to a segment, with approval above a size (`NTF.ACL.03`)

**Labels:** `module:wl-notify` `area:sme-gap` `priority:medium` `size:m` `surface:back-office` `surface:public` `surface:system` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** BL-SAL-19 (the campaign record that requests the broadcast) · Base: wl-notify Message lifecycle · Base: wl-parties consents (`PTY.CNS.01`) · Base: wl-kernel approvals · **Blocks:** BL-NTF-08, BL-NTF-09, BL-SAL-25, BL-AUT-18, BL-CLN-15, BL-EDU-17, BL-FNB-17
**Review items:** G4 (SME problem map 32, 35)

#### Goal
A retention campaign in wl-sales sends one marketing message to a segment of customers; in
the MVP nobody can start a marketing broadcast outside a campaign, so the campaign's guards
(05's advertising permit) and its report always apply. wl-notify resolves the recipients at
send time, keeps only those with marketing consent for the channel and no opt-out or
suppression, adds the opt-out, and records what was excluded and why. A large broadcast needs
a second person's approval.

#### Surfaces
- **Back office:** the campaign requester sees the estimated recipients; above 200 a second
  holder of `notifications.marketing.send` approves or rejects with a reason, seeing
  template, channel, estimate and segment rules but no names; the broadcast shows excluded
  counts by reason and the results.
- **Public page:** each marketing email carries an unsubscribe link to the unsubscribe page,
  which withdraws the email marketing consent.
- **System:** at send time `start` resolves the segment again, keeps only consented,
  reachable, unblocked recipients (guardians for minors), adds the opt-out, handles `STOP`
  replies and emits `BroadcastCompleted` once.

#### Files to read
- `docs/modules/wl-notify.md` §2 (paragraph *Marketing broadcasts*), §3 *Broadcast*, §4.3,
  §5.2, §5.3, §6 (`Notifier::broadcast`, `BroadcastCompleted`, `NTF.ACL.01`, `NTF.ACL.03`,
  `NTF.ACL.04`), §7 rows on broadcasts
- `docs/modules/wl-sales.md` §6 `Segments`

#### Terms
- **Broadcast:** one message of one type to a list of recipients, sent once.
- **Recipient source:** a wl-sales segment (`members(segment, at)`) or a filtered party list.
- **Excluded counts:** how many were left out per reason; names are not shown.
- `AD-` **sender ID:** the registered SMS sender ID used for promotions.

#### Spec excerpt (authoritative for this issue)

| Entity | Description | Important fields |
|---|---|---|
| Broadcast | One message of one marketing or manual type to a list of recipients | requester `Subject` (a wl-sales campaign in the MVP), notification type, template, channel, recipient source (a wl-sales segment; a filtered party list, Later), send at, recipients resolved, excluded counts by reason (no consent, opted out, suppressed, no contact point, frequency cap, blocked party, expired, broadcast stopped), requested by (a user, never the system: for a campaign, the user who last set its retention fields), approved by, cancel reason (rejected, cancelled, approval expired, requester guard failed), stopped by and at, results (sent, delivered, failed, opted out), status |

### 4.3 Broadcast

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `pending_approval` | warning | no | Waiting for a second person (`NTF.ACL.03`); expires 24 h after send at (`NTF.RUL.03`) |
| `scheduled` | info | no | Approved or below the limit; waits for its send time |
| `sending` | info | no | Recipients resolved; messages being queued |
| `completed` | success | yes | Every message queued; results keep updating from delivery events |
| `cancelled` | neutral | yes | Rejected, cancelled or expired before sending, or its requester's guards failed at start |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `request` | — | pending_approval, scheduled | the requester module on behalf of a user (marketing types: an `active` wl-sales campaign, `NTF.ACL.04`); `notifications.broadcast.send` (other types, Later) | type of category `marketing` or a manual type; template active (WhatsApp: `approved`) in every language used; SMS: an `AD-` sender ID configured; the requesting user named | estimated recipients shown (the members of the recipient source now who would pass consent, opt-out, suppression, contact point and blocked-party checks for the channel; the frequency cap is not estimated); `pending_approval` when the estimate is above the limit of `NTF.ACL.03`, otherwise `scheduled` |
| `approve` | pending_approval | scheduled | `notifications.marketing.send` | approver ≠ requester (`NTF.ACL.03`) | — |
| `reject` | pending_approval | cancelled | `notifications.marketing.send` | reason; rejecter ≠ requester | requester told; event `BroadcastCancelled` |
| `start` | scheduled | sending | system (send at reached) | the requester `Subject` still allows the send (`NTF.ACL.04`: campaign `active`, the vertical's guards pass), otherwise `cancel` with reason *requester guard failed* | recipients resolved now; blocked, archived and merged parties left out; consent, opt-out, suppression and `NTF.RUL.02` applied per recipient; excluded counts recorded |
| `complete` | sending | completed | system | every recipient has a message or an exclusion | event `BroadcastCompleted` |
| `cancel` | pending_approval, scheduled | cancelled | requester, `notifications.marketing.send`, or system (the requester `Subject` ended, e.g. the campaign `end`) | — | event `BroadcastCancelled` |
| `expire` | pending_approval | cancelled | system (`NTF.RUL.03`) | not approved N hours after send at | reason *approval expired*; requester told; event `BroadcastCancelled` |

Messages already sent by a broadcast are not recalled; a message deferred by the send window
is still sent when the window opens, unless the broadcast was stopped or the message went
stale (`NTF.RUL.03`). A broadcast is sent once: running it again is a new broadcast, and a
recipient gets at most one message per broadcast.

**Stop:** a holder of `notifications.marketing.send` may stop a broadcast that is `sending`, or
`completed` with messages still `queued` or `deferred` (wrong segment, wrong text). It is an
action, not a transition: those messages are suppressed with reason *broadcast stopped*, the
state stays, stopped by and at are recorded, and `BroadcastStopped` tells the requester.

### 5.3 Marketing broadcasts (MVP for 05, 06, 08, 10) and manual bulk messages (Later)

1. A retention campaign in wl-sales is activated (`SAL.LOY.01`) and requests the broadcast
   for the user who set its retention fields (`NTF.ACL.04`). The broadcast names a marketing
   template, the channel and the send time. Staff cannot start a marketing broadcast directly
   from a segment in the MVP.
2. Above the recipient limit, a second holder of `notifications.marketing.send` approves it
   (`NTF.ACL.03`). Not approved within 24 hours after the send time, it expires
   (`NTF.RUL.03`) and the campaign shows *not sent*.
3. At the send time the segment is evaluated again. Only recipients with marketing consent
   for that channel (`PTY.CNS.01`, `NTF.ACL.01`), no opt-out or suppression and a valid
   contact point get a message; `NTF.RUL.02` defers messages outside the marketing send
   window and skips recipients over the frequency cap. Blocked, archived and merged parties
   are left out. For a minor (`PTY.REL.01`), the message goes to the guardian, with the
   guardian's consent. The excluded counts are kept by reason; the names of excluded
   recipients are not listed to the requester. A marketing message not sent within 48 hours
   of the start (provider down, long deferral) is suppressed as *expired* (`NTF.RUL.03`), so no
   stale promotion goes out.
4. Every marketing message identifies the client's brand and carries an opt-out: an
   unsubscribe link in email, the reply keyword `STOP` in SMS and WhatsApp (the same keyword
   in Arabic messages; any letter case is accepted). An opt-out withdraws the marketing
   consent for that channel (§5.2).
5. Results (sent, delivered, failed, opted out) are kept on the broadcast and returned to
   the requester through `BroadcastCompleted` and the delivery log. A broadcast can be
   stopped while messages remain unsent (§4.3 *Stop*).

Manual bulk messages of other types (e.g. all tenants of a building, all guardians of a
batch) use the same broadcast with `notifications.broadcast.send` and stay Later. The same
consent, suppression and quiet-hour rules apply.

| ID | Rule | Pattern | Parameter (default) |
|---|---|---|---|
| `NTF.ACL.01` | Marketing messages go only to recipients with marketing consent for that channel; exporting recipient lists needs a sensitive-export permission and is audited. Generalises `FNB.ACL.10` and `CLN.ACL.11` | `SEN` | — |
| `NTF.ACL.03` | A marketing broadcast to more than N recipients needs approval by a second holder of `notifications.marketing.send`; the requester cannot approve it | `LIM`, `SOD` | N = 200 |
| `NTF.ACL.04` | A marketing broadcast is requested only by an `active` wl-sales campaign, on behalf of the user who last set its segment, template, channel or send time (that user is the requester for `NTF.ACL.03`, also when the campaign activates on its start date); the campaign's guards, including the vertical's (05: `CLN.ACL.13`), are checked at `request` and again at `start` | `STA`, `SOD` | — |

| Setting | Default |
|---|---|
| Broadcast approval above (`NTF.ACL.03`) | 200 recipients |
| Broadcast approval expiry; stale marketing messages (`NTF.RUL.03`) | 24 hours after send at; 48 hours after start |

#### Edge cases decided
- **Estimate vs actual:** the estimate counts the segment members at `request` who would
  pass the consent, opt-out, suppression, contact point and blocked-party checks for the
  channel (not the raw segment size; the frequency cap is not estimated). The approval limit
  uses this estimate; a segment that grows past the limit by send time is not sent back for
  approval.
- **Requester in tests:** this issue calls `Notifier::broadcast` with an `active` campaign
  record of BL-SAL-19 as requester `Subject` and a named user; the campaign that requests the
  broadcast on its own `activate` is BL-SAL-25. The vertical guard at `start` is tested with a
  guard stub that fails (05's real permit guard is the 05 epic).
- **Opt-out keyword:** `STOP` in SMS and WhatsApp, in English and Arabic messages alike; a
  reply in any letter case opts out.
- **Approver's scope:** the approver sees the template, channel, estimate and segment rules, not
  the recipient names.
- **Same person twice:** a party with two contact points on the channel gets one message (the
  primary).
- **Consent withdrawn between `request` and `start`:** excluded (consent is checked at `start`
  and again by the message `send` guard).
- **WhatsApp template paused at send time:** the message falls back as any message does;
  marketing types have no fallback to another channel unless consent exists there.
- **Idempotency:** `start` runs once per broadcast; a retry after a crash skips recipients that
  already have a message of this broadcast.
- **Approval pending at send time:** stays `pending_approval`; it starts when approved if the
  send time passed, otherwise at the send time; not approved 24 hours after the send time, it
  expires (BL-NTF-09).
- **Requester** (`NTF.ACL.04`): a marketing `request` without an `active` campaign as requester
  `Subject` and a named user is refused; the named user is the one who last set the campaign's
  segment, template, channel or send time, also when the campaign activated on its start date.
- **Guards at `start`:** the campaign must still be `active` and its vertical guards pass (05:
  permit valid); otherwise `cancel` with reason *requester guard failed*, requester told.
- **Blocked, archived, merged parties:** left out at `start`, counted as *blocked party*.
- **Minors:** a recipient party that is a minor (`PTY.REL.01`) is replaced by the guardian,
  whose marketing consent for the channel is checked; with no consenting guardian, excluded
  *no consent*.

#### Must not
- Must not send to a recipient without marketing consent for that channel.
- Must not let the requester approve their own broadcast.
- Must not send a marketing SMS without the `AD-` sender ID.
- Must not list excluded recipients' names to the requester.
- Must not accept a marketing broadcast from anything but an active campaign.

#### Done when
- [ ] Given a segment of 150 with 100 SMS-consented, requester A's broadcast shows an
      estimate of 100, goes straight to `scheduled` and at send time queues 100 messages;
      excluded: 50 *no consent*.
- [ ] Given a segment of 300 with 180 SMS-consented, the estimate is 180 and the broadcast is
      `scheduled` without approval.
- [ ] Given an estimate of 350 (limit 200), A's request is `pending_approval`; A cannot approve;
      B approves and it becomes `scheduled`.
- [ ] B rejects with a reason: `cancelled`, A told, `BroadcastCancelled` emitted.
- [ ] A party who opts out after the request and before the send gets no message.
- [ ] Each marketing email has an unsubscribe link; each marketing SMS uses the `AD-` sender
      and carries "STOP" as the opt-out keyword, in English and Arabic messages; a reply
      "stop" withdraws the SMS marketing consent.
- [ ] A crash after 40 of 100 messages and a retry queues the other 60 only;
      `BroadcastCompleted` is emitted once.
- [ ] A marketing broadcast requested by staff without a campaign is refused.
- [ ] A request naming active campaign R (BL-SAL-19) and user A as requester records A as
      requested by; a request naming an `ended` campaign is refused.
- [ ] R's vertical guard (a stub standing in for 05's permit) fails before the send time; at
      `start` the broadcast is cancelled with *requester guard failed* and A is told.
- [ ] A student's message goes to the guardian with SMS marketing consent; a student with no
      consenting guardian is excluded *no consent*.

#### Out of scope
Send window and frequency cap (BL-NTF-08); approval expiry, stale messages and stopping a
broadcast (BL-NTF-09); segments and the retention campaign (BL-SAL-25); manual bulk messages
of other types and broadcasts started by staff (Later).

---

### BL-NTF-08 — wl-notify: apply the marketing send window and frequency cap (`NTF.RUL.02`)

**Labels:** `module:wl-notify` `area:sme-gap` `priority:medium` `size:s` `surface:back-office` `surface:system` · **Milestone:** Launch 05 · **Size:** s
**Depends on:** BL-NTF-07 · Base: wl-notify Message lifecycle (`defer`, `suppress`) · **Blocks:** BL-NTF-09, BL-SAL-25, BL-AUT-18, BL-CLN-15, BL-EDU-17, BL-FNB-17
**Review items:** G4 (SME problem map 32)
**Progress:** parts 1 to 4 of 5 are on main: send window rules, quiet hours and the module clock (5c726b4); send-window settings and page (8a7ff70, test `send_window_page.rs`); defer and release (ed9e72a, test `send_deferral.rs`); frequency cap under a party lock (4b1bc68, test `cap_send.rs`); test fixes e3d1155, 864d513. Part 5 of 5: the six points of "Done when" as one test each in `send_window_acceptance.rs` (page, notifier, send and release jobs, two broadcasts through `run_due`).

#### Goal
Marketing messages go out only between 07:00 and 21:00 Asia/Dubai (the TDRA hours for
promotional SMS, applied to every channel), and no customer gets more than the cap of
marketing messages in 7 days, however many campaigns run.

#### Surfaces
- **Back office:** the admin sets the marketing send window per channel; widening the SMS
  window past 07:00-21:00 is refused on save.
- **System:** `NTF.RUL.02` defers marketing messages outside the window and suppresses those
  over the cap of 2 per 7 days, checked at `send` under a lock per recipient party.

#### Files to read
- `docs/modules/wl-notify.md` §4.1 rows `defer` and `suppress`, §6 `NTF.RUL.02`, §7 row on
  the send window, §10 *TDRA SMS rules*

#### Terms
- **Send window:** the hours in which a marketing message may be sent.
- **Frequency cap:** the largest number of marketing messages a recipient may get in 7 days.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `defer` | queued | deferred | system | normal urgency, external recipient, inside quiet hours; or category `marketing` outside the marketing send window (`NTF.RUL.02`) | scheduled for the end of quiet hours or the next opening of the window |
| `suppress` | queued, deferred | suppressed | system | a send guard fails permanently; the recipient is over the marketing frequency cap (`NTF.RUL.02`); a marketing message of a broadcast not sent within the stale limit, or of a stopped broadcast (`NTF.RUL.03`, §4.3) | reason recorded; event `MessageSuppressed` |

| ID | Rule | When | Then | Parameter (default) |
|---|---|---|---|---|
| `NTF.RUL.02` | Marketing send window and frequency cap | a message of category `marketing` is queued | outside the send window (Asia/Dubai) → `defer` to the next opening; when the recipient already got the cap of marketing messages in the last 7 days (across all broadcasts of the client) → `suppress` with reason *frequency cap*. The cap is checked at `send` under a lock per recipient party, and the message is `sent` under the same lock, so two broadcasts cannot both pass it | window 07:00–21:00, every channel; cap 2 per 7 days; on |

| Setting | Default |
|---|---|
| Marketing send window; frequency cap (`NTF.RUL.02`) | 07:00–21:00 on every channel, set per channel: SMS may be narrowed but not widened (§10); email and WhatsApp may be narrowed or widened; 2 messages per 7 days |
| Quiet hours for external recipients | 21:00–08:00 Asia/Dubai, normal urgency only |

- **TDRA SMS rules:** sender IDs must be registered through a licensed provider. Promotional
  SMS carry the `AD-` prefix in the sender ID, may only be sent 07:00–21:00, need consent
  and must offer an opt-out. Transactional SMS (OTP, reminders, receipts) need no prefix.
  `NTF.RUL.02` applies the 07:00–21:00 window to every marketing channel; for SMS it may not
  be widened.
  Registration with e& and du takes about 5–25 working days ✔ [S2] (vendor sources).

#### Edge cases decided
- **Counting:** messages of category `marketing` that reached `sent` or later in the last 7 × 24
  hours, any channel, any broadcast; suppressed and abandoned ones do not count.
- **Order:** the cap is checked when the message would be sent (after a `defer`), not when
  queued.
- **Concurrency:** the cap check and the `send` run under a lock per recipient party, so two
  broadcasts releasing messages to the same party at once cannot both pass the cap; the
  second sees the first as `sent`.
- **Window settings:** one window per channel. A client may narrow any window; widening the SMS
  window past 07:00–21:00 is refused on save; email and WhatsApp windows may be widened.
- **Day boundary:** a message deferred at 21:00 is released at 07:00 the next day.
- **Transactional, reminder and security types:** not affected.

#### Must not
- Must not send a marketing message outside the window.
- Must not count a suppressed message towards the cap.

#### Done when
- [x] A marketing SMS queued at 22:15 is deferred and sent at 07:00 the next day.
- [x] Given recipient R with 2 marketing messages sent in the last 7 days, a third is suppressed
      with reason *frequency cap*.
- [x] Setting the SMS window to 06:00–22:00 is refused; 09:00–20:00 is accepted.
- [x] Given recipient R with 1 marketing message in the last 7 days, two broadcasts release a
      message to R at the same moment: one is sent, the other is suppressed *frequency cap*.
- [x] Setting the email window to 06:00–23:00 is accepted.
- [x] A normal-urgency booking reminder (category *reminder*) to a customer, queued at 22:15, is
      deferred by quiet hours and sent at 08:00 (not 07:00); it is never suppressed *frequency
      cap* and does not count towards the cap.

#### Out of scope
Quiet hours for other categories (existing); the broadcast itself (BL-NTF-07).

---

### BL-NTF-09 — wl-notify: expire unapproved broadcasts, drop stale marketing messages and stop a broadcast (`NTF.RUL.03`)

**Labels:** `module:wl-notify` `area:sme-gap` `priority:medium` `size:s` `surface:back-office` `surface:system` · **Milestone:** Launch 05 · **Size:** s
**Depends on:** BL-NTF-07 · BL-NTF-08 · **Blocks:** BL-SAL-25, BL-AUT-18, BL-CLN-15, BL-EDU-17, BL-FNB-17
**Review items:** GR-G4-17, GR-G4-18, GR-G4-19 (gap review of G4)

#### Goal
A promotion goes out on time or not at all. A broadcast nobody approved is cancelled a day
after its send time instead of going out days later; messages stuck behind a provider outage
are dropped after 48 hours; and a marketer who sees a wrong segment or text can stop the
messages not yet sent.

#### Surfaces
- **Back office:** a holder of `notifications.marketing.send` stops a `sending` broadcast,
  or a `completed` one with messages still queued or deferred (otherwise "nothing left to
  stop").
- **System:** `NTF.RUL.03` checks every 15 minutes, cancels broadcasts not approved 24 hours
  after send at, suppresses marketing messages still unsent 48 hours after start, and emits
  `BroadcastCancelled` or `BroadcastStopped` to the requester.

#### Files to read
- `docs/modules/wl-notify.md` §3 *Broadcast*; §4.1 row `suppress`; §4.3 rows `expire`,
  `cancel` and the *Stop* paragraph; §5.3 steps 2, 3 and 5; §6 `NTF.RUL.03`,
  `BroadcastCancelled`, `BroadcastStopped`; §7 row on broadcast expiry

#### Terms
- **Stale message:** a marketing message of a broadcast still `queued` or `deferred` 48 hours
  after the broadcast started.
- **Stop:** an action on a broadcast, not a state; it suppresses what is not yet sent.

#### Spec excerpt (authoritative for this issue)

| `suppress` | queued, deferred | suppressed | system | a send guard fails permanently; the recipient is over the marketing frequency cap (`NTF.RUL.02`); a marketing message of a broadcast not sent within the stale limit, or of a stopped broadcast (`NTF.RUL.03`, §4.3) | reason recorded; event `MessageSuppressed` |

| `expire` | pending_approval | cancelled | system (`NTF.RUL.03`) | not approved N hours after send at | reason *approval expired*; requester told; event `BroadcastCancelled` |

**Stop:** a holder of `notifications.marketing.send` may stop a broadcast that is `sending`, or
`completed` with messages still `queued` or `deferred` (wrong segment, wrong text). It is an
action, not a transition: those messages are suppressed with reason *broadcast stopped*, the
state stays, stopped by and at are recorded, and `BroadcastStopped` tells the requester.

| ID | Rule | When | Then | Parameter (default) |
|---|---|---|---|---|
| `NTF.RUL.03` | Stale broadcasts | a broadcast is `pending_approval` N hours after its send at; or a marketing message of a broadcast is still `queued` or `deferred` M hours after the broadcast's `start` | `expire` the broadcast (reason *approval expired*; requester told); `suppress` the message with reason *expired* (counted in the broadcast's excluded counts) | N = 24 h; M = 48 h; on |

| Event | When | Payload |
|---|---|---|
| `BroadcastCompleted` / `BroadcastCancelled` | a broadcast has queued all messages, or is cancelled, rejected or expired | broadcast id, requester subject, recipients, excluded counts by reason, reason (cancelled: rejected, cancelled, approval expired, requester guard failed) |
| `BroadcastStopped` | a broadcast is stopped (§4.3) | broadcast id, requester subject, stopped by, messages suppressed |

| Setting | Default |
|---|---|
| Broadcast approval expiry; stale marketing messages (`NTF.RUL.03`) | 24 hours after send at; 48 hours after start |

#### Edge cases decided
- **Expiry check:** every 15 minutes; a broadcast approved before the check runs is not
  expired. An approval and the expiry at the same moment: the first committed wins.
- **Stale limit:** measured from `start`, not from each message's deferral; a message
  released at 07:00 after a 10-hour deferral is not stale.
- **Stop on `completed`:** allowed only while some message of the broadcast is still `queued`
  or `deferred`; otherwise refused with *nothing left to stop*.
- **Messages already `sent`:** never recalled; results keep updating.
- **Requester told:** on expiry (reason *approval expired*) and on stop (messages
  suppressed); wl-sales shows *not sent* or *stopped* on the campaign.
- **Counts:** stale and stopped messages are excluded counts (*expired*, *broadcast stopped*)
  and do not count towards the frequency cap.

#### Must not
- Must not send a broadcast that was not approved within the expiry limit.
- Must not recall or retry a message suppressed as *expired* or *broadcast stopped*.

#### Done when
- [ ] A broadcast with send at 10:00 still `pending_approval` at 10:00 the next day becomes
      `cancelled` with *approval expired* at the first 15-minute check at or after 10:00 (by
      10:15 at the latest), the requester is told and `BroadcastCancelled` is emitted once.
- [ ] With the SMS provider down from `start` for 50 hours, the 100 queued messages are
      suppressed *expired* at 48 hours and none is sent when the provider recovers.
- [ ] Stopping a `sending` broadcast with 30 messages sent and 70 queued suppresses the 70 with
      *broadcast stopped* and emits `BroadcastStopped`; the 30 stay sent.
- [ ] Stopping a `completed` broadcast with 15 messages deferred until 07:00 suppresses the 15.
- [ ] A user without `notifications.marketing.send` cannot stop a broadcast.

#### Out of scope
The broadcast itself (BL-NTF-07); send window and frequency cap (BL-NTF-08); the campaign
view in wl-sales (BL-SAL-25).

---

### BL-NTF-10 — wl-notify: send stored documents as e-mail attachments through the kernel notifier

**Labels:** `module:wl-notify` `area:negative-flow` `priority:high` `size:s` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** Base: wl-notify Message lifecycle · **Blocks:** BL-PRP-31, BL-PAY-30
**Review items:** 01 MEDIUM "owner statement lifecycle and disputes" (review §6); follow-up of BL-PRP-19, Done-when point 3

#### Goal
A notification sometimes has to carry a document: the owner statement PDF, a signed contract, a handover
receipt. The kernel `NotifyRequest` and the wl-notify message have no attachments, so a module could only
queue a mail itself and bypass opt-outs, quiet hours, suppression and the message log. This issue adds
attachments to the request (references to stored documents) and sends them with the e-mail channel.

#### Surfaces
- **System:** `NotifyRequest` takes attachment references; wl-notify attaches the files to e-mails, logs
  them on the message and leaves other channels unchanged.

#### Files to read
- `docs/modules/wl-notify.md` §3 *Message*, §4.1, §5.1, §9 (row "Email with templates and attachments")
- `crates/wl-kernel/src/notify.rs` (`NotifyRequest`), `crates/wl-notify/src/send.rs`, `message.rs`

#### Terms
- **Attachment reference:** a document id from `DocumentStore` and the file name to show.
- **Channel:** e-mail, SMS, WhatsApp, in-app.

#### Spec excerpt (authoritative for this issue)
| Entity | Description | Important fields |
|---|---|---|
| Message | One message to one recipient on one channel | type, subject (`Subject` it is about), recipient, channel, language, rendered content, provider, provider message id, cost (if reported), status, scheduled for, attempts |

| Need | Renox provides | Gap |
|---|---|---|
| Email with templates and attachments | mail, `rx-mail-*` tags | Arabic RTL email layout → R1 |

#### Edge cases decided
- **Message field:** the message stores its attachment references; the file is read at send time, so a
  deferred message (quiet hours) sends the stored version.
- **Size:** total over 10 MB: the e-mail carries a signed download link instead (wl-portal
  `SignedLinks`, 30 days), and the message says so.
- **SMS and WhatsApp:** no attachment; the template may include a signed link.
- **Opt-out, suppression and give-up:** unchanged; a message that gives up runs its reaction as usual.
- **Document not readable at send time:** the message `fail`s with reason *attachment missing*.

#### Must not
- Send any mail outside the message pipeline.
- Attach a document the recipient's party may not see (the caller passes only documents of the subject).

#### Done when
- [ ] Given a stored PDF of 180 KB, a request of type `owner_statement.published` with it as attachment
      sends one e-mail with the file named "Owner statement 2026-10.pdf", and the message log lists it.
- [ ] The same request in quiet hours sends the attachment at the end of quiet hours.
- [ ] An attachment of 12 MB is replaced by a signed link.
- [ ] An opted-out recipient gets nothing, as without an attachment.
- [ ] An SMS for the same type carries no file.

#### Out of scope
Generating the document (BL-DOC-11); the Arabic RTL e-mail layout (R1).
