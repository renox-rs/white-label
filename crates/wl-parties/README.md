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
| The Party register (person or company, code from the kernel's `party` series), the person and company details, roles, contact points (one primary per type, `valid` until a hard failure), `create` with `PartyCreated` | §3, §5.1 | `party`, `details`, `roles`, `contacts`, `create` |
| The Party machine (`active`, `blocked`, `archived`, `merged`); `restore` of an archived party (`archive` itself is not built yet) | §4 | `machine`, `restore` |
| Blocking (`PTY.ACL.02`): `block` with the reason kept on a Block record, `unblock` with its evidence, who sees which reason (no tipping off), the neutral on-hold message, review tasks per agreement kind whose block effect is `review` | §4, §5.6 | `block`, `block_record`, `unblock`, `evidence`, `visibility`, `notify`, `review_tasks`, `block_effects` |
| The kernel's `BlockGuard` (refuses new agreements and money out to a blocked party; exceptions through an approval) and `PartyDirectory` | §6 | `block_guard`, `directory` |
| Sanctions screening (`PTY.SCR.01`): a `Screening` per run with its match candidates and review task, `confirm_match` with the freeze and report tasks, the manual check of a PDF-only list, the hand upload of a list, the re-run of screenings made on a stale list and the stale-list alert (`PTY.RUL.02`) | §3, §4, §5.4 | `screening`, `screening_task`, `confirm_match`, `compliance_tasks`, `manual_check`, `list_upload`, `list_parser`, `rescreen`, `freshness`, `freshness_alert`, `events` |
| The party list, the party page and the forms | §6 | `pages` |
| Sanctions lists (`SanctionsList`), `ListFeed`, `is_stale`, `WlParties::sanctions_freshness_hours` | §3, §7 | `sanctions_lists`, `feed` |
| `ScreeningGuard` (warn or block onboarding on a stale list), `WlParties::stale_list_at_onboarding`; provided only when screening is on | §7 | `screening_guard` |
| Relationships (`contact_of`, `guardian_of`, …), `relate`, `Directory::related_contacts` | §3 | `relationships`, `directory` |
| A company's e-invoicing recipient identifier (`set_recipient_id` emits `PartyUpdated` with the field name only, `lookup_recipient` asks the ASP by TRN and saves nothing), `BillingIdentity::einvoice_recipient_id`, the page `/parties/{id}/einvoice` | §3, §6 | `einvoice`, `directory`, `pages` |

The module owns `src/texts.rs`, the English texts (`wl_parties.*`). The app changes or
translates them in `resources/lang/{locale}.json`.
