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
