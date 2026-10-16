# Epic: wl-notify — negative flows

> Status: draft · Last updated: 2026-10-15 · Spec version: same commit as this file

What happens when a message gives up: after all retries and fallbacks, `give_up` emits
`MessageGaveUp` with full detail and `NTF.RUL.01` runs the notification type's give-up reaction,
so no module is left assuming a message arrived. The reactions are a staff task, an alternative
recipient, marking the contact point invalid, and the statutory-notice task for notices with
legal effect. Review item X-20 (and the 01 and 04 statutory notices that depend on it).

| ID | Title | Priority | Size | Milestone | Depends on |
|---|---|---|---|---|---|
| BL-NTF-01 | Run the notification type's give-up reaction when a message gives up | medium | m | Build 1 | Base: wl-notify Message lifecycle |
| BL-NTF-02 | Open a staff task when a message could not be delivered | medium | s | Build 1 | BL-NTF-01 |
| BL-NTF-03 | Send the message to another contact of the same party | medium | s | Build 1 | BL-NTF-01, BL-NTF-02 |
| BL-NTF-04 | Mark a contact point invalid after a hard failure | medium | s | Build 1 | BL-NTF-01 |
| BL-NTF-05 | Hold the deadline of a statutory notice until proof of delivery | high | m | Build 1 | BL-NTF-01, BL-NTF-02 |
| BL-NTF-06 | Seed the default give-up reaction per type group | medium | s | Build 1 | BL-NTF-01 to BL-NTF-05 |

---

### BL-NTF-01 — wl-notify: run the notification type's give-up reaction when a message gives up

**Labels:** `module:wl-notify` `area:negative-flow` `priority:medium` `size:m` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: wl-notify Message lifecycle (`send`, `fail`, `retry`, `fall_back`) · **Blocks:** BL-NTF-02, BL-NTF-03, BL-NTF-04, BL-NTF-05, BL-NTF-06, BL-ACC-24, BL-CLN-15
**Review items:** X-20

#### Goal
Each notification type declares whether it is statutory, which give-up reaction it uses and which
staff role is told. When the last message of a send ends `abandoned` through `give_up`,
`MessageGaveUp` is emitted with the channels tried and errors, and `NTF.RUL.01` runs the type's
reaction. Requester modules may also listen, but the default reaction always runs.

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

**Labels:** `module:wl-notify` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-NTF-01 · Base: wl-ticketing Ticket lifecycle (`TaskCreator`), wl-kernel business calendar · **Blocks:** BL-NTF-03, BL-NTF-05, BL-ACC-24, BL-CLN-14, BL-CLN-15, BL-NTF-06
**Review items:** X-20

#### Goal
The `staff_task` reaction gives the type's staff role in the subject's scope a task "Could not
reach {recipient} about {subject}", with the channels tried and the errors, due in 1 working day.
For urgent types the role also gets an in-app alert.

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

**Labels:** `module:wl-notify` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-NTF-01, BL-NTF-02 · **Blocks:** BL-ACC-24, BL-NTF-06
**Review items:** X-20

#### Goal
The `alternative_recipient` reaction queues the same message to another contact of the same party
with the same purpose (the company's other finance contact, the second guardian). If there is
none, it falls back to `staff_task`.

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

**Labels:** `module:wl-notify` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-NTF-01 · **Blocks:** BL-ACC-24, BL-NTF-06
**Review items:** X-20

#### Goal
On a hard bounce, an invalid number or an unknown WhatsApp account, the contact point is marked
`invalid`, the party's record shows "contact needs updating", and staff who open the party are
asked to fix it. This reaction is always combined with another one for non-marketing types.

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

**Labels:** `module:wl-notify` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-NTF-01, BL-NTF-02 · **Blocks:** BL-ACC-04, BL-ACC-24, BL-NTF-06, BL-PRP-03, BL-PRP-09, BL-PRP-14, BL-PRP-27
**Review items:** X-20, 01-N5-b, 01-N3-b, 04-N5-c

#### Goal
A notice with legal effect (notice of changed terms, formal payment demand, deadline notice)
that could not be delivered must not count as given. The `statutory_notice` reaction gives the
responsible staff a task to deliver it by another means and record the proof. Until proof is
recorded, the subject shows "notice not delivered" and any deadline that depends on the notice is
not started.

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

**Labels:** `module:wl-notify` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-NTF-01, BL-NTF-02, BL-NTF-03, BL-NTF-04, BL-NTF-05 · **Blocks:** BL-ACC-24, BL-EDU-17, BL-LOG-25, BL-MPW-20
**Review items:** X-20

#### Goal
Every notification type registered by a module gets a default give-up reaction from its group,
so no type is left with none. Verticals may change a type's reaction and mark statutory types.
OTP codes never create staff work: the user sees the error at once and may pick another channel.

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
