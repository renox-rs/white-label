# Epic: search across records and AI agents

> Status: in use (F1–F10 built; open items in [README](../README.md) §7) · Last updated: 2026-10-24

[Decision 0006](../../decisions/0006-search-and-ai-agents.md): global search and related
records in `wl-kernel` first (`PLT.SRC.01`, `PLT.REL.01`), then a connection that lets AI
assistants such as Claude and ChatGPT work in a client's app through MCP (`PLT.AGT.01`–`03`).
The generic parts are Renox issues: R26 OAuth 2.1 authorization server
([renox#82](https://github.com/renox-rs/renox/issues/82)), R27 `renox-mcp`
([renox#83](https://github.com/renox-rs/renox/issues/83)) and R28 transitions as tools
([renox#84](https://github.com/renox-rs/renox/issues/84)). This epic holds the business parts.

Not from a review: the **Review items** line names the decision point instead. On GitHub: epic
[#540](https://github.com/renox-rs/white-label/issues/540), issues #541–#550 and #552–#554
([github-issues.md](../github-issues.md)).

| ID | Title | Priority | Size | Milestone | Depends on |
|---|---|---|---|---|---|
| BL-AGT-01 | Register searchable record types and keep one search index | high | m | Build 1 | Base: wl-kernel Subject; Renox search, R22 |
| BL-AGT-02 | Global search page: identifiers first, grouped results, permission-filtered | high | m | Build 1 | BL-AGT-01 |
| BL-AGT-03 | Related records of any record, in a panel | high | m | Build 1 | Base: wl-kernel Subject |
| BL-AGT-04 | Register the F1 modules and Property entities for search and related records | high | m | Build 1 | BL-AGT-01, BL-AGT-03 |
| BL-AGT-05 | Portal search over portal-visible types | medium | s | Build 1 | BL-AGT-02 |
| BL-AGT-06 | Agent access policy per client: off by default, allowed types, sensitive classes withheld | high | m | Later | BL-AGT-01; renox#82 |
| BL-AGT-07 | Read-only AI connector: `search`, `fetch`, `related` and reports as MCP tools | high | m | Later | BL-AGT-02, BL-AGT-03, BL-AGT-06; renox#82, renox#83 |
| BL-AGT-08 | Agent actions: chosen transitions and drafts with preview and confirm | high | m | Later | BL-AGT-07; renox#84 |
| BL-AGT-09 | Agent inbox: what needs the user's attention | medium | s | Later | BL-AGT-07 |
| BL-AGT-10 | Send a message from a template through the agent | medium | s | Later | BL-AGT-08 |
| BL-AGT-11 | Register the shared modules built in F2–F10 for search and related records | high | m | Launch 02 | BL-AGT-01, BL-AGT-03 |
| BL-AGT-12 | Verticals 02–05: register their records for search and related records | high | m | Launch 02 | BL-AGT-11 |
| BL-AGT-13 | Verticals 06–10: register their records for search and related records | high | m | Launch 06 | BL-AGT-11 |

BL-AGT-11 to -13 were added on 2026-10-24, after F1–F10 were merged: the vertical phases that
BL-AGT-04 left the other verticals to are already built, so their registrations need issues of
their own.

**Milestone wording for BL-AGT-11 to -13.** The milestone column keeps the phase whose modules
the issue registers (Launch 02 for the F2–F5 records, Launch 06 for the F6–F10 records). It does
not mean "built with that launch": Launch 02 to 10 are already built and merged, and these three
issues depend on BL-AGT-01 and BL-AGT-03 (Build 1), which are not built. AGT-01 to AGT-13 are one
track with one order: BL-AGT-01 and -03 first, then BL-AGT-02, -04, -05, then BL-AGT-11, then
BL-AGT-12 and -13. The five Later issues (BL-AGT-06 to -10) follow in their own order. The GitHub
milestone of #552–#554 is not changed by this file; read it as "registers the Launch 02/06 records".

---

### BL-AGT-01 — wl-kernel: register searchable record types and keep one search index

**Labels:** `module:wl-kernel` `area:ai-agents` `priority:high` `size:m` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: wl-kernel `Subject`; Renox full-text search and Arabic normalisation (R22) · **Blocks:** BL-AGT-02, BL-AGT-04, BL-AGT-06, BL-AGT-11
**Review items:** Decision 0006 points 1 and "How search works" 1–2; `PLT.SRC.01`

#### Goal
Every module tells the kernel which of its records can be found and how a record reads as a
search document (title, secondary text, identifiers, keywords). The kernel keeps one index of
those documents, current on create, change, transition, void and import, so a search can cover
every module in one query. This is the base of the global search page and of the agents' `search`
tool.

#### Surfaces
- **System:** the index is written by the kernel when a registered record changes; a command
  rebuilds it. No screen in this issue.

#### Files to read
- `docs/decisions/0006-search-and-ai-agents.md` ("How search works")
- `docs/modules/wl-kernel.md` §3 (entity *Search document*), §5.8, §6 (`Search` contract)
- `crates/wl-kernel/src/subject.rs`; Renox `docs/search.md` (FTS5 / `tsvector`, `normalize = "arabic"`)
- `docs/SECURITY.md` §2 (data classes)

#### Terms
- **Search document:** one row per findable record: `Subject`, title, secondary text,
  identifiers, keywords, normalised Arabic and Latin forms, scope, legal entity, permission
  needed, sensitivity, updated at.
- **Identifier:** a value matched exactly, never by words: document numbers from sequences,
  TRN, Emirates ID, phone, unit or plate number.

#### Spec excerpt (authoritative for this issue)
- A module registers `Searchable { subject_type, permission, portal_visible, to_document(record) }`
  in its `register()`.
- The index table is `search_documents` with a full-text index on title, secondary text and
  keywords, and a separate table `search_identifiers (subject_type, subject_id, kind, value)`
  with a unique index on `(kind, value, subject_type, subject_id)`. Identifier values are
  stored normalised: upper case, without spaces, dashes and slashes; phone numbers in E.164.
- Fields of class `personal`, `health` or `financial` are never written to the full-text
  columns; an identifier of those classes is stored only as a keyed hash (HMAC with
  `APP_KEY`) and matches only on the exact value.
- The index is updated in the same transaction as the change (`*_in(&mut tx)`), so a rolled
  back change leaves no document.
- `rnx search:rebuild [--type <subject_type>]` rebuilds from the registered types.
- `PLT.REL.01` and `PLT.AGT.01`–`03` are added to `docs/CAPABILITY-MAP.md` and the ten vertical
  documents in this issue, with the counts that change.

#### Edge cases decided
- A voided record stays in the index, marked void, so it can still be found by its number.
- A deleted record (where deletion is allowed) leaves the index.
- An import writes documents per batch; an import rollback removes them.
- Two records with the same identifier (a phone shared by two parties) both match.
- A record whose scope changes gets its document's scope changed in the same transaction.

#### Must not
- Must not index sensitive fields in clear text.
- Must not update the index after commit in a separate job (it would drift on a crash).
- Must not add a search server; SQLite FTS5 and PostgreSQL `tsvector` only.

#### Done when
- [ ] Given a registered invoice `INV-PGP-2026-0004`, when it is issued, then one search document exists with the number as an identifier, and when the transaction rolls back, none exists.
- [ ] Given a party "محمد العلي" with Latin name "Mohammed Al Ali", then both forms are in the document (R22 normalisation).
- [ ] Given a party with Emirates ID `784-1990-1234567-1`, then the identifier table holds a hash, not the digits, and the full-text columns do not contain them.
- [ ] Given `rnx search:rebuild`, then the index equals the one built by the writes (test compares both).
- [ ] Given an import rolled back, then its documents are gone.
- [ ] The capability map and the vertical documents list `PLT.REL.01` and `PLT.AGT.01`–`03`, and the summary counts match.

#### Out of scope
The search page (BL-AGT-02); registering each module (BL-AGT-04, BL-AGT-11 to BL-AGT-13); portal search (BL-AGT-05).

---

### BL-AGT-02 — wl-kernel: global search page, identifiers first, grouped results, permission-filtered

**Labels:** `module:wl-kernel` `area:ai-agents` `priority:high` `size:m` `surface:back-office` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-AGT-01 · **Blocks:** BL-AGT-05, BL-AGT-07
**Review items:** Decision 0006 "How search works" 1–2; `PLT.SRC.01`

#### Goal
Staff type a number, a name or a few words into one search box in the top bar and get every
record they may open, grouped by type, best match first. A document number or an identifier
opens straight to the match. Nobody sees a record, or even a count, that their role, scope or
record rules hide.

#### Surfaces
- **Back office:** a search box in the shell's top bar (shortcut `mod+k`) on every page; `GET /search?q=`
  shows the results grouped by type (*Invoices*, *Parties*, *Leases*…), each row with its title,
  secondary text, status badge and link to the record. An exact identifier match with one result
  redirects to the record.

#### Files to read
- `docs/modules/wl-kernel.md` §5.8; `docs/ACCESS-CONTROL.md` (layers 3 and 4: scope and record rules)
- `docs/UX.md` (top bar, empty states), `docs/DESIGN-SYSTEM.md`
- `host/src/app/menu/`, the shell layout in `host/resources/views/`

#### Terms
- **Record rule:** `OWN` (records the user created or owns), `ASG` (records assigned to the user).

#### Spec excerpt (authoritative for this issue)
1. Normalise the query as identifiers are normalised (BL-AGT-01). If it matches identifiers,
   those records come first, then full-text matches.
2. Filter in the query, before counting and before the limit: the type's permission, the
   user's scopes and legal entities, record rules.
3. Up to 5 results per type on the page, with *Show all N* per type (`/search?q=&type=`), paged.
4. Ranking: identifier match, then full-text rank, then most recently updated.
5. Sensitive values show masked (`•••• 4567`).

#### Edge cases decided
- Query shorter than 2 characters: no search, a hint.
- No result: an empty state that says what was searched, with no suggestion from records the user can't see.
- A user with no searchable permission sees the box disabled with a reason.
- Arabic input on an English page and the reverse both work.
- Time shown in Asia/Dubai.

#### Must not
- Must not filter after the limit (pages would be short and counts would leak).
- Must not run one query per type per result row (one query for the page, one per relation used).
- Must not reveal a hidden record in the count, the redirect or the "did you mean".

#### Done when
- [ ] Given `finance@gulftower.example` searches `INV-PGP-2026-0004` (also typed `inv pgp 2026 0004`), then the invoice opens directly.
- [ ] Given `viewer@gulftower.example` without `invoices.view`, the same search shows no invoice group and no count.
- [ ] Given a user scoped to building A, a search for a tenant of building B returns nothing.
- [ ] Given a search for "Al Ali", then parties, their leases and their invoices appear in their groups, identifiers first.
- [ ] Given 40 matching invoices, then the page shows 5 and *Show all 40*, and the type page pages them.
- [ ] Query count for the page is constant whatever the number of results (`capture_queries`).
- [ ] At 390 px the results are cards with no horizontal scroll; `mod+k` focuses the box.

#### Out of scope
Related records on the record page (BL-AGT-03); portal (BL-AGT-05).

---

### BL-AGT-03 — wl-kernel: related records of any record, in a panel

**Labels:** `module:wl-kernel` `area:ai-agents` `priority:high` `size:m` `surface:back-office` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: wl-kernel `Subject` · **Blocks:** BL-AGT-04, BL-AGT-07, BL-AGT-11
**Review items:** Decision 0006 "How search works" 3; `PLT.REL.01`

#### Goal
On any record's page, a *Related* panel lists the records that belong to it, across modules:
for an invoice, its lines and their products, the customer, allocated payments and cheques,
credit notes, the delivery, documents, notes and the audit timeline. It answers "show me
everything about invoice X" without the user knowing which module holds what. The agents'
`related` tool uses the same function.

#### Surfaces
- **Back office:** a *Related* tab or panel on every registered record's page, grouped by
  relation (*Lines*, *Customer*, *Payments*…), each group with its count and up to 10 rows linking
  to the record.

#### Files to read
- `docs/ARCHITECTURE.md` §5 (module contract, rule 1: `Subject`, no cross-module foreign keys)
- `crates/wl-kernel/src/subject.rs`; Renox `docs/relations.md` (`belongs_to`, `has_many`, `Pivot`)
- `host/src/app/activity/` (the activity tab, as a pattern)

#### Terms
- **Relation:** a named link a module declares from one subject type to another, e.g.
  `invoicing.invoice → payments.payment` "Payments".

#### Spec excerpt (authoritative for this issue)
- A module registers `Relation { from_type, name, label, to_type, load(subject_ids) }`; `load`
  takes many subjects at once and returns `Vec<(from Subject, to Subject)>` in one query.
- Generic relations come for free: documents, notes and approval requests attached by `Subject`.
- Each target is filtered by the same permission, scope and record rules as search; a hidden
  record is neither listed nor counted.
- `kernel::related(&user, subject, depth = 1)` returns the groups; depth 2 is allowed only for
  relations marked `expand` (invoice → lines → products).

#### Edge cases decided
- A relation both modules declare (A → B and B → A) is shown from each side once.
- A target that no longer exists is skipped and logged once.
- Voided targets are listed with a *Void* badge.

#### Must not
- Must not load relations one row at a time.
- Must not add foreign keys between modules' tables.

#### Done when
- [ ] Given an issued invoice with 3 lines, 2 products, 1 allocated payment and 1 PDF, then *Related* shows *Lines 3*, *Products 2*, *Customer 1*, *Payments 1*, *Documents 1*, *Notes 0*.
- [ ] Given a user without `payments.view`, then *Payments* is absent, not *Payments 0*.
- [ ] The panel's query count does not grow with the number of related rows.
- [ ] `related(...)` returns the same groups as the panel (test).

#### Out of scope
Registering each module's relations (BL-AGT-04, BL-AGT-11 to BL-AGT-13).

---

### BL-AGT-04 — F1 modules and 01 Property: register for search and related records

**Labels:** `module:wl-kernel` `vertical:01-property` `area:ai-agents` `priority:high` `size:m` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-AGT-01, BL-AGT-03 · **Blocks:** BL-AGT-07
**Review items:** Decision 0006 "Order" 1

#### Goal
The records of the F1 modules and of 01 Property become findable and show their related records:
parties, invoices and credit notes, payments, cheques, deposits and refunds, tickets, documents,
units, buildings and leases. BL-AGT-11 to BL-AGT-13 follow this issue as the pattern.

#### Surfaces
- **System:** registrations in each module's `register()`; seeds rebuild the index.

#### Files to read
- `docs/modules/wl-parties.md`, `wl-invoicing.md`, `wl-payments.md`, `wl-ticketing.md`,
  `wl-documents.md` §3 (entities); `docs/verticals/01-property.md` §3
- `crates/*/src/lib.rs` `register()` of those modules; `apps/property/`

#### Terms
None.

#### Spec excerpt (authoritative for this issue)
| Type | Title | Identifiers | Relations |
|---|---|---|---|
| `parties.party` | display name (EN/AR) | TRN, Emirates ID (hashed), phone, email | invoices, payments, leases, tickets, documents |
| `invoicing.invoice` | number + customer | number | lines → products, customer, payments, cheques, credit notes, delivery, documents |
| `invoicing.credit_note` | number + customer | number | invoice, customer |
| `payments.payment` / `.cheque` / `.deposit` / `.refund` | number + payer | number, cheque number | invoices allocated, party, lease |
| `ticketing.ticket` | number + subject | number | unit, party, documents |
| `property.building` / `.unit` / `.lease` | name / unit no. / lease no. + tenant | unit no., lease no., Ejari no. | tenant, unit, invoices, cheques, deposits, tickets |

#### Edge cases decided
- Cheque numbers repeat across banks: the identifier is the cheque number; results show the bank.
- The demo seed ends with a full index (`migrate:fresh --seed`).

#### Must not
- Must not index the IBAN or the full card number of anything.

#### Done when
- [ ] Given the demo client, a search for a tenant's name finds the party, the lease, the unit and the open invoices.
- [ ] Given a cheque number, the cheque is found and *Related* shows its lease and the invoices it pays.
- [ ] Given a unit, *Related* shows the current lease, its cheques and the open tickets.
- [ ] Every registered type has a test that its document and its relations are produced.

#### Out of scope
The modules built in F2–F10 (BL-AGT-11) and verticals 02–10 (BL-AGT-12, BL-AGT-13).

---

### BL-AGT-05 — wl-portal: search over portal-visible types

**Labels:** `module:wl-portal` `area:ai-agents` `priority:medium` `size:s` `surface:portal` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-AGT-02 · **Blocks:** None
**Review items:** `docs/modules/wl-kernel.md` §5.8 point 2

#### Goal
A tenant or client in the portal can find their own invoices, receipts, tickets and documents
by number or words, without ever seeing another party's records.

#### Surfaces
- **Portal:** a search box on the portal home; results limited to the types registered
  `portal_visible` and to records of the signed-in party.

#### Files to read
- `docs/modules/wl-portal.md`; BL-AGT-02 above; `docs/modules/wl-kernel.md` §7 (portal searchable types setting)

#### Terms
- **Portal-visible type:** a searchable type the module marks visible to external users and the client has not turned off.

#### Spec excerpt (authoritative for this issue)
The query uses the same index, filtered by the party of the portal user; internal notes and
internal documents are never portal-visible.

#### Edge cases decided
- A portal user acting for several parties (a guardian, a company contact) sees the records of each, grouped by party.

#### Must not
- Must not reuse the staff permission filter alone; the party filter is required.

#### Done when
- [ ] Given tenant A searches tenant B's invoice number, then nothing is found.
- [ ] Given tenant A searches their own invoice number, then it opens in the portal.
- [ ] Given an internal note mentioning tenant A's invoice, then the portal search does not find it.

#### Out of scope
Staff search (BL-AGT-02).

---

### BL-AGT-06 — wl-kernel: agent access policy per client

**Labels:** `module:wl-kernel` `area:ai-agents` `priority:high` `size:m` `surface:back-office` `blocked:open-item` · **Milestone:** Later · **Size:** m
**Depends on:** BL-AGT-01; renox#82 (R26) · **Blocks:** BL-AGT-07
**Review items:** Decision 0006 points 4, 6, 7; open item L-40

#### Goal
The company admin decides whether staff may connect AI assistants at all, to which record types,
and with which rights. It is off by default. Turning it on needs the admin to accept a notice that
records read by an assistant go to the AI provider the user chose. Users see and revoke their
own connections.

#### Surfaces
- **Back office:** *Settings → AI assistants* (`settings.ai.manage`): on/off, allowed record
  types for reading and for actions, the notice and who accepted it when. *My account →
  Connected apps* (from R26) for each user; the admin sees all connections and can revoke any.
- **System:** the policy is checked on every agent call (BL-AGT-07 to BL-AGT-10).

#### Files to read
- `docs/decisions/0006-search-and-ai-agents.md` points 4–7; `docs/SECURITY.md` §2, §4.3
- `docs/OPEN-ITEMS.md` L-01, L-40; `docs/modules/wl-kernel.md` §7 (settings per client)

#### Terms
- **Agent:** a program acting for a user through an OAuth token from R26 (Claude, ChatGPT…).

#### Spec excerpt (authoritative for this issue)
| Setting | Default | Client can change |
|---|---|---|
| AI assistants allowed | off | yes (`settings.ai.manage`) |
| Types agents may read | none until turned on; then every searchable type except those of class `health` | yes |
| Types agents may act on | none | yes, only types with registered agent actions |
| Sensitive classes (`personal` identifiers, `health`, `financial`) in agent results | masked, never sent | no |
| Clinics (05) | off and locked | no, until L-40 is answered |

#### Edge cases decided
- Turning it off revokes nothing but refuses every agent call at once; turning it back on restores the existing connections.
- A user who loses access to a type loses it for their agents at the next call.
- Notice text and acceptance are kept in the audit log with the accepting admin and time.

#### Must not
- Must not let an agent see what its user can't.
- Must not send a sensitive value, even masked partially beyond the last 4 characters.

#### Done when
- [ ] Given a fresh client, an agent call is refused with "AI assistants are turned off for this company".
- [ ] Given the admin turns it on without accepting the notice, it stays off.
- [ ] Given *invoices* not allowed for agents, an agent search returns no invoice even for a user who may see invoices.
- [ ] Given a clinics deployment, the setting is shown locked with the reason.
- [ ] Given the admin revokes Budi's Claude connection, Budi's next agent call is 401.

#### Out of scope
The tools themselves (BL-AGT-07+).

---

### BL-AGT-07 — host: read-only AI connector, `search`, `fetch`, `related` and reports as MCP tools

**Labels:** `module:wl-kernel` `area:ai-agents` `priority:high` `size:m` `surface:system` · **Milestone:** Later · **Size:** m
**Depends on:** BL-AGT-02, BL-AGT-03, BL-AGT-04, BL-AGT-06; renox#82 (R26), renox#83 (R27) · **Blocks:** BL-AGT-08, BL-AGT-09
**Review items:** Decision 0006 points 2, 4, 6, 10; `PLT.AGT.01`

#### Goal
A user adds `https://<client domain>/mcp` as a connector in Claude or ChatGPT, signs in with
their app account and can ask "what does Al Ali owe us?", "summarise invoice INV-PGP-2026-0004
and everything about it", or "run the ageing report for building A". The assistant reads through
the same search, related records and reports as the user, nothing more.

#### Surfaces
- **System:** the MCP endpoint and its tools. The client's branding (name, logo) appears in the
  consent page from R26.
- **Back office:** the record's Activity tab shows "Viewed by Budi via Claude" for agent reads of
  sensitive records, as for people.

#### Files to read
- renox#83 (`renox-mcp`), renox#82; `docs/decisions/0006-search-and-ai-agents.md`
- BL-AGT-02, BL-AGT-03 above; `docs/modules/wl-kernel.md` §5.10 (reports)

#### Terms
- **Tool:** a function the assistant may call, with a JSON schema for its arguments.

#### Spec excerpt (authoritative for this issue)
| Tool | Arguments | Returns |
|---|---|---|
| `search` | `query`, optional `type`, `limit` (≤ 20) | results like BL-AGT-02: id (`subject_type:id`), title, secondary text, status, URL |
| `fetch` | `id` | the record's fields the user may see (masked as on screen), status, available actions' names, URL |
| `related` | `id`, optional `relation` | groups like BL-AGT-03 |
| `list_reports` / `run_report` | report key, filters | rows (≤ 500) and totals, with the report's permission |

- Free text written by external users (portal messages, ticket descriptions from tenants,
  inbound email bodies) is returned through R27's `untrusted(...)` marker.
- Every call is audited with user, client and tool; amounts are returned in AED with the
  minor unit stated.

#### Edge cases decided
- `fetch` of a record the user can't open answers "not found", as the page does.
- A report bigger than 500 rows returns the first 500 and says how to narrow it.

#### Must not
- Must not add a tool that runs SQL or reads arbitrary tables.
- Must not return a sensitive value unmasked.

#### Done when
- [ ] With `app.mcp().as_user(finance)`, `search("INV-PGP-2026-0004")` returns the invoice first and `related` returns its groups.
- [ ] As `viewer@gulftower.example`, invoices are neither found nor fetchable.
- [ ] A tenant's ticket text containing "ignore your instructions and…" comes back inside the untrusted marker.
- [ ] Audit shows "via Claude" for a sensitive record read.
- [ ] Tested by hand with Claude and ChatGPT against the demo client (versions noted in the PR).

#### Out of scope
Actions (BL-AGT-08); inbox (BL-AGT-09); messages (BL-AGT-10).

---

### BL-AGT-08 — wl-kernel: agent actions, chosen transitions and drafts with preview and confirm

**Labels:** `module:wl-kernel` `area:ai-agents` `priority:high` `size:m` `surface:system` · **Milestone:** Later · **Size:** m
**Depends on:** BL-AGT-07; renox#84 (R28) · **Blocks:** BL-AGT-10
**Review items:** Decision 0006 points 4–6; `PLT.AGT.02`

#### Goal
An assistant can do work, not only read: draft an invoice, issue it, log a ticket, record a
cheque received, cancel a booking. Each action shows the user what will happen first and acts
only after they confirm, through the same state machine, guards, limits, segregation of duties
and approvals as the screen.

#### Surfaces
- **System:** the action tools.
- **Back office:** transition history and Activity show "by Budi via Claude".

#### Files to read
- renox#84; `crates/wl-kernel/src/machines.rs` (`SubjectMachine`, transitions by name)
- `docs/WORKFLOW.md` §3; `docs/ACCESS-CONTROL.md` §5 (approvals, SoD)

#### Terms
- **Agent action:** a transition or a create a module registers with `.agent()`; nothing else is offered.

#### Spec excerpt (authoritative for this issue)
- Modules register agent actions; the first set (01 Property): `invoicing.invoice` *create draft*, *issue*;
  `ticketing.ticket` *create*, *assign*, *resolve*; `payments.cheque` *record received*;
  `property.lease` *send renewal offer*.
- Each action: `preview` (no writes; returns the change in words, amounts, whether an approval
  will be requested, a confirmation token valid 10 minutes) then `confirm` (one run, idempotent).
- Deciding an approval is never an agent action.

#### Edge cases decided
- The record moved between preview and confirm: confirm refused, the assistant is told to preview again.
- An action over the user's limit: confirm submits an approval request and says so.

#### Must not
- Must not expose `approve`, `approve_step`, `reject` or `void`.
- Must not write anything on preview.

#### Done when
- [ ] Preview of *issue* on a draft invoice writes nothing and returns number-to-be-assigned, total and VAT.
- [ ] Confirm issues it once; a second confirm returns the same invoice.
- [ ] An *issue* over the user's limit creates an approval request, not an issued invoice.
- [ ] History shows "via Claude".

#### Out of scope
Messages (BL-AGT-10); actions for verticals 02–10 (follow-up issues per vertical once this one has proven the pattern on Property).

---

### BL-AGT-09 — wl-kernel: agent inbox, what needs the user's attention

**Labels:** `module:wl-kernel` `area:ai-agents` `priority:medium` `size:s` `surface:system` · **Milestone:** Later · **Size:** s
**Depends on:** BL-AGT-07 · **Blocks:** None
**Review items:** Decision 0006 point 8; `PLT.AGT.03`

#### Goal
The assistant cannot be woken by our app, so it asks. An `inbox` tool returns what the user
should look at now: approvals waiting for them, their overdue items, unread mentions and
notifications, failed rule runs they own. A user can ask "what needs me today?" or schedule that
question in their assistant.

#### Surfaces
- **System:** the `inbox` and `mark_read` tools.

#### Files to read
- `host/src/app/approvals_inbox.rs`, `host/src/app/dashboard/`; Renox notifications (`docs/mail.md`)

#### Terms
None.

#### Spec excerpt (authoritative for this issue)
`inbox(since?)` returns groups: *Waiting for your approval*, *Overdue*, *Mentions*,
*Notifications*, *Failed automations*, each item with title, why, due date (Asia/Dubai) and URL;
the same items as the dashboard's "awaiting you". `mark_read(ids)` marks notifications read.

#### Edge cases decided
- Approvals are listed with a link; the agent cannot decide them (BL-AGT-08).

#### Must not
- Must not list items of other users or scopes.

#### Done when
- [ ] The inbox equals the dashboard's "awaiting you" for the same user (test).
- [ ] `mark_read` clears the bell count.

#### Out of scope
Push to the assistant (not possible; outbound webhooks R11 cover automation tools).

---

### BL-AGT-10 — wl-notify: send a message from a template through the agent

**Labels:** `module:wl-notify` `area:ai-agents` `priority:medium` `size:s` `surface:system` · **Milestone:** Later · **Size:** s
**Depends on:** BL-AGT-08 · **Blocks:** None
**Review items:** Decision 0006 point 9; `PLT.AGT.03`

#### Goal
"Send Al Ali a reminder about invoice 0004 on WhatsApp": the assistant previews the message
from an approved template, the user confirms, and `wl-notify` sends it, within consent rules and
rate limits, logged on the record.

#### Surfaces
- **System:** `send_message` as an agent action (preview, confirm).
- **Back office:** the message appears in the record's Activity "sent by Budi via Claude".

#### Files to read
- `docs/modules/wl-notify.md` (templates, channels, consent, quiet hours); BL-AGT-08

#### Terms
- **Template:** an approved message with placeholders, per channel and language.

#### Spec excerpt (authoritative for this issue)
Arguments: record id, template key, channel. Recipients: only parties linked to the record. The
template's own rules apply (consent, quiet hours, opt-out). No free-text body in this version.

#### Edge cases decided
- A party without consent for the channel: preview says so and offers the channels allowed.

#### Must not
- Must not send free text written by the model.
- Must not send to an address not on the party.

#### Done when
- [ ] Preview shows the rendered message in the party's language; confirm sends once (fake channel).
- [ ] A party who opted out of WhatsApp: refused with the reason.

#### Out of scope
Marketing broadcasts (BL-NTF-07).

---

### BL-AGT-11 — shared modules of F2–F10: register for search and related records

**Labels:** `module:wl-kernel` `area:ai-agents` `priority:high` `size:m` `surface:system` · **Milestone:** Launch 02 · **Size:** m
**Depends on:** BL-AGT-01, BL-AGT-03 · **Blocks:** BL-AGT-12, BL-AGT-13
**Review items:** Decision 0006 "Order" 1; `PLT.SRC.01`, `PLT.REL.01` are MVP in all ten verticals

#### Goal
The shared modules that F2–F10 built (`wl-attendance`, `wl-inventory`, `wl-delivery`,
`wl-scheduling`, `wl-hr`, `wl-sales`) register their main records, so a search for a purchase
order, a lot, a delivery, a booking, an employee or a sales order works in every vertical that
uses them, and their records show in each other's *Related* panel.

Build order: the milestone **Launch 02** names the records registered, not the time of the build. That launch is already built; this issue waits for BL-AGT-01 and BL-AGT-03 (Build 1, not built).

#### Surfaces
- **System:** registrations in each module's `register()`; the demo seeds of the ten clients
  end with a full index.

#### Files to read
- BL-AGT-01, BL-AGT-03, BL-AGT-04 (the pattern)
- `docs/modules/wl-attendance.md`, `wl-inventory.md`, `wl-delivery.md`, `wl-scheduling.md`,
  `wl-hr.md`, `wl-sales.md` §3 (entities); `crates/<module>/src/lib.rs` `register()`

#### Terms
- **Main record:** a record with its own number or name that people look for; lines, balances
  and movements are reached through *Related*, not indexed on their own.

#### Spec excerpt (authoritative for this issue)
| Module | Searchable types | Identifiers | Relations (examples) |
|---|---|---|---|
| `wl-inventory` | item, purchase order, goods receipt, lot, dispatch, asset | SKU, barcode, PO / GRN / dispatch number, lot number, serial number | PO → receipts → supplier bills; lot → movements, holds, recalls; item → suppliers |
| `wl-sales` | lead, sales order, counter sale, sales return, package | order / sale / return number | order → invoice, delivery, dispatch, returns, customer |
| `wl-delivery` | delivery, run | delivery and run number, tracking number | delivery → order, attempts, proof of delivery, COD |
| `wl-scheduling` | booking, resource | booking number | booking → party, resource, invoice |
| `wl-hr` | employee, leave request, payroll run, final settlement | employee number; Emirates ID, passport and labour card (hashed) | employee → visa case, leave, timesheets, payroll lines, documents |
| `wl-attendance` | timesheet, work location | timesheet number | timesheet → employee or crew, client, invoice |

- Salary, bank and payroll amounts are `financial`: never in the full-text columns
  (BL-AGT-01).

#### Edge cases decided
- A module not enabled in a client (`client.toml`) registers nothing there.
- A serial number shared by two items of different suppliers: both match.

#### Must not
- Must not index stock movements, attendance records or payroll lines one by one.

#### Done when
- [ ] Given the F7 demo client, a lot number finds the lot, and *Related* shows its holds and the dispatches that shipped it.
- [ ] Given the F9 demo client, an employee number finds the employee; a search for a passport number in clear text finds them only on the exact value, and the result shows it masked.
- [ ] Given the F3 demo client, a tracking number finds the delivery and its run.
- [ ] Every registered type has a test that its document and relations are produced.

#### Out of scope
Each vertical's own records (BL-AGT-12, BL-AGT-13).

---

### BL-AGT-12 — verticals 02–05: register their records for search and related records

**Labels:** `vertical:02-construction` `vertical:03-logistics` `vertical:04-accounting` `vertical:05-clinics` `area:ai-agents` `priority:high` `size:m` `surface:system` · **Milestone:** Launch 02 · **Size:** m
**Depends on:** BL-AGT-11 · **Blocks:** None
**Review items:** Decision 0006 "Order" 1

#### Goal
The records each of these verticals adds become findable by their number or name, with their
related records: a QS finds an IPA by number, a dispatcher a shipment by its AWB, a partner an
engagement by the client's name, a receptionist an appointment by the patient's file number.

Build order: the milestone **Launch 02** names the records registered, not the time of the build. That launch is already built; this issue waits for BL-AGT-01 and BL-AGT-03 (Build 1, not built).

#### Surfaces
- **System:** registrations in `apps/construction`, `apps/logistics`, `apps/accounting`,
  `apps/clinics`.

#### Files to read
- BL-AGT-04 (the pattern); `docs/verticals/02-construction.md` … `05-clinics.md` §4;
  `apps/<vertical>/src/lib.rs`

#### Terms
- **IPA:** interim payment application. **AWB:** air waybill. **CDD:** customer due diligence.

#### Spec excerpt (authoritative for this issue)
| Vertical | Searchable types | Identifiers |
|---|---|---|
| 02 Construction | project, IPA, variation, guarantee, retention | project code, IPA and variation number, guarantee number |
| 03 Logistics | shipment, freight job, remittance, claim | shipment and job number, AWB / B/L / container number, COD remittance number |
| 04 Accounting | engagement, tax return, CDD file, AML case | engagement number, client TRN, return period and reference |
| 05 Clinics | patient (name and file number only), appointment (visit), insurance claim, authorisation | patient file number, claim and authorisation number, Emirates ID (hashed) |

- Clinics: diagnoses, clinical notes, results and anything of class `health` are never
  indexed; a patient's result shows the name, file number and the next appointment only. Agent
  access for 05 stays locked (BL-AGT-06).

#### Edge cases decided
- A container number used on several freight jobs over time: all match, the latest first.

#### Must not
- Must not put any `health` field in a search document.

#### Done when
- [ ] For each of 02–05, a test finds a seeded record by each identifier listed above and shows its related records.
- [ ] Given the clinics demo, a search for a diagnosis word written in a visit note finds nothing.

#### Out of scope
Verticals 06–10 (BL-AGT-13); agent actions.

---

### BL-AGT-13 — verticals 06–10: register their records for search and related records

**Labels:** `vertical:06-fnb` `vertical:07-manufacturing` `vertical:08-auto` `vertical:09-manpower` `vertical:10-edtech` `area:ai-agents` `priority:high` `size:m` `surface:system` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** BL-AGT-11 · **Blocks:** None
**Review items:** Decision 0006 "Order" 1

#### Goal
The records each of these verticals adds become findable by their number or name, with their
related records: an outlet manager finds an aggregator order by its platform number, a planner a
work order, a service advisor a job card by the car's plate, an account manager a deployment, a
counsellor a student's enrolment.

Build order: the milestone **Launch 06** names the records registered, not the time of the build. That launch is already built; this issue waits for BL-AGT-01 and BL-AGT-03 (Build 1, not built).

#### Surfaces
- **System:** registrations in `apps/fnb`, `apps/manufacturing`, `apps/auto`, `apps/manpower`,
  `apps/edtech`.

#### Files to read
- BL-AGT-04 (the pattern); `docs/verticals/06-fnb.md` … `10-edtech.md` §4;
  `apps/<vertical>/src/lib.rs`

#### Terms
- **NCR:** non-conformance report. **Job card:** a workshop's work order for one vehicle.

#### Spec excerpt (authoritative for this issue)
| Vertical | Searchable types | Identifiers |
|---|---|---|
| 06 F&B | order (own and aggregator), production batch, aggregator payout | order number, aggregator order number, batch number |
| 07 Manufacturing | work order, NCR, return inspection | work order and NCR number, customer PO number |
| 08 Auto | job card, insurance claim, supplier warranty claim, fleet contract | job card number, plate number, VIN, claim number |
| 09 Manpower | client request, deployment, client contract, site | deployment and request number, contract number |
| 10 EdTech | course, batch, enrolment, certificate | enrolment number, certificate number |

- Plate numbers are normalised like other identifiers (`DXB A 12345` = `dxb-a-12345`).
- A certificate number search also serves the public verification page only through its own
  route, never through this index.

#### Edge cases decided
- A plate moved to another car: the job cards of both cars match, the latest first.

#### Must not
- Must not index a student's or worker's identity numbers in clear text.

#### Done when
- [ ] For each of 06–10, a test finds a seeded record by each identifier listed above and shows its related records.
- [ ] Given the auto demo, a plate typed with spaces, dashes or in lower case finds the same job cards.

#### Out of scope
Verticals 02–05 (BL-AGT-12); agent actions.
