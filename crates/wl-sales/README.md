# wl-sales

Sales: marketing campaigns with a purpose, a channel and a budget, their cost entries and the
budget alerts ([spec](../../docs/modules/wl-sales.md)). Other modules react to campaigns only
through the kernel (`wl-kernel`), never through this crate.

The host mounts it when `client.toml` enables `sales`:

```
use renox::prelude::*;

# let _ =
App::new().module(wl_sales::WlSales::new())
# ;
```

What is here so far:

| Part | Spec | Module |
|---|---|---|
| Permissions (`campaigns.*`, `acquisition.reports.view`) | §6 | `permissions` |
| Over-budget approval, budget alerts, late-response days | §7 | `config` |
| Lead sources, their nine seeds (`sales:install`, the seeder) and the `/sales/lead-sources` page | §3 | `lead_source`, `seed`, `pages` |
| Campaigns, their machine (`activate`, `end`, `reopen`), promo codes, the events and the owner's end notice | §3, §4.6 | `campaign`, `machine`, `promo_code`, `notices` |
| The daily task `sales:campaigns` (00:05 Asia/Dubai) and `campaigns::run_daily` | §5 | `daily` |
| Leads: capture with the source and campaign of a web form code (late-response days), notes, the machine (`contact`, `qualify`, `win`, `lose`, `reopen`, `revert_win`), `LeadWon`/`LeadLost`/`LeadWinReverted` | §3, §4.1, §5.1 | `lead`, `lead_machine`, `lead_events` |
| Party acquisitions: source and campaign per party (first touch, never overwritten), the first sale with its converted value (net of credit notes, or the attribution window), `CustomerAcquired`, the source check of a sale for a new party, the nightly `sales:acquisitions` run that replaces a first sale that no longer stands, and the party merge | §3, §5.8 | `acquisition`, `acquisition_daily` |
| Changing an acquisition or a won lead (`SAL.ACL.09`): a reason, a second `campaigns.manage` holder, the audit record and `CustomerAcquisitionChanged`; the exceptions report page | §5.8, §6 | `acquisition_change`, `pages::acquisition` |
| The acquisition report (`/sales/acquisition-report`, `.csv`): leads, conversion, new customers, cost (linked to bills and manual), cost per lead, CAC and ROAS per source, campaign, channel, brand, branch or month, within the viewer's branch scope, "—" for an undefined ratio | §5.8 step 5 | `report`, `pages::report` |
| The kernel's `CampaignAttribution` and `LeadConversion` for the verticals | §6 | `attribution` |

The module owns `src/texts.rs`, the English texts (`wl_sales.*`). The app changes or
translates them in `resources/lang/{locale}.json`.
