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

After `rnx migrate`, `rnx wl:install` creates the legal entities `client.toml` lists and
the database lacks; it never overwrites an entity changed in the admin. `rnx db:seed`
runs it too.

The page `/legal-entities` needs `legal_entities.view`. The app overrides it with
`resources/views/wl-kernel/legal_entities.html`, and translates the texts (`wl_kernel.*`)
in `resources/lang/{locale}.json`.

Migrations are SQLite only: each client runs one SQLite database
([ARCHITECTURE.md](../../docs/ARCHITECTURE.md) §1).
