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

The module owns `src/texts.rs`, the English texts (`wl_sales.*`). The app changes or
translates them in `resources/lang/{locale}.json`.
