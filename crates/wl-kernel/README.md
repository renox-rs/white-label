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
| `AutomationFailed`, the event of a failed rule run | §4 | `automation` |

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
`PartyDirectory::contacts`) or a bare address. `wl-notify` provides the real notifier and
emits `MessageFailed`, `MessageSuppressed` and `MessageGaveUp`. When `notify` is off,
`NoNotifier` still gives each user recipient a Renox database notification (the bell) with
the type's text, skips party and address recipients, and returns no message ids.

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

Migrations are SQLite only: each client runs one SQLite database
([ARCHITECTURE.md](../../docs/ARCHITECTURE.md) §1).
