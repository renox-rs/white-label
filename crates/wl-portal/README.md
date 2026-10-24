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
| Sections and the menu (`PortalSection` with audience, portal permission, order and label; `register_section`): the home `GET /portal/{party}` lists the sections the grants' portal roles allow, in order, with a party switcher (`GET /portal` opens the first party); `GET /portal/{party}/{section}` lists a section's rows, a 403 for a section of another audience or one the roles do not allow; the block is never read for either | §6, BL-POR-02 | `sections` |
| Audiences and portal roles (`AudienceDef`, `audiences::seed`; permissions are the union over a user's grants for the party); the company admin changes a role's permissions at `GET /portal/roles` (`portal_users.manage`) | §3, §7, BL-POR-02 | `audiences`, `settings` |
| Read-only grants and the close-after timer: `PortalAccess::contract_ended` runs `end` for the vertical's contract-end event (the host maps 01 `LeaseTerminated` / `LeaseForceTerminated`), a `read_only` grant reads every section with the notice `Your access is read-only since <date>` and its write actions are refused (the deposit deductions dispute is kept); the daily task `portal:close-read-only` closes it after the audience's period (day 91 for 90 days) and ends the user's sessions | §4, §7, `POR.ACL.01`, BL-POR-03 | `access`, `guard` |
| Who acts for a party (`GrantUsers`, the kernel's `PortalUsers`): the users of the `active` grants of a party on a day, with their roles, and whether a user acts for it | §3, §6 | `users` |
| Signed links (`SignedLinks::issue`, `GET /portal/link/{party}/{action}`): a payment link keeps working while the party is blocked, a link whose action is new business shows the neutral text and creates nothing (`PortalGuard::require_link`); `SignedLinkUsed` | §5.4, §6 | `links` |

Invitations (tokens, mail), login methods, the other signed-link actions (tracking, confirmation, approval), the
module-specific sections (leases, invoices, statements, refund requests) and request lists are other issues.

The module owns `src/texts.rs`, the English texts (`wl_portal.*`) and the Arabic neutral text. The app
changes or translates them in `resources/lang/{locale}.json`.
