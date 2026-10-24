# 0006 — Search across records, and AI agents that work in the app

- **Status:** accepted
- **Date:** 2026-10-24
- **Builds on:** [wl-kernel](../modules/wl-kernel.md) §5.8 (search), [WORKFLOW.md](../WORKFLOW.md)
  (state machines), [ACCESS-CONTROL.md](../ACCESS-CONTROL.md), [SECURITY.md](../SECURITY.md).
  Renox gaps R26–R28 are new ([ARCHITECTURE.md](../ARCHITECTURE.md) §4a).

## Context

Staff at our clients already use ChatGPT and Claude for their own work. Both can connect to
other software through the **Model Context Protocol (MCP)**: the app publishes *tools* (search,
read a record, do an action), the user signs in once with OAuth, and the assistant calls the
tools for them. The owner wants every white-label app to work this way: an assistant can enter
data, summarise records, send messages and see what needs attention.

The owner also wants a "Google" inside the app: type `INV-2026-0042` and see the invoice,
its lines, the products on it, the customer, the payments, the delivery, the documents and the
notes, in one place.

The two wishes share one foundation. An assistant can only find the right record if the app
can search across modules and say which records belong together, and the same search serves the
people who use the app by hand.

What exists today:

| Piece | Where | State |
|---|---|---|
| Full-text search on one model (SQLite FTS5, PostgreSQL `tsvector`), Arabic normalisation (R22) | Renox `docs/search.md`, `renox::text` | Built |
| `Subject`: a typed reference to any record (`property.lease:42`) | `crates/wl-kernel/src/subject.rs` | Built |
| Global search design: one search document per record, filtered by permission before counting | [wl-kernel](../modules/wl-kernel.md) §5.8, `PLT.SRC.01` | Designed, not built |
| API tokens with abilities, rate limits | Renox `docs/authorization.md`, R19 | Built |
| State machines with guards, approvals, segregation of duties, audit | Renox R12, [WORKFLOW.md](../WORKFLOW.md), `PLT.APR.01` | Built |
| Database notifications, outbound webhooks (R11) | Renox, `wl-notify` | Built |
| OAuth **provider** (an app that other software signs in to) | — | Missing: `renox-oauth` is a client only (social login) |
| MCP server | — | Missing |

## Decision

1. **Search first, agents second.** Global search (`PLT.SRC.01`) and related records (new,
   `PLT.REL.01`) are built in `wl-kernel` before any agent work. They are worth selling on their
   own, and they are what the agent tools stand on.
2. **One open standard: MCP.** Each client deployment serves a remote MCP endpoint at
   `https://<client domain>/mcp` (Streamable HTTP). It works with Claude, ChatGPT and any
   other MCP client. No assistant-specific plugin is built.
3. **The generic parts go to Renox** (open source, useful to every Renox app):
   - **[R26](https://github.com/renox-rs/renox/issues/82) — OAuth 2.1 authorization server:** authorization code with PKCE, client
     registration (dynamic registration and client metadata documents), a consent page,
     scopes mapped to token abilities, refresh and revoke, and a "Connected apps" list.
   - **[R27](https://github.com/renox-rs/renox/issues/83) — `renox-mcp`:** a crate that turns registered tools and resources into an MCP
     server, signs callers in through R26, and records the client name in the audit log.
   - **[R28](https://github.com/renox-rs/renox/issues/84) — state machine transitions as tools:** a transition becomes a tool with a
     *preview* step and a *confirm* step, running the same guards and permissions.

   `white-label` keeps only the business parts: which records are searchable, how they relate,
   which actions an agent may run, and the per-client policy.
4. **An agent is the user it acts for, never more.** The same roles, scopes, record rules
   (`OWN`, `ASG`), segregation of duties and limits apply. OAuth scopes can only narrow the
   user's rights, never widen them. In the first version an agent **cannot decide an approval**
   (`approve`, `approve_step`, `reject`): approvals stay with a person.
5. **Writes are named actions, never raw CRUD.** A module registers the actions an agent may
   run (mostly state machine transitions, and a few creates such as "draft an invoice"). Each
   action runs in two steps: `preview` says exactly what will change and returns a confirmation
   token; `confirm` does it, once (idempotency key). There is no generic "update any field"
   tool and no SQL tool.
6. **Everything an agent does is audited** with the user, the client (for example "Claude"),
   the tool and the record: "Issued by Budi via Claude".
7. **Agent access is off by default, per client.** The company admin turns it on and accepts
   a notice that the records an assistant reads are sent to the AI provider the user chose,
   which may be outside the UAE. The admin chooses which record types agents may read and act
   on. Sensitive classes ([SECURITY.md](../SECURITY.md) §2: health, identity numbers, salary,
   bank details) are never sent; they appear masked. The Clinics vertical (05) keeps agent
   access off and locked until [L-40](../OPEN-ITEMS.md) is answered.
8. **Notifications are pulled, not pushed.** An assistant cannot be woken by our app. The
   agent gets an `inbox` tool (approvals waiting, overdue items, mentions, failed rules) that
   the user can ask about or put on a schedule in their assistant. Push to automation tools
   goes through outbound webhooks (R11). Marketing never says an assistant "receives" our
   notifications.
9. **Messages go out through `wl-notify` only:** an approved template, to a party linked to
   the record, through preview and confirm, within the channel's rate limits.
10. **Text written by outsiders is marked as untrusted.** Tool results keep record fields
    apart from free text written by tenants, clients, patients or emails, and label that text
    as data, so an instruction hidden in a complaint cannot steer the assistant into an action.
    Actions always need the user's confirmation.

### How search works

1. **Identifiers before words.** Document numbers (sequences), TRN, Emirates ID, phone numbers
   and plate or unit numbers match exactly first; then full-text with Arabic and Latin forms
   (R22).
2. **Results are grouped by type** (invoices, parties, leases, tickets…) and filtered by
   permission, scope and record rules **before** they are counted ([wl-kernel](../modules/wl-kernel.md)
   §5.8).
3. **Related records.** Opening a result shows what belongs to it, from the module's declared
   relations and `Subject` links: for an invoice, its lines and their products, the customer,
   the allocated payments and cheques, the credit notes, the delivery, the documents, the notes
   and the audit timeline. One query per relation, never one per row.
4. **The same functions are the agent's tools:** `search`, `fetch` and `related`.
5. **No vector search in the first version.** Identifiers, full-text and related records are
   enough; the assistant supplies the understanding of what the user meant.

## Capabilities

| ID | Capability | Module | Scope |
|---|---|---|---|
| `PLT.SRC.01` | Global search across records, including Arabic names (existing) | wl-kernel + Renox search | MVP, all ten |
| `PLT.REL.01` | Related records of any record, in a panel and as a tool | wl-kernel | MVP, all ten |
| `PLT.AGT.01` | AI assistant connection (MCP): search, read, summarise, reports | wl-kernel + Renox (R26, R27) | Later, all ten |
| `PLT.AGT.02` | AI assistant actions: transitions and drafts, with preview and confirm | wl-kernel + Renox (R28) | Later, all ten |
| `PLT.AGT.03` | AI assistant inbox and messages | wl-kernel + wl-notify | Later, all ten |

## Order

1. Search and related records in `wl-kernel`, first on Property (01), then the modules and
   verticals of F2–F10, which were all built by 2026-10-24 (BL-AGT-11 to BL-AGT-13).
2. R26 and R27 in Renox, shown on the bike shop example.
3. Read-only connector in `white-label` (search, fetch, related, reports, inbox), piloted with
   one Property client.
4. Actions through R28, then messages.

The issues:

- Renox: [#82](https://github.com/renox-rs/renox/issues/82) (R26),
  [#83](https://github.com/renox-rs/renox/issues/83) (R27),
  [#84](https://github.com/renox-rs/renox/issues/84) (R28).
- `white-label`: epic [#540](https://github.com/renox-rs/white-label/issues/540), BL-AGT-01 to
  BL-AGT-13 (#541–#550, #552–#554, [backlog](../backlog/modules/wl-agents.md)).
- renoxium.com: test assumption A6 with 5 prospects
  ([renoxium#13](https://github.com/renox-rs/renoxium/issues/13)), the bikeshop demo with an
  assistant ([#14](https://github.com/renox-rs/renoxium/issues/14)), a blog post
  ([#15](https://github.com/renox-rs/renoxium/issues/15)) and the site copy, gated on a live
  client ([#16](https://github.com/renox-rs/renoxium/issues/16)).

## Consequences

- `wl-kernel` gains three registries (searchable types, relations, agent actions); every module
  and vertical registers its own entries, as it does for reports and import templates.
- A module that adds a transition decides whether an agent may run it. Nothing is exposed by
  default.
- `PLT.REL.01` and `PLT.AGT.01`–`03` are added to the [capability map](../CAPABILITY-MAP.md) and
  the vertical documents by BL-AGT-01, with the counts that change.
- Selling it: a pricing tier and partner terms are **not decided** (PRICING §7). Following GTM
  rule 1, the site never calls the connector "available" until it is live for a client.
- Renoxium's strategy note for this decision is
  `company/strategy/2026-10-24-ai-agents-and-search.md` in renox-rs/renoxium.

## Not decided

- The PDPL basis for sending records to an AI provider outside the UAE ([L-40](../OPEN-ITEMS.md)).
  The default above (off, admin notice, sensitive classes never sent) is the safe one.
- Whether an assistant built into the app itself (calling a model from our server) follows
  later. It would use the same tools.
