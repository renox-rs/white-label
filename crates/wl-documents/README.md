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
| Document kinds and their `blocks`: `KindSeed`, the seeder that locks legal blocks again, `blocks_for`, client changes that leave locked entries alone | §3, §5.6, §7 | `kinds` |

The module owns `src/texts.rs`, the English texts (`wl_documents.*`). The app changes or
translates them in `resources/lang/{locale}.json`.
