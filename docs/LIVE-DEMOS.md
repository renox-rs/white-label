# Live demos

> Status: live since 2026-10-24 · Hosted by Renoxium · Data is fictional

One public demo per vertical, each on its own subdomain of `renoxium.com`. Every demo is the
same `host` binary started with that vertical's demo client (`clients/demo-*`), with its own
SQLite database.

| # | Vertical | Address | Demo company | Demo client |
|---|---|---|---|---|
| 01 | [Property & building management](verticals/01-property.md) | <https://property.renoxium.com> | Palm Grove Properties LLC | `demo-properties` |
| 02 | [Construction & fit-out](verticals/02-construction.md) | <https://construction.renoxium.com> | Al Waha Interiors & Fit-out LLC | `demo-construction` |
| 03 | [Logistics & freight](verticals/03-logistics.md) | <https://logistics.renoxium.com> | Najm Express Cargo LLC | `demo-logistics` |
| 04 | [Accounting firms](verticals/04-accounting.md) | <https://accounting.renoxium.com> | Falcon Ledger Accountants LLC | `demo-accounting` |
| 05 | [Private clinics](verticals/05-clinics.md) | <https://clinic.renoxium.com> | Palm Grove Clinic LLC | `demo-clinics` |
| 06 | [Cloud kitchens & F&B](verticals/06-fnb.md) | <https://kitchen.renoxium.com> | Zaatar & Co. Kitchens LLC | `demo-fnb` |
| 07 | [Light manufacturing](verticals/07-manufacturing.md) | <https://factory.renoxium.com> | Emirates Pack Industries LLC | `demo-manufacturing` |
| 08 | [Auto parts & fleet services](verticals/08-auto.md) | <https://garage.renoxium.com> | Al Noor Auto Parts & Garage LLC | `demo-auto` |
| 09 | [Manpower supply](verticals/09-manpower.md) | <https://manpower.renoxium.com> | Desert Star Manpower Services LLC | `demo-manpower` |
| 10 | [Training centres & EdTech](verticals/10-edtech.md) | <https://academy.renoxium.com> | Gulf Skills Training Centre LLC | `demo-edtech` |

## Using them

- **Logins.** The sign-in page of each demo lists one-tap accounts per role and shows the shared
  password ([DEMO-DATA.md](DEMO-DATA.md) §1.5). Only demo data belongs in them: do not upload real
  client data, documents or personal details.
- **Reset.** Every demo is put back to its freshly seeded state every night at 03:00 Gulf time
  (23:00 UTC), as [DEMO-DATA.md](DEMO-DATA.md) §1.3 says. Changes, uploads and created users
  disappear at the reset, and a demo is briefly unreachable while it runs (5 to 40 seconds).
- **No outbound messages.** Mail is written to a log, never sent; SMS and WhatsApp are simulated.
- **Not a production setup.** The demos run with `APP_ENV=local` and `APP_DEBUG=false`, because the
  demo data commands and the one-tap logins exist only in `local`. Real clients run in
  `production` ([deploy/README.md](../deploy/README.md)), hosted in the UAE.

## Demo data per vertical

State of the demos at release `73f7d2b` (2026-10-24):

| Demo | Data |
|---|---|
| 01, 02, 03, 05, 06, 07, 09, 10 | Demo company with its own generator (`<vertical>:demo`): people in every role, history, and the records that trigger the automations |
| 04 Accounting, 08 Auto | **Base seed only** (the starter tables, 2 users, a few generic parties and invoices). The demo company of these two has no generator yet, so the demos are nearly empty |

## How they are hosted

- **Build.** [`release-main.yml`](../.github/workflows/release-main.yml) builds `host` on every push
  to `main` and uploads it as the artifact `white-label-<sha>` (the binary `white-label`, the
  revision, a checksum and the `clients/` presets).
- **Deploy.** The server pulls the newest artifact from `main` every five minutes and restarts the
  ten instances after the checksum matches; it rolls back when a `/health` check fails. A new release
  does not reseed: data added by a release shows after the next nightly reset, or after
  `systemctl start white-label-reset@<name>` on the server.
- **Layout.** One release in `/opt/white-label/current`; per demo a systemd unit
  `white-label@<name>` (socket activated, sandboxed, own OS user), `/etc/white-label/<name>/env`
  and `/var/lib/white-label/<name>/`. Ports 3101 to 3110 follow the vertical number, behind Caddy and
  Cloudflare.
- **Names.** `<name>` is the subdomain: `property`, `construction`, `logistics`, `accounting`,
  `clinic`, `kitchen`, `factory`, `garage`, `manpower`, `academy`.

A client install follows [deploy/README.md](../deploy/README.md), not this page.
