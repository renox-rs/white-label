# wl-kernel

The base every `wl-*` module and vertical app stands on
([spec](../../docs/modules/wl-kernel.md)). It turns one Renox binary into one client's
branded product.

The host reads the client's `client.toml` and applies the kernel to the app:

```
use renox::prelude::*;
use wl_kernel::ClientConfig;

let config = ClientConfig::parse(r#"
    [client]
    name = "Acme Properties LLC"
    app = "property"

    [modules]
    enabled = ["parties", "invoicing"]
"#)?;
let app = wl_kernel::apply(App::new(), config)?;
# let _ = app;
# Ok::<(), wl_kernel::ConfigError>(())
```

`apply` fails, and the boot stops, when a module needs one that is not enabled, when a
brand colour is not readable (the message names the nearest colour that is), or when the
file is malformed.

What is here so far:

| Part | Spec | Module |
|---|---|---|
| `client.toml` and the module manifest | §3, §5.1 | `config`, `manifest` |
| Branding on Renox `Theme::from_brand` | §2 `PLT.ORG.01` | `branding` |
| `Subject` references | §2 | `subject` |
| Legal entities, scope links, issuer snapshots (`PLT.ENT.01`) | §5.6, decision 0003 | `entities` |
| Document numbering on Renox `Sequence` | §5.5 | `numbering` |
| State machines by name: `SubjectMachine`, `back_to_<state>` and `<action>_approved` (always `system`) | §4.1, WORKFLOW §3.4 | `machines` |
| The party contract: `PartyDirectory` (`NoParties` when `parties` is off), `PartyStatus`, party events, the `party` series, `contacts` (`PartyContacts`) | ARCHITECTURE §5, wl-parties §6 | `parties` |
| The notice contract: `Notifier` (`NoNotifier` when `notify` is off), `register_type`, `NotifyRequest`, message events | ARCHITECTURE §5, wl-notify §6 | `notify` |
| `AutomationFailed`, the event of a failed rule run | §4 | `automation` |

After `rnx migrate`, `rnx wl:install` creates the legal entities `client.toml` lists and
the database lacks; it never overwrites an entity changed in the admin. `rnx db:seed`
runs it too.

The page `/legal-entities` needs `legal_entities.view`. The app overrides it with
`resources/views/wl-kernel/legal_entities.html`, and translates the texts (`wl_kernel.*`)
in `resources/lang/{locale}.json`.

Other modules read parties only through `wl_kernel::parties::directory(&state)` and listen
to `PartyCreated`, `PartyBlocked` and `PartyUnblocked`. Summaries and events carry the
display block reason: `on_hold` (`NEUTRAL_BLOCK_REASON`) when the real one is restricted.

Every notice goes through one contract. A module registers its notification types in
`register` with `wl_kernel::notify::register_type(NotificationTypeDef::new(..))` (statutory
flag, give-up reactions, staff role told, channels in order, default texts with `{{ name }}`
placeholders), then sends with
`notify::notifier(&state).notify(&state, NotifyRequest::new(type, subject).to_party(p).to_user(u).data(..))`.
A recipient is a staff user (by user id), a party (by `PartyId`, reached through
`PartyDirectory::contacts`) or a bare address. `wl-notify` provides the real notifier and
emits `MessageFailed`, `MessageSuppressed` and `MessageGaveUp`. When `notify` is off,
`NoNotifier` still gives each user recipient a Renox database notification (the bell) with
the type's text, skips party and address recipients, and returns no message ids.

Migrations are SQLite only: each client runs one SQLite database
([ARCHITECTURE.md](../../docs/ARCHITECTURE.md) §1).
