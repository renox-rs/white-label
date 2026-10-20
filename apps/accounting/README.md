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
| The client KYC document kinds that block `activate` | §9 *Document blocks* | `seed` |
| The engagement list and page with its history and actions, and the CDD file page | §3, §7.1 | `pages` |

The app owns `src/texts.rs`, the English texts (`accounting.*`). A client changes or translates them
in `resources/lang/{locale}.json`.
