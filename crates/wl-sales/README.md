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
| The public web form `POST /enquiries?c=CODE` (no login): a lead with the campaign and the source it implies, else *website* | §4.1, §5.8 | `pages::enquiry`, `lead` |
| Campaigns, their machine (`activate`, `end`, `reopen`), promo codes, the events and the owner's end notice | §3, §4.6 | `campaign`, `machine`, `promo_code`, `notices` |
| The daily task `sales:campaigns` (00:05 Asia/Dubai) and `campaigns::run_daily` | §5 | `daily` |
| Leads: capture with the source and campaign of a web form code (late-response days), notes, the machine (`contact`, `qualify`, `win`, `lose`, `reopen`, `revert_win`), `LeadWon`/`LeadLost`/`LeadWinReverted` | §3, §4.1, §5.1 | `lead`, `lead_machine`, `lead_events` |
| Party acquisitions: source and campaign per party (first touch, never overwritten), the first sale with its converted value (net of credit notes, or the attribution window), `CustomerAcquired`, the source check of a sale for a new party, the nightly `sales:acquisitions` run that replaces a first sale that no longer stands, and the party merge | §3, §5.8 | `acquisition`, `acquisition_daily` |
| Changing an acquisition or a won lead (`SAL.ACL.09`): a reason, a second `campaigns.manage` holder, the audit record and `CustomerAcquisitionChanged`; the exceptions report page | §5.8, §6 | `acquisition_change`, `pages::acquisition` |
| The acquisition report (`/sales/acquisition-report`, `.csv`): leads, conversion, new customers, cost (linked to bills and manual), cost per lead, CAC and ROAS per source, campaign, channel, brand, branch or month, within the viewer's branch scope, "—" for an undefined ratio | §5.8 step 5 | `report`, `pages::report` |
| The kernel's `CampaignAttribution` and `LeadConversion` for the verticals | §6 | `attribution` |
| Prepaid packages: `sell`, `activate` when the invoice is paid, `consume` / `consume_last` with a movement per source, `reverse`, the daily `expire` (`sales:packages`), `extend`, `cancel`; the kernel's `PackageLedger` and `PackageActivated`/`PackageConsumed`/`PackageExtended`/`PackageExpired`. The sale pages and `refund` come later | §3, §4.4, §5.5, §6 | `package`, `package_machine`, `packages` |
| A reversed payment of a package (`InvoiceReopened`, `PaymentReversed`): the package stays active with its used credits, the seller gets a "collect before next use" task, the holder is told the balance is due, and `WlSales::suspend_package_consumption_while_unpaid` refuses `consume` until the invoice is paid | §4.4, §6, §7 | `package_reopen`, `packages` |
| Counter sales: a cart with sale lines, the machine (`complete`: tenders that add up, the customer account only with the setting), the receipt number, the invoice drafted for a named customer, `CounterSaleCompleted`, and the `/counter-sales` pages behind `pos.sell`. A completed sale is never edited (`SAL.ACL.04`); the till session lifecycle comes later | §3, §4.3, §5.4, §6 | `counter_sale`, `counter_sale_machine`, `counter_sales`, `sale_line`, `pages::counter_sales` |
| Sales orders: all ten states, the machine (`submit`, `submit_online`, `submit_for_acceptance`, `confirm_payment`, `payment_failed`, `accept`, `reject`, `cancel`, `fulfil`/`fulfil_last`, `reverse_fulfilment`/`reverse_fulfilment_all`, `close`), idempotent hand-overs through the kernel's `OrderFulfilment`, the invoice drafted on `fulfilled`, the `SalesOrder*` events and the `/sales-orders` pages. `hold`, `release`, `amend` and the line cancellations come with their issues | §3, §4.2, §5.2, §5.3, §6 | `sales_order`, `sales_order_machine`, `sales_orders`, `pages::sales_orders` |

The module owns `src/texts.rs`, the English texts (`wl_sales.*`). The app changes or
translates them in `resources/lang/{locale}.json`.
