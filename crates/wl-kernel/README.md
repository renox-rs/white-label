# wl-kernel

The base every `wl-*` module and vertical app stands on
([spec](../../docs/modules/wl-kernel.md)). It turns one Renox binary into one client's
branded product.

The host reads the client's `client.toml` and applies the kernel to the app:

```
use renox::prelude::*;
use wl_kernel::ClientConfig;

let config = ClientConfig::parse(r#"
    [client]
    name = "Acme Properties LLC"
    app = "property"

    [modules]
    enabled = ["parties", "invoicing"]
"#)?;
let app = wl_kernel::apply(App::new(), config)?;
# let _ = app;
# Ok::<(), wl_kernel::ConfigError>(())
```

`apply` fails, and the boot stops, when a module needs one that is not enabled, when a
brand colour is not readable (the message names the nearest colour that is), or when the
file is malformed.

What is here so far:

| Part | Spec | Module |
|---|---|---|
| `client.toml` and the module manifest | §3, §5.1 | `config`, `manifest` |
| Branding on Renox `Theme::from_brand` | §2 `PLT.ORG.01` | `branding` |
| `Subject` references | §2 | `subject` |
| Legal entities, scope links, issuer snapshots (`PLT.ENT.01`) | §5.6, decision 0003 | `entities` |
| Document numbering on Renox `Sequence` | §5.5 | `numbering` |
| State machines by name: `SubjectMachine`, `back_to_<state>` and `<action>_approved` (always `system`) | §4.1, WORKFLOW §3.4 | `machines` |
| Approvals (`PLT.APR.01`): `ApprovalPolicy` (who must approve an action, `PolicyDefault`, `Require`, `Required`), `ApprovalRequest` (its machine has only `system` transitions; `ApprovalStatus`, `ApprovalEnd`), `ApprovalDecision`, the Renox `ApprovalInterceptor` that turns a transition over a limit into a request, `approvals::approve`, `reject`, `return_request`, `withdraw`, `expire`, `awaiting` (the requests a user can decide), `register_final_check`, and the events `ApprovalRequested` and `ApprovalDecided` | §3, §4.1, WORKFLOW §3.4 | `approvals` |
| Voids of records entered in error (`PLT.ACL.09`): `Voidable` (`Voidable::of` registers an entity with its permission, scope, kept number, third-party flag and one reverse handler per `EffectKind`), `VoidRecord` (table `wl_voids`), `RecordVoided`, `Effect`, `Reverse`, `ReverseError`, `voids::void`, `exclude_voided`, `is_voided`, `ensure_editable` | §5.12 | `voids` |
| Tasks for a role: `role_tasks::open` with a `RoleTask` (a ticket through `TaskCreator` assigned to one holder of the role, or of `COMPANY_ADMIN_ROLE` when nobody holds it; `None` without `ticketing`), `role_tasks::assignee`; used for the reconciliation task of a recovered connection and the owner's task of a failed compliance-critical rule | §5.14, §5.15 | `role_tasks` |
| The block guard contract: `BlockGuard` (`check_in` inside the caller's transaction; `NoBlockGuard` when `parties` is off allows everything), `provide_block_guard`, `block_guard::guard`, `run_guarded`, `request_exception_in`, `ActionKind`, `GuardOutcome`, `BlockReasonLine`, `BLOCK_EXCEPTION_ACTION` (see below) | ARCHITECTURE §5, wl-parties §6 | `block_guard` |
| The screening guard contract: `ScreeningGuard` (`check_in`, `latest_notice_in`; `NoScreeningGuard` when `parties` or screening is off allows everything), `provide_screening_guard`, `screening_guard`, `run_screened` (see below) | wl-parties §7 | `screening_guard` |
| The delivery contract: `DeliveryId`, `DELIVERY_SUBJECT_TYPE`, `ReviewOutcome` and the events `DeliveryDisputed`, `DeliveryDisputeClosed`, `DeliveryReversed`, `DeliveryLost` (`wl-delivery` is not built yet; see *Deliveries* below) | wl-delivery §6 | `delivery` |
| The party contract: `PartyDirectory` (`NoParties` when `parties` is off; `search` is what a party picker in another module's form asks, default none), `PartyStatus`, party events, the `party` series, `contacts` (`PartyContacts`), `related_contacts` (`RelatedContact`, `choose_contact`), `billing_identity` (`BillingIdentity`, with the e-invoicing recipient identifier), `PartyUpdated` (changed field names only), `guardians` (default none), `ScreeningId`, `ScreeningStatus`, `PartyRestored`, `ScreeningCompleted`, `ScreeningFlagged` (a possible match) and `ScreeningMatchConfirmed` (a confirmed match); the last two are for compliance listeners only, check `parties.block_reason.view` | ARCHITECTURE §5, wl-parties §6 | `parties` |
| E-invoicing recipient lookup: `RecipientLookup` (`NoRecipientLookup` when `invoicing` is off; the ASP adapter of a legal entity finds a company's Peppol participant ID by TRN), `einvoice::lookup` (asks only while the entity's ASP connection is `up`, reports the call to the integration health; `Lookup::Found`, `NotFound`, `Failed`, `Unavailable`) and the events `EInvoiceAccepted` and `EInvoiceRejected` of an e-invoice transmission | wl-parties §3, wl-invoicing §4 | `einvoice` |
| The invoicing contract: `Invoicer` (`NoInvoicer` when `invoicing` is off; `update_draft` replaces the lines of a draft and does nothing once it is issued; `discard_draft` deletes a draft and never an issued invoice; `issue` issues a draft through the invoice's own `issue` transition (a user needs `invoices.issue`; `None`, the system, only where the module issues that source automatically; refused by default); `write_off` writes off an invoice's balance for a user and answers `WrittenOff` or the approval request that waits), `InvoiceDraft`, `DraftLine` (`net`, `vat`, `gross` in fils, half-up), `InvoiceSummary`, `VatCategory`, invoice events, the `invoice` series | ARCHITECTURE §5, wl-invoicing §6 | `invoicing` |
| The subscription contract: `Subscriptions` (`NoSubscriptions` when `invoicing` is off): `start` a subscription from `StartSubscription`, `find`, `of_agreement`, and, for the vertical's agreement, `pause` with a `PauseSubscription` (cause `agreement`, `user` with its end date; a party block is handled by wl-invoicing itself), `resume` (removes only that cause) and `cancel` with a `CancelSubscription` (at once, prorated unless `settled`); `SubscriptionSummary`, `SubscriptionStatus`, `BillingCycle`, `PaymentMode`, `PauseCause`, events `SubscriptionPastDue`, `SubscriptionSuspended`, `SubscriptionSettled`, `SubscriptionCancelled`, and, per pause cause added or removed, `SubscriptionPaused` and `SubscriptionResumed` (so 10 can freeze and unfreeze the enrolment of a membership a user paused, `EDU.RUL.14`) | ARCHITECTURE §5, wl-invoicing §4 *Subscription*, §6 | `invoicing` |
| The deadline task contract: `DeadlineTasks` (`NoDeadlineTasks` when `ticketing` is off): `create` a task from `NewDeadlineTask` (source agreement, type, title, due date, lead days, party, owner, reviewer role, evidence required, client input, `TaskRecurrence`), `find`, `of_subject`, and, for the vertical's agreement, `hold` / `release` (past-due tasks made `overdue`, the ones that went overdue while held listed for the manager: `ReleasedTasks`), `waive` one task with a `WaiveTask`, and `end_source` (upcoming tasks waived with `SOURCE_ENDED_REASON`, no next occurrence: `EndedTasks`); `DeadlineTaskSummary`, `DeadlineTaskStatus`, events `TaskOverdue`, `TaskCompleted` | wl-ticketing §4.4 *Task*, §6 `Tasks` | `ticketing` |
| The supplier-bill part of the invoicing contract: `SupplierBills` (`line`; `NoSupplierBills` finds none, so a module hides its bill-link option), `provide_supplier_bills`, `SupplierBillId`, `SupplierBillLineId`, `SupplierBillLine` (`amount_aed`), `SupplierBillStatus` (the seven wl-invoicing states), `SupplierBillApproved`, `SupplierBillVoided`; `LandedCosts` (`entity_of`, the legal entity of a landed cost a bill line is coded to; `NoLandedCosts` knows none, so such a line is refused), `provide_landed_costs`, `landed_costs` (the inventory module provides it) | wl-invoicing §3, §6; wl-sales | `invoicing` |
| The supplier debit note part of the invoicing contract: `SupplierDebitNotes` (`draft_for_return`, `find`, `debit_balance`, `settle_from_payment`, `cancel`; `NoSupplierDebitNotes` refuses 422 / finds nothing / 0 / refuses 422 / refuses 422), `provide_supplier_debit_notes`, `SupplierDebitNoteId`, `SupplierDebitNoteStatus` (draft, issued, settled, cancelled), `supplier_debit_note_series(active_entities)` (`DN-ACME-2026-0001`), `SupplierDebitNoteIssued`, `SupplierDebitNoteSettled`, `SupplierDebitNoteCancelled`, `AllocationTarget::SupplierDebitNote` (`invoicing.supplier_debit_note`), `SupplierReturnDispatched`; `CostPoster` (`post(PostCost)`, `reverse(ReverseCost)`, `restore(ReverseCost)`; `NoCostPoster` is a no-op), `post_cost` (the call every caller posts through: a cost code mapped to owner drawings is refused 422, `FIN.ACL.21`), `provide_cost_poster` | wl-invoicing §3, §4, §6 | `invoicing` |
| The sales contract: `CampaignId`, the `sales.campaign` and `sales.campaign_cost_entry` subject types, `CampaignActivated`, `CampaignEnded`; the lead and acquisition part: `LeadId`, the `sales.lead` subject type, `LeadCreated`, `LeadWon`, `LeadLost`, `LeadWinReverted`, `CustomerAcquired`, `CustomerAcquisitionChanged`, `CampaignAttribution` (`NoCampaignAttribution` when `sales` is off: `attribute`, `record_first_sale`, `sale_gone`; `Attribute`, `FirstSale`) and `LeadConversion` (`NoLeadConversion`: `win`, `win_for_party`); the events `InvoiceIssued` (net total, issue date) and `CreditNoteIssued` (net amount) carry the values the first sale needs, and `PartiesMerged` (survivor, merged) is the wl-parties merge event | wl-sales §6 | `sales` |
| The package sale part of the sales contract: `PackageSales` (`sell` records a package `pending_payment` for a holder and returns its `PackageId`; `NoPackageSales` sells nothing, so an enrolment for a package has none), `SellPackage` (holder, payer, item, credits, price, validity, invoice, seller), `provide_package_sales`, `package_sales`; the credits are then used and given back through `PackageLedger` (`consume`, `reverse`, `balance`), and the days an enrolment stood frozen are given back with `PackageLedger::extend_days` (the system's `extend`, not counted against the extension limit; the default moves nothing) | wl-sales §4.4, §6; 10 `EDU.ENR.01` | `sales` |
| The workforce contract: `Workforce` (`NoWorkforce` when `hr` is off: `employees_in(role, scope)`, `all_staff`, `staff` with the `StaffProfile` of one employee, `employee_of_user`, `is_freelance`), `EmployeeId`, `OnboardingId`, the `hr.employee`, `hr.onboarding` and `hr.onboarding_step` subject types, and the events `EmployeeActivated`, `EmployeeOnboardingCancelled`, `EmployeeExited`, `EmployeeDeceased`, `EmployeeAssignmentChanged`, `OnboardingStarted` and `OnboardingCompleted`, and the payroll run's `PayrollRunId` (subject type `hr.payroll_run`) with the events `PayrollRunReturned`, `PayrollLinesFailed` (with `FailedPayrollLine`) and `PayrollRunPaid`, and the final settlement's `SettlementId` (subject type `hr.settlement`) with the events `SettlementPaid` and `SettlementDisputed`; `Checklists` (`NoChecklists` when `ticketing` is off: `start` a run on a record, `void` it), `ChecklistCompleted`; `SopAcknowledgements` (`NoSopAcknowledgements` when `documents` is off: `pending_for`, `acknowledge_in_person`), `SopStaff` and `SopAcknowledged` | wl-hr §6, wl-ticketing §6 (`WRK.CHK.01`), wl-documents §6 | `workforce`, `tickets` (`Checklists`), `documents` (`SopAcknowledgements`) |
| The payment contract: `PaymentRecorder` (`NoPayments` when `payments` is off), `RecordPayment`, `PaymentSummary`, `PaymentMethod`, `ReversalReason`, `AllocationTarget`, `PaymentReceived`, `PaymentReversed`, `RefundPaid`, `RefundFailed`, `RefundSource`, `RefundRequest`, `RefundFailureKind`, `DepositId`, the `receipt` and `payment_voucher` series | ARCHITECTURE §5, wl-payments §6 | `payments` |
| The cheque contract: `Cheques` (`NoCheques` when `payments` is off; register a received or issued cheque, register the post-dated cheques of an agreement with `ChequeSchedule`, read them per agreement, the open bounced amount of a party, the cheques not cleared with `OpenCheque` and `ChequeDocument`; `return_undeposited` hands cheques that are not deposited back against one handover receipt, `ReturnCheques`, `ReturnedCheques`; `relink` moves a cheque's link from one document to another, such as the charge of an amended lease; `hold_as_evidence` marks cheques that stay in the register and are never deposited, such as those of a lease ended by a forced termination), `RegisterCheque`, `ChequeSummary`, `ChequeStatus`, `ChequeId`, `BounceFees` (`NoBounceFees`: the bounce fee charge wl-invoicing adds), `ChargeInvoicer` (`NoChargeInvoicer`: invoices a scheduled charge before a cheque `write_off`), `ChequeBounced` and `ChequeBounceResolved` | ARCHITECTURE §5, wl-payments §4 *Cheque*, §6 | `cheques` |
| The bank statement contract: `BankStatements` (`NoBankStatements` when `payments` is off; the latest closing balance of each bank account, `BankBalance`; the lines marked as owner drawings, `DrawingLine`; events `BankStatementImported`, `BankStatementCompleted`) and the cash-flow reads of `PaymentRecorder` (`payments_after` with `BankPayment`, `refunds_unpaid` with `UnpaidRefund`) | ARCHITECTURE §5, wl-payments §3 *Bank statement import*, §6 | `bank_statements`, `payments` |
| The cash-flow forecast extension: `CashForecastSource` (registered by name with `register_cash_forecast_source`; expected receipts and payments of an entity as `ForecastLine`s that name the documents they stand for; also what the source paid after a bank balance (`paid_after`: payroll runs and WPS-paid final settlements), its warnings and the days its documents were paid late (`paid_late`: payer bills)), `ActualCostSource` (registered with `register_actual_costs`; paid payroll runs and expense claim lines that budget vs actual maps through their key), `Cheques::bounced` (the received cheques still owed, with their documents) and the event `CashForecastBelowThreshold` (an entity's projected balance falls below its threshold) | ARCHITECTURE §5, wl-invoicing §8, `FIN.CFL.01` | `cash_forecast`, `budgets` (`ActualCostSource`), `cheques` |
| The deposit contract: `DepositDeductions` (`NoDepositDeductions` when `payments` is off; what the portal shows of the published deductions and the party's dispute of lines), `PublishedDeductions`, `DisputeDeductions`, `DeductionKind`, and the events `DepositDeductionsPublished`, `DepositDeductionsDisputed`, `DepositDisputeAnswered` and `DepositSettled`; `Deposits` (`NoDeposits`: `adjust` the amount of a held deposit when its source changes, from a lease amendment or renewal; `held_for` a source; `settle_cancelled`, which starts the settlement of the deposit of a cancelled agreement and publishes the forfeited part with its clause; `start_settlement`, which starts the settlement of the deposit of an agreement that ended, such as a terminated lease, and publishes nothing yet), `HeldDeposit`, `CancelSettlement`, `SettlementStarted`, `DepositTopUps` (`NoDepositTopUps`: the top-up charge of an increase), `DepositAdjusted`, and `Deposits::transfer` (hands a held deposit to a new holder against its receipt: `TransferDeposit`, `DepositTransferred`) and `Deposits::in_settlement` (a deposit in `settlement_pending` cannot be transferred), `Deposits::apply_to_deductions` (forfeits a deposit with the reason `APPLIED_TO_DEDUCTIONS` and applies it to the unpaid invoices named in `ApplyDeposit`, `ApplyToInvoice`, result `DepositApplied`) | ARCHITECTURE §5, wl-payments §4 *Deposit*, §5.4, §6 | `deposits` |
| The company budget contract: `CompanyBudgets` (`NoCompanyBudgets` when `invoicing` is off; the approved amount and the actual per entity, category and month, and whether a budget category is owner drawings), `CompanyBudgetId`, the `invoicing.company_budget` subject type and the events `CompanyBudgetApproved` and `CompanyBudgetVarianceReached` | ARCHITECTURE §5, wl-invoicing §3, §5.15, §6 | `budgets` |
| The owner drawings contract: `OwnerDrawings` (`NoOwnerDrawings` when `invoicing` is off; the drawings category and its owner party, and the approval request of a drawing on a record of the caller), `DrawingRequest`, `DrawingStart`, `DrawingCategory`, the event `OwnerDrawingDecided`, and the `DrawingPaymentSource` registry that budget vs actual reads the drawings payments from (`DrawingPayment`); `RegisterCheque::drawing` | ARCHITECTURE §5, wl-invoicing §5.16, wl-payments §3, §5.3, `FIN.ACL.21` | `drawings`, `cheques` |
| The cash custody contract: `CashCustody` (`NoCustody` when `payments` is off; `collect` a holder's cash into the handover of a run, shift or day with `CashCollection` and `HandoverPeriod`, `release` it again while the handover is open, `end_period` to start the escalation clock, `handover_of`, `find`), `HandoverSummary`, `HandoverStatus`, `HandoverKind`, `PeriodKind`, the events `CashHandoverCounted`, `CashShortage` and `CashShortageSettled`, and `ShortageRecovery` (`NoShortageRecovery`: the salary deduction wl-hr will carry out, `ShortageDeduction`); `DeliveryDelivered::cod_absorption` (`CodAbsorption`, `CodAbsorber`) and `billable_milli` for the COD difference and the delivered quantity | wl-payments §3, §4 *Cash handover*, §6 `FIN.CSH.01`, `FIN.ACL.19`; wl-invoicing §5.2 step 4 | `custody`, `delivery` |
| The notice contract: `Notifier` (`NoNotifier` when `notify` is off), `register_type`, `NotifyRequest`, message events, `NoticeStatus`, `UndeliveredNotice`, `ProofMethod`, `StatutoryNoticeProofRecorded`; `Notifier::broadcast` (refused by `NoNotifier`) | ARCHITECTURE §5, wl-notify §6 | `notify` |
| The document guard contract: `DocumentGuard` (`NoDocuments` when `documents` is off), `BlockEntry`, `BlockSubjects` by transition, `BlockRefusal`, `today()` in Asia/Dubai, document events | ARCHITECTURE §5, wl-documents §5.6, §6 | `documents` |
| The ticket contract: `TaskCreator` (`NoTickets` when `ticketing` is off), `OpenTicket` (`bill_to`: the party a chargeable ticket is invoiced to when the requester is a user; `planned_due`: the planned due date of a planned visit; `under_contract`: the service contract a planned visit is handled under), `TicketRef`, `ComplianceFlagReason`, `ComplianceFlagRaised` and `ComplianceFlagCleared` (subject, category, reason, contract), `ServiceContractId`, `ServiceContractActivated`, `ServiceContractEnded` (contract, outcome, new version, visits cancelled, visits left) and `VendorNoShow` (ticket, vendor, contract, scheduled at), `Assignee`, ticket events, the `ticket` and `service_contract` series; `ServiceContractCoverage::covers` (`NoServiceContractCoverage` when `ticketing` is off: whether a contract covers work done on a day, up to its end or termination date) and `ContractCover` | §8, wl-ticketing §6 | `tickets` |
| The chase-hold contract: `ChaseHolds` (a vertical registers a provider for its subject type with `register_chase_holds`, e.g. `property.lease`, which says whether chasing what a record is owed is `Held` while a legal case is open, `Resumed` after it ended, or `Free`), `ChaseHold`, `chase_holds::hold` (a source with no provider is free); `wl-invoicing` reads it for the reminders of an invoice and `wl-payments` for the bounce reminders of a cheque; the event `ChequeReferredToLegal` of a bounced cheque referred to a legal case; `Deposits::set_legal_case` (keeps the deductions dispute flag of a deposit in settlement while a legal case is open); `TaskCreator::open_of` (the open tickets of a category about a subject) | 01 Property §7.1 *Lease* (Legal case flag), wl-invoicing §5.3, wl-payments §4 | `chase_holds`, `cheques`, `deposits`, `tickets` |
| The block-effect contract: `BlockEffect` (pause / continue / review) per agreement kind, the 12 built-in kinds, `AgreementKind` + `AgreementSource`, `BlockEffectOverride`, `effect_for`, `announces_pause` and `suppressed_while_blocked` | ARCHITECTURE §5, wl-parties §5.6, §7 | `block_effects` |
| The held-funds contract: `HeldFundsSource` + `FundsSource` (a module lists the money it holds for a party, read only), `HeldFund`, `held_for_all`; pre-fills the freeze task of a confirmed sanctions match | wl-parties §5.4, §7 | `held_funds` |
| The segment contract: `Segments` (`NoSegments` has no members), `SegmentId`, `provide_segments` | wl-notify §4.3 | `segments` |
| The consent contract: `Consents` (`has_consent`, `withdraw`; `NoConsents` grants nothing), `provide_consents`, `MARKETING` | wl-notify §4.3, `NTF.ACL.01` | `consents` |
| The broadcast contract: `BroadcastRequest`, `BroadcastRequesters` + `RequesterCheck` (`NoRequesters` answers `Ended`, so every marketing request is refused), `BroadcastId`, `BroadcastCompleted`, `BroadcastCancelled` | wl-notify §4.3, §6, `NTF.ACL.04` | `broadcast` |
| `AutomationFailed`, the event of a failed rule run | §4 | `automation` |
| The daily digest of voids and late entries (`PLT.RUL.06`): `digest::period_for` (07:00 to 07:00 Asia/Dubai), `register_info` (a record's number and whether it is confidential), `register_rule_title`, `Item`, `Section`, `render`, `gather` / `by_manager` (the managers of a scope hold `late_entries.approve` there), `run_digest` (one digest per manager per date, a section per scope; the scheduled task `digest:voids-late` at 07:00 Asia/Dubai), `scopes::register_label` | §5.13 step 6 | `digest` |
| Compliance-critical rules: `CriticalRule` (a module or vertical marks a rule; `daily_at` or `occurrences` make its missed runs watched), `Planned`, `occurrence_key`, `schedule_subject`, `is_critical`; a failed, stopped or missed run raises one `ComplianceAlert` per occurrence (`raise`, `check_missed` = the scheduled task `compliance:check` every 15 minutes, `run_succeeded`, `close_by_hand` with a required note), tells the owner role and the company admin (e-mail through the notice contract and an in-app notice, `AlertNotice`) and opens a task for the owner role; the page `/compliance-alerts`; a run is missed when its planned time is 60 minutes or more in the past | §3, §5.15, `PLT.RUL.04` | `compliance` |
| Integration health: `IntegrationDef` (a module registers a connection with its screens, manual fallback EN/AR, owner role, N and the handler that sends one call), `Outbound`, `Delivery`, `retry_wait` (1, 2, 4, 8 minutes, then 15, no attempt limit), `IntegrationStatusChanged`, `IntegrationDef::inbound` with `Inbound` and `Match` (a late callback is matched to the record done by hand by its natural key), `IntegrationConnection` and its machine (`up`, `degraded`, `down`, `disabled`), the outbound `QueuedCall`s (first in, first out), `integrations::submit` (a natural key prevents a second call) / `report_success` (recovers, then drains the queue in order) / `report_failure` / `drain` / `receive` (late inbound data matched to the record done by hand; mismatches and obsolete calls go on one reconciliation task for the owner role) / `run_due` (the scheduled task `integrations:run`, `PLT.RUL.05`), the banner partial `integration_banner.html` and the pages `/integrations`, permissions `integrations.view` and `integrations.manage` | §4.5, §5.14, `PLT.RUL.05` | `integrations` |
| Late entries: `late_entries::assess` (flag after M minutes, approval after N hours or in a closed period, refused in a locked one), `LateEntryDef` (a module's opt-in with its `create` handler, business-time and parent-subject fields and period check), `NewRecord`, `late_entries::enter` with `EntrySpec` and `Entered` (on time, recorded, or pending approval: the record is created only on approval), `LateEntry` and its machine, `set_limits` / `limits_for` per entity, `late_entries::web` (the `late_entry_fields.html` partial for the entry form and the `late_entry_card.html` card that lists the runs "already fired, review"), `FiredRun`, `LateEntryRecorded`, permission `late_entries.approve` | §5.13, `PLT.ACL.10` | `late_entries` |
| The business calendar: `BusinessCalendar` (Asia/Dubai, Sat–Sun weekend, 09:00–18:00, `[calendar]` holidays), `add_working` and `add_working_days` (same clock time, no working hours) | §6 | `calendar` |
| Scope of a subject: `scopes::scope_of` (its machine, then a resolver from `scopes::register`, then global) and `COMPANY_ADMIN_ROLE` (`"admin"`, the fallback holder of a task) | §4.1 | `scopes` |
| One URL, a page for a browser and JSON for the rest: `negotiate::wants_html` reads the `Accept` header (`text/html`, or an `hx-boost` navigation) so a back-office route that began as a JSON endpoint can add its page without changing its API answer | ARCHITECTURE §5 | `negotiate` |
| The timesheet contract: `Timesheets` (`NoTimesheets` when `attendance` is off): `hours_for_payroll(period)` gives payroll the hours of the timesheets whose last day is in a `PayrollPeriod` (from, to, cut-off) as `PayrollHours` of `HoursLine`s per day and category, approved ones as they are and the accepted attendance hours of unapproved ones marked `provisional`; `record_payroll_use(run, used)` tells it which hours a submitted run paid (approved timesheets lock with `paid_by`, the others get `paid_provisionally_by`); `TimesheetReconciliationNeeded` (timesheet, payroll run, provisional, approved and delta per category, the adjustment); `ApproverResolver` (`NoApprovers` when the vertical provides none: `account_manager`, `escalation_contacts`, `client_approvers`, `deemed_approval_days` of a context's contract); and `Invoicer::dispute_state` with `DisputeState` and `DISPUTE_SUBJECT_TYPE` (whether a dispute is still open, for an adjustment timesheet of a client dispute) | wl-attendance §5.7, §6, §8, wl-invoicing §4 | `timesheets`, `invoicing` |
| The scheduling event `AvailabilityBlockAdded` (a new availability block on a resource: its source, reason, period and the employee behind it, announced once whether or not a booking is in it) next to `BookingConflict`, for a vertical with work of its own on the resource | wl-scheduling §6 | `scheduling` |
| The scheduling `BookingActions` (`link_work_done`, which links the record a vertical opened at check-in so that `WorkDoneStatus` is asked about it, and `complete`, which completes the booking as the system when that record is done; `NoBookingActions` does nothing), `provide_booking_actions`, `booking_actions` (wl-scheduling provides it) | wl-scheduling §8 *Bookings*; 08 §7.1 *Booking* | `scheduling` |
| The stock return `StockService::return_to_stock` with `ReturnRequest` (item, location, quantity, the source and cost object, the lot of a lot-tracked item, a reason) and `Returned`: goods back into stock as a `return_in` movement, for the parts a job card issued and did not fit (`NoStockService` posts nothing; wl-inventory provides it) | wl-inventory §6 *StockService*; 08 `AUT.ACL.21` | `inventory` |

After `rnx migrate`, `rnx wl:install` creates the legal entities `client.toml` lists and
the database lacks; it never overwrites an entity changed in the admin. `rnx db:seed`
runs it too.

The page `/legal-entities` needs `legal_entities.view`. The app overrides it with
`resources/views/wl-kernel/legal_entities.html`, and translates the texts (`wl_kernel.*`)
in `resources/lang/{locale}.json`.

Other modules read parties only through `wl_kernel::parties::directory(&state)` and listen
to `PartyCreated`, `PartyBlocked` and `PartyUnblocked`. Summaries and events carry the
display block reason: `on_hold` (`NEUTRAL_BLOCK_REASON`) when the real one is restricted.

Other modules draft, read and pay invoices only through
`wl_kernel::invoicing::invoicer(&state)`: `draft(db, InvoiceDraft::new(party, source).line(..))`
gives a `draft` invoice, `find(db, id)` an `InvoiceSummary` (number, status, party, legal
entity, currency, gross total, balance, due date), and `allocate(state, id, source, amount)`
settles part of it from a payment or credit note (on a `written_off` invoice it recovers the bad debt; `InvoiceStatus::accepts_allocation` and `allocation_proposal(&summary, available)` tell whether and how much). Money is `i64` fils, quantities are
thousandths, and each line has a `VatCategory` (`standard` 5%; `zero_rated`, `exempt`,
`out_of_scope` and `reverse_charge` carry no VAT). When `invoicing` is off, `NoInvoicer`
drafts and finds nothing, so callers hide the feature, and refuses an allocation (422).
`InvoiceSummary` also carries `disputed` and `chasing_paused_until` (`chasing_paused(today)`).
`dispute(state, DisputeRequest::new(invoice, reason, DisputeBy::User { id }).amount(..).lines([..]))`
flags an invoice as disputed (a flag, not a state) and gives the dispute id; the default refuses
(422). `.ticket(id)` reuses an existing ticket (none is opened) and `.source(subject)` names the record that caused
the dispute. `wl-invoicing` emits `InvoiceDisputed` and `InvoiceDisputeResolved` (`DisputeOutcome`
`upheld` or `credited`, `DisputedLine`).
Deliveries: `wl_kernel::delivery` defines `DeliveryId` (`subject()` is `delivery.delivery:<id>`), `ReviewOutcome`
(`upheld`) and the events `DeliveryDisputed`, `DeliveryDisputeClosed`, `DeliveryReversed` and `DeliveryLost`. Each carries
the delivery, its `source`, and `lines` (the source line references, matched to an invoice line's `origin`); `wl-delivery`
emits them and `wl-invoicing` listens.
Credit notes: `CreditNoteId` (`subject()` is `invoicing.credit_note:<id>`,
`CREDIT_NOTE_SUBJECT_TYPE`), `CreditNoteStatus` (`draft`, `issued`, `applied`), `CreditLine`
(net fils per invoice line) and `CreditNoteSummary`. `credit_in_full(state, invoice, reason, by)` and
`credit_partly(state, invoice, lines, reason, by)` give the credit note's id,
`find_credit_note`, `credit_notes_for` and `open_credit(db, party, currency)` read them, and
`take_credit_for_refund(state, credit_note, refund, attempt, amount)` takes credit for a refund. The defaults
(and `NoInvoicer`) refuse (422) or find nothing (`None`, an empty list, 0). `wl-invoicing` emits
`CreditNoteIssued`, `InvoiceCredited` and `CreditNoteReopened`. Credit note numbers come from
`credit_note_series(active_entities)` (`CN-ACME-2026-0001`, or `CN-2026-0001` with one entity): gapless,
per entity, reset yearly.
`unallocate(state, Unallocate::new(invoice, source, amount, key, ReopenReason::PaymentReversed))`
undoes an allocation and reopens the invoice (`Reopened { new_status, balance, skipped }`; a repeated
`key`, e.g. `payments.allocation:<id>`, is `skipped`), and `unallocate_all(state, id, reason)` removes every
active allocation and returns each source with its amount (the caller releases its own side of the money).
Both refuse (422) by default and in `NoInvoicer`. `wl-invoicing` emits `InvoiceReopened`.
Modules listen to `InvoiceIssued`, `InvoiceOverdue`, `InvoicePaid`, `InvoiceWrittenOff` , `InvoiceReopened` and `BadDebtRecovered`.
Tax invoice numbers come from `INVOICE_SERIES` (`INV-ACME-2026-0001`) or, with exactly one
active legal entity, `INVOICE_SERIES_SINGLE` (`INV-2026-0001`): gapless, per entity, reset
yearly. `PartyDirectory::billing_identity` gives the legal names, TRN, billing address and
billing e-mail an invoice copies at issue; the default (and `NoParties`) knows none.

Other modules record, allocate and reverse payments and read a party's credit only through
`wl_kernel::payments::payment_recorder(&state)`: `record(&state, RecordPayment::new(party, entity, method, amount), by)`
gives a `PaymentSummary` (number, status, allocated and unallocated fils), `allocate(&state, payment, AllocationTarget::Invoice(id), amount)`
goes through `Invoicer::allocate`, `reverse(&state, payment, reason, note, by)` (`by: None` = the system)
marks every allocation removed and `credit_balance(db, party, currency)` is the party's unallocated credit.
Money is `i64` fils. When `payments` is off, `NoPayments` finds nothing, has no credit and refuses
record, allocate, reverse and `request_refund` (422). `request_refund(&state, RefundRequest, by)` asks a refund
of a source the caller owns (credit note, order, booking) and gives its `RefundId`; the refund still
waits for another user's approval. Modules listen to `PaymentReceived`, `PaymentReversed` (the removed
allocations with their ids, so the invoice module reopens each target; the bank statement line and the
prepaid `Subject` ride along), `RefundPaid` and `RefundFailed` (both name the `RefundSource`, so the
owner of a credit note, order or booking restores its side). Receipts come from `RECEIPT_SERIES` (`RCT-ACME-2026-0001`)
and payment vouchers from `PAYMENT_VOUCHER_SERIES` (`PV-ACME-2026-0001`), or the `_SINGLE` forms with
exactly one active legal entity: gapless, per entity, reset yearly. The record a payment is made in advance
for (a booking, an enrolment) is asked about through `prepaid_subjects(&state)`: the app has one provider
(`provide_prepaid_subjects`), and a module that owns another type of prepaid record next to it registers a
check for that type with `register_prepaid_subject_type(subject_type, check)`.

Every notice goes through one contract. A module registers its notification types in
`register` with `wl_kernel::notify::register_type(NotificationTypeDef::new(..))` (statutory
flag, give-up reactions, staff role told, channels in order, default texts with `{{ name }}`
placeholders), then sends with
`notify::notifier(&state).notify(&state, NotifyRequest::new(type, subject).to_party(p).to_user(u).data(..))`.
A recipient is a staff user (by user id), a party (by `PartyId`, reached through
`PartyDirectory::contacts`) a rule (`.to_contact(of, ContactPurpose::contact_of("finance"))`, resolved through `PartyDirectory::related_contacts`) or a bare address. `.subject_label("TKT-2026-0042")` names the
subject in staff-facing text; `MessageGaveUp` carries it with `gave_up_at`. `wl-notify` provides the real notifier and
emits `MessageFailed`, `MessageSuppressed`, `MessageGaveUp` and `ContactPointInvalidated`. A party contact point carries `valid`; `PartyContacts::point_for` never chooses an invalid one, and `PartyDirectory::invalidate_contact` (default `None`) marks one invalid. `check_give_up` refuses `mark_contact_invalid` alone on a non-marketing type, and `register_type` panics on it. `default_give_up(&def)` is the pure default table (`GiveUpDefault { reactions, staff_role }`, group keys `GROUP_INVOICES` and the like): statutory, then `otp`, then the group row, then urgency adds `staff_task`; `register_type` applies it when `give_up` is empty (role only when `None`) and keeps a module's own list, even `[none]`. When `notify` is off,
`NoNotifier` still gives each user recipient a Renox database notification (the bell) with
the type's text, skips party and address recipients, and returns no message ids.
`Notifier::notice_status(state, subject, type_key)` answers `NoticeStatus` (`Delivered { at }`, `ProofRecorded { delivered_on }`, `NotDelivered`) and `Notifier::undelivered_notices(state, subject)` lists the given-up statutory notices without proof; both have default bodies (`NotDelivered`, empty), so `NoNotifier` never reads a notice as delivered. `NotificationTypeDef::proof_methods(&[..])` sets the `ProofMethod`s staff may use (all four by default); `StatutoryNoticeProofRecorded` is emitted when a proof is recorded.

Other modules open tickets (tasks) only through `wl_kernel::tickets::task_creator(&state)`:
`open(&state, OpenTicket::new(subject, "general", title).description(..).requester_party(p))`
gives a `TicketRef` (id and number, e.g. `TKT-2026-0001`), and fails for an unknown category
key. The channel is `system` unless set, and `.assign_to(Assignee::user(id))` (or
`Assignee::party(vendor)`) has `wl-ticketing` run the first `assign` as the system, so the
SLA starts. `.due_at(..)` sets a due time and `.source_key(..)` a key to find the open ticket
by: `find_open(&state, key)` gives its `TicketRef` while it is not resolved, closed or
cancelled, `resolve_from_source(&state, ticket, note)` resolves a ticket for the record that
opened it (`false` when it did nothing), and `comment(&state, ticket, TaskComment::internal(text))`
adds a comment (all default to nothing or `false`, as `NoTickets` does). Internal staff tasks use `TASK_CATEGORY` (`task`);
`ticket_path(id)` is the ticket page, `/tickets/{id}`. `OpenTicket` is built only with its
builder, so new optional fields never break a caller. When `ticketing` is off, `NoTickets` opens nothing and returns `None`, so callers
carry on without a task. `wl-ticketing` emits `TicketAssigned` (assignee, previous assignee
and reason on a reassignment, priority, due times) and `TicketClosed` (resolution working
minutes, within SLA). Ticket numbers come from `TICKET_SERIES` (`TKT-2026-0001`): reset
yearly, gaps allowed.

Before a module starts, renews or extends an agreement with a party, or releases money to
it, it asks `wl_kernel::block_guard::guard(&state).check_in(&mut tx, Some(&user), party, kind)`
in its own transaction, with `ActionKind::Start`, `Renew`, `Extend` or `MoneyOut` (guard the
step that releases money, not the drafting; money already sent or scheduled is not the
guard's concern). Receipts call it with `MoneyIn`, which is never refused.
`GuardOutcome::Blocked(refusal)` refuses: everyone without `parties.block_reason.view` (and the
system, `viewer = None`) sees "This party is on hold" with code `on_hold`; holders see
"Blocked: <reason>" with a link to each Block record. `refusal.into_error()` is a 422 with the
neutral text. A blocked action can go ahead through an approved exception (approval action
`BLOCK_EXCEPTION_ACTION`, `parties.block_exception`), whose approver `exception_step()` names.
`wl-parties` provides the guard with `provide_block_guard`; when `parties` is off,
`NoBlockGuard` allows everything. The refusal type is `wl_kernel::block_guard::BlockRefusal`,
not re-exported at the root because `documents::BlockRefusal` owns that name there.

Before a guarded transition onboards a party while a sanctions list may be stale, a module
calls `wl_kernel::screening_guard::run_screened(&state, &user, &record, "activate", party,
input)`: it asks `screening_guard(&state)` (`ScreeningGuard::check_in`, in the transition's own
transaction) and then runs the transition. `ScreeningOutcome::Allowed` and `Warn(lists)` let it
run (`Warn` is the default `StaleListPolicy`, and the caller shows the lists in its flash
message); `Blocked(refusal)` runs nothing and is a 422 with code `stale_list`, text "Screening
list is out of date: UN Consolidated List of 2026-11-02" (each stale list once, its last
successful update date in Asia/Dubai, or "no update yet"). The text never names a candidate,
a match or a block reason, so everyone who can run the transition sees it. A module without a
transition (a plain insert) calls `check_in` directly. The page includes the partial
`wl-kernel/screening_notice.html` with a `ScreeningNoticeView` (`ScreeningRefusal::view`, or
`stale_notice_view` from `ScreeningGuard::latest_notice_in`). `wl-parties` provides the guard
with `provide_screening_guard` when screening is on; otherwise `NoScreeningGuard` allows
everything and shows no notice.

A module that owns a guarded transition (an entry of a document kind's `blocks`, e.g.
`deployment.activate`) registers, in `register`, a `BlockSubjects` provider with
`wl_kernel::documents::register_block_subjects("deployment.activate", provider)`: it maps a
record to the document subjects and returns none when the record does not match the entry's
condition. Before `record.transition(..)` it calls
`documents::guard(&state).check(&state, "deployment.activate", id, documents::today())`.
`Ok(Ok(CheckOutcome))` lets the transition run; `Ok(Err(BlockRefusal))` refuses it, and
`refusal.into_error()` is a 422 with one line per failing document ("Residence visa expired
on 2026-11-30; renew: PRO; override: not allowed"); `Err(..)` is a failure (e.g. no provider
for an entry), never a *yes*. `is_valid(db, subject, kind, on_date)` is the guard
`document_valid`, and `blocks_for(db, guarded)` lists the entries with their mode, override
mode and condition. `today()` is the Asia/Dubai date of `renox::db::now()`, so tests that
travel move it; a document is valid through its expiry date. `wl-documents` provides the guard
and emits `DocumentUploaded`, `DocumentAccepted`, `DocumentRejected` and
`DocumentSuperseded`; when `documents` is off, `NoDocuments` guards nothing and answers valid.

An entry in `flag` mode does not refuse: `check` checks the `block` entries first (a failing
one refuses and wins), then returns `CheckOutcome { flags }` with one `FlagFailure` per
failing flag-mode document. A module calls
`documents::guarded_transition(&state, "attendance_record.accept", id, on_date, || async { .. })`
instead of `check`: it runs `check`, the closure (the `record.transition(..)` call) and then
`record_flags`, so a refusal runs nothing and a failing closure writes no flag. `on_date` is
the record's business date. `flags_for(db, record_type, id)` lists a record's `RecordFlag`s
(label, reason, dates, review state, reviewer and note), `open_flag_counts(db, record_type,
&ids)` counts the unreviewed ones for list badges, and `review_flag(db, id, reviewer, note)`
marks one reviewed (a note is required, once only; a flag is never removed). Without
`wl-documents` the closure just runs, nothing is flagged and the lists are empty.

A refused `with_approval` kind that is not a legal block can be overridden once with a second
user's approval. `BlockFailure::override_allowed()` says so, `BlockRefusal::override_links()`
gives the `/documents/overrides/new?kind=..&guarded=..&record=..` links to offer (none for a
`never` kind), and `BlockSubjects::scope(state, entry, id)` (default `None`, global) names the
record's scope for the approvers. `check_as(.., actor)` is `check` for a user: a failing `block`
entry that an approved, unused, valid override requested by `actor` covers is listed in
`CheckOutcome.overrides` (`CoveredBlock`, with its `OverrideId`) instead of refusing; `check` is
`check_as(.., None)` and covers nothing. A module runs `documents::guarded_transition_as(&state,
guarded, id, on_date, Some(user), || async { .. })`: it claims the covered overrides
(`claim_overrides`), runs the closure, then gives them back (`unclaim_overrides`) if it failed or
emits `DocumentBlockOverridden` (`overrides_used`) if it succeeded. A lost claim is refused with
the block message and the closure does not run. Without `wl-documents` all of this is a no-op.

Planned work is warned about, never refused: after a module saved a run, a deployment or a
batch of sessions it calls
`documents::guard(&state).check_planned(&state, "deployment.activate", &items)` with one
`PlannedItem { record_id, on_date }` per item. An item that spans several days passes only its
start date; the guard does not know about end dates. Only `block` entries of `guarded` count.
`Ok(Ok(Some(PlannedWarning)))` has one `PlannedWarningLine` per subject and kind with the sorted
dates it fails on; `message()` is "Ravi S.: Residence visa expires 2026-12-15; planned:
2026-12-20" per line and `toast()` is a persistent warning toast the module returns with its
redirect. A `BlockSubjects` provider can override `label` (the subject's name, else
`<type> #<id>`) and `bookable` (default false): for a bookable slot (`booking.book`,
`booking.reschedule`, `booking.reassign`) `check_planned` gives `Ok(Err(PlannedError::BookableSlot))`,
because a booking is refused through `is_valid` (`SCH.ACL.04`). Without `wl-documents` it returns
`Ok(Ok(None))`.

Look-ahead: a module that owns planned work behind a guarded transition can also register a
`FutureWork` provider with `documents::register_future_work("ticket.resolve", provider)`
(optional: an entry without one is skipped). `planned_from(state, entry, subject, from)` returns
the `PlannedRecord`s (`record`, `date`, `label`, optional `scope`) planned on or after `from`.
`wl-documents` passes `from` = **expiry date + 1 day**: a document is valid through its expiry
date, so a record planned on that date is never listed. It groups the records by guarded
transition as `AffectedGroup`s in the `DocumentExpiring` event (document, subject, kind,
`expires_on`, `days_left`, `offset`, `affected`); the event changes no planned item. A vertical
registers a `ReminderSchedule` per document kind with
`documents::register_reminder_schedule(kind, schedule)`: `scopes(state, subject)` names the
scopes whose staff get the reminder when neither the subject nor the affected records have one;
`None` means every scope. Without `wl-documents` neither registry is read.

`FutureWork` also has `open_for(state, entry, subject)`, default empty: the records already
running (an active deployment). When a blocking document has expired, `wl-documents` emits
`DocumentExpired` once (document, subject, kind, `expires_on`, every `blocks` entry, and
`blocked`: per guarded transition a `BlockedGroup` with `mode`, `open` and `planned` records).
`TaskCreator::complete(state, ticket, note)` closes a task when its work is done; the default
returns `false`. `[roles] owner = "<role key>"` in `client.toml` names the business owner role,
read with `wl_kernel::owner_role(&config)`; it defaults to `COMPANY_ADMIN_ROLE`.

Working time comes from one calendar: `wl_kernel::calendar::calendar(&state)` gives the
client's `BusinessCalendar` (the default one with no holidays when the kernel is not
mounted). Asia/Dubai is UTC+04:00 all year; the weekend is Saturday and Sunday, a working
day runs 09:00–18:00, and the public holidays are listed in `client.toml`:

```toml
[calendar]
holidays = ["2026-12-02", "2026-12-03"]
```

`add_working(from, minutes)` gives the instant that many working minutes later: a start
outside working hours counts from the next opening, and a result exactly on the closing time
stays there (Tue 09:00 + 27 h = Thu 18:00). `working_minutes_between(a, b)` is its inverse.
`is_working_day(date)` and `next_working_day(date)` step whole days, `local(at)` shows an
instant on the Dubai clock, `from_local(naive)` reads one, and `today()` is the same date as
`documents::today()`.

Migrations are SQLite only: each client runs one SQLite database
([ARCHITECTURE.md](../../docs/ARCHITECTURE.md) §1).
