# manpower

The vertical app for manpower supply and outsourcing agencies ([spec](../../docs/verticals/09-manpower.md)):
the client contract, the manpower request and the cover request, and the deployment of a worker to a
client site, on top of the shared `wl-*` modules. Manpower reaches the other modules only through the
contracts of `wl-kernel` (the party directory, the workforce, rate cards, attendance, the portal,
tickets, notifications), never through their crates.

The host mounts it when `client.toml` names `app = "manpower"`:

```
use renox::prelude::*;

# let _ =
App::new().module(manpower::Manpower)
# ;
```

This is the base of 09 (`Base 09`): the lifecycles the negative-flow issues BL-MPW-01 to BL-MPW-21 build
on. Each machine declares every state of the spec so those issues only add transitions.

What is here so far:

| Part | Spec | Module |
|---|---|---|
| Permissions (`contracts.*`, `sites.*`, `requests.*`, `deployments.*`), the client company and the site as scopes, and the role names the rules use | §3.1, §3.2 | `permissions` |
| The Site of a client | §6 | `site` |
| The Client contract with its machine: `draft`, `active`, `on_hold`, `terminating`, `ended`, `cancelled`; `activate` (needs the signed contract and a rate card version accepted by the client and in force on the start date, through the kernel's `RateCards`) and `cancel` (from `draft`, with a reason) are built. **`hold` and `release` come with BL-MPW-01 and BL-MPW-03, `terminate` with BL-MPW-04, `renew` with BL-MPW-05, `end` with BL-MPW-06** | §6, §7.1 *Client contract* | `contract`, `contracts` |
| The back-office actions and the contracts list (a page for a browser, JSON otherwise) | §7.1 | `pages` |
| The event `ContractActivated` | §7.1 | `events` |
| The Manpower request with its machine: `draft`, `open`, `partially_filled`, `filled`, `cancelled`; `submit` (the contract `active`, the dates within the contract and a rate card that covers the role and the shift, through the kernel's `RateCards`; the account manager is told), `fill_partly` and `fill` (the system's, one transition per target because a Renox transition has one: a deployment `activate` takes a position) and `cancel` (with a reason; `auto_cancel` is the system's twin for the contract's `terminate` and `end`) are built. Staff write requests for their client companies; the client approver (the portal roles `owner` and `approver`) for their own company only (`MPW.ACL.01`). The refusal does not mention a hold. **The refusal while the contract is on hold is tested with `hold`, BL-MPW-02; the blocked-client guard comes with BL-MPW-15** | §6, §7.1 *Manpower request*, `MPW.ACL.01` | `request`, `requests` |
| The requests list (a page for a browser, JSON otherwise): staff see the client companies they work for, a client user only their own company | §3.3 `MPW.ACL.01` | `pages` |
| Notices to the account manager of a client (the user on the contract, else the holders of the role in the client company) and to the client company | §4.1, §7.1 | `notices` |
| The Deployment with its machine: `planned`, `active`, `suspended`, `ended`, `cancelled`; `activate` (the employee `active` in HR through the kernel's `Workforce::is_active`, the contract `active` or `terminating`, the end date not past, the request it fills still taking workers; the request takes a position in the same transaction with `fill_partly` or `fill`; the client company is told by name), `extend` (the end date later, within the contract, while the contract takes deployments), `end` (a user gives *end date*, once it has come, or *replaced*; `auto_end` is the system's and takes any of the nine reasons) and `cancel` (while planned, with a reason; `auto_cancel` is the system's, used when a request is cancelled) are built; the daily run at 02:00 Asia/Dubai (`manpower:deployment-end-daily`) ends the active and suspended deployments whose end date has passed. **`suspend`, `resume` and `extend` of a suspended deployment come with BL-MPW-11 and BL-MPW-12; the reasons *client rejected*, *client non-payment* and *entered in error* with BL-MPW-14, *contract end* with BL-MPW-06, *exit*, *absconded* and *deceased* with BL-MPW-13; the document and licence check `MPW.ACL.07` with BL-MPW-08, the overlap check `MPW.ACL.15` with BL-MPW-10, the blocked-client check with BL-MPW-15. Closing the open timesheet of the period at `end` comes with BL-MPW-06, which owns the timesheet contract** | §6, §7.1 *Deployment* | `deployment`, `deployments` |
| Cancelling a request cancels its planned deployments in the same transaction (`auto_cancel`, reason *request cancelled*); a deployment already `active` stays | §7.1 *Manpower request* `cancel` | `requests` |
| Replacements and cover (`MPW.DEP.02`): a request of the kind `replacement`, one worker with the role, site and shift of the deployment covered, for the days the worker is away (open-ended for a suspension), opened at once; one per cause; the account manager is told and `CoverRequested` is emitted. A worker planned for it is a `cover` deployment of the one covered, runs to the end of the cover period by default and fills the request. The account manager may open one by hand for a worker on site. `cover::open_cover` is what BL-MPW-11 (suspension) and BL-MPW-14 (client rejection) call; **the notice of cover not found in time, `MPW.RUL.13`, is BL-MPW-16** | §4.1, §4.6, `MPW.DEP.02` | `cover` |
| `MPW.RUL.06`: when a leave request is approved (the kernel's `LeaveApproved`, which wl-hr emits once its leave lifecycle is built) every `active` deployment of the employee that overlaps the leave gets a cover request for the days of the overlap, when its contract requires cover; the same leave twice changes nothing | §4.6, §7.2 | `cover` |
| The deployments list (a page for a browser, JSON otherwise): staff see the deployments of their client companies and sites; a client user sees only the workers on site of their own company (`MPW.ACL.01`), by name, role, site, shift and dates (`MPW.ACL.02`) | §3.2, §3.3 | `pages` |
| The documents a deployment depends on (`BlockSubjects` for `deployment.activate`, `deployment.resume` and `deployment.extend`: the employee, filtered by the entry's condition on the role, scoped to the site; `FutureWork` for `deployment.activate`: the deployments `planned` after the expiry date and those `active` or `suspended` that run past it, or have no end date; the nightly `DOC.RUL.02` run adds them to `DocumentExpiring.affected` with the client company as their scope, so the kind's responsible role (the account manager, when the kind lists it in `also_notify`) gets one message per document per night). A listed deployment is flagged *document expiring <kind> <date>* in the deployments list until a renewed document of the kind is accepted. **The guard itself (`MPW.ACL.07`) comes with BL-MPW-08; the kinds and their `blocks` are the client's data** | §4.5 step 3, §9 | `documents` |
| The events `DeploymentActivated`, `DeploymentEnded`, `DeploymentCancelled` and `CoverRequested` | §7.1 | `events` |

Kernel contracts the base added: `RateCards` (`wl_kernel::rate_cards`; wl-invoicing provides it with its rate card versions, `MPW.ACL.16` (BL-MPW-07); without wl-invoicing no contract has a card and `activate` of a contract and `submit` of a request are refused), `Workforce::is_active` (provided by wl-hr) and the event `LeaveApproved` (wl-hr emits it with its leave lifecycle).

Not in the base: the access rules `MPW.ACL.04` (bill rates and margins, with the reports), `MPW.ACL.06`, `MPW.ACL.09` and `MPW.ACL.10` (HR, timesheets and the worker portal) and the shared `HRM.ACL.*`, which their modules own; `MPW.ACL.07`, `MPW.ACL.14`, `MPW.ACL.15` and `MPW.ACL.17` come with BL-MPW-08, -02 and -03, -10 and -04; `MPW.ACL.16` (rate card versions, pricing by the version in force) is wl-invoicing's, BL-MPW-07. The candidate pipeline (`MPW.REC.01`) is a later scope of the spec.
