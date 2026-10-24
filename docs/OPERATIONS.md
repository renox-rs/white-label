# Operations

> Status: target design with a partly built base (section 12) · Last updated: 2026-10-24

How we run the hosted platform: where it runs, how releases reach clients, backups,
monitoring, support, and how a client is onboarded and offboarded. The tools for this live in
the control plane, [wl-operator](modules/wl-operator.md). Security controls are in
[SECURITY.md](SECURITY.md).

> **What exists today (F1-F10 built).** The per-client deployment files and the runbook are in
> [`deploy/`](../deploy/README.md): Dockerfile, systemd service and socket units, Litestream
> configuration, production environment file, Caddyfile and two release workflows
> (`.github/workflows/release.yml` for tagged client releases, `release-main.yml` for the
> public demo builds). Everything about the control plane (`wl-operator`), the
> host agent, automatic rollback, the nightly snapshot job, uploads sync, monitoring and
> alerting below is still a **target**. Section 12 says what has been built and what has been
> tested.

## 1. Model

Hosted per client ([decision table in ARCHITECTURE.md](ARCHITECTURE.md) §1): one Renox binary,
one SQLite database and one Litestream replica per client, all in the UAE. Clients
without an extension crate run the same `host` binary; clients with one get their own build.

## 2. Topology

```
                 ┌──────────────── control plane (wl-operator) ────────────────┐
                 │ client registry · entitlements · releases · health · billing │
                 └──────────────┬───────────────────────────────┬──────────────┘
                                │ deploy, config, secrets        │ health, metrics
     ┌──────────────────────────▼──────────┐   ┌────────────────▼─────────────────┐
     │ VPS "app-1" (own VPS, UAE)          │   │ VPS "app-2" (own VPS, UAE)        │
     │ Caddy (TLS per client domain)       │   │ …                                 │
     │ client A: systemd unit, OS user,    │   │                                   │
     │   SQLite file, uploads dir          │   │                                   │
     │ client B: …                         │   │                                   │
     └───────────────┬─────────────────────┘   └───────────────────────────────────┘
                     │ Litestream (continuous), nightly snapshots, uploads sync
              ┌──────▼──────────────────────────────┐
              │ Backup VPS, UAE, other provider:     │
              │ one prefix and key per client        │
              └──────────────────────────────────────┘
```

- **Hosting** (D-01, decided 2026-10-15): client deployments run on our **own VPS in the UAE**.
  Replicas, snapshots and uploads go to a **second VPS in the UAE at another provider**,
  running S3-compatible storage (e.g. MinIO) with one bucket prefix and key per client, so
  one provider's incident cannot remove both copies. Data never leaves the UAE. We operate
  the backup host ourselves: disk space, encryption, updates and alerts are part of
  monitoring (§6). The backup VPS has enough CPU and memory to run clients in emergency
  mode (§5).
- **Per client:** its own OS user, directory, systemd unit (with CPU/memory limits and
  sandboxing), SQLite file, uploads directory, `.env` with its own secrets, and domain.
- **Shared per host:** Caddy, the host agent that reports health to the control plane.
- **Clinic clients** may be placed on a dedicated host if health rules require it. The health
  data law requires UAE storage ✔ [S1] and ADHICS v2.0 covers SaaS systems used by Abu Dhabi
  facilities ✔ [S2], but neither was found to require a dedicated host
  `⚠ open: authority (AU-02)` (open question 2).

## 3. Environments

| Environment | Purpose | Data | Who uses it |
|---|---|---|---|
| **Development** | Local work on modules and apps | Factories and demo data | Developers, agents |
| **Demo** | One deployment per vertical for sales demos | Demo datasets ([DEMO-DATA.md](DEMO-DATA.md)), reset nightly (**target**: no reset job is built, `OPS.DMO.01` needs `wl-operator`; today a demo is made by its `<vertical>:demo` command and the public demos are redeployed on every push to main, section 12) | Sales, prospects |
| **Staging / canary** | First to receive every release; our internal demo client | Demo data + synthetic load | Us |
| **Production** | Paying clients | Real data | Clients |

No production data is ever copied to another environment. Bugs are reproduced with demo data
or with an anonymised extract approved by the client.

## 4. Releases and rollout

1. **Build once:** one signed `host` binary per release; client extension binaries are built
   from the same release.
2. **Checks before rollout:** all tests ([TESTING.md](TESTING.md)), `rnx db:check`,
   `rnx view:check`, migration dry run against a copy of every schema version in use.
3. **Waves:**

   | Wave | Who | Wait before next wave |
   |---|---|---|
   | 0 | Canary (internal demo) and demo deployments | 24 hours |
   | 1 | Clients who opted into early updates | 48 hours |
   | 2 | All other clients, outside their business hours | — |
   | Hotfix | Affected clients only, after canary | 1 hour on canary |

4. **Per deployment:** stop accepting new requests (socket activation keeps connections),
   snapshot the database, run migrations on boot, health check, switch over. If the health
   check fails, roll back automatically: previous binary + snapshot.
   *Today* this is a manual checklist with commands ([deploy/README.md](../deploy/README.md)
   sections 5 and 6); the automatic rollback is a target.
5. **Two-person rule:** wave 2 and every hotfix need a second operator's approval in
   `wl-operator`.
6. **Release notes** per client, in English and Arabic, listing what changed in the modules
   they have.

### Migrations

- Forward-only and backwards-compatible for one release: a column is added before it is used
  and removed one release after it stops being used.
- Data migrations on legal records (invoices, audit) are not allowed; issued documents never
  change.
- Long migrations run as queued jobs after boot, not during the switch.

## 5. Backups and restore

| Item | Method | Target |
|---|---|---|
| Database | Litestream continuous replication | **RPO ≤ 1 minute** |
| Database snapshot | Nightly snapshot, kept 30 days; monthly kept 12 months | — |
| Uploads | Synced to the backup VPS storage within 5 minutes; versioned | RPO ≤ 5 minutes |
| Configuration and secrets | In the control plane and its secret store | — |
| **Restore of one client** | New process from the latest replica on any host | **RTO ≤ 1 hour** |
| **Loss of a host** | All its clients restored on another host | RTO ≤ 4 hours |
| **Loss of the primary provider** (data centre or provider down; D-02, decided 2026-10-15) | All clients restored on the backup VPS (or a new VPS in the UAE) from their replicas, highest service level first; data stays in the UAE, so no client consent is needed | **RTO ≤ 24 hours** |

- **Restore drills:** every month one random client is restored to a scratch host and checked
  (row counts, last transaction time, a smoke test), so each client is drilled at least once a
  year; every other deployment gets an automatic check that its latest replica can be read.
  Results are recorded in `wl-operator`.
- **Point-in-time restore** of a client is possible within the replica's retention, on request.

## 6. Monitoring and alerting

| Signal | Source | Alert when |
|---|---|---|
| Health endpoint per client | `GET /health` (200 and `"status":"ok"` when the database answers, else 503; also reports queue `pending`/`failed`), polled by the host agent | 2 failures in a row |
| Error rate and latency | Request logs | 5xx rate > 1% or p95 > 1 s for 10 min |
| Queue | Renox queue dashboard data | Jobs waiting > 15 min, or failed jobs |
| Failed automation runs | `PLT.RUL.03` digest, rule run log | Any failure for a money or deadline rule |
| Outbound providers | Message and payment logs | Delivery failure rate > 5% |
| Replication | Litestream lag | Lag > 5 min |
| Disk, memory, CPU per host | Host agent | Disk > 80%, memory > 85% |
| Certificates | Caddy | Expiry < 14 days |
| Backups | Snapshot job, restore drill | Missed snapshot, failed drill |

Alerts go to the on-call operator; client-facing incidents are posted on a status page per
client.

## 7. Capacity

- Starting assumption: one 2 vCPU / 4 GB host serves 20–40 SME clients, given Renox's
  memory footprint of about 12 MiB per process under load. **To be measured** during F1 with
  synthetic load per vertical.
- A client moves to a larger or dedicated host when it exceeds agreed limits (users, storage,
  requests) or needs isolation (clinics).
- Storage is the usual limit: photos and documents. Quotas per plan are set in `wl-operator`.

## 8. Support and service levels

| Severity | Example | First response | Target resolution |
|---|---|---|---|
| **S1** | Deployment down, data loss, security incident | 1 hour, 24/7 | 4 hours |
| **S2** | A core flow broken (invoicing, payments, check-in) | 4 business hours | 1 business day |
| **S3** | A non-core feature broken, workaround exists | 1 business day | Next release |
| **S4** | Question, change request | 2 business days | Planned |

- **Availability target:** 99.5% per month per client, excluding announced maintenance.
- **Channels:** support portal (our own `wl-ticketing` instance in `wl-operator`), email,
  WhatsApp for S1.
- **Access to client data** only through break-glass (`PLT.ACL.04`), logged and notified.
- Business hours: Monday–Friday, 09:00–18:00 Gulf Standard Time.

## 9. Onboarding a client

| Step | What happens | Tool | Done when |
|---|---|---|---|
| 1. Contract | Plan, modules, entities, users, data processing agreement | `wl-operator` | Signed |
| 2. Provision | Client created from its `client.toml`: host chosen, OS user, domain, TLS, secrets, empty database, branding | `wl-operator` | Health check green |
| 3. Configure | Legal entities, scopes (buildings, projects, branches), roles and users, document kinds, numbering, automations | Admin | Checklist signed by client admin |
| 4. Import | Master data and open balances from Excel templates: parties, units/items/employees, open invoices, cheques, documents (`PLT.IMP.01`): dry run, fix, import | Admin import | Reconciled totals approved by the client |
| 5. Integrations | Email domain, gateway, SMS/WhatsApp sender, accounting export | Admin | Test message and test payment succeed |
| 6. Training | Role-based sessions; recorded videos; quick guides in EN/AR | — | Each role completed |
| 7. Go-live | Cut-over date, opening balances frozen, first live transactions watched | — | First week without S1/S2 |
| 8. Hypercare | Daily check-ins for 2 weeks | Support | Hand-over to normal support |

Target from [ARCHITECTURE.md](ARCHITECTURE.md) §9 (rollout track): steps 2–3 in under one day for a client
without an extension crate.

## 10. Offboarding

1. Notice per contract; the deployment becomes read-only on the end date.
2. **Export:** a full export of the client's data (database, documents, audit) in open formats,
   delivered securely; confirmation of receipt.
3. **Retention:** we keep an encrypted archive only if the contract or a legal duty requires
   it; otherwise data, replicas and backups are deleted within 30 days.
4. **Certificate of deletion** issued to the client.
5. Domain, TLS, secrets and the OS user are removed; the client is closed in `wl-operator`.

## 11. Open questions

1. ~~Which cloud and region?~~ Decided (2026-10-15): own VPS in the UAE, backups on a second
   UAE VPS at another provider (D-01, D-02).
2. Do clinic clients need dedicated hosts? *(authority)*
3. Is 99.5% availability enough for clients, or do some need 99.9% with a higher price? *(sales)*
4. Who staffs out-of-hours S1 support at the start? *(us)*

## 12. Deployment status

State at 2026-10-24 (the F1 checks of 2026-10-20 stand; F2-F10 added no deployment files). *Built* = the file or procedure exists in the repository. *Tested* = it was
run, and the note says where. Nothing is marked tested that was not run.

| Item | Built | Tested | Notes |
|---|---|---|---|
| Release binary (`cargo build --release -p host --locked`) | yes (CI job) | **no** | Skipped locally: under 15 GB of free disk. First run is the `release.yml` workflow. |
| Dockerfile and container image | yes | **no** | No Docker daemon in the build sandbox. CI builds it and boots it (`release.yml`, job `image`). |
| Production boot (`APP_ENV=production`, demo `client.toml`, temp SQLite) | yes | yes | Debug binary: `migrate`, `db:check`, `view:check`, `wl:install` and the other `:install` commands, `serve`, `/health`, `users:invite`, `users:admin`, sign-in, secure cookie and HSTS behind simulated proxy headers, `TRUSTED_HOSTS`. Failures met are in [deploy/README.md](../deploy/README.md) section 8. |
| Socket activation | yes (units) | partly | The app took the socket handed over by `systemd-socket-activate` and stopped cleanly on SIGTERM. The unit files pass `systemd-analyze verify`. No restart under load, no real systemd run. |
| systemd service, socket, litestream units | yes | **no** | Not started by systemd. |
| Caddy TLS and proxy | example | **no** | No Caddy; proxy headers simulated with curl. |
| Health endpoint | Renox `GET /health` | yes | There is no `/up`. |
| Litestream replication to MinIO | configuration | **no** | No Litestream and no MinIO available. Written for Litestream 0.3.x. |
| **Backups tested** | | **no** | No replica was written or read. |
| **Restore drill** | procedure | **no** | `deploy/README.md` section 7. Never run, not even on a scratch host. |
| Nightly snapshot kept 30 days, monthly 12 months | partly | no | Litestream snapshot and retention settings only; no monthly copy job. |
| Uploads sync to the backup VPS (RPO 5 min) | **no** | no | Not built. |
| Release rollout checklist and rollback | runbook | no | Manual commands; the switch (symlink + restart + health loop) was run against stand-ins, `sqlite3 .backup` was not run. |
| Rollout rings, two-person rule, automatic rollback | **no** | no | Need `wl-operator` / `deploy/rollout`. |
| Monitoring and alerting (section 6) | **no** | no | Only the data source exists (`/health`, journal). |
| Capacity figure (12 MiB per process) | no | no | Still to be measured. |
| Public demos (`<vertical>.renoxium.com`) | yes (workflow) | no | `.github/workflows/release-main.yml` builds the artifact `white-label-<sha>` (binary `white-label`, `REVISION`, sha256, `clients/`) on every push to main; the demo server pulls the newest artifact every few minutes (autodeploy). CI holds no server keys. The server side is not in this repository and not described in these docs. |
| Production hardening of the host | yes | yes (tests and a debug run) | `migrate:fresh`, `db:seed` and every `<vertical>:demo` command (the list `DEMO_COMMANDS` in `host/src/lib.rs`: `property:demo`, `construction:demo`, `logistics:demo`, `manpower:demo`, `clinics:demo`, `fnb:demo`, `edtech:demo`, `auto:demo`, `manufacturing:demo`) refuse to run when `APP_ENV` is production (`host/tests/production_guard.rs` tests three of them). There is no public sign-up: `/register` is gone, users are added on `/users` or with `users:invite` (a mail that fails leaves the user and no 500: `host/tests/users_invite.rs`). `/` redirects to `/dashboard` or `/login` (no starter-kit page). `CSP=strict` was tried in a browser on the main pages with no violation; the default stays `relaxed` ([deploy/README.md](../deploy/README.md) section 8). Not covered: portal pages, PDFs, uploads under `strict`. |

**The EdTech client (F10).** `apps/edtech` is built and boots from `clients/demo-edtech/client.toml`
like the other verticals (each has its own `<vertical>:install`: accounting, auto, clinics,
construction, edtech, fnb, logistics, manpower, manufacturing, property). `edtech:install` (also run by the seeders after `wl:install`) stores the
service types, ticket categories, lead sources, loyalty programme, send windows, onboarding
checklists, SOPs, budget category and notification reactions the 10 settings ask for; it adds what is
missing and keeps what the client changed, so it is safe to run on a live database. `edtech:demo`
(`host/src/app/edtech_demo`, see [DEMO-DATA.md](DEMO-DATA.md)) makes the demo company; like the other
demo commands it is refused in production. Nothing of F10 has been run in production: the release checks above
apply to it as to the others.

## Sources

Checked 2026-10-15. *Official* = government or legislation; others are secondary.

| # | Source | Type | Used for |
|---|---|---|---|
| S1 | Federal Law 2 of 2019 on ICT in health fields and Cabinet Resolution 32 of 2020 (see [05-clinics.md](verticals/05-clinics.md) S3) | Official | Health data stored in the UAE |
| S2 | DoH, "ADHICS v2.0", https://www.doh.gov.ae/-/media/78A323607B4C4ACAA58D0C9ACCFB3D59.ashx | Official | ADHICS covers SaaS systems |
