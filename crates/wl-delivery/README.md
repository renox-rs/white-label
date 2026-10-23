# wl-delivery

Delivery: deliveries grouped onto runs, a driver and a vehicle for each run, the driver's day
list on the phone, and the outcome of each stop with its proof (photo, signature or OTP, time
and location) and the delivered and refused quantity of every line
([spec](../../docs/modules/wl-delivery.md)). The module does not know what is delivered: the
source is a `Subject`, asks for a delivery through the kernel's `DeliveryRequester`, and reacts
to the delivery events of `wl-kernel` (`DeliveryAssigned`, `DeliveryOutForDelivery`,
`DeliveryDelivered`, `DeliveryAttemptFailed`, `RunDispatched`, `DeliveryDisputed`, `DeliveryLost`, `RunClosed`, …). No crate depends on this one.

The host mounts it when `client.toml` enables `delivery`:

```
use renox::prelude::*;

# let _ =
App::new().module(wl_delivery::WlDelivery::new())
# ;
```

What is here so far:

| Part | Spec | Module |
|---|---|---|
| Permissions (`deliveries.*`, `runs.*`, `pod.view`) | §6 | `permissions` |
| The Delivery lifecycle: `assign`, `unassign`, `dispatch` (the system, with the run), `cancel`, `deliver`, `fail`; every state of §4.1 is declared | §4.1 | `delivery`, `plan`, `proof` |
| The Run lifecycle: `planned` to `dispatched` | §4.2 | `run`, `plan` |
| Asking for a delivery (`Deliveries`, the kernel's `DeliveryRequester`): lines with quantities in thousandths and optional amounts, the proof required and partial acceptance copied from the settings of the source type | §3, §6, §7 | `request`, `settings` |
| The driver's day list and stop screen (the phone and full address only while the stop is open; another driver's stop is 404) | §5.2, `WRK.ACL.05` | `day_list`, `pages` |
| The driver and vehicle document guard (`delivery.assign` on the run's date, `run.dispatch` today): the run is the guarded record, its driver (employee) and vehicle (asset) the subjects, the kinds come from the document kinds' `blocks` | §4.1, §4.2, #157 | `doc_guard` |
| Proof of delivery and partial acceptance: the required methods, delivered plus refused equal to the quantity, a reason (and a photo for *damaged*) for a refused quantity, at least one unit delivered, the expected cash after refused lines; the proof and quantities are saved in the transition and never edited | §4.1, §5.2, `WRK.POD.01`, #158 | `proof` |
| A disputed delivery under review: `open_review` (a user, or the system for a dispute ticket) freezes its cash and billing and opens a ticket; `close_review` as upheld with a note releases them; the kernel's `is_frozen` and `remittable_cod` tell the remittance and the sources | §4.1, §5.5, #160 | `review`, `trace` |
| Lost and found: `WRK.RUL.17` marks a delivery lost after 7 days untraced (daily at 02:00 Asia/Dubai), a manual `mark_lost` needs a second holder of `deliveries.reverse` and is re-checked at final approval (`WRK.ACL.10`), one claim ticket per loss, `found` puts it back to `attempted` | §4.1, §5.5, `WRK.RUL.17`, #162 | `lost`, `trace` |
| Run close: the driver brings the run back (`return_to_depot`), the depot ticks each undelivered delivery and refused line received or missing and counts the cash, then the system `close`s the run, records the quantities on the lines and emits `DeliveryRefusedReceived` and `RunClosed` | §4.2, §5.4, #167 | `close` |
| After a failed attempt: `reattempt` and `return` (`deliveries.manage`, never a reattempt for a perishable delivery, `return` only once the goods are confirmed at the origin) and `discard` (the run's driver, with a photo and a reason). `WRK.RUL.16` reads the policy of the source type (`perishable_policy`, default *discard*; 06: *return* after a failed visit, *discard* after an abort) at the failed attempt and either waits for the goods at the origin or gives the driver the "discard with photo" task; the wastage goes to the stock books through the kernel's `DeliveryWastage` with the source as reference, above the limit (`discard_approval_limit`, `STK.ACL.02`) after a second person approves; `DeliveryReturned` and `DeliveryDiscarded` follow. `WRK.RUL.08` (reattempt while attempts are left, else return) is off unless `attempt_rule(true)` | §4.1, §5.2, `WRK.RUL.08`, `WRK.RUL.16`, #163 | `attempted`, `settings`, `notices` |
| A perishable stop is handed over to another run only with the arrival the dispatcher expects on it, not after the source's placed time plus its delivery promise (`DeliveryRequest::promise`, `promise_minutes`); a refused stop stays on the giving run | §4.2, §5.3, #166 | `hand_over` |
| The configurable failure and refusal reasons, seeded with the §7 defaults | §3, §7 | `reasons`, `seed` |

The module owns `src/texts.rs`, the English texts (`wl_delivery.*`). The app changes or
translates them in `resources/lang/{locale}.json`.
