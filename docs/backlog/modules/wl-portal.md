# Epic: wl-portal — negative flows

> Status: draft · Last updated: 2026-10-15 · Spec version: same commit as this file

How the portal behaves while the party behind a portal user is blocked: it never shows a block, a
status or a reason (no tipping off), keeps history and payments working, and refuses actions that
start new business or move money out with one neutral text whatever the reason. Review items
X-18 and 04-N11-a.

| ID | Title | Priority | Size | Milestone | Depends on |
|---|---|---|---|---|---|
| BL-POR-01 | Refuse new business and money out in the portal with a neutral text while the party is blocked | high | m | Build 1 | BL-PTY-01, BL-PTY-04; Base: wl-portal Access grant lifecycle |

---

### BL-POR-01 — wl-portal: refuse new business and money out in the portal with a neutral text while the party is blocked

**Labels:** `module:wl-portal` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:portal` `surface:public` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-PTY-01, BL-PTY-04 · Base: wl-portal Access grant lifecycle · **Blocks:** BL-LOG-19
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
