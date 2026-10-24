# White-Label Business Apps for UAE SMEs

**One platform, ten businesses.** An open blueprint for white-label business software for
small and medium companies in the United Arab Emirates, built from shared modules on
[Renox](https://www.renox.rs), a batteries-included Rust web framework (Axum, HTMX, Alpine.js,
SQLite, one binary).

Each client gets their own branded app, in English and Arabic, hosted in the UAE. Behind the
brand, every app runs the same tested modules for parties, documents, invoicing, payments,
stock, scheduling, HR and more. Only the vertical's own workflows differ.

> **Status: phase F*n* builds vertical *n*; all ten verticals are built (F1–F10, `apps/*`, one
> demo client each in `clients/`, one `<vertical>:demo` command each except 04).** This repository
> holds the full design (business context, state machines, access rules, automations and UAE
> compliance notes) and, since [decision 0004](docs/decisions/0004-start-f1-build.md), the code:
> `crates/wl-*`, `apps/*`, `host`.

## The ten verticals

| # | Vertical | Who it is for | Reuse from shared modules |
|---|---|---|---|
| 01 | [Property & building management](docs/verticals/01-property.md) | Leases, Ejari, PDCs, maintenance, owner statements | 80% |
| 02 | [Construction & fit-out](docs/verticals/02-construction.md) | Subcontractors: IPAs, retention, variations, site labour | 79% |
| 03 | [Logistics & freight](docs/verticals/03-logistics.md) | Last-mile delivery, COD, freight forwarding | 80% |
| 04 | [Accounting firms](docs/verticals/04-accounting.md) | VAT and corporate tax filing, AML/CDD, fractional CFO | 78% |
| 05 | [Private clinics](docs/verticals/05-clinics.md) | Appointments, insurance claims, licences | 85% |
| 06 | [Cloud kitchens & F&B](docs/verticals/06-fnb.md) | Aggregators, own delivery, recipes, food safety | 76% |
| 07 | [Light manufacturing](docs/verticals/07-manufacturing.md) | Work orders, lots, quality, landed cost | 85% |
| 08 | [Auto parts & fleet services](docs/verticals/08-auto.md) | Workshop job cards, insurance repairs, parts sales | 81% |
| 09 | [Manpower supply](docs/verticals/09-manpower.md) | Deployments, timesheets, visas, WPS payroll | 86% |
| 10 | [Training centres & EdTech](docs/verticals/10-edtech.md) | Batches, enrolments, tutors, certificates | 81% |

Across the ten verticals there are 254 distinct capabilities. 106 of them are shared, so each
vertical adds only 9–24 of its own ([capability map](docs/CAPABILITY-MAP.md)).

## What makes it different

- **Built for the UAE.** VAT and the 2027 e-invoicing mandate, WPS salary files, Ejari,
  post-dated cheques, the midday work break, PDPL data protection, health and education
  regulators. Every legal fact cites its source; open questions are tracked in
  [OPEN-ITEMS.md](docs/OPEN-ITEMS.md).
- **Negative flows, not just happy paths.** Bounced cheques, chargebacks, recalls, lost
  parcels, rejected WPS files, expired licences that block work, absconding workers. A
  [review of 580 negative flows](docs/reviews/2026-10-15-negative-flows.md) was closed across
  all modules and verticals.
- **State machines you can read.** Every entity's statuses, transitions, guards and effects
  are tables with a Mermaid diagram, and one small engine enforces them
  ([WORKFLOW.md](docs/WORKFLOW.md)).
- **Access control by design.** Roles, scopes, limits and segregation of duties on every
  sensitive action ([ACCESS-CONTROL.md](docs/ACCESS-CONTROL.md)).
- **Written for humans and agents.** Fixed templates, stable IDs (`FIN.INV.01`,
  `PRP.RUL.03`) and self-contained specifications that a developer or a coding model can
  build from.

## Shared modules

`wl-kernel` · `wl-parties` · `wl-portal` · `wl-documents` · `wl-notify` · `wl-invoicing` ·
`wl-payments` · `wl-sales` · `wl-ticketing` · `wl-delivery` · `wl-scheduling` ·
`wl-attendance` · `wl-hr` · `wl-inventory`, plus `wl-operator` (our control plane for running the platform, specified in
[docs/modules/wl-operator.md](docs/modules/wl-operator.md), **not built yet**).
See [docs/modules/](docs/modules/).

## Run it

Rust 1.94 or newer. From the repository root:

```bash
cargo build -p host --locked
cd host
export APP_ENV=local APP_DEBUG=true APP_KEY=base64:$(head -c 32 /dev/urandom | base64) \
  DATABASE_URL=sqlite://$PWD/storage/app.db?mode=rwc STORAGE_PATH=$PWD/storage APP_PORT=3000 MAIL_MAILER=log
../target/debug/host migrate:fresh --seed   # creates the demo database and the demo people (local only)
../target/debug/host serve                  # http://127.0.0.1:3000, admin@example.com / password123
```

The demo client is `clients/demo-properties/client.toml`. Other verticals: set
`CLIENT_CONFIG=clients/demo-<x>/client.toml`, run `<vertical>:demo` (see
[docs/DEMO-DATA.md](docs/DEMO-DATA.md)); 04 accounting has `db:seed` only. Tests: `cargo test --workspace --locked`.
Deploying a client to a server (production variables, systemd, Caddy, Litestream, rollout,
restore drill): [deploy/README.md](deploy/README.md) and [docs/OPERATIONS.md](docs/OPERATIONS.md).
Never run `migrate:fresh`, `db:seed` or a `<vertical>:demo` command against a production
database (the host refuses them when `APP_ENV` is production).

## Where to start

- [docs/README.md](docs/README.md): reading guide and conventions
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): how the platform is layered and deployed
- [docs/CAPABILITY-MAP.md](docs/CAPABILITY-MAP.md): what is shared and what is specific
- [docs/verticals/](docs/verticals/): one document per business
- [docs/backlog/](docs/backlog/README.md): 478 build-ready issues in 25 epics (15 modules, 10
  verticals), created on GitHub, each self-contained enough for a developer or a coding model
- [SME problem map](docs/reviews/2026-10-16-sme-problem-map.md): 50 common SME problems and
  how the platform answers them

## Built on Renox

[Renox](https://github.com/renox-rs/renox) is a Laravel-inspired Rust web framework: Axum,
HTMX and Alpine.js, SQLite or PostgreSQL, deployed as one binary. This project is a reference
application for it.
