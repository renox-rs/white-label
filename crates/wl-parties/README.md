# wl-parties

The parties a client deals with: people and companies, their roles, contact points and
blocks ([spec](../../docs/modules/wl-parties.md)). Other modules reach parties only through
the kernel (`wl-kernel`), never through this crate.

The host mounts it when `client.toml` enables `parties`:

```
use renox::prelude::*;

# let _ =
App::new().module(wl_parties::WlParties)
# ;
```

What is here so far:

| Part | Spec | Module |
|---|---|---|
| Permissions (`parties.*`) | §6 | `permissions` |
| Relationships (`contact_of`, `guardian_of`, …), `relate`, `Directory::related_contacts` | §3 | `relationships`, `directory` |

The module owns `src/texts.rs`, the English texts (`wl_parties.*`). The app changes or
translates them in `resources/lang/{locale}.json`.
