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
| Notification types as the client has them, their templates in English and Arabic and the rendering of a message | §3 | `types`, `render` |
| The kernel's `Notifier`: a message per recipient and channel with its language and address | §6 | `notifier`, `recipients` |
| The Message and its machine: `send` through a channel provider (`ChannelProvider`, errors told apart), `fail`, `retry`, `fall_back` to the next channel, `give_up` (`NTF.RUL.01`), the suppression list | §3, §4.1, §8 | `message`, `machine`, `channels`, `send`, `give_up`, `suppressions` |
| The give-up reactions: `staff_task` (with an in-app alert when urgent), `alternative_recipient`, `mark_contact_invalid`, the OTP failure notice, the "not delivered" query for a record | §5.4 | `reactions`, `staff_task`, `alert`, `alternative`, `contact_invalid`, `otp`, `delivered`, `mask` |
| Send windows and quiet hours (`NTF.RUL.02`), deferred messages released when the window opens (`notify:release`) | §4.1, §7 | `window`, `send_window`, `release` |
| Broadcasts (`NTF.ACL.04`): one marketing message to a segment asked for by a campaign through `Notifier::broadcast`, approval above the size limit, the scheduled run every minute (`notify:broadcasts`), results, opt-out of marketing | §3, §4.3 | `broadcast`, `broadcast_machine`, `broadcasts`, `broadcast_approval`, `broadcast_recipients`, `broadcast_run`, `broadcast_results`, `opt_out` |
| Statutory notice records, `notice_status`, `undelivered_notices` | §5.4, §6 | `notices` |
| The `statutory_notice` give-up reaction and its task | §5.4 | `statutory_notice` |
| Proof of delivery: form, attachment, `StatutoryNoticeProofRecorded` (`notifications.notices.record_proof`) | §5.4 | `pages` |
| Marketing frequency cap: `WlNotify::marketing_frequency_cap(n)` (default 2 per 7 days, any channel and broadcast), `FREQUENCY_CAP` suppression under a lock per party | §6 `NTF.RUL.02`, §7 | `cap`, `send` |

`types::install` also fills the default give-up reaction (kernel `default_give_up`, §5.4) into a
stored type whose reactions are empty and which the client never edited; edited rows are kept.

The module owns `src/texts.rs`, the English texts (`wl_notify.*`). The app changes or
translates them in `resources/lang/{locale}.json`.
