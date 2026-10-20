# construction

The vertical app for fit-out and construction subcontractors ([spec](../../docs/verticals/02-construction.md)):
the project, its termination mid-way and the settlement of its final account, on top of the shared
`wl-*` modules. Construction reaches the other modules only through the contracts of `wl-kernel`
(attendance, purchasing and stock, tickets, notifications, invoices), never through their crates.

The host mounts it when `client.toml` names `app = "construction"`:

```
use renox::prelude::*;

# let _ =
App::new().module(construction::Construction)
# ;
```

What is here so far:

| Part | Spec | Module |
|---|---|---|
| Permissions (`projects.*`) and the roles the rules name | §3, §7.1 | `permissions` |
| The Project with its machine: `setup`, `in_progress`, `on_hold`, `practical_completion`, `terminated`, `closed`, `cancelled`; `start` (needs a budget), `hold`, `resume`, `terminate`, `settle_final_account` and `cancel` are built (`complete` and `close` come with the practical completion and the defects liability period) | §6, §7.1 *Project* | `project` |
| `terminate` (`CON.ACL.15`): the project manager requests it with the reason (terminated by the client, our default, main contractor insolvent, agreed), the termination notice and the effective date, which may be in the past; the kernel defers it as an approval request for the MD (never the requester) and the project keeps its state until then. At the approval the project is `terminated` (not an end), both retention parts are made due unless `retention_due_on_termination` is off (`⚠ open: lawyer (L-37)`) and the final account IPA is made: a `draft` IPA of the project becomes it, valued to the effective date, an IPA still `submitted` stays. After the commit: attendance for a day after the effective date is refused (the attendance module asks `ContextClosure`) and attendance already recorded after it is flagged for the reviewer; the project manager gets a task to reassign the crews; open purchase requests with no order are cancelled; procurement gets one task listing the open orders to cancel or short-close (nothing is cancelled for the supplier); a stock count of the site store is opened; the accountant gets one task per `active` guarantee; open snag and defect tickets are cancelled with the reason; the client contacts are e-mailed; `ProjectTerminated` is emitted | §3.3, §4.8, §7.1 `terminate`, `CON.ACL.15` | `termination` |
| The IPA, the Retention and the Guarantee with their states; only the final account IPA, `make_due` of a retention part and the active guarantees of a project are used so far | §6, §7.1 | `ipa`, `retention`, `guarantee` |
| The event `ProjectTerminated` | §7.1 | `events` |
| The back-office actions (terminate, closing screen, settle, cancel) | §7.1 | `pages` |

Not built yet: sites and geofences, the BOQ, the calculation and the other transitions of the IPA, the
retention and guarantee lifecycles beyond what is above, crews and their assignments (the reassign
task stands for ending them), the project pages and the client portal (read-only after the final
account, `POR.ACL.01`, BL-CON-19).
