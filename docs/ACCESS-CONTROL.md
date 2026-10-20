# Access Control

> Status: draft for review · Last updated: 2026-10-20 (§10 added)

Every business actor in every vertical is controlled by the same model: **RBAC for what a
person may do, ABAC for where, when, and on which records**. This document defines the model
once. Each vertical then lists only its roles, its scope and its access rules
([templates/vertical.md](templates/vertical.md) §3).

## 1. Principles

1. **Deny by default.** No permission, no access. A new module's actions are closed until a
   role grants them.
2. **Checked on the server, for every action and every list.** Hiding a button is a
   convenience, never the control.
3. **Least privilege.** Default roles grant what the job needs, nothing more.
4. **Rule types are code; their parameters are configuration.** A client can change a limit,
   a duration or the approving role. A client cannot invent a new kind of rule (see [§6](#6-what-a-client-can-configure)).
5. **Everything that changes access is audited:** role grants, revocations, approvals and
   views of sensitive data.

## 2. Actors

| Actor type | Logs in to | Examples | Notes |
|---|---|---|---|
| **Staff** | back office | manager, accountant, coordinator | Two-factor login required (configurable per client, on by default) |
| **Field staff** | PWA (back office, mobile layout) | technician, driver, site supervisor | Staff with a narrow role, usually limited to assigned work |
| **External user** | portal | tenant, owner, client contact, patient, student | Linked to exactly one **person** party. Acts for a company or a dependant through relationships and access grants ([wl-parties](modules/wl-parties.md), [wl-portal](modules/wl-portal.md)). Never holds staff permissions |
| **Device / API client** | API | kiosk, integration | API token with abilities (Renox tokens), scoped like a user |
| **System** | — | scheduler, queue jobs | Runs with explicit, logged system rights |
| **Platform operator** | back office | us, for support | Break-glass account: disabled by default, enabled per incident, every action audited |

## 3. The four layers

A request passes all four layers, in this order.

### Layer 1 — Authentication

Who is this? A logged-in staff user (with 2FA), a portal user, a token, or a system job.

### Layer 2 — RBAC: what may this actor do?

- A **permission** is `<resource>.<action>`, e.g. `leases.view`, `invoices.issue`,
  `deposits.refund.approve`. Each module declares its permissions in its specification
  (`docs/modules/`).
- A **role** is a named set of permissions. Each vertical ships **default roles**
  (e.g. *Property manager*). A client admin can create custom roles from existing
  permissions, but cannot create new permissions.
- **Portal roles** are separate from staff roles. They are assigned to the users of a
  `Party`, e.g. a company tenant's *Primary contact* and *Finance contact*.

### Layer 3 — Scoped assignment (ABAC: where and when)

A role is assigned **within a scope** and optionally **for a period** (Renox:
`assign_role_in`, `has_permission_in`, `scopes_with`, assignment dates). A role means the same
everywhere; only who holds it where and when changes.

| Vertical | Scope unit |
|---|---|
| 01 Property | building |
| 02 Construction | project / site |
| 03 Logistics | branch / depot |
| 04 Accounting | client company (engagement) |
| 05 Clinics | clinic branch |
| 06 F&B | outlet / kitchen |
| 07 Manufacturing | plant / production line |
| 08 Auto | branch / workshop |
| 09 Manpower | client company / site |
| 10 EdTech | campus / course |

A role assigned without a scope is **global**: it applies across the client's whole
deployment.

Each scope unit belongs to exactly one **legal entity** ([decision 0003](decisions/0003-legal-entities.md)).
A client with several entities that needs entity-level separation assigns roles in an
**entity scope**; the role then applies to every scope unit of that entity.

### Layer 4 — Record and field rules (ABAC on data)

Policies decide whether this actor may perform this action **on this record**, and which
fields they see. Every rule in every vertical uses one of these patterns:

| Code | Pattern | Rule shape | Example |
|---|---|---|---|
| `OWN` | Party ownership | An external user acts only on records where their party is a participant | A tenant sees only their own leases and invoices |
| `ASG` | Assignment | A field user acts only on records assigned to them | A technician sees only their tickets |
| `STA` | State | An action is allowed only in certain statuses | An issued invoice cannot be edited; it is corrected with a credit note |
| `LIM` | Limit | Above a threshold, a higher role must approve | Deposit deductions above AED 5,000 need a manager |
| `SOD` | Segregation of duties | The person who creates something cannot approve it | Whoever records a refund cannot approve it |
| `TIM` | Time window | Access exists only during a period | A tenant's portal becomes read-only after move-out and closes after 90 days |
| `SEN` | Sensitive data | Extra permission to view; every view is audited | Emirates ID and passport scans, medical records, salaries |
| `FLD` | Field visibility | Some fields are hidden from some roles | Owners see the tenant's name but not their phone or ID |

## 4. Evaluation

```
request
  → 1. authenticated?                      no  → login
  → 2. role grants the permission?         no  → 403
  → 3. … within this record's scope,
        and the assignment is current?     no  → 403
  → 4. record rules (OWN/ASG/STA/LIM/SOD/TIM) pass?
        LIM/SOD not met                    → create an approval request instead
        otherwise failing                  → 403
  → 5. render, applying field rules (SEN/FLD)
```

**Lists** apply layers 3 and 4 as query filters, so a user never receives rows they may not
see, and counts and pagination stay correct. Renox: `scopes_with`, `default_scope`.

## 5. Approvals

`LIM` and `SOD` rules do not just refuse. They turn the action into an **approval request**:
pending → approved / rejected, by a user who holds the approving permission in the same
scope. When it is approved, the action runs, recorded with both users. This is a shared
platform capability (`PLT.APR.01`, `wl-kernel`) used by every vertical.

## 6. What a client can configure

| Configurable | Not configurable |
|---|---|
| Custom roles built from existing permissions | New permissions or new rule patterns |
| Who holds which role, in which scope, for how long | Turning off layer 3 or 4 for a module |
| `LIM` thresholds (amounts, percentages) | Removing the audit of sensitive data |
| `TIM` durations | Giving external users staff permissions |
| Which role approves a `LIM`/`SOD` request | |
| Whether `SOD` is enforced (on by default) | |
| 2FA for staff (on by default) | |

Defaults are written in each rule's *Parameter (default)* column, in the vertical documents.

## 7. Renox fit

| Need | Renox provides | Gap |
|---|---|---|
| Permissions, roles, role management UI | `Permissions` module, roles page (as in the bike shop) | — |
| Scoped, time-bound role assignments | `assign_role_in`, `has_permission_in`, `scopes_with`, assignment dates | — |
| Record rules | Policies per model, gates, `gate_before` | Rules are code. Their parameters need the settings store (R2) |
| List filtering | `default_scope`, `scopes_with` | — |
| API access | Tokens with abilities, device tokens | — |
| 2FA | `renox-2fa` | — |
| Audit | `Audit` module | Logging *views* of sensitive fields, not only changes → `wl-kernel` |
| Approvals | — | Approval requests → `wl-kernel` (`PLT.APR.01`) |
| Field visibility | Templates can check `can(...)` | A consistent helper for `SEN`/`FLD` fields → `wl-kernel` |

## 8. Platform-wide rules

These apply to every vertical and are not repeated in vertical documents.

| ID | Rule | Pattern | Parameter (default) |
|---|---|---|---|
| `PLT.ACL.01` | Every action and list is denied unless a role grants it | — | — |
| `PLT.ACL.02` | Staff must enable two-factor login before the back office opens | — | on |
| `PLT.ACL.03` | Only users with `roles.manage` can grant roles, and they cannot grant a permission they do not hold | `SOD` | — |
| `PLT.ACL.04` | The platform operator account is disabled until enabled for an incident; each session is audited and notified to the client admin | `TIM`, `SEN` | session ≤ 24 h |
| `PLT.ACL.05` | External users are linked to exactly one person party and hold only portal roles; access to other parties comes from relationships and access grants | `OWN` | — |
| `PLT.ACL.06` | Sensitive data (identity documents, medical, salary, bank details) needs the permission of its sensitivity class (personal, health, financial; see [wl-documents](modules/wl-documents.md) §3); each view is audited. Vertical permission names such as `kyc.sensitive.view` are aliases of a class | `SEN` | — |
| `PLT.ACL.07` | Records are archived, not deleted, when money, tax or legal documents depend on them | `STA` | — |
| `PLT.ACL.08` | Users who leave lose all access at once; their open assignments are flagged for reassignment | `TIM` | — |

## 9. Shared access rule register

Every shared access rule, with the module that owns it and the vertical rules it generalises.
*In use* = referenced by vertical documents. *Module* = defined in the module specification.
*Proposed* = numbered here; the listed vertical rules are replaced by it when that vertical is
reviewed (the vertical row keeps the old ID as *formerly*).

| ID | Rule | Owner | Generalises | Status |
|---|---|---|---|---|
| `PLT.ACL.01`–`08` | Platform-wide rules (§8) | wl-kernel | — | in use |
| `PLT.ACL.09` | A record is voided only with a reason, from allowed states, never after it went to a third party or authority; approval if money or stock was posted | wl-kernel | — | module |
| `PLT.ACL.10` | An entry more than N hours late, or in a reopenable closed period, needs a manager's approval | wl-kernel | — | module |
| `PTY.ACL.01` | Guardian access ends when the dependant reaches the age of majority | wl-parties | `CLN.ACL.06`, `EDU.ACL.02` | in use |
| `PTY.ACL.02` | Restricted block reasons are visible only with `parties.block_reason.view`; others see "On hold" | wl-parties | — | module |
| `PTY.ACL.03` | No agreement is started, renewed or extended with a blocked party, and no money paid out, without a compliance exception | wl-parties | — | module |
| `POR.ACL.01` | After the relationship ends, the portal becomes read-only, then closes after a period per audience | wl-portal | `PRP.ACL.09`, `ACC.ACL.12`, `MPW.ACL.11` | in use |
| `DOC.ACL.01` | External users see only documents of subjects they own, and only kinds marked visible in the portal | wl-documents | — | module |
| `DOC.ACL.02` | A blocked transition runs only with a one-use override approved by another user; never for legal-block kinds | wl-documents | — | module |
| `DOC.ACL.03` | The expiry date or number of an accepted blocking document is never edited; a new version is accepted by another user | wl-documents | — | module |
| `NTF.ACL.01` | Marketing messages go only to recipients with recorded consent; list exports are audited | wl-notify | `CLN.ACL.11`, `FNB.ACL.10` | in use |
| `NTF.ACL.02` | No health or identity data in message bodies | wl-notify | — | module |
| `NTF.ACL.03` | A marketing broadcast above a recipient count needs a second approver | wl-notify | — | module |
| `NTF.ACL.04` | Marketing broadcasts come only from an active campaign; the user who last set its audience, template, channel or time is the requester | wl-notify | — | module |
| `FIN.ACL.01` | An issued invoice is never edited; it is corrected by a credit note | wl-invoicing | — | in use |
| `FIN.ACL.02` | A bounced cheque is marked replaced only by linking the replacement | wl-payments | — | in use |
| `FIN.ACL.03` | A supplier bill is approved only after a 3-way match within tolerance, otherwise by approval; a possible duplicate is held for a second user | wl-invoicing | `CON.ACL.10` (merged) | in use |
| `FIN.ACL.04` | Whoever requests or records a refund cannot approve it | wl-payments | `PRP.ACL.06`, `CLN.ACL.08`, `EDU.ACL.08` | in use |
| `FIN.ACL.05` | Cash is counted by someone other than its holder | wl-payments | `LOG.ACL.05` | in use |
| `FIN.ACL.06` | A cash shortage above a limit is written off only with approval | wl-payments | `LOG.ACL.06` | in use |
| `FIN.ACL.07` | A waiver or discount on an invoice or charge needs approval | wl-invoicing | `PRP.ACL.07` | in use |
| `FIN.ACL.08` | Costs, cost rates and margins are hidden from operational roles (`costs.view`) | wl-invoicing | `CON.ACL.07`, `FNB.ACL.02`, `AUT.ACL.07`, `MFG.ACL.07` | in use |
| `FIN.ACL.09` | No document dated in a locked VAT period is issued, changed or approved; late items go to the next open period | wl-invoicing | — | module |
| `FIN.ACL.10` | Whoever prepares a VAT summary cannot review it | wl-invoicing | — | module |
| `FIN.ACL.11` | A claimant cannot approve their own expense claim; a float holder cannot approve their own top-up | wl-payments | — | module |
| `FIN.ACL.12` | An expense claim above an approver's limit needs the next approval step | wl-payments | — | module |
| `FIN.ACL.13` | A claimant sees only their own claims; approvers see claims in their scope | wl-payments | — | module |
| `FIN.ACL.14` | Writing off a third-party payer's open amount above a limit needs approval | wl-invoicing | 05 claim `write_off` | in use |
| `FIN.ACL.15` | Writing off an invoice as bad debt above the limit needs approval; the requester cannot approve | wl-invoicing | — | module |
| `FIN.ACL.16` | Moving a payer overpayment to credit, or recording a recoupment, needs a second person | wl-invoicing | — | module |
| `FIN.ACL.17` | A supplier debit note not drafted from a return needs approval by someone other than its creator | wl-invoicing | — | module |
| `FIN.ACL.18` | Accepting a chargeback needs approval by someone other than the requester | wl-payments | — | module |
| `FIN.ACL.19` | Recovering a cash shortage from salary needs another person's approval and the holder's acknowledgement | wl-payments | — | module |
| `FIN.ACL.20` | A cheque collected in the field is deposited only after a handover counted by someone other than the collector | wl-payments | — | module |
| `FIN.ACL.21` | Every owner drawing needs approval by someone other than the requester | wl-invoicing | — | module |
| `FIN.ACL.22` | Only holders of `drawings.approve` see drawings lines, payees and notes; others see totals | wl-invoicing | — | module |
| `FIN.ACL.23` | In a one-owner company the owner's own drawing is recorded without a second approver, flagged self-approved, and the accountant is told | wl-invoicing | — | module |
| `SAL.ACL.01` | Cancelling or refunding a paid order above a value, or after fulfilment started, needs a manager | wl-sales | `FNB.ACL.11` (void after fire, proposed) | in use |
| `SAL.ACL.02` | A manual discount or a price below list beyond a limit needs a manager | wl-sales | `CLN.ACL.07`, `EDU.ACL.07`, `AUT.ACL.03`, `MFG.ACL.05` | in use |
| `SAL.ACL.03` | A credit hold is released, and an account hold cleared, only by a role with `credit.release`, not by the order's creator or the requester | wl-sales | `MFG.ACL.04`, `AUT.ACL.04`, `AUT.ACL.05` | in use |
| `SAL.ACL.04` | A completed counter sale is never edited; it is corrected by a return | wl-sales | `AUT.ACL.06` | in use |
| `SAL.ACL.05` | Amending or cancelling after fulfilment started, finishing to stock, scrapping or waiving the fee needs a manager | wl-sales | — | module |
| `SAL.ACL.06` | A refund or credit after completion with nothing returned needs a complaint category and responsible party; a manager above a limit | wl-sales | — | module |
| `SAL.ACL.07` | Large manual points adjustments, closing an account with points, and campaign cost over budget need a manager other than the requester | wl-sales | — | module |
| `SAL.ACL.08` | A member is attached to a sale only before it completes, never by the cashier to their own account; one account per mobile number | wl-sales | — | module |
| `SAL.ACL.09` | Changing the source or campaign of an acquired customer or won lead needs a reason and a second person | wl-sales | — | module |
| `WRK.ACL.01` | An external requester sees only their tickets and the comments marked visible | wl-ticketing | — | module |
| `WRK.ACL.02` | A field user or vendor sees only assigned tickets; requester contact only while open | wl-ticketing | `PRP.ACL.03` | in use |
| `WRK.ACL.03` | Confidential ticket categories are visible only with `tickets.confidential.view` | wl-ticketing | — | module |
| `WRK.ACL.04` | Completed checklist results are never edited, only voided with a reason | wl-ticketing | — | module |
| `WRK.ACL.05` | A driver sees only today's deliveries assigned to them | wl-delivery | `LOG.ACL.01`, `AUT.ACL.14`, `FNB.ACL.09` | in use |
| `WRK.ACL.06` | Proof of delivery is never edited; the only correction is an audited `reverse_delivery` | wl-delivery | `LOG.ACL.04` | in use |
| `WRK.ACL.07` | Identified feedback needs `feedback.view`; nobody sees identified feedback about their own work | wl-ticketing | — | module |
| `WRK.ACL.08` | Service contract value and terms need `costs.view` | wl-ticketing | — | module |
| `WRK.ACL.09` | A COD amount collected below the expected amount beyond tolerance needs approval; the driver cannot approve | wl-delivery | — | module |
| `WRK.ACL.10` | Reversing a delivery or marking it lost manually needs an operations manager's approval; the driver neither requests nor approves | wl-delivery | — | module |
| `WRK.ACL.11` | A ticket guarded by a vendor document cannot be resolved while the document is expired or missing | wl-ticketing | — | module |
| `SCH.ACL.01` | A portal user manages only bookings of their own party or dependants | wl-scheduling | — | module |
| `SCH.ACL.02` | Reschedule or cancel only until the service type's cut-off | wl-scheduling | — | module |
| `SCH.ACL.03` | Other resources' bookings show as busy/free only | wl-scheduling | `CLN.ACL.03` | in use |
| `SCH.ACL.04` | A resource cannot be booked when its eligibility (licence, skill) fails | wl-scheduling | `CLN.ACL.04` | in use |
| `SCH.ACL.05` | A deposit is forfeited only on a party late cancel, walk-out or no-show; a business-initiated cancel refunds it in full | wl-scheduling | — | module |
| `SCH.ACL.06` | Cancelling after check-in is staff only, with a reason, after work done is captured | wl-scheduling | — | module |
| `HRM.ACL.01` | A supervisor marks attendance only for their crews or site, and only for recent days | wl-attendance | `CON.ACL.01`, `MPW.ACL.13` | in use |
| `HRM.ACL.02` | An approved timesheet is reopened only by the approver and only before it is consumed; a locked one is corrected by an adjustment (`HRM.ACL.08`) | wl-attendance | `CON.ACL.09`, `MPW.ACL.08` | in use |
| `HRM.ACL.03` | Salaries, bank details and identity documents need `employees.sensitive.view` | wl-hr | `CON.ACL.08`, `MPW.ACL.03` | in use |
| `HRM.ACL.04` | Payroll runs and final settlements are prepared and approved by different users | wl-hr | `MPW.ACL.05`, `MPW.ACL.12` | in use |
| `HRM.ACL.05` | A pay account change needs a second approver, an SMS to the known number and a waiting period | wl-hr | — | module |
| `HRM.ACL.06` | Salary recoveries stay within the legal caps; no approval can exceed them | wl-hr | — | module |
| `HRM.ACL.07` | Non-standard exits and their reversal need a second user with `employees.exit.approve` | wl-hr | — | module |
| `HRM.ACL.08` | A locked timesheet is corrected only by an adjustment approved by someone other than its creator | wl-attendance | — | module |
| `HRM.ACL.09` | Attendance flagged for overtime cap, invalid permit or overlap is accepted or corrected only by `attendance.review`, not the marker | wl-attendance | — | module |
| `HRM.ACL.10` | Only the employee completes their own onboarding steps and SOP acknowledgements; an assignee cannot waive their own step | wl-hr | — | module |
| `OPS.ACL.01`–`05` | Operator-side rules (two-factor login, support sessions, second operator to delete data, no client business data, second operator for wave 2 and hotfix rollouts) | wl-operator | — | module |
| `STK.ACL.01` | The creator of a purchase order cannot approve it | wl-inventory | `CON.ACL.03` (merged) | in use |
| `STK.ACL.02` | Stock adjustments and write-offs above a value need approval | wl-inventory | `MFG.ACL.09`, `AUT.ACL.10`, `FNB.ACL.03` | in use |
| `STK.ACL.03` | Purchase requests and orders above value limits need approval, in steps | wl-inventory | `CON.ACL.02`, `FNB.ACL.06` | in use |
| `STK.ACL.04` | Stock is received and moved only at the user's own locations | wl-inventory | `CON.ACL.04`, `AUT.ACL.11` | in use |
| `STK.ACL.05` | Stock held for a listed reason (default recall) is released only by a second holder of `lots.release` | wl-inventory | — | module |
| `STK.ACL.06` | A landed cost adjustment or return to supplier above a value needs approval by someone other than its creator | wl-inventory | — | module |
| `STK.ACL.07` | Changing an item's preferred supplier needs approval by a second person | wl-inventory | — | module |

## 10. Lists for a user who holds a permission in some scopes (built in F1)

A route guard (`require_permission`) counts global roles only, so a building-scoped role
(`property_manager.bay`) got `403` on every module-wide list while the sidebar still showed
the link. The rule now (layer 3 of §3, for lists, §4): the list routes are guarded by
`wl_kernel::scopes::anywhere(permission)` (the permission globally **or in at least one
scope**; `403` only when it is held nowhere), and the handler narrows what it shows with
`wl_kernel::scopes::reach(db, user, permission)`: *everything* for a global holder, else the
records of the scopes where a role grants the permission today (dates honoured). A module
says which records lie in a scope with `register_members` (the vertical: units, leases,
agreements; `register_derived_members` for what is reached through others: invoices,
payments, deposits, credit notes, parties). The sidebar links use the same test
(`NavLink::permission` is "anywhere"), so a link shows exactly when its page opens
(`host/tests/scoped_lists.rs` opens every sidebar link of a scoped manager).

| List | In a scope when | Not in any scope |
|---|---|---|
| Invoices, credit notes (`/invoices`, `/credit-notes`) | the invoice was billed for the building (its scope unit); a credit note follows its invoice | invoices without a building: company-wide holders only |
| Payments (`/payments`) | allocated to an invoice of the building, prepaid for a lease of it, or the payment of a deposit held for it | a payment on account with no allocation: company-wide holders only |
| Refunds | the refunded payment, credit note or deposit is in the building | |
| Deposits, cheques | the lease (the agreement it was taken or received for) is in the building | |
| Tickets, compliance flags | the ticket's or flag's subject is the building, one of its units or leases | tickets about a vendor or a party: company-wide holders only |
| Document flags | the flagged record (lease) or the flag's document subject is in the building | |
| Parties | the tenant of a lease in the building, or the owner of an agreement with a unit there | a prospect, a vendor, a company with no lease: company-wide holders only |
| Management agreements | one of its units is in the building (a draft with no unit yet: company-wide only); a manager of the building may add its units to the agreement and give notice; **creating** one is company-wide | |
| Enquiries | about a unit of the building, or naming no unit (a prospect's first call belongs to no building, and holds no tenant or financial data) | |
| SOPs | company-wide documents: a scoped holder reads the active and withdrawn ones, never a draft | |
| Documents of a record (`/documents/of/...`) | the record is in the building (a lease) or a member of it (a tenant) | |

**Known gaps** (the pages open and are right for reading; these actions stay behind a global
permission, so a scoped manager sees the record and cannot change it there):

- the transitions and actions of the module pages: invoice transitions, ticket assign/hold/
  charges/service report, creating a ticket, payment and refund transitions, deposit
  deductions and refunds, cheque actions. The Property pages (lease, deposit, cheques of a
  lease, units) check the lease's building (`has_permission_in`) and are the scoped way to
  act. A scoped manager who must also work these pages needs the role globally or a custom
  role;
- roles given in a **legal-entity** scope are not applied to these lists (the budget and
  cash-flow pages do use them); only the building is;
- owner drawings and the payment follow-up tasks are company-level: a scoped holder gets
  `403` on a drawing and the tasks list is by role;
- `/documents/review` and the document kinds are company-wide (the reviewer role is global).
