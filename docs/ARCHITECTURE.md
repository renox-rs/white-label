# White-Label UAE SME Platform — Architecture

> Status: **concept draft for review**. No code yet. Once approved, each phase in the
> [Roadmap](#9-roadmap) is split into GitHub issues.

## 1. Agreed decisions

| # | Topic | Decision |
|---|---|---|
| 1 | Business model | **Hosted per client**: 1 binary + 1 SQLite database per client, setup fee + monthly subscription |
| 2 | Customisation | **Configuration + extension crate**. No per-client forks |
| 3 | App order | **Property** (proves the core), then **Manpower** (tests reuse). See [decision 0001](decisions/0001-first-two-verticals.md) |
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
| Billing | `renox-billing`: Stripe/Xendit subscriptions + a custom gateway trait |
| Deployment | `rnx build` → one binary; `rnx make:deploy` (Docker, systemd, Litestream) |

## 4. Gaps to fill

### 4a. Contributions to Renox (OSS, useful to everyone)

Each gap is an issue in [renox-rs/renox](https://github.com/renox-rs/renox/issues) (#28–#52,
created 2026-10-16 after checking Renox 1.1.1; the R number links to it). Only R3 needs a new
crate (`renox-pdf`); the others extend renox-core, renox-blocks, renox-admin, renox-billing,
renox-oauth or renox-cli. R19 turned out to exist for jobs (`Middleware::rate_limited`); its
issue covers only Renox's own notification and mail sends.

| # | Feature | Reason |
|---|---|---|
| [R1](https://github.com/renox-rs/renox/issues/28) | **RTL + Arabic support** (automatic `dir="rtl"` per locale, CSS logical properties throughout `renox-ui.css`, Arabic font) | Required in the UAE. Today there is a single RTL rule |
| [R2](https://github.com/renox-rs/renox/issues/29) | **Runtime settings store** (`settings` table, typed accessors, cache, admin page) | Branding, TRN, invoice number format and enabled modules must change without a redeploy |
| [R3](https://github.com/renox-rs/renox/issues/30) | **PDF generation** (recommended: the `typst` crate, pure Rust, supports Arabic shaping, stays inside one binary) | Invoices, contracts, payslips, receipts |
| [R4](https://github.com/renox-rs/renox/issues/31) | **Admin hook for modules**: `Registry::admin_resource(...)` | Today the app has to collect every resource into one `Admin`, so a module cannot register its own |
| [R5](https://github.com/renox-rs/renox/issues/32) | **New components**: `rx-signature-pad`, `rx-camera-capture`, `rx-geolocate` (check-in), `rx-status-stepper`/timeline, `rx-week-calendar` (resource schedule), `rx-barcode-scan` (camera or keyboard-wedge scanner), `rx-rating` (feedback stars) | Needed for e-POD, attendance, ticketing and booking. Generic, fits `renox-blocks` |
| [R6](https://github.com/renox-rs/renox/issues/33) | **PWA**: manifest + minimal service worker (`rnx make:pwa`) | Field workers can install the app on their phones |
| [R7](https://github.com/renox-rs/renox/issues/34) | **Package scaffold**: `rnx make:package <name>` (a `Module` crate with migrations, views and tests) | Speeds up writing `wl-*` crates and third-party modules |
| [R8](https://github.com/renox-rs/renox/issues/35) | **Starter from git**: `rnx new acme --starter git+https://…` | Creates a new client from the white-label template |
| [R9](https://github.com/renox-rs/renox/issues/36) | **Brand-token generator**: `Theme::from_brand(accent, accent_2)` → light and dark tokens with a WCAG contrast guard; brand chosen per request domain (several brands in one deployment) | Per-client branding without forking the kit. See [DESIGN-SYSTEM.md](DESIGN-SYSTEM.md) §4 |
| [R10](https://github.com/renox-rs/renox/issues/37) | **Navigation contributions from modules**: `Registry::sidebar_link(section, link, permission)` for the back office and the portal menu | Each `wl-*` module adds its own staff and portal navigation |
| [R11](https://github.com/renox-rs/renox/issues/38) | **Outbound webhooks and signed requests**: event subscriptions, HMAC signing, queued delivery with retry, delivery log; signed server-to-server requests (operator admin API) | Client-specific integrations (n8n, Zapier, Make) live outside the platform. See [WORKFLOW.md](WORKFLOW.md) §5 |
| [R12](https://github.com/renox-rs/renox/issues/39) | **State machines for models**: declared states and transitions with permission, guards and effects; transition history; available actions per user; generated diagram | One engine for every status in every module. See [WORKFLOW.md](WORKFLOW.md) §3 |
| [R13](https://github.com/renox-rs/renox/issues/40) | **Gapless sequences**: per-series counters that never skip or repeat, safe under concurrency | Invoice, credit note and receipt numbers (tax rules) |
| [R14](https://github.com/renox-rs/renox/issues/41) | **Custom fields**: JSON-backed fields with a schema, rendered in forms, grids and filters | Per-client fields without migrations ([wl-kernel](modules/wl-kernel.md)) |
| [R15](https://github.com/renox-rs/renox/issues/42) | **Passwordless login**: magic link, one-time codes by SMS or WhatsApp (✔ signed, expiring URLs already exist in Renox: `state.signed_url`, `ValidSignature`) | Portal users and recipients without passwords ([wl-portal](modules/wl-portal.md)) |
| [R16](https://github.com/renox-rs/renox/issues/43) | **Upload pipeline hooks**: malware scanning, image processing (resize, thumbnails, HEIC → JPG), per-file encryption | Identity documents, site photos, medical files ([wl-documents](modules/wl-documents.md)) |
| [R17](https://github.com/renox-rs/renox/issues/44) | **renox-billing beyond subscriptions**: one-off payments, payment links, refunds; gateway credentials per deployment owner; a Network International N-Genius adapter (Stripe and N-Genius in the MVP, Telr and Checkout.com on demand) | Rent, invoices and deposits paid online ([wl-payments](modules/wl-payments.md)) |
| [R18](https://github.com/renox-rs/renox/issues/45) | **Money and quantities**: currency per record (formatting, validation); decimal quantities with unit-of-measure conversion | Multi-currency and inventory ([wl-invoicing](modules/wl-invoicing.md), [wl-inventory](modules/wl-inventory.md)) |
| [R19](https://github.com/renox-rs/renox/issues/46) | **Rate limiting**: per provider in the queue (SMS, WhatsApp, gateways). ✔ Checked in Renox: HTTP rate limits already exist (`Routes::throttle`, named limiters keyed by user, device token or IP, `rate_limit.rs`), so API tokens are covered; the gap is only outbound limits per provider in the queue | Notifications and the client API |
| [R20](https://github.com/renox-rs/renox/issues/47) | **Geo helpers**: point-in-radius/polygon geofence check; map picker and map view components (tile provider and licence to choose) | Attendance geofences, dispatch maps |
| [R21](https://github.com/renox-rs/renox/issues/48) | **Calendar and print output**: `.ics` files for bookings; 80 mm thermal receipt layout | Booking confirmations, counter sales |
| [R22](https://github.com/renox-rs/renox/issues/49) | **Arabic text search**: normalisation and Arabic–Latin transliteration for search and duplicate matching | Party names in both scripts ([wl-parties](modules/wl-parties.md)) |
| [R23](https://github.com/renox-rs/renox/issues/50) | **Import beyond CSV**: Excel files, a dry run that validates without writing, a rollback window after commit (✔ Renox import today: CSV, row validation, savepoints, `all_or_nothing`, templates) | Client data migration ([wl-kernel](modules/wl-kernel.md) `PLT.IMP.01`) |
| [R24](https://github.com/renox-rs/renox/issues/51) | **Enterprise sign-in for renox-oauth**: a Microsoft Entra ID provider; restricting Google sign-in to the client's Workspace domain | Staff single sign-on (`PLT.SSO.01`, Later) |
| [R25](https://github.com/renox-rs/renox/issues/52) | **Offline queue for field work** (Later): actions recorded without a connection (attendance, proof of delivery, POS sales, line output) are stored on the device and sent in order when online, with conflict handling | Field PWA in 02, 03, 06, 07, 08; MVP is online only with paper fallback ([UX.md](UX.md)) |

### 4b. Business modules (private, `white-label`)

| Module | Contents | First used by |
|---|---|---|
| `wl-kernel` | Enabled-module manifest, business calendar (working days, UAE public holidays, Ramadan hours), branding from settings, sequential document numbers (`INV-2026-0001`), custom fields (JSON values + a definitions table, Renox #41), cross-module traits, polymorphic `Subject` reference; legal entities and per-entity numbering, data import, global search, notes and mentions, report library, signed admin API and entitlements | all |
| `wl-parties` | Parties: companies and people (tenants, clients, employees, patients, students), contacts, addresses, TRN, Emirates ID | all |
| `wl-portal` | External-user login (magic link/OTP), branded portal layout, data scoped to the user's `Party` | property, accounting, logistics, clinics |
| `wl-documents` | Documents attached to a `Subject`, document types, **expiry dates + tiered reminders** (90/30/7 days), versions, access control | property, manpower, accounting |
| `wl-invoicing` | Invoices, credit notes, **UAE VAT 5%** (standard/zero-rated/exempt), recurring invoices, PDF, receivables ageing; FTA e-invoicing adapter in a later phase | property, manpower, … |
| `wl-payments` | One-off payments + payment links, reconciliation against invoices, gateway trait (Stripe and Network International N-Genius in the MVP; Telr and Checkout.com on demand), post-dated cheques (PDC) | property |
| `wl-ticketing` | Tickets/work orders per `Subject`, **configurable status workflow**, priority, SLA, assignment, photos, comments | property (maintenance), manufacturing (QC), auto (warranty) |
| `wl-notify` | WhatsApp Business and SMS channels (via `App::channel`), per-language templates, recipient preferences | all |
| `wl-attendance` | GPS + geofence check-in/out, timesheets, approval, rosters | construction, manpower |
| `wl-hr` | Employee records and licences, visa cases, leave, payroll, WPS SIF export, final settlement | manpower, clinics, EdTech |
| `wl-sales` | Leads, catalogue and price lists, sales orders, POS, prepaid packages, discounts, returns, credit limits | manufacturing, auto, F&B, clinics, EdTech |
| `wl-delivery` | Dispatch to drivers and runs, proof of delivery | logistics, auto, manufacturing, F&B |
| `wl-scheduling` | Resources (doctors, tutors, service bays), slots, bookings, reminders | clinics, EdTech, auto |
| `wl-inventory` | Items, stock per location, movements, counts, purchasing, lots, BOMs and recipes, pick-pack-dispatch, assets | construction, F&B, manufacturing, auto |

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
// wl-kernel (sketch)
#[async_trait]
pub trait Invoicer: Send + Sync {
    async fn draft(&self, d: InvoiceDraft) -> Result<InvoiceId>;
}

pub struct InvoiceDraft {
    pub party: PartyId,
    pub source: Subject,           // e.g. Subject::new("property.lease", 42)
    pub lines: Vec<InvoiceLine>,   // each line carries a VatCategory
    pub due_on: Date,
}

// apps/property: bills rent without depending on wl-invoicing's internals
registry.schedule().daily(|ctx| async move {
    for lease in Lease::due_for_billing(today()).await? {
        ctx.get::<dyn Invoicer>().draft(lease.invoice_draft()).await?;
    }
    Ok(())
});
```

`wl-invoicing` provides the `Invoicer` implementation through `registry.provide(...)`. If
invoicing is disabled for a client, the app uses a no-op implementation or hides the feature.

**3. Events for reactions.** `InvoicePaid`, `DocumentExpiring` and `TicketStatusChanged` are
dispatched as Renox events. Vertical apps and client extension crates listen to them to add
behaviour without changing the module.

**Extension points every module must offer:**

- overridable views (the Renox mechanism),
- custom fields through `wl-kernel`,
- events before and after important actions,
- small strategy traits, e.g. `VatRule`, `ReminderSchedule`, `TicketRouter`.

## 6. Module × app matrix

Replaced by [CAPABILITY-MAP.md](CAPABILITY-MAP.md): 87 shared capabilities across 14 modules,
mapped to the ten verticals, with 139 vertical-specific capabilities. Each module is
specified in [modules/](modules/).

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
│   └── manpower/
├── host/                      # binary: reads client.toml, assembles the App from enabled modules
├── operator/                  # our control plane (wl-operator): clients, plans, billing, rollout
├── clients/
│   └── acme-properties/
│       ├── client.toml        # app, modules, branding, locales, TRN
│       ├── resources/views/   # view overrides (optional)
│       └── ext/               # extension crate (optional)
├── deploy/                    # fleet scripts: build, migrate, rollout, backup
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
enabled = ["portal", "documents", "invoicing", "payments", "ticketing", "notify"]

[invoicing]
trn    = "100123456700003"
prefix = "ACM-INV"

[ticketing]
statuses = ["open", "assigned", "in_progress", "awaiting_parts", "done"]
```

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
- **Fleet upgrades:** `deploy/rollout` builds once, then rolls out client by client.
  Migrations run on boot, followed by a health check and automatic rollback on failure. A
  canary client (the internal demo) is always updated first.
- **Cost:** one small VPS (2 vCPU/4 GB) is expected to host dozens of SME clients, given
  Renox's footprint of about 12 MiB per process. To be measured in phase F1.
- **Details:** environments, release waves, backups and restore drills, monitoring, support
  SLAs and client onboarding are in [OPERATIONS.md](OPERATIONS.md); host hardening and the
  threat model in [SECURITY.md](SECURITY.md); the control plane in
  [wl-operator](modules/wl-operator.md).

## 9. Roadmap

| Phase | Scope | Done when |
|---|---|---|
| **F0 — Renox foundations** | R1 RTL/Arabic, R2 settings, R3 PDF, R4 admin hook, R7 make:package, R9 brand tokens, R10 sidebar hook, R12 state machines (with the approval interceptor requested on Renox #39). Also needed before Build 1 ships: R13 sequences, R14 custom fields, R15 passwordless login, R17 + R18 payments and money, R22 search, R23 import, and R5, R6, R20 for 09 attendance | A module crate can ship an admin page and an Arabic PDF; a transition blocked by a limit becomes an approval request |
| **F1 — Property** | `wl-kernel`, `wl-parties`, `wl-portal`, `wl-documents`, `wl-invoicing`, `wl-payments` (Stripe), `wl-ticketing`, `wl-notify` (email first) + `apps/property` + `host` + 1 demo client | End-to-end demo: unit → lease → VAT invoice → payment → maintenance ticket from the tenant portal |
| **F2 — Manpower** | `wl-attendance`, R5 (geolocate, camera), R6 PWA + `apps/manpower`. **Measure** new code vs reuse; refactor modules that leaked domain logic | ≥ 60% of Manpower's features run on existing modules without modification |
| **F3 — Fleet rollout** | `deploy/rollout`, tested backup/restore, R8 git starter, client onboarding in < 1 day | A new client goes live from `client.toml` alone |
| **F4+ — Next verticals** | Chosen by prospects: Accounting (cheapest, modules nearly complete) → Clinics/EdTech (`wl-scheduling`) → Construction/F&B/Manufacturing (`wl-inventory`) → Logistics | — |

## 10. Risks and items to verify

| Risk | Mitigation |
|---|---|
| **UAE FTA e-invoicing** (Peppol-based model; Ministerial Decisions 243 and 244 of 2025: pilot from 1 July 2026, mandatory from 1 January 2027 for revenue ≥ AED 50m and from 1 July 2027 for others; B2C excluded for now, see [01-property.md](verticals/01-property.md) [S18]) | Design `wl-invoicing` around structured data from day one, then add an adapter through an Accredited Service Provider. Most clients must be ready by **mid-2027**; the adapter is therefore part of the first year, not Later. ✔ ASP appointment deadlines: 30 October 2026 for revenue of AED 50m or more (amended by Ministerial Resolution 66/2026), 31 March 2027 for the rest ([01-property.md](verticals/01-property.md) Sources) |
| Local payment gateways (N-Genius, Telr) are not in Renox | Gateway trait in `wl-payments`. Stripe and N-Genius in the MVP; other local gateways when a client asks for them |
| WhatsApp Business API requires a BSP (e.g. Twilio, 360dialog) | Trait-based channel in `wl-notify`, provider chosen per client |
| F&B aggregator APIs (Talabat etc.) and eClaimLink are not openly available | These verticals are deliberately placed in F4+. Validate partner access first |
| Premature abstraction | Rule of three: new modules are extracted from app code that has proven reuse |
| Client customisation leaking into the core | Mandatory review for every client request: configuration? event? extension crate? The core changes only if the change helps ≥ 2 clients |

## 11. Open questions (not blocking F0)

1. ~~First payment gateway for UAE clients?~~ Decided 2026-10-15: Stripe and N-Genius.
2. Which WhatsApp provider?
3. Which UAE hosting provider?
4. Pricing: setup fee + subscription per client, per module, or per unit/employee?
