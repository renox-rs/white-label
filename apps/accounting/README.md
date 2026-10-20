# accounting

The vertical app for accounting firms and fractional CFO services
([spec](../../docs/verticals/04-accounting.md)): the engagement with a client company, its CDD
file and tax profile, and the compliance calendar and retainer an activation starts, on top of the
shared `wl-*` modules. Accounting reaches the other modules only through the contracts of
`wl-kernel` (the party directory, the block and screening guards, the document guard, deadline
tasks, subscriptions and notifications), never through their crates.

The host mounts it when `client.toml` names `app = "accounting"`:

```
use renox::prelude::*;

# let _ =
App::new().module(accounting::Accounting)
# ;
```

What is here so far:

| Part | Spec | Module |
|---|---|---|
| Permissions (`engagements.*`, `cdd.*`, `tax_profiles.*`) and the role names the rules use (partner, engagement manager, accountant, tax specialist, MLRO, billing) | §3.1, §3.3 | `permissions` |
| The Engagement with its client party, services, fee basis and retainer, the signed letter and the neutral reason code; the CDD file with its screened parties (client company, UBOs, directors, signatories), rating and next review; the tax profile versioned by effective date | §6 | `engagement`, `cdd`, `tax_profile` |
| The Engagement machine (onboarding, active, suspended, ended, declined): `activate` refused until the CDD file is approved, the letter signed and the tax profile complete (every failing check in one refusal); `decline`, `suspend`, `resume` and `end` with a neutral reason, and their system twins | §7.1 *Engagement*, `ACC.ACL.07` | `engagement_machine`, `engagements`, `activation` |
| The CDD file machine (in progress, pending approval, approved, review due, rejected, exited): `submit` needs UBOs, identity documents and a rating; `approve` sets the next review from the rating; `reject` declines the onboarding engagements; the reject and exit reasons are kept for the MLRO only, never in the history | §7.1 *CDD file*, `ACC.KYC.02` | `cdd_machine`, `cdd_files` |
| On activation, after the commit: the compliance calendar (the next VAT return and corporate tax return as recurring deadline tasks) and the retainer subscription | §4.3, `ACC.CMP.01`, §7.1 *Subscription* | `calendar`, `retainer`, `events` |
| The neutral reason codes (*On hold*, *Ended*, …): a restricted reason is never stored on an engagement | §6, `ACC.ACL.14` | `reasons` |
| Neutral reasons for a restricted suspension or end: a client block suspends its active engagements with *On hold* (an already suspended one shows *On hold*), and an end after a CDD exit or rejection is always *Ended*; the page, the list, the history and the CSV export show only that code, the portal rows show *On hold* or *Ended* with no reason and nothing about a block, no automatic message goes to the client, and the MLRO or deputy chooses the neutral template (EN, AR) that tells the client, recorded on the engagement | §3.3 `ACC.ACL.14`, `PTY.ACL.02`, §4.6 step 7 | `reasons`, `engagements`, `client_notices`, `portal`, `pages::engagements` |
| `activate` also refuses, in the same refusal, an expired or missing KYC document of the client company or a UBO (the document guard `engagement.activate`; the manager's one-use override approved by the MLRO, valid 72 hours, covers one activation), a stale list or a latest screening run on one, an open possible match on any screened party (the client company, its UBOs, directors and signatories; "Screening pending" to users without `parties.block_reason.view`) and a blocked client ("On hold"; no exception); the screening and block checks run again in the transaction that moves the status | §7.1 `activate`, `DOC.ACL.02`, `PTY.RUL.02`, `PTY.ACL.03` | `activation`, `documents`, `screening` |
| CDD `submit` is refused while a screened party (the client company, its UBOs, directors and signatories) has no screening result, a list is stale or a party's latest screening ran on a stale list (naming each list and its last update); `approve` is refused while a list is stale, a latest screening ran on one, or a possible match is open (the MLRO sees which party; the preparer's page shows only "Screening pending"); both pass again after the list update and a `clear` re-run, and read the screening in the transaction that moves the status | §7.1 *CDD file* `submit`, `approve`, `PTY.RUL.02` | `cdd_files`, `screening`, `pages::cdd` |
| The client KYC document kinds that block `activate` | §9 *Document blocks* | `seed` |
| An expired KYC document (trade licence, a UBO's passport or Emirates ID) of a client with an `active` or `suspended` engagement opens one CDD update task per document and client for the compliance officer, due 14 days after the expiry, the engagement's manager told; no engagement or CDD file changes state; the accepted renewal completes the task | §9 *Document blocks* | `kyc_updates` |
| The engagement list and page with its history and actions, and the CDD file page | §3, §7.1 | `pages` |

The app owns `src/texts.rs`, the English texts (`accounting.*`). A client changes or translates them
in `resources/lang/{locale}.json`.
