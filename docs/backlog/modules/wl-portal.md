# Epic: wl-portal — negative flows

> Status: in use (F1–F10 built; open items in [README](../README.md) §7) · Last updated: 2026-10-24

How the portal behaves while the party behind a portal user is blocked: it never shows a block, a
status or a reason (no tipping off), keeps history and payments working, and refuses actions that
start new business or move money out with one neutral text whatever the reason. Review items
X-18 and 04-N11-a.

| ID | Title | Priority | Size | Milestone | Depends on |
|---|---|---|---|---|---|
| BL-POR-01 | Refuse new business and money out in the portal with a neutral text while the party is blocked | high | m | Build 1 | BL-PTY-01, BL-PTY-04; Base: wl-portal Access grant lifecycle |
| BL-POR-02 | Build each audience's menu from registered sections and portal-role permissions | high | m | Build 1 | Base: wl-portal Access grant lifecycle; BL-POR-01 |
| BL-POR-03 | Make a grant read-only when the vertical's contract ends and close it after the period (`POR.ACL.01`) | medium | s | Build 1 | Base: wl-portal Access grant lifecycle; BL-POR-02 |

---

### BL-POR-01 — wl-portal: refuse new business and money out in the portal with a neutral text while the party is blocked

**Labels:** `module:wl-portal` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:portal` `surface:public` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-PTY-01, BL-PTY-04 · Base: wl-portal Access grant lifecycle · **Blocks:** BL-LOG-19, BL-POR-02, BL-TKT-14, BL-INV-31, BL-PRP-30, BL-MPW-28
**Review items:** X-18, 04-N11-a

#### Goal
When a party is blocked, its portal users must not learn it. Their access grants stay as they
are, with no suspension and no message. They can still see history, documents and statements,
and pay open invoices. Actions that start new business or move money out are refused with the
same neutral text for every reason. The refusal is logged for compliance staff only. On unblock
the actions work again.

#### Surfaces
- **Back office:** staff with `parties.block_reason.view` (the MLRO) see one refusal-log
  entry per refused portal action (portal user, party, action, time) (screen not specified
  yet).
- **Portal:** a user acting for a blocked party still sees history, documents and statements
  and pays open invoices; new booking, order, refund or deposit-release request and new
  signature stay visible and are refused on submit with the neutral text in EN or AR; no "On
  hold", badge or reason appears.
- **Public page:** a signed payment link keeps working; a signed link whose action is new
  business shows the same neutral text.
- **System:** on `PartyBlocked` the grants stay as they are and no message is sent; on
  `PartyUnblocked` the refused actions work again.

#### Files to read
- `docs/modules/wl-portal.md` §6 ("Reacts to" and the paragraph on `PartyBlocked`), §7 (setting
  "Neutral text"), §6 portal permissions
- `docs/modules/wl-parties.md` §6 (`PTY.ACL.02`, `PTY.ACL.03`), §10 (No tipping off)

#### Terms
- **Access grant:** what a portal user may do for one party (states `active`, `read_only`,
  `suspended`…).
- **Acting-for party:** the party the portal user acts for in the current grant (self, their
  company, a dependant).
- **New business:** a new booking, order, request for a refund or deposit release, or signing a
  new agreement. The list is closed (spec §6): any other portal action (maintenance request,
  complaint, message, document upload, profile change) is not new business and works as usual.
- **Block status:** "On hold", blocked, a block reason, or any badge or banner that reveals the
  block. The ordinary status of a request or record (e.g. *Submitted*, *In review*, *Paid*) is
  not a block status.
- **Money in:** paying an invoice; continues while blocked.

#### Spec excerpt (authoritative for this issue)

"**Reacts to:** `RelationshipEnded`, `PartyArchived`, `PartyBlocked` / `PartyUnblocked` (from
`wl-parties`)."

"On `PartyBlocked` the portal never shows a block, a block status or a block reason
(ordinary request statuses are shown as usual) (`PTY.ACL.02`, no tipping off). The grants of
the party's users stay as they are: no `suspend`, no message. History, documents and
statements stay visible, and paying an open invoice still works (money in continues).
Actions that start new business or move money out (new booking, order, request for a refund
or a deposit release, signing a new agreement) are refused with one neutral text, the same
whatever the reason: *"This request cannot be completed online. Please contact us."*
(Arabic: *"هذا الطلب لا يمكن إتمامه عبر الإنترنت. يرجى التواصل معنا."*). The list of refused
actions is closed: every other portal action (a maintenance request, a complaint, a message,
a document upload, a profile change, cancelling without a refund) works as usual; a cancellation that would produce
a refund is money out and refused with the same text. Refused actions stay visible and are
refused on submit, because hiding them would reveal a change. The
refusal is logged for staff with `parties.block_reason.view`. On `PartyUnblocked` the
actions are available again."

| Setting | Default |
|---|---|
| Neutral text for actions refused while the party is blocked (EN/AR) | EN "This request cannot be completed online. Please contact us."; AR "هذا الطلب لا يمكن إتمامه عبر الإنترنت. يرجى التواصل معنا." |

Portal permissions affected: `portal.bookings.manage`, `portal.orders.create`, signing new
agreements (signature requests), refund and deposit-release requests (vertical pages);
`portal.invoices.pay` stays available.

#### Edge cases decided
- **Which party is checked:** the acting-for party of the current grant. A guardian acting for a
  blocked dependant is refused for the dependant; acting for themselves they are not.
- **Group contacts** (04 group CFO with grants per company): only the blocked member company's
  actions are refused.
- **Buttons stay visible:** actions are shown as usual and refused on
  submit, because hiding them would reveal a change.
- **Cancelling a booking:** allowed when the cancellation produces no
  refund; refused with the neutral text when it would produce a refund (money out).
- **Requests already submitted** before the block are handled by staff in the back office; the
  portal shows their ordinary request status as usual (*Submitted*, *In review*, *Approved*…).
  If staff hold such a request because of the block, the portal keeps showing the status it
  had (e.g. *In review*), never "On hold" or a block status.
- **Other actions** (maintenance request, complaint, message, document upload, profile change)
  are not new business and are never refused because of the block.
- **Signed links** (pay this invoice) keep working; a signed link whose action is new business is
  refused with the same text.
- **Log:** one entry per refusal (portal user, party, action, time), visible only with
  `parties.block_reason.view`; not shown to the party or to other staff.
- **Language:** the text is shown in the user's portal language (EN or AR).

#### Must not
- Must not suspend, end or close grants, or send any message, because of `PartyBlocked`.
- Must not show "On hold", a block status, a badge or a block reason anywhere in the portal
  (ordinary request and record statuses are shown as usual).
- Must not use a different text for different block reasons.
- Must not refuse paying an open invoice.

#### Done when
- [ ] Given tenant "Sara M." whose party is blocked with `non_payment`, she logs in, sees her
      lease, invoices and statements, and pays invoice INV-ACME-2026-0310 of AED 4,500.
- [ ] Her request for a deposit release is refused with "This request cannot be completed
      online. Please contact us." and an entry appears in the refusal log for the MLRO only.
- [ ] Given a patient blocked with `sanctions`, booking a new appointment gives exactly the same
      text, and no page contains "On hold" or a reason.
- [ ] Her grant is still `active` and no message was queued.
- [ ] After `PartyUnblocked`, the deposit-release request is accepted.
- [ ] In the Arabic portal the same refusal shows exactly
      "هذا الطلب لا يمكن إتمامه عبر الإنترنت. يرجى التواصل معنا.".
- [ ] Sara M. submits a maintenance request "AC not cooling" while blocked; it is accepted as
      usual.
- [ ] A refund request she submitted before the block still shows *In review*, with no "On hold"
      text or badge.
- [ ] Given a blocked party with an issued invoice, when its portal user opens the signed payment
      link, then the payment page works; when the user opens a signed link for new business
      (e.g. a booking), then the page shows only the neutral text and nothing is created.

#### Out of scope
The block itself and the guard (BL-PTY-01, BL-PTY-04); back-office handling of blocked parties'
requests (module epics).

---

### BL-POR-02 — wl-portal: build each audience's menu from registered sections and portal-role permissions

**Labels:** `module:wl-portal` `area:negative-flow` `priority:high` `size:m` `surface:portal` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: wl-portal Access grant lifecycle; BL-POR-01 · **Blocks:** BL-TKT-14, BL-INV-31, BL-PAY-27, BL-PRP-30, BL-POR-03, BL-MPW-28
**Review items:** X-18; follow-up of BL-POR-01, Done-when points 1 and 8

#### Goal
Modules contribute portal pages, but today a section is a process-wide key with a title and rows: it has
no audience, no required portal permission, no order and no menu, and a grant's portal roles are keys
without permissions. A tenant finance contact must see only *Invoices*, a primary contact everything.
This issue makes `PortalSections` what wl-portal §6 describes: a section registers its audience, the
portal permission it needs, its order and label (EN/AR), and the portal home and menu show only the
sections the user's grant allows, for the party the user acts for.

#### Surfaces
- **Portal:** after login, the home page and menu list the sections of the user's audience allowed by
  their portal role, in order; a user with grants for several parties switches party; a section the
  role does not allow is a 403 by URL.
- **Back office:** the audience's portal roles and their permissions are listed in the portal settings
  (seeded per vertical; editable by the company admin).
- **System:** modules register sections at start-up; the vertical seeds the audience's roles.

#### Files to read
- `docs/modules/wl-portal.md` §3 (*Audience*, *Portal role*, *Portal section*, *Access grant*), §6
  (`PortalSections`, portal permissions), §7, §8 (`PortalSections` registration)
- `docs/verticals/01-property.md` §3.1 (tenant primary contact, finance contact)
- `crates/wl-portal/src/sections.rs`, `grants.rs`, `access.rs`

#### Terms
- **Audience:** a kind of external user per vertical (`tenant`, `owner`).
- **Portal role:** a named set of portal permissions inside an audience (`primary_contact`,
  `finance_contact`).
- **Portal section:** a page or menu entry contributed by a module: module, audience, required portal
  permission, order, label.

#### Spec excerpt (authoritative for this issue)
| Entity | Description | Important fields |
|---|---|---|
| **Audience** | A kind of external user, configured per vertical | key (e.g. `tenant`, `owner`, `client`, `worker`), label, login methods, available portal roles, home page, close-after period, self-registration allowed |
| **Portal role** | A named set of portal permissions inside an audience | key (e.g. `primary_contact`, `finance_contact`, `approver`, `viewer`, `guardian`), permissions |
| **Portal section** | A page or menu entry contributed by a module | module, audience, required portal permission, order, label |

| Service | Purpose |
|---|---|
| `PortalSections` | Modules register portal pages and menu entries per audience, guarded by portal permissions |

01 §3.1: Tenant primary contact: "portal: lease, finance, requests, users"; Tenant finance contact: "Sees
and pays invoices only — portal: finance".

#### Edge cases decided
- **01 seed:** audience `tenant` with roles `primary_contact` (`portal.leases.view`,
  `portal.invoices.view`, `portal.invoices.pay`, `portal.documents.view`, `portal.requests.create`,
  `portal.users.manage`) and `finance_contact` (`portal.invoices.view`, `portal.invoices.pay`).
- **Existing section keys:** kept; the registration gains the new fields with defaults (all audiences,
  no permission) only for tests.
- **Grant `read_only`:** sections show; their write actions are refused (BL-POR-03).
- **Party block:** unchanged (BL-POR-01): nothing in the menu reveals it.
- **Two grants for the same party:** the union of the roles' permissions.

#### Must not
- Show a section of another audience, or one the role does not allow, even by URL.
- Read the party block to build the menu.

#### Done when
- [ ] Given Sara M. with role `primary_contact` for "Al Noor Trading LLC", her menu shows Lease,
      Invoices, Cheques, Deposit, Documents and Requests in their order.
- [ ] Omar with role `finance_contact` for the same party sees only Invoices; opening `/portal/{party}/requests`
      gives 403.
- [ ] A user with grants for two parties switches party and sees each party's rows only.
- [ ] A section registered for audience `owner` never appears for a tenant.
- [ ] The company admin adds `portal.documents.view` to `finance_contact`; Omar then sees Documents.

#### Out of scope
The sections themselves (BL-PRP-30, BL-INV-31, BL-PAY-27, BL-TKT-14); invitations and login methods (base
wl-portal Access grant lifecycle); the owner portal (`POR.SLF.02`, Later).

---

### BL-POR-03 — wl-portal: make a grant read-only when the vertical's contract ends and close it after the period (`POR.ACL.01`)

**Labels:** `module:wl-portal` `area:negative-flow` `priority:medium` `size:s` `surface:portal` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** Base: wl-portal Access grant lifecycle; BL-POR-02 · **Blocks:** BL-MPW-28
**Review items:** 01-N12-a; follow-up of BL-PRP-11, Done-when point 1 (c)

#### Goal
After move-out a tenant can still read and download their history, but cannot raise requests, dispute or
pay anything new; 90 days later the access closes. The grant lifecycle has `end` (to `read_only`) and
`close`, but nothing runs them: the vertical contract-end events are not mapped to `end`, and the
close-after timer does not exist. This issue maps 01 `LeaseTerminated` to `end`, removes write actions
from read-only grants and closes them after the audience's period.

#### Surfaces
- **Portal:** a read-only grant shows every section with a notice "Your access is read-only since
  <date>"; write actions (pay, dispute, new request, reopen, users) are refused with that notice.
- **System:** a listener runs `end` on the mapped contract-end event; a daily job closes read-only grants
  whose close-after period passed.

#### Files to read
- `docs/modules/wl-portal.md` §4 *Access grant* (transitions `end`, `close`; vertical mapping), §6
  (*Reacts to*), §7 (close-after period)
- `docs/verticals/01-property.md` §3.3 `POR.ACL.01`, §7.1 *Lease* (`terminate`)
- `apps/property/src/events.rs` (`LeaseTerminated`), `crates/wl-portal/src/access.rs` (`end`, `close`)

#### Terms
- **Contract-end event:** the vertical event that ends the relationship the grant was given for (01:
  lease `terminated`).
- **Close-after period:** days from `end` to `close`, per audience (01 tenant: 90).

#### Spec excerpt (authoritative for this issue)
| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `end` | active | read_only | system (relationship or contract ended), or `portal_users.manage` | — | write actions removed; event `PortalAccessReadOnly` |
| `close` | read_only, active, suspended | closed | system (close-after period), or `portal_users.manage`, or a primary contact for their company's users | — | sessions ended; event `PortalAccessClosed` |

| Vertical | What ends the grant (`end`) | Close-after period |
|---|---|---|
| 01 Property | lease `terminated` (`POR.ACL.01`) | 90 days |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `POR.ACL.01` | After move-out, the tenant portal is read-only, then closes | `TIM` | tenant | closes after 90 days |

#### Edge cases decided
- **A tenant with another running lease:** the grant is for the party; it ends only when the party has no
  lease left in `active`, `renewal_due`, `expired` (holding over) or `notice_given`.
- **Force-terminate:** `force_terminated` also ends the grant (same event family).
- **Deposit deductions dispute:** stays allowed for a read-only grant during its window (BL-PAY-27);
  it is the one write action a read-only tenant keeps.
- **Day count:** day 1 is the day after `end`; the grant closes on day 91 at 00:00 Asia/Dubai.
- **Suspended at the event:** the suspension stays; `reinstate` returns it to `read_only`.
- **A new lease before closing:** staff invite the tenant again (a new grant); the read-only or closed
  grant is not reopened.

#### Must not
- Hide history or documents from a read-only grant.
- Close a grant the day the lease ends.

#### Done when
- [ ] Given Sara's only lease is terminated on 2026-12-31, her grant is `read_only` and
      `PortalAccessReadOnly` is emitted; she can open her invoices and download a document, and *Pay*,
      *Dispute* and *New request* are refused with the read-only notice.
- [ ] On 2027-03-31 the grant is still `read_only`; on 2027-04-01 it is `closed` and her sessions end.
- [ ] A tenant with a second active lease keeps an `active` grant when one lease terminates.
- [ ] A `force_terminated` lease ends the grant the same way.

#### Out of scope
The sections (BL-POR-02 and the module issues); owner grants after the agreement ends (BL-PRP-29, Later).
