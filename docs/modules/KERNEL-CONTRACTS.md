# Kernel contracts

**Build status (2026-10-24):** generated index of every `pub trait` in `crates/wl-kernel/src` (120 traits in 44 files, scripted from `grep`). A contract is the seam between two modules: the module that needs an answer asks the kernel trait, the module that knows the answer provides it. The provider column lists every crate or app with an `impl <Trait> for` outside tests; "plus a kernel default" means the kernel also ships a no-op or in-memory implementation, "kernel default only" that no module provides it, "tests only" or "none yet" means no production provider exists today (a client app or a later module provides it).

Regenerate by scanning `grep -rn "pub trait" crates/wl-kernel/src` for the traits and `grep -rn "impl .* for" crates apps host` (outside `tests/` and `#[cfg(test)]` modules) for the providers, after adding a trait; the narrative for each contract is in the spec of the module that owns it ([wl-kernel.md](wl-kernel.md) section 6 for the F1 group) and in `crates/wl-kernel/README.md`.

Traits with no production provider yet: `Checklists`, `Consents`, `CostPoster`, `CostRateResolver`, `FundsSource`, `PayrollCalendar`, `Quotations`, `SelfPayPrices`, `ShiftCalendar`, `ShortageRecovery`, `UnbilledTime`.

## `attendance.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `AttendanceHistory` | The days an employee was recorded as present: the last attended day an absence case counts from (`CON.RUL.08`, `MPW.RUL.09`). | `wl-attendance`; plus a kernel default |
| `AttendanceReview` | Lets a vertical send attendance to review. | `wl-attendance`; plus a kernel default |
| `ContextClosure` | Says that a context (a project, a deployment...) stops accepting attendance after a day (wl-attendance §4 `record`, 02 Construction §7.1 *Project* `terminate`). | `apps/construction`, `apps/manpower`; plus a kernel default |
| `CostRateResolver` | Says whether a context posts labour cost from approved hours and at what rate (wl-attendance §4 *Labour cost*, §8 `CostRateResolver`). | tests only (`wl-attendance`); plus a kernel default |
| `ShiftCalendar` | When a user's shift ends on a day (wl-attendance rosters, `HRM.ROS.01`): the clock `LOG.RUL.13` starts from for a run still out. | tests only (`apps/logistics`); plus a kernel default |

## `backorders.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `BackOrderDemand` | The back-ordered demand for the reorder suggestions (`STK.RUL.04`). | `wl-sales`; plus a kernel default |
| `StockReservations` | Reservations of stock for sales order lines (wl-inventory §3 *Stock balance*, *reserved*). | `wl-inventory`; plus a kernel default |

## `bank_statements.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `BankStatements` | Reads the bank statement imports for other modules. | `wl-payments`; plus a kernel default |

## `block_effects.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `AgreementSource` | The owning module's side of a review: it lists the party's open agreements and carries out the decider's choice. | `apps/auto`, `wl-ticketing` |

## `block_guard.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `BlockGuard` | Refuses guarded actions on a blocked party. | `wl-parties`; plus a kernel default |

## `broadcast.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `BroadcastRequesters` | Checks the requester of a broadcast. | `wl-sales`; plus a kernel default |

## `budgets.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `ActualCostSource` | Reads the actual that other modules own, for budget vs actual (`FIN.BUD.01`). | `wl-payments` |
| `CompanyBudgets` | What other modules ask of the company budget. | `wl-invoicing`; plus a kernel default |

## `cash_forecast.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `CashForecastSource` | Adds expected receipts and payments to the cash-flow forecast. | `apps/logistics`, `apps/property` |

## `chase_holds.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `ChaseHolds` | Says whether chasing for the records of one subject type is held. | `apps/property` |

## `cheques.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `BounceFees` | Adds the bounce fee charge of a bounced cheque. | `apps/property`; plus a kernel default |
| `ChargeInvoicer` | Invoices a scheduled charge that is not invoiced yet, for a cheque `write_off` (wl-payments §4 *Cheque*: "a linked charge not yet invoiced is invoiced first"): [`ChargeInvoicer::invoice`]. | `apps/property`; plus a kernel default |
| `Cheques` | Registers and reads cheques for other modules. | `wl-payments`; plus a kernel default |

## `consents.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `Consents` | Reads and withdraws consents. | tests only (`wl-notify`, `wl-sales`, `host`); plus a kernel default |

## `custody.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `CashCustody` | What other modules ask of cash custody. | `wl-payments`; plus a kernel default |
| `ShortageRaiser` | How a vertical raises a later cash shortage against a holder on behalf of the user who decides it (`FIN.ACL.05`). | `wl-payments`; plus a kernel default |
| `ShortageRecovery` | What wl-payments asks of wl-hr to deduct a shortage from wages. | tests only (`wl-payments`); plus a kernel default |

## `deadlines.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `DeadlineTasks` | Creates, holds, releases, waives and reads deadline tasks for the verticals. | `wl-ticketing`; plus a kernel default |

## `delivery.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `CodDifferenceRule` | A source module's rule on who may absorb a cash difference of its deliveries (03: the *client* only on its recorded instruction, never the *customer*). | `apps/logistics` |
| `DeliveryRequester` | How a source module asks for a delivery and takes the request back (wl-delivery §6 *Provides*). | `wl-delivery`; plus a kernel default |
| `DeliveryWastage` | How a discarded delivery's goods reach the stock books. | `wl-inventory`; plus a kernel default |
| `ManualRefunds` | A source module that pays back a payer who is no party itself (03: the recipient of a shipment is a person on the delivery, not a party record). | `apps/logistics` |

## `deposits.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `DepositDeductions` | Shows the published deductions to the portal and takes the party's dispute. | `wl-payments`; plus a kernel default |
| `DepositTopUps` | Raises the top-up charge of a deposit that grew. | `apps/property`; plus a kernel default |
| `Deposits` | What other modules do with deposits. | `wl-payments`; plus a kernel default |

## `documents.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `BlockSubjects` | Maps a guarded record to its document subjects. | `apps/accounting`, `apps/auto`, `apps/edtech`, `apps/manpower`, `apps/manufacturing`, `apps/property`, `wl-delivery`, `wl-scheduling`, `wl-ticketing` |
| `DocumentGuard` | The standard document guard. | `wl-documents`; plus a kernel default |
| `FutureWork` | Lists the planned work of a guarded transition that a document would not cover. | `apps/accounting`, `apps/edtech`, `apps/manpower`, `apps/manufacturing`, `wl-attendance`, `wl-delivery`, `wl-scheduling`, `host` |
| `ReminderSchedule` | Names the scopes whose staff get a document kind's reminders when the subject and the affected records have no scope. | `apps/manufacturing`, `host` |
| `SopAcknowledgements` | What the onboarding asks of the SOP acknowledgements (wl-documents §6). | `wl-documents`; plus a kernel default |

## `drawings.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `DrawingPaymentSource` | Reads the drawings payments for budget vs actual. | `wl-payments` |
| `OwnerDrawings` | What other modules ask of the owner drawings approval. | `wl-invoicing`; plus a kernel default |

## `einvoice.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `RecipientLookup` | Looks up a recipient identifier through an ASP adapter. | `wl-invoicing`; plus a kernel default |

## `held_funds.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `FundsSource` | The owning module's side: it lists what it holds for a party. | tests only (`wl-kernel`, `wl-parties`) |

## `inventory.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `BatchStock` | The stock side of a production batch (06 F&B §7.1 *Production batch*, wl-inventory §3 *Lot*). | `wl-inventory` |
| `DispatchDirectory` | The dispatches, for the verticals that guard or follow them: the item flags at `pick` and `dispatch` (07 `MFG.ACL.11`), the export evidence (`MFG.RUL.08`) and the recall trace. | `wl-inventory` |
| `DispatchGuard` | The item-flag guard of a dispatch (wl-inventory §4 *Dispatch* rows `pick` and `dispatch`, STK.DSP.01; 07 `MFG.ACL.11`): the vertical says why the goods on a dispatch may not be picked or may not leave, with no override. | `apps/manufacturing` |
| `DispatchMaker` | Makes the draft dispatch that delivers goods again (wl-inventory §5.4 step 1, 07 `MFG.RUL.10`): the new draft goes to the same recipient and address from the same location, with the item and lot of the original lines. | `wl-inventory` |
| `LotTrace` | The forward trace of a recall, for other modules (wl-inventory §6 `LotTrace`): the clinics vertical reads the patients to call back, 06 the customers. | `wl-inventory` |
| `PurchaseMatchSource` | The purchase order and goods-received lines for the supplier bill match (wl-inventory §6). | `wl-inventory`; plus a kernel default |
| `PurchaseOrderRules` | Rules a vertical adds to the purchase order (wl-inventory §4 *Purchase order* `issue` and `revise`). | `apps/construction`; plus a kernel default |
| `SaleLineStockSource` | The stock side of the lines of a sales order. | `apps/fnb` |
| `ScopeStock` | What a vertical does with the purchasing and the stock of one scope (a construction project, wl-inventory §3 *Location* scope link) when the scope stops: lists its open orders, cancels its open purchase requests and opens the count of its stores (02 Construction §7.1 *Project* `terminate`). | `wl-inventory`; plus a kernel default |
| `StockHoldService` | The stock hold service (wl-inventory §6 `StockHoldService`). | `wl-inventory` |
| `StockService` | The stock service other modules call (wl-inventory §6 `StockService`). | `wl-inventory` |
| `SupplierIncidentSource` | Quality incidents per supplier and date for the supplier scorecard (wl-inventory §8 `SupplierIncidentSource`). | `apps/manufacturing`; plus a kernel default |
| `SupplierReturnDrafts` | Drafts Returns to supplier for other modules (wl-inventory §6). | `wl-inventory` |
| `TracePartyResolver` | The party behind a subject in a recall trace (wl-inventory §6 `TracePartyResolver`), provided by the vertical: 05 visit to patient, 06 order to customer, 07 sales order or dispatch to customer, 08 job card to vehicle owner or fleet. | `apps/auto`, `apps/clinics`, `apps/fnb`, `apps/manufacturing` |

## `invoicing.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `CostPoster` | Posts and reverses cost on cost objects. | tests only (`apps/construction`, `wl-attendance`, `wl-hr`, `wl-inventory`, `wl-invoicing`); plus a kernel default |
| `Invoicer` | Drafts, reads and allocates invoices for other modules. | `wl-invoicing`; plus a kernel default |
| `LandedCosts` | Knows the landed costs of the app (owned by the inventory module). | `wl-inventory`; plus a kernel default |
| `PeriodLocks` | Knows which accounting periods are locked (VAT periods that were filed). | `wl-invoicing`; plus a kernel default |
| `SupplierBills` | Reads supplier bill lines. | `wl-invoicing`; plus a kernel default |
| `SupplierDebitNotes` | Drafts and reads supplier debit notes. | `wl-invoicing`; plus a kernel default |

## `lot_disposal.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `DispatchLots` | The lots a dispatch sent (`MFG.RUL.04`: an invoice line is drafted from a dispatch line, and the dispatch line records its lots). | `wl-inventory`; plus a kernel default |
| `LotDisposal` | The lot operations an NCR uses. | `wl-inventory`; plus a kernel default |
| `LotReleaseGuard` | Whether something outside stock keeps a lot from being released: 07 keeps a lot with an NCR `open` or `dispositioned` (§7.1 *NCR*). | `apps/manufacturing`; plus a kernel default |

## `notify.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `Notifier` | Sends notifications for other modules. | `wl-notify`; plus a kernel default |

## `parties.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `PartyDirectory` | Reads parties for other modules. | `wl-parties`; plus a kernel default |

## `payer_bills.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `PayerBills` | Drives payer bills for a vertical. | `wl-invoicing`; plus a kernel default |
| `PayerCredits` | What wl-payments reads of a payer's credit balance (wl-invoicing §3 *Payer credit balance*): the refundable balance guard of a refund whose source is the balance. | `wl-invoicing`; plus a kernel default |

## `payments.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `PaymentLinks` | Makes payment links for other modules. | `wl-payments`; plus a kernel default |
| `PaymentRecorder` | Records, allocates, reverses and reads payments for other modules. | `wl-payments`; plus a kernel default |
| `PrepaidSubjects` | Whether the record a payment was made in advance for still waits for the money. | `wl-scheduling`; plus a kernel default |

## `payroll_calendar.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `PayrollCalendar` | The dates of a payroll period, per legal entity. | tests only (`wl-hr`); plus a kernel default |

## `placements.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `Placements` | The placements of a worker. | `apps/manpower`; plus a kernel default |

## `portal.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `PortalUsers` | Who acts for a party in the portal. | `wl-portal`; plus a kernel default |

## `production.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `BillsOfMaterials` | The bills of materials (wl-inventory §4 *BOM*). | `wl-inventory`; plus a kernel default |
| `OrderLines` | The sales order lines a work order serves (wl-sales §6). | `wl-sales`; plus a kernel default |
| `ProductionStock` | The stock movements of a work order, posted inside the caller's transaction (07 §7.1 *Work order*: `abort` posts them in one transaction at the final approval; a production report posts its `produce`). | `wl-inventory`; plus a kernel default |

## `qualifications.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `Qualifications` | The standard qualification guard. | `wl-hr`; plus a kernel default |

## `quotations.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `Quotations` | Reads the terms of the quotation a record was opened from. | tests only (`apps/logistics`) |

## `rate_cards.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `RateCards` | The rate cards of wl-invoicing. | `wl-invoicing`; plus a kernel default |

## `sales.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `CampaignAttribution` | What verticals ask of the campaign attribution (wl-sales §6). | `wl-sales`; plus a kernel default |
| `CreditChecker` | What the dispatch of an order on account asks of the credit control (wl-sales §6 `CreditChecker`). | `wl-sales`; plus a kernel default |
| `CreditExposure` | The credit position of a customer. | `wl-sales`; plus a kernel default |
| `CustomerPoCheck` | Whether a customer PO number is used already (wl-sales `BL-SAL-18`, setting *Unique customer PO number per customer*). | `wl-sales`; plus a kernel default |
| `DeliveryDestination` | Where the goods of a sales order are delivered, for a vertical that treats export differently from domestic delivery (07 single-use plastics: *export only* items). | `wl-sales`; plus a kernel default |
| `DiscountEngine` | What the sales and POS flows ask of the discount rules (wl-sales §6 `DiscountEngine`). | `wl-sales`; plus a kernel default |
| `FulfilmentProgress` | What the vertical knows about the fulfilment of an order: which lines have started (goods picked, a work order or production batch started, the kitchen started the line). | `apps/fnb`, `apps/manufacturing`; plus a kernel default |
| `LeadConversion` | What verticals ask to mark a lead won from their conversion event (wl-sales §4.1). | `wl-sales`; plus a kernel default |
| `OrderFulfilment` | What `wl-inventory`, `wl-delivery` and the verticals' own fulfilment ask of a sales order (wl-sales §6 `OrderFulfilment`). | `wl-sales`; plus a kernel default |
| `OrderLineRules` | What a vertical refuses on a sales order line at `submit` and `amend` (07: a domestic line for an *export only* item). | `apps/manufacturing`; plus a kernel default |
| `OutcomeProposals` | The outcome a vertical proposes for the started goods of a line that is cancelled (wl-sales §4.2 *Cancelling after fulfilment started*, 07 §7.1): *finish to stock* for a standard item, *scrap* for an item made for one customer (printed labels and packaging). | `apps/manufacturing`; plus a kernel default |
| `PackageLedger` | What verticals and `wl-scheduling` ask of the prepaid packages (wl-sales §6). | `wl-sales`; plus a kernel default |
| `PackageSales` | What verticals ask to sell a prepaid package (wl-sales §4.4, §5.5): the package starts `pending_payment` and activates when its invoice is paid in full. | `wl-sales`; plus a kernel default |
| `PromisedLines` | The confirmed order lines not yet picked (wl-sales §6 `PromisedLines`). | `wl-sales`; plus a kernel default |
| `SaleCustomers` | The customer of a sale record. | `wl-sales`; plus a kernel default |
| `SalesOrderDesk` | What a vertical may ask of the sales orders without depending on `wl-sales`: read an order with its lines, accept or reject it as a user, cancel it as the system, change it as a user (amend, cancel a line or the order) and refund it after completion. | `wl-sales`; plus a kernel default |
| `SelfPayPrices` | The self-pay price list of the service catalogue (`SAL.CAT.01`): what a vertical quotes a party who pays a service itself, such as the estimate on the self-pay consent of 05 (`CLN.ACL.15`). | tests only (`apps/clinics`); plus a kernel default |
| `TillDesk` | What a vertical may ask of the till sessions without depending on `wl-sales`: open one, read it, count and close it, reopen it and mark it reviewed. | `wl-sales`; plus a kernel default |

## `scheduling.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `AvailabilitySource` | Another module's blocks of a resource (wl-scheduling §8): leave from `wl-hr`, rosters from `wl-attendance`, sessions from an app. | `apps/edtech`; plus a kernel default |
| `BookingActions` | What a vertical may do to a booking from its own record (wl-scheduling §8 `Bookings`): link the record it opened at check-in, so that a cancellation after the check-in asks [`WorkDoneStatus`] about it, and complete the booking when that record is done (08: the job card is delivered). | `wl-scheduling`; plus a kernel default |
| `BookingGate` | May a new booking be made for this subject (wl-scheduling §4.1 `book`, `request`)? | `apps/auto`; plus a kernel default |
| `CheckInGuard` | May this party be checked in (wl-scheduling §4.1 `check_in`)? | `apps/clinics`; plus a kernel default |
| `CompleteGuard` | May this booking be completed (wl-scheduling §4.1 `complete`)? | `apps/clinics`; plus a kernel default |
| `ResourceEligibility` | May this resource deliver this service at this time (wl-scheduling §8, `SCH.ACL.04`)? | `apps/edtech`; plus a kernel default |
| `ResourceNames` | The names of the resources a module books (a tutor, a room), by id: what a vertical shows where it holds only a resource's id. | `wl-scheduling`; plus a kernel default |
| `ServiceNames` | The names of the services a module books, by code, in a language: what a vertical shows where it has only a booking's service code (`dental_checkup` is *Dental check-up*). | `wl-scheduling`; plus a kernel default |
| `WorkDoneStatus` | What was done for a booking (wl-scheduling §8, `SCH.ACL.06`)? | `apps/auto`, `apps/clinics`; plus a kernel default |

## `screening_guard.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `ScreeningGuard` | What `wl-parties` provides. | `wl-parties`; plus a kernel default |

## `segments.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `Segments` | Resolves segments to parties. | `wl-sales`; plus a kernel default |

## `subscriptions.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `Subscriptions` | Starts, pauses, resumes and cancels subscriptions for the verticals. | `wl-invoicing`; plus a kernel default |

## `tickets.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `Checklists` | Checklist runs for other modules (wl-ticketing §6). | tests only (`apps/manufacturing`, `wl-hr`); plus a kernel default |
| `ResolveRule` | A vertical's rule on who may resolve the tickets of one category (08 Auto: a vehicle incident is closed only with the branch manager's liability decision, taken on the job card, never by an involved person). | `apps/auto` |
| `ServiceContractCoverage` | What other modules ask of the service contracts. | `wl-ticketing`; plus a kernel default |
| `TaskCreator` | Opens tickets for other modules. | `wl-ticketing`; plus a kernel default |

## `timesheets.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `ApproverResolver` | Who stands behind the timesheets of a context (a deployment, a project...) that an external approver approves, as the context's contract says (wl-attendance §8 `ApproverResolver`). | `apps/manpower`; plus a kernel default |
| `TimesheetClosing` | Closes the open timesheet of a context when the work ends (09 Manpower §7.1 *Deployment* `end` and *Client contract* `end`: "each open timesheet closed at the end date and submitted to the client at once"). | `wl-attendance`; plus a kernel default |
| `Timesheets` | The timesheet service payroll uses (wl-attendance §6 `Timesheets`, §5.7). | `wl-attendance`; plus a kernel default |

## `usage.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `UsageRecorder` | Records priced usage and bills it (wl-invoicing §6 `UsageRecorder`). | `wl-invoicing`; plus a kernel default |

## `visa_steps.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `VisaSteps` | The open steps of the visa cases. | `wl-hr`; plus a kernel default |

## `wip.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `UnbilledTime` | Reads and writes off the unbilled time of an agreement. | tests only (`apps/accounting`); plus a kernel default |

## `workforce.rs`

| Trait | Purpose | Providers |
|---|---|---|
| `LeaveCalendar` | The approved leave of an employee (wl-hr §4 *Leave request*): what the verticals need to tell an absence from a leave (`CON.RUL.08`). | `wl-hr`; plus a kernel default |
| `Workforce` | What other modules ask of the workforce. | `wl-hr`; plus a kernel default |
