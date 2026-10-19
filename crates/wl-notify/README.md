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
| Statutory notice records, `notice_status`, `undelivered_notices` | §5.4, §6 | `notices` |
| The `statutory_notice` give-up reaction and its task | §5.4 | `statutory_notice` |
| Proof of delivery: form, attachment, `StatutoryNoticeProofRecorded` (`notifications.notices.record_proof`) | §5.4 | `pages` |
| Marketing frequency cap: `WlNotify::marketing_frequency_cap(n)` (default 2 per 7 days, any channel and broadcast), `FREQUENCY_CAP` suppression under a lock per party | §6 `NTF.RUL.02`, §7 | `cap`, `send` |

`types::install` also fills the default give-up reaction (kernel `default_give_up`, §5.4) into a
stored type whose reactions are empty and which the client never edited; edited rows are kept.

The module owns `src/texts.rs`, the English texts (`wl_notify.*`). The app changes or
translates them in `resources/lang/{locale}.json`.
