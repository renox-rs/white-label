# wl-notify

Notifications: the types modules register, the messages sent from them on email (and SMS
when a gateway is configured), with retry, fallback to the next channel and the give-up
reaction ([spec](../../docs/modules/wl-notify.md)). Other modules ask for a message only
through the kernel (`wl-kernel`), never through this crate.

The host mounts it when `client.toml` enables `notify`:

```
use renox::prelude::*;

# let _ =
App::new().module(wl_notify::WlNotify::new())
# ;
```

What is here so far:

| Part | Spec | Module |
|---|---|---|
| Permissions (`notifications.*`) | §6 | `permissions` |

`types::install` also fills the default give-up reaction (kernel `default_give_up`, §5.4) into a
stored type whose reactions are empty and which the client never edited; edited rows are kept.

The module owns `src/texts.rs`, the English texts (`wl_notify.*`). The app changes or
translates them in `resources/lang/{locale}.json`.
