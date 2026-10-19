# property

The vertical app for property management companies ([spec](../../docs/verticals/01-property.md)):
the portfolio, the lease and its lifecycle, and the lease-to-cash cycle on top of the shared `wl-*`
modules. Property reaches the other modules only through the contracts of `wl-kernel` (the
invoicer, the cheque register, the deposit book, the document guard, the party directory, tasks and
notifications), never through their crates.

The host mounts it when `client.toml` names `app = "property"`:

```
use renox::prelude::*;

# let _ =
App::new().module(property::Property)
# ;
```

What is here so far:

| Part | Spec | Module |
|---|---|---|
| Permissions (`units.*`, `leases.*`) and the roles the rules name | §3, §7.1 | `permissions` |
| Buildings, floors and units with type, use, size, bedrooms and asking rent | §6, `PRP.PRT.01` | `building`, `unit` |
| The Unit machine: available, reserved, occupied, notice given, under maintenance, off the market | §7.1, `PRP.PRT.02` | `unit_machine` |
| The Lease, one record per version (new, renewal, amendment), with the tenant as a wl-parties party | §6, `PRP.LSE.01` | `lease`, `leases` |
| The Lease machine with all its states and transitions; `submit`, `activate`, `cancel`, `flag_renewal`, `expire`, `renew`, `record_notice`, `withdraw_notice` and `terminate` are built, the others declare their states and permissions for BL-PRP-08 and BL-PRP-12 | §7.1 | `lease_machine`, `lease_checks` |
| The payment plan and the charge schedule of a version; the daily run drafts one tax invoice per instalment on its due date through the `Invoicer` | §4.1, `FIN.BIL.01` | `charge`, `invoicing` |
| The post-dated cheque schedule against the lease as its agreement, each cheque linked to the charge it pays | §4.1, `FIN.CHQ.01` | `cheques` |
| The document guard hooks: the tenant's documents on `submit`, the signed contract on `activate`, the Ejari certificate when the registration is recorded | §7.1, §7.2, `PRP.LSE.02` | `documents` |
| Ejari or Tawtheeq registration typed in, with the task that chases it after activation | §8, `PRP.RUL.02` | `leases`, `lease_events` |
| The renewal window opens 120 days before the end date (or the contract's notice period + 30); the notice deadline (end date − 90 days, or the contract's period) is shown on the lease; the renewal decision and the statutory notice of changed terms are the property manager's; `PRP.RUL.09`, compliance-critical, warns 14 and 3 days before the deadline and sets the current-terms-only marker the day after it; the daily command `property:renewals` | §4.3, `PRP.RUL.01`, `PRP.RUL.09` | `renewal`, `notices` |
| A renewal or amendment with changed terms against the tenant is refused at `submit` unless the notice of changed terms was delivered in time (renewal) or the tenant's signed acceptance is attached to the draft; the answer *no* to "against the tenant?" needs a reason, kept in the history | §3.3, §4.3, `PRP.ACL.17` | `terms`, `leases` |
| A lease that passes its end date becomes `expired` and stays billed: the charge schedule is extended on the same terms for the statutory renewal period (the lease's own period or one year), again for each period it holds over, and the property manager gets a task 7 days later and at each period | §2, §4.3, §7.1, `PRP.RUL.03`, `PRP.RUL.14` | `holding`, `renewal` |
| A renewal version that is activated ends the old version as `renewed`, also from `expired`: the versions are linked both ways, the old version's scheduled charges from the new start date are cancelled, the invoices of a holding-over period stay and the new schedule skips the periods already invoiced; the rent difference per period is shown on the new version and the accountant gets one task to relink the cheques of cancelled charges | §7.1 `renew`, §4.3 | `renew` |
| A notice to end the lease is recorded from `active`, `renewal_due` or `expired` with its date, party and reason: a tenant's notice needs the planned move-out date and may carry the early termination penalty of the contract, a charge of the configured *early termination fee* type due on the planned date (moved with it while not invoiced); a landlord's notice is a statutory notice that shows "notice not delivered" until its proof is recorded, and its planned move-out date is computed from the proof date; the unit moves to `notice_given` and the move-out inspection ticket is created | §7.1 `record_notice`, `PRP.RUL.04` | `notice`, `charge_type` |
| `withdraw_notice` with the landlord side's written agreement attached (`PRP.ACL.20`): the lease goes back to the state its dates give (`active`, `renewal_due`, or `expired`, running `expire`'s effects when no extension exists yet), the notice is cleared, the unit returns to `occupied`, the move-out inspection ticket is closed and a penalty that is not invoiced is cancelled; refused after the move-out inspection and once a lease for another tenant on the unit was submitted | §3.3, §4.3, §7.1 `withdraw_notice` | `withdraw` |
| The move-out inspection and the keys are recorded on the lease; `terminate` needs both, stores the move-out date, vacates the unit (to maintenance when the inspection found defects) and cancels the recurring charges after the move-out date (or after `rent_due_until`); after the commit the invoices of later periods are credited pro rata by partial credit note (rounded half up to a fils), later cheques are returned against one receipt, the deposit settlement starts and the tenant's portal turns read-only (`LeaseTerminated`); a step that fails becomes one task for the accountant. `PRP.RUL.13` chases the Ejari cancellation 7 days later | §4.3, §7.1 `terminate`, `PRP.RUL.13`, `POR.ACL.01` | `termination` |
| The events `LeaseActivated`, `LeaseCancelled`, `LeaseNoticeWithdrawn` and `LeaseTerminated` | §7.1 | `events` |
| The portfolio, the building, the leases and the lease pages | §3, §4 | `pages` |

The app owns `src/texts.rs`, the English texts (`property.*`). A client changes or translates them in
`resources/lang/{locale}.json`.
