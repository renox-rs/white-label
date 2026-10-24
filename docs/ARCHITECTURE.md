# White-Label UAE SME Platform — Architecture

> Status: **approved; F1–F10 built** ([decision 0004](decisions/0004-start-f1-build.md), [0005](decisions/0005-phases-follow-vertical-numbers.md)). The
> code follows the layout in [§7](#7-repository-layout) (directories marked "planned" do not exist yet: `operator/` and `deploy/rollout`).

## 1. Agreed decisions

| # | Topic | Decision |
|---|---|---|
| 1 | Business model | **Hosted per client**: 1 binary + 1 SQLite database per client, setup fee + monthly subscription |
| 2 | Customisation | **Configuration + extension crate**. No per-client forks |
| 3 | App order | **Follows the vertical numbers**: F1 Property (proves the core), F2 Construction, F3 Logistics … F10 EdTech. See [decision 0005](decisions/0005-phases-follow-vertical-numbers.md) |
| 4 | Platform | **Responsive web + PWA** (Renox/HTMX). No native apps or offline mode yet |
| 5 | Database | **SQLite per client** + Litestream replication to S3-compatible storage on a backup VPS in the UAE (another provider) |
| 6 | Library location | **Two layers**: generic components go into `renox` (OSS), business modules stay in `white-label` (private) |

Principles that follow from these decisions:

- **One client = one process = one database.** Clients are isolated at the deployment level,
  so business modules do **not** need a `tenant_id` on every table. This keeps every module
  simpler.
- **Rule of three.** Code becomes a generic module once two apps use it and a third is
  expected to. Until then it stays in the vertical app.
- **Modules do not know each other directly.** Business modules talk through traits, events
  and polymorphic references (see [§5](#5-module-contract)), so each one can be switched on or
  off per client.

## 2. Layers

```
┌───────────────────────────────────────────────────────────────┐
│ 5. Client deployment    clients/acme-properties/  (client.toml, .env, branding)
├───────────────────────────────────────────────────────────────┤
│ 4. Client extension     small optional crate, only when configuration is not enough
├───────────────────────────────────────────────────────────────┤
│ 3. Vertical app         apps/property, apps/manpower, ... (domain-specific, ~20–30%)
├───────────────────────────────────────────────────────────────┤
│ 2. Business modules (PRIVATE, white-label repo)
│    wl-kernel · wl-parties · wl-portal · wl-documents · wl-notify ·
│    wl-invoicing · wl-payments · wl-sales · wl-ticketing · wl-delivery ·
│    wl-scheduling · wl-attendance · wl-hr · wl-inventory
├───────────────────────────────────────────────────────────────┤
│ 1. Renox (OSS)          Module/Registry, <rx-*> UI kit, blocks, admin, auth,
│                         permissions, queue, scheduler, storage, mail, audit, search
└───────────────────────────────────────────────────────────────┘
```

Beside the client stack runs **`operator/`**, our own control plane
([wl-operator](modules/wl-operator.md)): a separate Renox app on our server that provisions
clients, holds plans and entitlements, bills our clients, rolls out releases and monitors
every deployment through a signed admin API. It stores no client business data.

Dependencies only point downwards. A vertical app may use many modules. A business module may
depend on `wl-kernel` and `renox`, but **not** on another business module except through
traits defined in `wl-kernel`.

## 3. What Renox already provides (no need to build)

Based on a survey of `renox` on 2026-10-14:

| Need | Already in Renox |
|---|---|
| External module system | `Module` trait (`routes`, `migrations`, `register`). An external crate can embed its own migrations, views, components, assets, jobs, listeners and schedules (example: `renox-billing`) |
| Per-client view overrides | A file with the same name in `resources/views/` overrides a module's view |
| UI | `<rx-*>`: shell, sidebar, table, grid, form, wizard, repeater, file, date-picker, tabs, infolist, notification-bell, etc. Blocks: kanban, month-calendar, availability, history, gallery, datetime-range |
| Theming | CSS tokens (`--rx-accent`, …), `data-rx-theme`, dark mode, optional Tailwind |
| Access control | Gates, policies, `Permissions` module (roles/permissions), **per-record roles** (`has_permission_in`) |
| Infrastructure | Database-backed queue, scheduler, local/S3/R2 storage, mail, notifications (mail/database/custom channel), audit, inbound webhooks, search, import/export, charts |
| Billing | `renox-billing`: Stripe/Xendit subscriptions + a custom gateway trait (1.2: one-off payments, links, refunds, N-Genius) |
| Deployment | `rnx build` → one binary; `rnx make:deploy` (Docker, systemd, Litestream) |

## 4. Gaps to fill

### 4a. Contributions to Renox (OSS, useful to everyone)

**Status (2026-10-18): all 25 are delivered in Renox 1.2.0** (issues #28–#52 closed, plus #69 and
#71 split off from them; checked against the 1.2.0 changelog, guides and source). The table keeps
the original request; each row ends with *Delivered* and the real API name. Where 1.2.0 differs
from the request, the row says so and [§4a-1](#4a-1-what-1-2-0-did-not-cover) lists what is left
for us. Each gap was an issue in [renox-rs/renox](https://github.com/renox-rs/renox/issues) (#28–#52,
created 2026-10-16 after checking Renox 1.1.1; the R number links to it). Only R3 needs a new
crate (`renox-pdf`); the others extend renox-core, renox-blocks, renox-admin, renox-billing,
renox-oauth or renox-cli. R19 turned out to exist for jobs (`Middleware::rate_limited`); its
issue covers only Renox's own notification and mail sends.

| # | Feature | Reason |
|---|---|---|
| [R1](https://github.com/renox-rs/renox/issues/28) | **RTL + Arabic support** (automatic `dir="rtl"` per locale, CSS logical properties throughout `renox-ui.css`, Arabic font) | Required in the UAE. Today there is a single RTL rule **Delivered:** `app.dir`, `App::rtl_locales`, Noto Sans Arabic, logical CSS (#28). |
| [R2](https://github.com/renox-rs/renox/issues/29) | **Runtime settings store** (`settings` table, typed accessors, cache, admin page) | Branding, TRN, invoice number format and enabled modules must change without a redeploy **Delivered:** opt-in `Settings` module, `SettingsGroup`, `rnx settings:*`, `Admin::settings` (#29). The enabled-module manifest stays in `wl-kernel`. |
| [R3](https://github.com/renox-rs/renox/issues/30) | **PDF generation** (recommended: the `typst` crate, pure Rust, supports Arabic shaping, stays inside one binary) | Invoices, contracts, payslips, receipts **Delivered:** new crate `renox-pdf` (Typst, Inter + Noto Sans Arabic embedded, `PDF_MAX_CONCURRENT`) (#30). |
| [R4](https://github.com/renox-rs/renox/issues/31) | **Admin hook for modules**: `Registry::admin_resource(...)` | Today the app has to collect every resource into one `Admin`, so a module cannot register its own **Delivered under another name:** `Registry::contribute` and `renox_admin::contribute`; `Admin::named` for two panels (#31, #70). |
| [R5](https://github.com/renox-rs/renox/issues/32) | **New components**: `rx-signature-pad`, `rx-camera-capture`, `rx-geolocate` (check-in), `rx-status-stepper`/timeline, `rx-week-calendar` (resource schedule), `rx-barcode-scan` (camera or keyboard-wedge scanner), `rx-rating` (feedback stars) | Needed for e-POD, attendance, ticketing and booking. Generic, fits `renox-blocks` **Delivered** in `renox-blocks` (21 blocks now): `signature_pad`, `camera_capture`, `geolocate`, `barcode_scan`, `rating`, `status_stepper`, `week_calendar` (#53–#55). Geolocation and camera are off in the default Permissions-Policy: the app enables them with `App::permissions_policy`. |
| [R6](https://github.com/renox-rs/renox/issues/33) | **PWA**: manifest + minimal service worker (`rnx make:pwa`) | Field workers can install the app on their phones **Delivered:** `App::pwa(Pwa::new(…))`, `pwa_for` per host, `<rx-pwa/>`, `rnx make:pwa` (#33). |
| [R7](https://github.com/renox-rs/renox/issues/34) | **Package scaffold**: `rnx make:package <name>` (a `Module` crate with migrations, views and tests) | Speeds up writing `wl-*` crates and third-party modules **Delivered:** `rnx make:package` (#34). |
| [R8](https://github.com/renox-rs/renox/issues/35) | **Starter from git**: `rnx new acme --starter git+https://…` | Creates a new client from the white-label template **Delivered under another name:** `rnx new acme --from <dir or git+url>` with questions in `renox-template.toml`; the flag is `--from`, not `--starter` (#35). |
| [R9](https://github.com/renox-rs/renox/issues/36) | **Brand-token generator**: `Theme::from_brand(accent, accent_2)` → light and dark tokens with a WCAG contrast guard; brand chosen per request domain (several brands in one deployment) | Per-client branding without forking the kit. See [DESIGN-SYSTEM.md](DESIGN-SYSTEM.md) §4 **Delivered:** `Theme::from_brand(&Brand::new(…).second(…))` with `ContrastError`, `App::theme_for` per host or tenant, `Mail::theme` (#36). |
| [R10](https://github.com/renox-rs/renox/issues/37) | **Navigation contributions from modules**: `Registry::sidebar_link(section, link, permission)` for the back office and the portal menu | Each `wl-*` module adds its own staff and portal navigation **Delivered under another name:** `NavLink` with `App::nav` / `Registry::nav` (`.permission`, `.gate`, `.group`, `.order`, `.badge`), `menu="staff"` on `<rx-sidebar>`; no `sidebar_link` (#37). |
| [R11](https://github.com/renox-rs/renox/issues/38) | **Outbound webhooks and signed requests**: event subscriptions, HMAC signing, queued delivery with retry, delivery log; signed server-to-server requests (operator admin API) | Client-specific integrations (n8n, Zapier, Make) live outside the platform. See [WORKFLOW.md](WORKFLOW.md) §5 **Delivered:** `Endpoint::create`, `emit_in` (Standard Webhooks, retries, auto-disable), `SignedRequest<T>`, `rnx webhook:*` (#38). |
| [R12](https://github.com/renox-rs/renox/issues/39) | **State machines for models**: declared states and transitions with permission, guards and effects; transition history; available actions per user; generated diagram | One engine for every status in every module. See [WORKFLOW.md](WORKFLOW.md) §3 **Delivered:** `StateMachine`, `Machine`, `Transition`, `state_transitions` history, `rnx state:diagram`, and the `TransitionInterceptor` hook for approvals (#39). |
| [R13](https://github.com/renox-rs/renox/issues/40) | **Gapless sequences**: per-series counters that never skip or repeat, safe under concurrency | Invoice, credit note and receipt numbers (tax rules) **Delivered:** `Sequence::next` inside the caller's transaction, `Period`, `NumberFormat` (#40). |
| [R14](https://github.com/renox-rs/renox/issues/41) | **Custom fields**: JSON-backed fields with a schema, rendered in forms, grids and filters | Per-client fields without migrations ([wl-kernel](modules/wl-kernel.md)) **Delivered:** `CustomField`, `#[model(custom_fields)]`, `<rx-custom-fields>`, admin resource; `sensitive()` fields sealed with `APP_KEY` (#41, #71). |
| [R15](https://github.com/renox-rs/renox/issues/42) | **Passwordless login**: magic link, one-time codes by SMS or WhatsApp (✔ signed, expiring URLs already exist in Renox: `state.signed_url`, `ValidSignature`) | Portal users and recipients without passwords ([wl-portal](modules/wl-portal.md)) **Delivered:** `Auth::magic_link()`, `login_codes(LoginCodes::new()…)` by mail, SMS or WhatsApp, `OneTimeCode` (#42). |
| [R16](https://github.com/renox-rs/renox/issues/43) | **Upload pipeline hooks**: malware scanning, image processing (resize, thumbnails, HEIC → JPG), per-file encryption | Identity documents, site photos, medical files ([wl-documents](modules/wl-documents.md)) **Partly delivered:** `UploadStep` `Pipeline`, `ClamAv`, `Images` (size, orientation, thumbnails, `images` feature), `Storage::put_encrypted`. **HEIC and AVIF are not decoded**: we write an `UploadStep` that calls an external converter (#43). |
| [R17](https://github.com/renox-rs/renox/issues/44) | **renox-billing beyond subscriptions**: one-off payments, payment links, refunds; gateway credentials per deployment owner; a Network International N-Genius adapter (Stripe and N-Genius in the MVP, Telr and Checkout.com on demand) | Rent, invoices and deposits paid online ([wl-payments](modules/wl-payments.md)) **Delivered:** `Customer::charge`, `payment_link`, `refund`, payment events, keys per installation, several accounts per gateway, `Billing::ngenius()` (#44). Telr and Checkout.com stay ours through the `Gateway` trait. |
| [R18](https://github.com/renox-rs/renox/issues/45) | **Money and quantities**: currency per record (formatting, validation); decimal quantities with unit-of-measure conversion | Multi-currency and inventory ([wl-invoicing](modules/wl-invoicing.md), [wl-inventory](modules/wl-inventory.md)) **Delivered:** `renox::money::{Money, Currency, Rounding, Rate}` with `#[model(money)]`; `Quantity<SCALE>` and `Units` (#45, #69). |
| [R19](https://github.com/renox-rs/renox/issues/46) | **Rate limiting**: per provider in the queue (SMS, WhatsApp, gateways). ✔ Checked in Renox: HTTP rate limits already exist (`Routes::throttle`, named limiters keyed by user, device token or IP, `rate_limit.rs`), so API tokens are covered; the gap is only outbound limits per provider in the queue | Notifications and the client API **Delivered:** `App::channel_rate_limit`, `App::mailer_rate_limit` (#46). |
| [R20](https://github.com/renox-rs/renox/issues/47) | **Geo helpers**: point-in-radius/polygon geofence check; map picker and map view components (tile provider and licence to choose) | Attendance geofences, dispatch maps **Delivered:** `renox::geo::{Point, Circle, Polygon}`, blocks `map` and `map_picker`; no default tile server, set `BLOCKS_MAP_TILES` (#47). |
| [R21](https://github.com/renox-rs/renox/issues/48) | **Calendar and print output**: `.ics` files for bookings; 80 mm thermal receipt layout | Booking confirmations, counter sales **Delivered:** `renox::calendar::{Ics, Event}`, `Mail::attach_ics`, block `receipt` for 80 and 58 mm (#48). |
| [R22](https://github.com/renox-rs/renox/issues/49) | **Arabic text search**: normalisation and Arabic–Latin transliteration for search and duplicate matching | Party names in both scripts ([wl-parties](modules/wl-parties.md)) **Delivered:** `#[model(search, normalize = "arabic")]`, `renox::text::{normalize_arabic, transliterate, match_key}` (#49). |
| [R23](https://github.com/renox-rs/renox/issues/50) | **Import beyond CSV**: Excel files, a dry run that validates without writing, a rollback window after commit (✔ Renox import today: CSV, row validation, savepoints, `all_or_nothing`, templates) | Client data migration ([wl-kernel](modules/wl-kernel.md) `PLT.IMP.01`) **Delivered:** `Import::xlsx`, `from_upload`, `.dry_run()`, `.undoable(window)`, `rnx import:prune` (#50). |
| [R24](https://github.com/renox-rs/renox/issues/51) | **Enterprise sign-in for renox-oauth**: a Microsoft Entra ID provider; restricting Google sign-in to the client's Workspace domain | Staff single sign-on (`PLT.SSO.01`, Later) **Delivered:** `Microsoft` provider, `Google::hosted_domain`, `OAuth::allow_domains`, `OAuthRefused` event (#51). |
| [R25](https://github.com/renox-rs/renox/issues/52) | **Offline queue for field work** (Later): actions recorded without a connection (attendance, proof of delivery, POS sales, line output) are stored on the device and sent in order when online, with conflict handling | Field PWA in 02, 03, 06, 07, 08; MVP is online only with paper fallback ([UX.md](UX.md)) **Partly delivered:** `Routes::offline()`, `Offline` extractor (`recorded_at`, `skew_exceeded`), `<rx-form offline>`, `Idempotency-Key` replay in order (#52). **No merge of conflicting edits**: that rule stays in each `wl-*` module. |
| [R26](https://github.com/renox-rs/renox/issues/82) | **OAuth 2.1 authorization server**: the app as a provider other programs sign in to (authorization code + PKCE, client registration, consent page, scopes as token abilities, connected apps, revoke) | AI assistants connecting to a client's app ([decision 0006](decisions/0006-search-and-ai-agents.md)); later mobile apps and integrations |
| [R27](https://github.com/renox-rs/renox/issues/83) | **`renox-mcp`**: the app's tools and resources served over the Model Context Protocol at `/mcp`, signed in through R26, audited with the client's name | `search`, `fetch`, `related`, reports and inbox for assistants (BL-AGT-07, BL-AGT-09) |
| [R28](https://github.com/renox-rs/renox/issues/84) | **State machine transitions as tools**: opt-in transitions offered to assistants with a preview step and a confirm step, through the same guards and approvals | Agent actions (BL-AGT-08, BL-AGT-10) |

### 4a-1. What 1.2.0 did not cover

| Item | What is left | Where it lands |
|---|---|---|
| HEIC/AVIF → JPG (R16) | A custom `UploadStep` that calls a converter on the host | `wl-documents` |
| Conflicting offline edits (R25) | Renox gives idempotency, order and clock skew only; two devices editing one record need a rule per module | `wl-delivery`, `wl-attendance`, `wl-sales` |
| Telr, Checkout.com (R17) | Our own `Gateway` implementations, on demand | `wl-payments` |
| Map tiles (R20) | No default tile server; choose a provider and licence, set `BLOCKS_MAP_TILES` and `BLOCKS_MAP_ATTRIBUTION` | `apps/*` config |
| Camera and geolocation (R5) | Off in the default Permissions-Policy; enable per app | `apps/construction` (F2), `apps/logistics` (F3) |
| N-Genius disputes (R17) | renox-billing's N-Genius adapter has no disputes API and `accept_dispute` is Stripe only; chargebacks and their fee are recorded by hand | `wl-payments` §5.2a |
| Run-time targets in state machines (R12) | One fixed `to` per transition, no run by name, no replay of an approved request: `back_to_<state>` transitions and a by-name registry. **Done in F1**: `crates/wl-kernel/src/machines.rs` | `wl-kernel` §4.1, [WORKFLOW.md](WORKFLOW.md) §3.4 |
| Enabled-module manifest (R2) | `Settings` stores values only; which modules a client has is `wl-kernel` | `wl-kernel` |

Upgrading from 1.1: run `migrate` (framework migrations for imports, webhooks, state transitions,
sequences, custom fields, idempotency keys, one-time codes, settings and billing payments), pin
`renox = "1.2"` and `renox-pdf = "1.2"`, and use `<rx-shell fill>` and `<html dir="{{ app.dir }}">`
in our layouts.

### 4b. Business modules (private, `white-label`)

Planned: global search and agent access in `wl-kernel` ([decision 0006](decisions/0006-search-and-ai-agents.md)), no new crate.

| Module | Contents | First used by |
|---|---|---|
| `wl-kernel` | Enabled-module manifest, business calendar (working days, UAE public holidays, Ramadan hours), branding from settings, sequential document numbers (`INV-2026-0001`), custom fields (JSON values + a definitions table, Renox #41), cross-module traits, polymorphic `Subject` reference; legal entities and per-entity numbering, data import, global search, notes and mentions, report library, signed admin API and entitlements | all |
| `wl-parties` | Parties: companies and people (tenants, clients, employees, patients, students), contacts, addresses, TRN, Emirates ID | all |
| `wl-portal` | External-user login (magic link/OTP), branded portal layout, data scoped to the user's `Party` | all |
| `wl-documents` | Documents attached to a `Subject`, document types, **expiry dates + tiered reminders** (90/30/7 days), versions, access control, SOP library with acknowledgements | all |
| `wl-invoicing` | Invoices, credit notes, **UAE VAT 5%** (standard/zero-rated/exempt), recurring invoices, PDF, receivables ageing, supplier bills, company budget, cash-flow forecast; FTA e-invoicing adapter in a later phase | all |
| `wl-payments` | One-off payments + payment links, reconciliation against invoices, gateway trait (Stripe and Network International N-Genius in the MVP; Telr and Checkout.com on demand), post-dated cheques (PDC) | property |
| `wl-ticketing` | Tickets/work orders per `Subject`, **configurable status workflow**, priority, SLA, assignment, photos, comments | property (maintenance), manufacturing (QC), auto (warranty) |
| `wl-notify` | WhatsApp Business and SMS channels (via `App::channel`), per-language templates, recipient preferences, marketing broadcasts with consent | all |
| `wl-attendance` | GPS + geofence check-in/out, timesheets, approval, rosters | construction, manpower |
| `wl-hr` | Employee records and licences, visa cases, leave, payroll, WPS SIF export, final settlement, staff onboarding checklists | all |
| `wl-sales` | Leads, lead sources and campaigns (cost per lead, CAC, ROAS), catalogue and price lists, sales orders, POS, prepaid packages, discounts, loyalty, returns, credit limits | all (leads and campaigns); orders and POS: manufacturing, auto, F&B, clinics, EdTech |
| `wl-delivery` | Dispatch to drivers and runs, proof of delivery | logistics, auto, manufacturing, F&B |
| `wl-scheduling` | Resources (doctors, tutors, service bays), slots, bookings, reminders | clinics, EdTech, auto |
| `wl-inventory` | Items, stock per location, movements, counts, purchasing, lots, BOMs and recipes, pick-pack-dispatch, assets, supplier scorecards | construction, F&B, manufacturing, auto |

## 5. Module contract

Every `wl-*` module is an ordinary `renox::Module` that follows three extra rules.

**1. Polymorphic references, not cross-module foreign keys.** Documents, tickets and invoices
attach to "something" through `subject_type` + `subject_id`. `wl-documents` does not need to
know that `Lease` exists.

```sql
CREATE TABLE wl_documents (
  id INTEGER PRIMARY KEY,
  subject_type TEXT NOT NULL,   -- 'property.lease', 'manpower.employee'
  subject_id   INTEGER NOT NULL,
  kind         TEXT NOT NULL,   -- 'tenancy_contract', 'visa', 'emirates_id'
  expires_on   DATE,
  ...
);
```

**2. Integration through traits in `wl-kernel`.** A module calls a capability, not another
module.

```rust
// wl-kernel/src/invoicing.rs (abridged)
pub trait Invoicer: Send + Sync + 'static {
    /// A `draft` invoice from `draft`; `None` when invoicing is off.
    fn draft<'a>(&'a self, db: &'a Db, draft: InvoiceDraft)
        -> InvoiceFuture<'a, Option<InvoiceId>>;
    fn issue<'a>(&'a self, state: &'a AppState, id: InvoiceId, by: Option<&'a AuthUser>)
        -> InvoiceFuture<'a, ()>;
    fn find<'a>(&'a self, db: &'a Db, id: InvoiceId) -> InvoiceFuture<'a, Option<InvoiceSummary>>;
    // also: update_draft, discard_draft, allocate, unallocate(_all), dispute,
    // credit_in_full, credit_partly, write_off, take_credit_for_refund, ...
}

// apps/property/src/invoicing.rs: the daily task `property:invoice-charges`
// bills each due lease charge without depending on wl-invoicing's internals
let draft = InvoiceDraft::new(lease.tenant(), lease.subject())   // party, source
    .scope(Subject::new(BUILDING_SUBJECT_TYPE, lease.building_id))
    .due_date(charge.due_date)
    .line(DraftLine::new(&text, charge.amount, charge.vat_category)); // VAT per line
if let Some(id) = invoicer(state).draft(&state.db, draft).await? {
    invoicer(state).issue(state, id, None).await?;
}
```

`wl-invoicing` provides the `Invoicer` implementation in its `register()` with
`wl_kernel::invoicing::provide_invoicer(app, Invoices)`. If invoicing is disabled for a client,
`invoicer(state)` returns the no-op `NoInvoicer` (drafts return `None`, the rest refuse with a
422) and the app hides the feature.

**3. Events for reactions.** `InvoicePaid`, `DocumentExpiring` and `TicketClosed` are
dispatched as Renox events. Vertical apps and client extension crates listen to them to add
behaviour without changing the module.

**Extension points every module must offer:**

- overridable views (the Renox mechanism),
- custom fields through `wl-kernel`,
- events before and after important actions,
- small strategy traits, e.g. `VatRule`, `ReminderSchedule`, `TicketRouter`.

## 6. Module × app matrix

Replaced by [CAPABILITY-MAP.md](CAPABILITY-MAP.md): 110 shared capabilities across 14 client-side
modules, mapped to the ten verticals, with 148 vertical-specific capabilities (258 distinct).
Each module is specified in [modules/](modules/).

## 7. Repository layout

```
white-label/
├── Cargo.toml                 # workspace
├── crates/                    # wl-* business modules
│   ├── wl-kernel/
│   ├── wl-parties/
│   ├── wl-documents/
│   └── ...
├── apps/
│   ├── property/              # library crate: the property domain as a Module
│   ├── construction/, logistics/, accounting/, clinics/, fnb/, manufacturing/, auto/,
│   └── manpower/, edtech/     # one per vertical (§9), all ten built
├── host/                      # binary: reads client.toml, assembles the App from enabled modules
├── operator/                  # planned (rollout track): our control plane (wl-operator)
├── clients/
│   ├── demo-accounting/, demo-clinics/, demo-construction/, demo-fnb/, demo-logistics/,
│   ├── demo-manpower/, demo-manufacturing/   # the demo clients of F2–F7 and F9
│   ├── demo-auto/             # the F8 demo client: a parts trader with a workshop
│   ├── demo-edtech/           # the F10 demo client: a training centre
│   └── demo-properties/       # the F1 demo client
│       ├── client.toml        # app, modules, branding, locales, TRN
│       ├── resources/views/   # view overrides (optional)
│       └── ext/               # extension crate (optional)
├── deploy/                    # Dockerfile, systemd, Litestream, Caddy, runbook (README.md);
│                              # `deploy/rollout` (automatic rollout) is still planned
└── docs/
```

Example `client.toml`:

```toml
[client]
name     = "Acme Properties LLC"
app      = "property"
domain   = "portal.acmeproperties.ae"
locales  = ["en", "ar"]

[branding]
logo   = "branding/logo.svg"
accent = "#0F766E"
theme  = "classic"

[modules]
# Without the `wl-` prefix. A module whose dependency is missing stops the boot
# (`payments` needs `invoicing`; `portal` needs `parties` and `notify`).
enabled = ["parties", "portal", "documents", "invoicing", "payments", "ticketing", "notify"]

# The issuing companies (decision 0003); exactly one is the default.
[[legal_entities]]
code    = "ACME"
name_en = "Acme Properties LLC"
trn     = "100123456700003"
default = true

[invoicing]
prefix = "ACM-INV"

[ticketing]
statuses = ["open", "assigned", "in_progress", "awaiting_parts", "done"]
```

The host reads the file named by `CLIENT_CONFIG` (outside production, the demo client in
`clients/demo-properties/`). `wl_kernel::ClientConfig` parses it, and `rnx wl:install` creates
the listed legal entities after `migrate`.

The values in `client.toml` seed the **settings store** (R2) on first run. After that, the
client can change some of them through the admin.

Clients without an extension crate run the **same** `host` binary. A client with an
extension gets its own binary: `host` + the `ext/` crate linked at build time.

## 8. Operations (hosted per client)

- **Hosting:** our own VPS in the UAE, with backups on a second UAE VPS at another provider
  ([OPERATIONS.md](OPERATIONS.md) §2), so data stays in the UAE (PDPL; health data falls
  under stricter DHA/DoH rules).
- **Per client:** 1 systemd unit + 1 SQLite file + a Litestream replica. A reverse proxy
  (Caddy) handles automatic TLS for each client domain.
- **Fleet upgrades:** target: `deploy/rollout` (**not built**) builds once, then rolls out
  client by client. Migrations run on boot, followed by a health check and automatic rollback
  on failure. A canary client (the internal demo) is always updated first. Today the rollout
  is a manual checklist ([deploy/README.md](../deploy/README.md) §5-6, [OPERATIONS.md](OPERATIONS.md) §12);
  `.github/workflows/release-main.yml` builds one artifact per push to main for the public demos.
- **Cost:** one small VPS (2 vCPU/4 GB) is expected to host dozens of SME clients, given
  Renox's footprint of about 12 MiB per process. To be measured in phase F1.
- **Details:** environments, release waves, backups and restore drills, monitoring, support
  SLAs and client onboarding are in [OPERATIONS.md](OPERATIONS.md); host hardening and the
  threat model in [SECURITY.md](SECURITY.md); the control plane in
  [wl-operator](modules/wl-operator.md).

## 9. Roadmap

F1–F10 are built (all merged by 2026-10-24): each vertical has its `apps/*` crate and a demo
client in `clients/`. The rows below keep the scope each phase had; the rollout track is still
open.

| Phase | Scope | Done when |
|---|---|---|
| **F0 — Renox foundations** (**done in Renox 1.2.0, 2026-10-18**; what is left is adoption: pin 1.2, run `migrate`, build the module crates on the new APIs) | R1 RTL/Arabic, R2 settings, R3 PDF, R4 admin contributions, R7 make:package, R9 brand tokens, R10 `NavLink`, R12 state machines (with the approval interceptor from Renox #39). Also needed before Build 1 ships: R13 sequences, R14 custom fields, R15 passwordless login, R17 + R18 payments and money, R22 search, R23 import | A module crate can ship an admin page and an Arabic PDF; a transition blocked by a limit becomes an approval request |
| **F1 — Property** | `wl-kernel`, `wl-parties`, `wl-portal`, `wl-documents`, `wl-invoicing`, `wl-payments` (cheques, deposits, bank statements and refunds; no online payment gateway yet, `NoGateway` by default), `wl-ticketing`, `wl-notify` (email first), `wl-sales` (lead sources and campaigns), `wl-hr` (minimal employee records and onboarding, no payroll or visas) + `apps/property` + `host` + 1 demo client | End-to-end demo: unit → lease → VAT invoice → payment → maintenance ticket from the tenant portal |
| **F2 — Construction** | `wl-attendance` (geofenced crew attendance, timesheets by cost code), `wl-inventory` (purchasing, site stores), job costing and supplier-bill matching in `wl-invoicing`, R5 (geolocate, camera), R6 PWA, R20 maps (attendance geofences) + `apps/construction`. **Measure** new code vs reuse; refactor modules that leaked domain logic | ≥ 70% of Construction's features run on existing modules without modification |
| **F3 — Logistics** | `wl-delivery` (dispatch, runs, proof of delivery, COD) + `apps/logistics` | Last-mile run from dispatch to proof of delivery and COD settlement |
| **F4 — Accounting** (**built**: `apps/accounting`, `clients/demo-accounting`, the settings of `host/src/app/accounting_*.rs`; the demo command `accounting:demo` in `host/src/app/accounting_demo`) | `apps/accounting` on existing modules (cheapest; modules nearly complete) | Filing calendar and client engagement end to end |
| **F5 — Clinics** | `wl-scheduling` (resources, bookings, reminders) + `apps/clinics` | Appointment to invoice, with licence expiry blocks |
| **F6 — F&B** | Recipes and lots in `wl-inventory`, orders and POS in `wl-sales`, aggregator integrations + `apps/fnb` | Order to dispatch to food-safety log. Validate partner API access first |
| **F7 — Manufacturing** | BOMs, lots and quality in `wl-inventory` + `apps/manufacturing` | Work order to finished stock with landed cost |
| **F8 — Auto** (**built**: `apps/auto`, `clients/demo-auto`, the settings of `host/src/app/auto_*.rs`, the demo command `auto:demo` in `host/src/app/auto_demo/`) | Orders and POS in `wl-sales` (if not done in F6), job cards + `apps/auto`; back-orders in `wl-sales` and reservations in `wl-inventory` | Job card to invoice, insurance repair |
| **F9 — Manpower** | The rest of `wl-hr` (visas, leave, payroll, WPS, final settlement) + `apps/manpower` | Deployment to client timesheet to WPS payroll |
| **F10 — EdTech** (**built**: `apps/edtech`, `clients/demo-edtech`, the settings of `host/src/app/edtech_*.rs`; the demo command `edtech:demo` in `host/src/app/edtech_demo`) | Batches, enrolments, tutors, certificates on `wl-scheduling` + `apps/edtech` | Enrolment to attendance to certificate |
| **Rollout track** (after F1, before the second paying client) | `deploy/rollout`, tested backup/restore, R8 `rnx new --from`, client onboarding in < 1 day | A new client goes live from `client.toml` alone |

## 10. Risks and items to verify

| Risk | Mitigation |
|---|---|
| **UAE FTA e-invoicing** (Peppol-based model; Ministerial Decisions 243 and 244 of 2025: pilot from 1 July 2026, mandatory from 1 January 2027 for revenue ≥ AED 50m and from 1 July 2027 for others; B2C excluded for now, see [01-property.md](verticals/01-property.md) [S18]) | Design `wl-invoicing` around structured data from day one, then add an adapter through an Accredited Service Provider. Most clients must be ready by **mid-2027**; the adapter is therefore part of the first year, not Later. ✔ ASP appointment deadlines: 30 October 2026 for revenue of AED 50m or more (amended by Ministerial Resolution 66/2026), 31 March 2027 for the rest ([01-property.md](verticals/01-property.md) Sources) |
| Local payment gateways (N-Genius, Telr) are not in Renox | Gateway trait in `wl-payments`. Stripe and N-Genius in the MVP; other local gateways when a client asks for them |
| WhatsApp Business API requires a BSP (e.g. Twilio, 360dialog) | Trait-based channel in `wl-notify`, provider chosen per client |
| F&B aggregator APIs (Talabat etc.) and eClaimLink are not openly available | These verticals are deliberately placed late (F6, F5). Validate partner access first |
| Premature abstraction | Rule of three: new modules are extracted from app code that has proven reuse |
| Client customisation leaking into the core | Mandatory review for every client request: configuration? event? extension crate? The core changes only if the change helps ≥ 2 clients |

## 11. Open questions (not blocking F0)

1. ~~First payment gateway for UAE clients?~~ Decided 2026-10-15: Stripe and N-Genius.
2. Which WhatsApp provider?
3. Which UAE hosting provider?
4. Pricing: setup fee + subscription per client, per module, or per unit/employee?
