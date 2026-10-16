# Security

> Status: draft for review · Last updated: 2026-10-15

How the platform protects client data: what we protect, from whom, with which controls, and
what happens when something goes wrong. Access rules themselves are in
[ACCESS-CONTROL.md](ACCESS-CONTROL.md). Hosting and day-to-day operations are in
[OPERATIONS.md](OPERATIONS.md).

## 1. Principles

1. **One client, one process, one database** ([ARCHITECTURE.md](ARCHITECTURE.md) §1). A bug
   in one client's deployment cannot read another client's data through the application.
2. **Deny by default** (`PLT.ACL.01`). Every action and every list is checked on the server.
3. **Least data.** Collect only what a capability needs. Sensitive classes are opt-in per
   document kind and field.
4. **Everything sensitive is audited:** changes, views of sensitive data, role grants,
   operator access.
5. **Secure defaults ship with Renox** (CSP, CSRF, escaping). We do not turn them off for
   convenience.

## 2. Assets and data classes

The sensitivity classes come from [wl-documents](modules/wl-documents.md) §3 and apply to
fields as well as files ([ACCESS-CONTROL.md](ACCESS-CONTROL.md) `PLT.ACL.06`).

| Class | Examples | Where it appears | Extra protection |
|---|---|---|---|
| `normal` | Invoices, tickets, catalogue, bookings | all verticals | Role and scope rules |
| `personal` | Emirates ID, passport, visa, phone, address, photos of people, location at check-in | 01, 02, 03, 04, 05, 09, 10 | `SEN` permission, view audit, per-file encryption (R16), retention purge |
| `health` | Medical fitness results, insurance cards, clinical notes attached by mistake, health cards | 05, 06, 09 | As `personal`, plus no health data in messages (`NTF.ACL.02`), UAE-only hosting, retention at least 25 years from the last health procedure ✔ [S2] |
| `financial` | Salaries, bank details, bank statements, payroll files, card tokens | 04, 09, all (payroll) | As `personal`. Card numbers are never stored: gateways hold them |
| Secrets | Gateway keys, WhatsApp/SMS credentials, API tokens, signing keys, backup keys | every deployment | Per-client secrets (§5.6), never in the repository or logs |
| Evidence | Audit trail, approval history, transition history | every deployment | Append-only; archived, never deleted before retention ends (`PLT.ACL.07`) |

## 3. Trust boundaries

| Boundary | Who crosses it |
|---|---|
| Internet → portal | External users (tenants, patients, parents), recipients with signed links |
| Internet → back office | Client staff (2FA), field staff on phones (PWA) |
| Internet → API and webhooks | Client integrations with API tokens; payment gateways and BSPs posting webhooks |
| Upload → storage | Any file a user or the portal uploads |
| Deployment → outbound | Email, SMS, WhatsApp, gateways, accounting exports, outbound webhooks (R11) |
| Operator → deployment | Us: provisioning, upgrades, break-glass support (`PLT.ACL.04`) |
| Deployment → host | Several client processes on one VPS (§4.7) |

## 4. Threat model

STRIDE: **S**poofing, **T**ampering, **R**epudiation, **I**nformation disclosure, **D**enial
of service, **E**levation of privilege.

### 4.1 Portal

| Threat | STRIDE | Control |
|---|---|---|
| Account takeover by guessing or reused passwords | S | Passwordless login or OTP (R15), login rate limit, lockout, breach-password check. ✔ Checked in Renox: login throttling per account, address and pair (`auth/throttle.rs`) and a breached-password rule (`Password::uncompromised`, Have I Been Pwned range API) exist |
| A tenant reads another tenant's invoice by changing an ID in the URL | I | Record rules `OWN` on every query (`PRP.ACL.01`, `WRK.ACL.01`, `DOC.ACL.01`); IDs are not the control |
| A guardian keeps access after the child is 18 | E | Relationship end closes the grant (`PTY.ACL.01` proposed) |
| A former tenant still sees data | I | Access grant `end` → read-only → `close` (`POR.ACL.01` proposed) |
| A forwarded signed tracking link leaks a recipient's address | I | Signed links expire, show the minimum data, and can be revoked (R15) |
| Script injection in a ticket description shown to staff | T, E | Template auto-escaping, CSP on by default, no inline scripts |

### 4.2 Back office

| Threat | STRIDE | Control |
|---|---|---|
| Stolen staff password | S | 2FA required (`PLT.ACL.02`), optional SSO (`PLT.SSO.01`), session timeout |
| A staff member grants themselves more rights | E | `PLT.ACL.03`: cannot grant a permission you do not hold; grants audited |
| Insider fraud: refund to self, fake supplier bill | T, R | Segregation of duties (`FIN.ACL.04`, `STK.ACL.01`, `FIN.ACL.03`), approvals with history, audit |
| Bulk export of personal data by a leaving employee | I | Exports need permission and are audited; `NTF.ACL.01` for marketing lists; leavers lose access at once (`PLT.ACL.08`) |
| Editing an issued invoice to hide revenue | T | `FIN.ACL.01`: issued invoices are immutable; corrections by credit note |
| Changing a status to skip a check | E | Status changes only through state machine transitions with guards (WORKFLOW §3.2) |
| CSRF from a malicious page | T | Renox CSRF protection on every form and htmx request |

### 4.3 API and webhooks

| Threat | STRIDE | Control |
|---|---|---|
| Leaked API token | S, I | Tokens with abilities and scope, hashed at rest, rotatable, last-used shown; rate limited (R19) |
| Forged gateway or BSP webhook marking an invoice paid | S, T | Signature verification per provider (Renox webhooks), idempotent processing, amounts re-checked with the gateway |
| Replay of an old webhook | T | Stored event IDs, unique per provider (Renox `webhook_calls`) |
| Our outbound webhook spoofed to a client's n8n | S | HMAC signature and timestamp on every outbound call (R11) |
| Flood of API calls | D | Per-token and per-IP limits (R19), queue back-pressure |

### 4.4 Uploads

| Threat | STRIDE | Control |
|---|---|---|
| Malware in an uploaded file opened by staff | T, E | Malware scanning hook (R16) before a file becomes visible; allowed types per document kind |
| Oversized or many uploads | D | Size limits per kind, quotas per party |
| Image with hidden location data | I | Strip EXIF metadata on image processing (R16) |
| Direct link to a stored file | I | Files served only through the app with an access check, or short-lived signed URLs |

### 4.5 Outbound messaging

| Threat | STRIDE | Control |
|---|---|---|
| Health or identity data in an SMS or WhatsApp body | I | `NTF.ACL.02`: messages carry a link, not the data |
| Message to the wrong phone after a number change | I | Phone changes need verification; message log per recipient |
| Spoofed sender name | S | Sender IDs registered with e& and du under TDRA policy ✔ [S5] and Meta-approved WhatsApp templates ✔ [S6] |

### 4.6 Operator access

| Threat | STRIDE | Control |
|---|---|---|
| Our support staff browse client data without need | I | Break-glass account disabled by default; enabled per incident, time-boxed, audited, client admin notified (`PLT.ACL.04`) |
| A compromised operator machine pushes a bad release to every client | T | Signed builds, canary first, rollout waves ([OPERATIONS.md](OPERATIONS.md) §4), two-person approval for production rollout |
| Operator credentials reused across clients | S | Per-client secrets; operator access goes through the control plane with SSO and 2FA |

### 4.7 Multi-client host

| Threat | STRIDE | Control |
|---|---|---|
| One client's process reads another's database file | I | Separate OS user per client, file permissions 0600, systemd sandboxing (`ProtectSystem`, `PrivateTmp`, `ReadWritePaths` only to its own directory) |
| A noisy client starves others | D | systemd CPU and memory limits per unit; disk quotas per client directory |
| Backup of one client restored into another | I | Per-client backup bucket prefix and encryption key; restore tooling checks the client ID |

## 5. Controls

### 5.1 Web

- Content-Security-Policy on by default (Renox), no inline scripts, Alpine CSP build.
- CSRF tokens on every state-changing request; `without_csrf` only for verified webhooks.
- Template auto-escaping; no raw HTML from users.
- HTTPS only, HSTS, secure and `SameSite` cookies, TLS by Caddy.

### 5.2 Authentication

- Staff: password + 2FA (`renox-2fa`) required by default; SSO with Microsoft 365 or Google
  Workspace optional (`PLT.SSO.01`, renox-oauth).
- Portal: magic link or one-time code by SMS/WhatsApp, or password + optional 2FA (R15).
- Sessions: idle timeout (staff 30 min, portal 60 min defaults), absolute timeout, logout on
  password or role change, list and revoke sessions.
- Login and OTP attempts rate limited; lockout with notification.

### 5.3 Authorisation

Four layers of [ACCESS-CONTROL.md](ACCESS-CONTROL.md): authentication, RBAC, scoped
assignment, record and field rules. Lists filter in the query, never after loading.

### 5.4 Data protection

- **In transit:** TLS everywhere, including to gateways, BSPs and the backup VPS storage.
- **At rest:** disk encryption on the VPS; Litestream replicas encrypted on the backup VPS storage;
  per-file envelope encryption for `personal`, `health` and `financial` documents (R16).
- **Minimisation:** location only at check-in and check-out; card data never stored; message
  bodies without sensitive data.
- **Retention:** per document kind and record type; a nightly job purges what has expired,
  except records under legal hold or retention duty (tax, AML, health).

### 5.5 Rate limiting and abuse

Login, OTP, portal forms, public booking and ordering pages, API tokens and outbound
providers are rate limited (R19; ✔ Renox already limits HTTP requests per route, user, device token or IP; outbound limits per provider are the gap).

### 5.6 Secrets

- Each deployment has its own `.env` with its own keys; nothing shared between clients except
  the operator's public signing key.
- Secrets are stored in the operator's secret store and written to the host at deploy time,
  readable only by the client's OS user.
- Rotation: gateway and BSP keys on request or incident; signing keys yearly; API tokens by
  the client.
- Secrets never appear in logs, issues, chat or screenshots.

### 5.7 Audit

Renox `Audit` records changes; `wl-kernel` adds views of sensitive data, exports, role
grants, approvals and operator sessions. The audit log is append-only for the application and
kept for the longest retention period that applies to the client: health at least 25 years
✔ [S2], tax 5 years (7 for corporate tax, 15 for real estate VAT) ✔ [S4], AML at least
5 years after the relationship ends ✔ [S3].

## 6. Compliance

| Rule | Applies to | What we do |
|---|---|---|
| **PDPL** (Federal Decree-Law 45 of 2021) | all clients | Lawful basis and consent records (`PTY.CNS.01`), data subject requests (download, correction, deletion within retention limits), processor agreement with each client, breach handling (§7). Executive regulations not confirmed as published on the official legislation portal ✔ [S1]; what they require `⚠ open: lawyer (L-01)` |
| **Data residency** | all; mandatory for health | Hosting on our own VPS in the UAE, backups on a second UAE VPS ([OPERATIONS.md](OPERATIONS.md) §2) |
| **Health data** (Federal Law 2 of 2019 on ICT in health; DHA, DoH ADHICS, MOHAP) | 05 clinics, health fields in 06 and 09 | UAE-only storage, access audit, retention at least 25 years, no transfer abroad except approved cases (Ministerial Decision 51 of 2021) ✔ [S2]; ADHICS v2.0 covers SaaS systems used by Abu Dhabi facilities ✔ [S7]. Certification needs for a non-EMR system `⚠ open: authority (AU-03)` |
| **AML/CFT** | 04 accounting, 01 brokers | CDD records and screening logs kept at least 5 years from the end of the relationship (Decree-Law 10 of 2025); sanctions matches frozen within 24 hours (Cabinet Decision 74 of 2020) ✔ [S3] |
| **Tax records** (FTA) | all | Invoices and supporting records retained for the statutory period; immutable after issue. 5 years after the tax period in general, 7 for corporate tax, 15 for real estate VAT records, plus 2 while a refund is pending ✔ [S4] |
| **Free zone data rules** (DIFC, ADGM) | clients registered there | Their own laws apply instead of PDPL: DIFC Data Protection Law 5 of 2020, ADGM Data Protection Regulations 2021 ✔ [S1]; checked per client at onboarding |

## 7. Incident response

1. **Detect:** alerts from monitoring ([OPERATIONS.md](OPERATIONS.md) §6), audit anomalies,
   client reports, vulnerability reports.
2. **Triage** within 1 hour in business hours: severity, affected clients, data classes.
3. **Contain:** revoke tokens or sessions, disable an integration, take one client into
   maintenance mode, block an IP range.
4. **Eradicate and recover:** patch, roll back, restore from backup if data was altered.
5. **Notify:** the affected client's admin without undue delay. Under PDPL the controller
   (our client) must notify the data office and data subjects; we support them with facts and
   timelines. The PDPL requires notice "immediately upon becoming aware" (Art. 9), with the
   deadline left to the executive regulations ✔ [S1]; the exact deadline `⚠ open: lawyer (L-02)`.
   Health incidents may also need notice to the health authority (DHA, DoH under ADHICS)
   `⚠ open: authority (AU-04)`.
6. **Learn:** written post-incident review within 5 working days; actions become issues.

Severity levels and response targets are in [OPERATIONS.md](OPERATIONS.md) §8.

## 8. Vulnerability management

- **Dependencies:** `cargo deny` (advisories, licences, sources) in CI for every module and
  app; RustSec advisories checked daily; Renox's own `deny.toml` is the baseline.
- **Updates:** security fixes rolled out within 7 days (critical: 48 hours) through the normal
  rollout waves.
- **Penetration test:** before the first production client, then yearly and after major
  changes to authentication or the portal. Clinics clients may require their own test
  evidence, as ADHICS applies to SaaS systems they use ✔ [S7]; what each asks for
  `⚠ open: client (C-01)`.
- **Reporting:** a `security.txt` and a contact address; acknowledgement within 3 working days.
- **Secure development:** code review for every change; tests for every access rule
  ([TESTING.md](TESTING.md) §4); no secrets in the repository (pre-commit scan).

## 9. Open questions

1. Which PDPL executive regulations are in force, and what is the breach notification deadline? *(lawyer)*
2. Does a booking and billing system for clinics need DHA/DoH certification or ADHICS compliance? *(authority)*
3. Do any clients need an ISO 27001 or SOC 2 report from us before signing? *(sales)*
4. Which S3-compatible storage on the backup VPS (e.g. MinIO) gives the encryption, versioning
   and object locks we need? *(us)*

## Sources

Checked 2026-10-15. *Official* = government or legislation; others are secondary.

| # | Source | Type | Used for |
|---|---|---|---|
| S1 | DLA Piper, "Data protection laws in UAE", https://www.dlapiperdataprotection.com/countries/uae-general/law.html; CMS, "ADGM Data Protection Regulations", https://cms.law/en/are/legal-updates/ADGM-Data-Protection-Regulations-what-businesses-need-to-know | Secondary | PDPL executive regulations status, breach notice (Art. 9), DIFC and ADGM regimes |
| S2 | Federal Law 2 of 2019 on ICT in health fields and Cabinet Resolution 32 of 2020; Ministerial Decision 51 of 2021 via Al Tamimi and Baker McKenzie (see [05-clinics.md](verticals/05-clinics.md) S3, S4) | Official / secondary | Health data in the UAE, 25-year retention, exceptions |
| S3 | MoET, "Financial crimes legislation", https://www.moet.gov.ae/en/financial-crimes-legislations; Executive Office, "UN and local lists", https://www.uaeiec.gov.ae/en-us/un-page | Official | AML retention, sanctions freeze |
| S4 | Cabinet Decision 17 of 2026 and Decree-Law 17 of 2025, via DLA Piper and PwC (see [04-accounting.md](verticals/04-accounting.md) S9) | Secondary | Tax record retention |
| S5 | Telerivet, "UAE SMS compliance: TDRA and the AD- prefix", https://www.telerivet.com/blog/uae-sms-compliance-tdra-ad-prefix; D7 Networks, https://d7networks.com/sms/uae/sms-regulations/ | Secondary | SMS sender ID registration |
| S6 | Meta, "Pricing on the WhatsApp Business Platform", https://developers.facebook.com/docs/whatsapp/pricing | Secondary (vendor) | WhatsApp templates |
| S7 | DoH, "ADHICS v2.0", https://www.doh.gov.ae/-/media/78A323607B4C4ACAA58D0C9ACCFB3D59.ashx | Official | ADHICS covers SaaS systems |
