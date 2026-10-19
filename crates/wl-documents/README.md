# wl-documents

Documents attached to records: their kinds, versions, review and expiry, and the guard that
blocks a transition while a required document is missing or expired
([spec](../../docs/modules/wl-documents.md)). Other modules reach documents only through
the kernel (`wl-kernel`), never through this crate.

The host mounts it when `client.toml` enables `documents`:

```
use renox::prelude::*;

# let _ =
App::new().module(wl_documents::WlDocuments)
# ;
```

What is here so far:

| Part | Spec | Module |
|---|---|---|
| Permissions (`documents.*`, `document_kinds.manage`) | §6 | `permissions` |
| Document kinds and their `blocks`: `KindSeed`, the seeder that locks legal blocks again, `blocks_for`, client changes that leave locked entries alone, `set_enabled` (a kind that is off takes no new documents; the ones it has stay) | §3, §5.6, §7 | `kinds` |
| The seeded kinds for registrations and certifications (trademark, Halal, ISO, product registration / conformity, other certification): EN/AR names, expiry and reminders, no `blocks`, on or off per vertical, the responsible role per vertical (`register_vertical`) | §3, §7, `DOC.EXP.01` | `registrations` |
| SOP acknowledgements (`DOC.RUL.04`, `HRM.ACL.10`): a reader per SOP version, requested when a version is published or a reader joins or changes role, replaced by a newer version, cancelled when the reader leaves, the role or scope ends, the readers change or the SOP is withdrawn; own session (`click-to-accept`, with the opened file's language and checksum) or in person with a drawn signature and a witness; daily reminders every 3 days and one escalation to the manager and the owner role after 7; the staff inbox, the in-person list and the report on the SOP page; the kernel contract `SopAcknowledgements` wl-hr's onboarding asks | §3, §4.3, §5.7, §6 | `acknowledgements` |

The module owns `src/texts.rs`, the English texts (`wl_documents.*`). The app changes or
translates them in `resources/lang/{locale}.json`.
