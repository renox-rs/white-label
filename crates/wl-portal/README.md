# wl-portal

The front door for external users: the access grant that says whom a portal user may act for, and the
neutral refusal of new business and money out while that party is blocked
([spec](../../docs/modules/wl-portal.md)). Other modules reach the portal's rules only through the
kernel (`wl-kernel`) and the guard of this crate.

The host mounts it when `client.toml` enables `portal`:

```
use renox::prelude::*;

# let _ =
App::new().module(wl_portal::WlPortal)
# ;
```

What is here so far:

| Part | Spec | Module |
|---|---|---|
| Permissions (`portal_users.*`, and `parties.block_reason.view` for the refusal log) | §6 | `permissions` |
| The access grant and its lifecycle (`invited`, `active`, `suspended`, `read_only`, `closed`, `expired`, `revoked`; `accept`, `expire`, `revoke`, `suspend`, `reinstate`, `end`, `close`), self-registration, and the events `PortalAccessActivated`, `PortalAccessSuspended`, `PortalAccessReadOnly`, `PortalAccessClosed` | §3, §4, §6 | `grants`, `access`, `events` |
| New business and money out refused with one neutral text (English or Arabic) while the party the user acts for is blocked, through the kernel `BlockGuard`; the list of refused actions is closed (a new booking, order or agreement, a refund or deposit-release request, a cancellation that refunds); paying an invoice and every other action work as usual; grants and messages are untouched by `PartyBlocked` | §6, §7, `PTY.ACL.02` | `guard`, `texts` |
| The refusal log (portal user, party, action, time) for staff with `parties.block_reason.view` | §6 | `refusals` |

Invitations (tokens, mail), portal roles with their permissions, login methods, signed links, request
lists and the close-after timer are other issues.

The module owns `src/texts.rs`, the English texts (`wl_portal.*`) and the Arabic neutral text. The app
changes or translates them in `resources/lang/{locale}.json`.
