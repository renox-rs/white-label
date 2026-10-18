# wl-kernel

The base every `wl-*` module and vertical app stands on
([spec](../../docs/modules/wl-kernel.md)). It turns one Renox binary into one client's
branded product.

The host reads the client's `client.toml` and applies the kernel to the app:

```
use renox::prelude::*;
use wl_kernel::ClientConfig;

let config = ClientConfig::parse(r#"
    [client]
    name = "Acme Properties LLC"
    app = "property"

    [modules]
    enabled = ["parties", "invoicing"]
"#)?;
let app = wl_kernel::apply(App::new(), config)?;
# let _ = app;
# Ok::<(), wl_kernel::ConfigError>(())
```

`apply` fails, and the boot stops, when a module needs one that is not enabled, when a
brand colour is not readable (the message names the nearest colour that is), or when the
file is malformed.

What is here so far:

| Part | Spec | Module |
|---|---|---|
| `client.toml` and the module manifest | §3, §5.1 | `config`, `manifest` |
| Branding on Renox `Theme::from_brand` | §2 `PLT.ORG.01` | `branding` |
| `Subject` references | §2 | `subject` |
| Legal entities, scope links, issuer snapshots (`PLT.ENT.01`) | §5.6, decision 0003 | `entities` |
| Document numbering on Renox `Sequence` | §5.5 | `numbering` |
| State machines by name: `SubjectMachine`, `back_to_<state>` and `<action>_approved` (always `system`) | §4.1, WORKFLOW §3.4 | `machines` |
| The party contract: `PartyDirectory` (`NoParties` when `parties` is off), `PartyStatus`, party events, the `party` series, `contacts` (`PartyContacts`), `billing_identity` (`BillingIdentity`) | ARCHITECTURE §5, wl-parties §6 | `parties` |
| The invoicing contract: `Invoicer` (`NoInvoicer` when `invoicing` is off), `InvoiceDraft`, `InvoiceSummary`, `VatCategory`, invoice events, the `invoice` series | ARCHITECTURE §5, wl-invoicing §6 | `invoicing` |
| The notice contract: `Notifier` (`NoNotifier` when `notify` is off), `register_type`, `NotifyRequest`, message events | ARCHITECTURE §5, wl-notify §6 | `notify` |
| The document guard contract: `DocumentGuard` (`NoDocuments` when `documents` is off), `BlockEntry`, `BlockSubjects` by transition, `BlockRefusal`, `today()` in Asia/Dubai, document events | ARCHITECTURE §5, wl-documents §5.6, §6 | `documents` |
| The ticket contract: `TaskCreator` (`NoTickets` when `ticketing` is off), `OpenTicket`, `TicketRef`, `Assignee`, ticket events, the `ticket` series | §8, wl-ticketing §6 | `tickets` |
| The block-effect contract: `BlockEffect` (pause / continue / review) per agreement kind, the 12 built-in kinds, `AgreementKind` + `AgreementSource`, `BlockEffectOverride`, `effect_for`, `announces_pause` and `suppressed_while_blocked` | ARCHITECTURE §5, wl-parties §5.6, §7 | `block_effects` |
| `AutomationFailed`, the event of a failed rule run | §4 | `automation` |
| The business calendar: `BusinessCalendar` (Asia/Dubai, Sat–Sun weekend, 09:00–18:00, `[calendar]` holidays), `add_working` and `add_working_days` (same clock time, no working hours) | §6 | `calendar` |
| Scope of a subject: `scopes::scope_of` (its machine, then a resolver from `scopes::register`, then global) and `COMPANY_ADMIN_ROLE` (`"admin"`, the fallback holder of a task) | §4.1 | `scopes` |

After `rnx migrate`, `rnx wl:install` creates the legal entities `client.toml` lists and
the database lacks; it never overwrites an entity changed in the admin. `rnx db:seed`
runs it too.

The page `/legal-entities` needs `legal_entities.view`. The app overrides it with
`resources/views/wl-kernel/legal_entities.html`, and translates the texts (`wl_kernel.*`)
in `resources/lang/{locale}.json`.

Other modules read parties only through `wl_kernel::parties::directory(&state)` and listen
to `PartyCreated`, `PartyBlocked` and `PartyUnblocked`. Summaries and events carry the
display block reason: `on_hold` (`NEUTRAL_BLOCK_REASON`) when the real one is restricted.

Other modules draft, read and pay invoices only through
`wl_kernel::invoicing::invoicer(&state)`: `draft(db, InvoiceDraft::new(party, source).line(..))`
gives a `draft` invoice, `find(db, id)` an `InvoiceSummary` (number, status, party, legal
entity, currency, gross total, balance, due date), and `allocate(state, id, source, amount)`
settles part of it from a payment or credit note. Money is `i64` fils, quantities are
thousandths, and each line has a `VatCategory` (`standard` 5%; `zero_rated`, `exempt`,
`out_of_scope` and `reverse_charge` carry no VAT). When `invoicing` is off, `NoInvoicer`
drafts and finds nothing, so callers hide the feature, and refuses an allocation (422).
Modules listen to `InvoiceIssued`, `InvoiceOverdue`, `InvoicePaid` and `InvoiceWrittenOff`.
Tax invoice numbers come from `INVOICE_SERIES` (`INV-ACME-2026-0001`) or, with exactly one
active legal entity, `INVOICE_SERIES_SINGLE` (`INV-2026-0001`): gapless, per entity, reset
yearly. `PartyDirectory::billing_identity` gives the legal names, TRN, billing address and
billing e-mail an invoice copies at issue; the default (and `NoParties`) knows none.

Every notice goes through one contract. A module registers its notification types in
`register` with `wl_kernel::notify::register_type(NotificationTypeDef::new(..))` (statutory
flag, give-up reactions, staff role told, channels in order, default texts with `{{ name }}`
placeholders), then sends with
`notify::notifier(&state).notify(&state, NotifyRequest::new(type, subject).to_party(p).to_user(u).data(..))`.
A recipient is a staff user (by user id), a party (by `PartyId`, reached through
`PartyDirectory::contacts`) or a bare address. `.subject_label("TKT-2026-0042")` names the
subject in staff-facing text; `MessageGaveUp` carries it with `gave_up_at`. `wl-notify` provides the real notifier and
emits `MessageFailed`, `MessageSuppressed` and `MessageGaveUp`. When `notify` is off,
`NoNotifier` still gives each user recipient a Renox database notification (the bell) with
the type's text, skips party and address recipients, and returns no message ids.

Other modules open tickets (tasks) only through `wl_kernel::tickets::task_creator(&state)`:
`open(&state, OpenTicket::new(subject, "general", title).description(..).requester_party(p))`
gives a `TicketRef` (id and number, e.g. `TKT-2026-0001`), and fails for an unknown category
key. The channel is `system` unless set, and `.assign_to(Assignee::user(id))` (or
`Assignee::party(vendor)`) has `wl-ticketing` run the first `assign` as the system, so the
SLA starts. `.due_at(..)` sets a due time and `.source_key(..)` a key to find the open ticket
by: `find_open(&state, key)` gives its `TicketRef` while it is not resolved, closed or
cancelled, and `comment(&state, ticket, TaskComment::internal(text))` adds a comment (both
default to nothing, as `NoTickets` does). Internal staff tasks use `TASK_CATEGORY` (`task`);
`ticket_path(id)` is the ticket page, `/tickets/{id}`. `OpenTicket` is built only with its
builder, so new optional fields never break a caller. When `ticketing` is off, `NoTickets` opens nothing and returns `None`, so callers
carry on without a task. `wl-ticketing` emits `TicketAssigned` (assignee, previous assignee
and reason on a reassignment, priority, due times) and `TicketClosed` (resolution working
minutes, within SLA). Ticket numbers come from `TICKET_SERIES` (`TKT-2026-0001`): reset
yearly, gaps allowed.

A module that owns a guarded transition (an entry of a document kind's `blocks`, e.g.
`deployment.activate`) registers, in `register`, a `BlockSubjects` provider with
`wl_kernel::documents::register_block_subjects("deployment.activate", provider)`: it maps a
record to the document subjects and returns none when the record does not match the entry's
condition. Before `record.transition(..)` it calls
`documents::guard(&state).check(&state, "deployment.activate", id, documents::today())`.
`Ok(Ok(CheckOutcome))` lets the transition run; `Ok(Err(BlockRefusal))` refuses it, and
`refusal.into_error()` is a 422 with one line per failing document ("Residence visa expired
on 2026-11-30; renew: PRO; override: not allowed"); `Err(..)` is a failure (e.g. no provider
for an entry), never a *yes*. `is_valid(db, subject, kind, on_date)` is the guard
`document_valid`, and `blocks_for(db, guarded)` lists the entries with their mode, override
mode and condition. `today()` is the Asia/Dubai date of `renox::db::now()`, so tests that
travel move it; a document is valid through its expiry date. `wl-documents` provides the guard
and emits `DocumentUploaded`, `DocumentAccepted`, `DocumentRejected` and
`DocumentSuperseded`; when `documents` is off, `NoDocuments` guards nothing and answers valid.

An entry in `flag` mode does not refuse: `check` checks the `block` entries first (a failing
one refuses and wins), then returns `CheckOutcome { flags }` with one `FlagFailure` per
failing flag-mode document. A module calls
`documents::guarded_transition(&state, "attendance_record.accept", id, on_date, || async { .. })`
instead of `check`: it runs `check`, the closure (the `record.transition(..)` call) and then
`record_flags`, so a refusal runs nothing and a failing closure writes no flag. `on_date` is
the record's business date. `flags_for(db, record_type, id)` lists a record's `RecordFlag`s
(label, reason, dates, review state, reviewer and note), `open_flag_counts(db, record_type,
&ids)` counts the unreviewed ones for list badges, and `review_flag(db, id, reviewer, note)`
marks one reviewed (a note is required, once only; a flag is never removed). Without
`wl-documents` the closure just runs, nothing is flagged and the lists are empty.

Planned work is warned about, never refused: after a module saved a run, a deployment or a
batch of sessions it calls
`documents::guard(&state).check_planned(&state, "deployment.activate", &items)` with one
`PlannedItem { record_id, on_date }` per item. An item that spans several days passes only its
start date; the guard does not know about end dates. Only `block` entries of `guarded` count.
`Ok(Ok(Some(PlannedWarning)))` has one `PlannedWarningLine` per subject and kind with the sorted
dates it fails on; `message()` is "Ravi S.: Residence visa expires 2026-12-15; planned:
2026-12-20" per line and `toast()` is a persistent warning toast the module returns with its
redirect. A `BlockSubjects` provider can override `label` (the subject's name, else
`<type> #<id>`) and `bookable` (default false): for a bookable slot (`booking.book`,
`booking.reschedule`, `booking.reassign`) `check_planned` gives `Ok(Err(PlannedError::BookableSlot))`,
because a booking is refused through `is_valid` (`SCH.ACL.04`). Without `wl-documents` it returns
`Ok(Ok(None))`.

Look-ahead: a module that owns planned work behind a guarded transition can also register a
`FutureWork` provider with `documents::register_future_work("ticket.resolve", provider)`
(optional: an entry without one is skipped). `planned_from(state, entry, subject, from)` returns
the `PlannedRecord`s (`record`, `date`, `label`, optional `scope`) planned on or after `from`.
`wl-documents` passes `from` = **expiry date + 1 day**: a document is valid through its expiry
date, so a record planned on that date is never listed. It groups the records by guarded
transition as `AffectedGroup`s in the `DocumentExpiring` event (document, subject, kind,
`expires_on`, `days_left`, `offset`, `affected`); the event changes no planned item. A vertical
registers a `ReminderSchedule` per document kind with
`documents::register_reminder_schedule(kind, schedule)`: `scopes(state, subject)` names the
scopes whose staff get the reminder when neither the subject nor the affected records have one;
`None` means every scope. Without `wl-documents` neither registry is read.

Working time comes from one calendar: `wl_kernel::calendar::calendar(&state)` gives the
client's `BusinessCalendar` (the default one with no holidays when the kernel is not
mounted). Asia/Dubai is UTC+04:00 all year; the weekend is Saturday and Sunday, a working
day runs 09:00–18:00, and the public holidays are listed in `client.toml`:

```toml
[calendar]
holidays = ["2026-12-02", "2026-12-03"]
```

`add_working(from, minutes)` gives the instant that many working minutes later: a start
outside working hours counts from the next opening, and a result exactly on the closing time
stays there (Tue 09:00 + 27 h = Thu 18:00). `working_minutes_between(a, b)` is its inverse.
`is_working_day(date)` and `next_working_day(date)` step whole days, `local(at)` shows an
instant on the Dubai clock, `from_local(naive)` reads one, and `today()` is the same date as
`documents::today()`.

Migrations are SQLite only: each client runs one SQLite database
([ARCHITECTURE.md](../../docs/ARCHITECTURE.md) §1).
