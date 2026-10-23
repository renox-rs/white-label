#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod approvals;
pub mod attendance;
pub mod automation;
pub mod bank_statements;
pub mod block_effects;
pub mod block_guard;
pub mod branding;
pub mod broadcast;
pub mod budgets;
pub mod calendar;
pub mod cash_forecast;
pub mod chase_holds;
pub mod cheques;
pub mod compliance;
pub mod config;
pub mod consents;
pub mod custody;
pub mod deadlines;
pub mod delivery;
pub mod deposits;
pub mod digest;
pub mod documents;
pub mod drawings;
pub mod einvoice;
pub mod entities;
pub mod held_funds;
pub mod integrations;
pub mod inventory;
pub mod invoicing;
pub mod late_entries;
pub mod machines;
pub mod manifest;
pub mod negotiate;
pub mod notify;
pub mod numbering;
pub mod parties;
pub mod payer_bills;
pub mod payments;
pub mod payroll_calendar;
pub mod placements;
pub mod portal;
pub mod qualifications;
pub mod quotations;
pub mod rate_cards;
pub mod role_tasks;
pub mod sales;
pub mod scheduling;
pub mod scopes;
pub mod screening_guard;
pub mod segments;
pub mod subject;
pub mod subscriptions;
mod texts;
pub mod tickets;
pub mod timesheets;
pub mod usage;
pub mod visa_steps;
pub mod voids;
pub mod wip;
pub mod workforce;

use std::sync::Arc;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;
use serde::Serialize;

pub use approvals::{
    ApprovalDecided, ApprovalDecision, ApprovalEnd, ApprovalInterceptor, ApprovalPolicy,
    ApprovalRequest, ApprovalRequested, ApprovalStatus, PolicyDefault, Require, Required,
};
pub use automation::{AutomationFailed, RuleRun, RuleRunSpec, RuleRunStatus, RunOutcome, run_rule};
pub use bank_statements::{BankBalance, BankStatements, NoBankStatements};
pub use block_effects::{
    AgreementKind, AgreementRef, AgreementSource, BlockEffect, BlockEffectOverride,
    BlockEffectRule, Decision, RuleSource,
};
pub use block_guard::{
    ActionKind, BLOCK_EXCEPTION_ACTION, BlockGuard, BlockReasonLine, ExceptionStep, GuardOutcome,
    NoBlockGuard,
};
pub use broadcast::{
    BROADCAST_SUBJECT_TYPE, BroadcastCancelReason, BroadcastCancelled, BroadcastCompleted,
    BroadcastId, BroadcastReport, BroadcastRequest, BroadcastRequesters, BroadcastStopped,
    NoRequesters, RequesterCheck,
};
pub use budgets::{
    BudgetFuture, COMPANY_BUDGET_SUBJECT_TYPE, CompanyBudgetApproved, CompanyBudgetId,
    CompanyBudgetVarianceReached, CompanyBudgets, NoCompanyBudgets,
};
pub use calendar::BusinessCalendar;
pub use cash_forecast::{
    CashForecastBelowThreshold, CashForecastSource, ForecastFuture, ForecastLine,
    cash_forecast_sources, register_cash_forecast_source,
};
pub use cheques::{
    BounceFeeCharge, BounceFees, CHARGE_SUBJECT_TYPE, CHEQUE_SUBJECT_TYPE, ChargeInvoicer,
    ChequeBounceResolved, ChequeBounced, ChequeDocument, ChequeId, ChequeOutcome,
    ChequeReferredToLegal, ChequeSchedule, ChequeStatus, ChequeSummary, Cheques, NoBounceFees,
    NoChargeInvoicer, NoCheques, OpenCheque, RegisterCheque, ReturnCheques, ReturnedCheques,
    ScheduledCheque,
};
pub use compliance::{ComplianceAlert, ComplianceRule, CriticalRule, Planned};
pub use config::{ClientConfig, ConfigError, RolesConfig, owner_role};
pub use consents::{Consents, NoConsents};
pub use custody::{
    CASH_HANDOVER_SUBJECT_TYPE, CashCollection, CashCustody, CashHandoverCounted, CashShortage,
    CashShortageSettled, HandoverId, HandoverKind, HandoverPeriod, HandoverStatus, HandoverSummary,
    NoCustody, NoShortageRecovery, PeriodKind, ShortageDeduction, ShortageRecovery, custody,
    provide_custody, provide_shortage_recovery, shortage_recovery,
};
pub use deadlines::{
    DEADLINE_TASK_SUBJECT_TYPE, DeadlineTaskId, DeadlineTaskStatus, DeadlineTaskSummary,
    DeadlineTasks, EndedTasks, HANDED_OVER_EVIDENCE, HandOverTask, NewDeadlineTask,
    NoDeadlineTasks, ReleasedTasks, TaskCompleted, TaskOverdue, TaskRecurrence, WaiveTask,
};
pub use delivery::{
    CodAbsorber, CodAbsorption, DELIVERY_SUBJECT_TYPE, DeliveredLine, DeliveryAssigned,
    DeliveryAttemptFailed, DeliveryDelivered, DeliveryDisputeClosed, DeliveryDisputed,
    DeliveryFound, DeliveryFuture, DeliveryId, DeliveryItem, DeliveryLost, DeliveryOutForDelivery,
    DeliveryRefusedReceived, DeliveryRequest, DeliveryRequester, DeliveryReversed, NoDeliveries,
    PodMethod, RUN_SUBJECT_TYPE, RefusedLineReceived, ReviewOutcome, RunAborted, RunClosed,
    RunDispatched, RunHandedOver, RunId, provide_requester,
};
pub use deposits::{
    AdjustDeposit, CancelSettlement, DeductionKind, DeductionLineView, DepositAdjusted,
    DepositDeductions, DepositDeductionsDisputed, DepositDeductionsPublished,
    DepositDisputeAnswered, DepositOutcome, DepositSettled, DepositTopUps, Deposits, DisputeAnswer,
    DisputeDeductions, HeldDeposit, NoDepositDeductions, NoDepositTopUps, NoDeposits,
    PublishedDeductions, SettlementStarted, TopUpCharge,
};
pub use documents::{
    BlockCondition, BlockEntry, BlockFailure, BlockMode, BlockRefusal, BlockSubjects, BlockedGroup,
    CheckOutcome, DocumentAccepted, DocumentExpired, DocumentGuard, DocumentId, DocumentRejected,
    DocumentSuperseded, DocumentUploaded, FlagFailure, FlagReason, InvalidReason, NoDocuments,
    NoSopAcknowledgements, OverrideMode, PendingRenewal, PlannedError, PlannedItem, PlannedWarning,
    PlannedWarningLine, RecordFlag, SopAcknowledged, SopAcknowledgements, SopId, SopPublished,
    SopReaderRef, SopReadersChanged, SopStaff, SopWithdrawn, Validity,
    provide_sop_acknowledgements, sop_acknowledgements,
};
pub use drawings::{
    DRAWING_REQUEST_SUBJECT_TYPE, DrawingCategory, DrawingPayment, DrawingPaymentSource,
    DrawingRequest, DrawingStart, NoOwnerDrawings, OwnerDrawingDecided, OwnerDrawings,
    drawing_payment_sources, owner_drawings, provide_owner_drawings, register_drawing_payments,
};
pub use entities::{IssuerSnapshot, LegalEntity, ScopeEntity};
pub use held_funds::{FundsSource, HeldFund, HeldFundsSource, held_for_all};
pub use integrations::{
    ConnectionStatus, Delivery, Inbound, IntegrationConnection, IntegrationDef,
    IntegrationStatusChanged, Match, Outbound, QueuedCall,
};
pub use invoicing::{
    BillBalance, BillReopenReason, CostObjectAmount, CostPoster, CostReversal, CreditLine,
    CreditNoteId, CreditNoteIssued, CreditNoteReopened, CreditNoteStatus, CreditNoteSummary,
    DebitedLine, DraftLine, InvoiceCredited, InvoiceDraft, InvoiceId, InvoiceIssued,
    InvoiceOverdue, InvoicePaid, InvoiceStatus, InvoiceSummary, InvoiceWrittenOff, Invoicer,
    LandedCostLine, LandedCosts, NoCostPoster, NoInvoicer, NoLandedCosts, NoSupplierBills,
    NoSupplierDebitNotes, PayBill, PostCost, ReverseCost, SupplierBillApproved, SupplierBillId,
    SupplierBillLine, SupplierBillLineId, SupplierBillReopened, SupplierBillStatus,
    SupplierBillVoided, SupplierBills, SupplierDebitNoteCancelled, SupplierDebitNoteId,
    SupplierDebitNoteIssued, SupplierDebitNoteSettled, SupplierDebitNoteStatus,
    SupplierDebitNoteSummary, SupplierDebitNotes, UnallocateBill, VatCategory, WriteOffAnswer,
};
pub use late_entries::{
    ApprovalWhy, Assessment, FiredRun, LateEntry, LateEntryDef, LateEntryLimits, LateEntryRecorded,
    LateEntryStatus, Limits, NewRecord, Period,
};
pub use machines::SubjectMachine;
pub use manifest::Manifest;
pub use notify::{
    GiveUpReaction, MessageFailed, MessageGaveUp, MessageId, MessageSuppressed, NoNotifier,
    NotificationTypeDef, Notifier, NotifyChannel, NotifyRecipient, NotifyRequest, OtpRequest,
    OtpSent, OtpVerdict, PartyContacts,
};
pub use numbering::{Reset, Series};
pub use parties::{
    BillingIdentity, NEUTRAL_BLOCK_REASON, NoParties, PartiesMerged, PartyBlocked, PartyCreated,
    PartyDirectory, PartyId, PartyKind, PartyRestored, PartyStatus, PartySummary, PartyUnblocked,
    PartyUpdated, ScreeningCompleted, ScreeningFlagged, ScreeningId, ScreeningMatchConfirmed,
    ScreeningStatus,
};
pub use payer_bills::{
    NewPayerBill, NewPayerBillLine, NoPayerBills, NoPayerCredits, PayerAnswer, PayerBillClosed,
    PayerBillFuture, PayerBillId, PayerBillIssued, PayerBillOutcome, PayerBillStatus,
    PayerBillSubmissionRejected, PayerBillSummary, PayerBills, PayerCredits,
    PayerRecoupmentRecorded, PayerRemittanceRecorded, RecoupedLine, Recoupment, RejectionDeadline,
    Remittance, RemittanceLine, SubmissionError,
};
pub use payments::{
    AllocationTarget, BankPayment, ChargebackClosed, ChargebackOutcome, DEPOSIT_SUBJECT_TYPE,
    DepositId, Direction, NoPaymentLinks, NoPayments, NoPrepaidSubjects, PAYMENT_LINK_SUBJECT_TYPE,
    PaymentFailed, PaymentFailureKind, PaymentId, PaymentLinkExpired, PaymentLinkRef,
    PaymentLinkRequest, PaymentLinks, PaymentMethod, PaymentReceived, PaymentRecorder,
    PaymentReversed, PaymentSource, PaymentStatus, PaymentSummary, PrepaidCheck, PrepaidSubjects,
    RecordPayment, RefundApproved, RefundAttemptRef, RefundFailed, RefundFailedFrom,
    RefundFailureKind, RefundId, RefundPaid, RefundRequest, RefundSource, ReversalReason,
    UnpaidRefund, register_prepaid_subject_type,
};
pub use qualifications::{NoQualifications, QualificationStatus, Qualifications};
pub use rate_cards::{NoRateCards, RateCardVersion, RateCards};
pub use sales::{
    Attribute, AttributionTarget, CampaignActivated, CampaignAttribution, CampaignEnded,
    CampaignId, CustomerAcquired, CustomerAcquisitionChanged, FirstSale, LEAD_SUBJECT_TYPE,
    LeadConversion, LeadCreated, LeadId, LeadLost, LeadWinReverted, LeadWon, NoCampaignAttribution,
    NoLeadConversion, campaign_attribution, lead_conversion, provide_campaign_attribution,
    provide_lead_conversion,
};
pub use sales::{
    ConsumePackage, NoPackageLedger, PACKAGE_SUBJECT_TYPE, PackageActivated, PackageConsumed,
    PackageConsumption, PackageExpired, PackageExtended, PackageId, PackageLedger, package_ledger,
    provide_package_ledger,
};
pub use sales::{NoPackageSales, PackageSales, SellPackage, package_sales, provide_package_sales};
pub use sales::{NoSelfPayPrices, SelfPayPrices, provide_self_pay_prices, self_pay_prices};
pub use scopes::COMPANY_ADMIN_ROLE;
pub use segments::{NoSegments, SegmentId, Segments};
pub use subject::Subject;
pub use subscriptions::{
    BillingCycle, CancelSubscription, NoSubscriptions, PauseCause, PauseSubscription, PaymentMode,
    SUBSCRIPTION_SUBJECT_TYPE, StartSubscription, SubscriptionCancelled, SubscriptionId,
    SubscriptionPastDue, SubscriptionPaused, SubscriptionResumed, SubscriptionSettled,
    SubscriptionStatus, SubscriptionSummary, SubscriptionSuspended, Subscriptions,
};
pub use tickets::{
    Assignee, CONFIDENTIAL_TASK_CATEGORY, ChecklistCompleted, ChecklistOutcome, ChecklistRunId,
    Checklists, ComplianceFlagCleared, ComplianceFlagRaised, ComplianceFlagReason, ContractCover,
    NoChecklists, NoServiceContractCoverage, NoTickets, OpenTicket, Requester,
    SERVICE_CONTRACT_SERIES, SERVICE_CONTRACT_SUBJECT_TYPE, ServiceContractActivated,
    ServiceContractCoverage, ServiceContractEnded, ServiceContractId, TASK_CATEGORY, TaskComment,
    TaskCreator, TicketAssigned, TicketChannel, TicketClosed, TicketId, TicketRef, VendorNoShow,
    checklists, provide_checklists, provide_service_contract_coverage, service_contract_coverage,
    ticket_path,
};
pub use voids::{
    Effect, EffectKind, RecordVoided, Reverse, ReverseError, VoidRecord, Voidable, VoidableBuilder,
};
pub use wip::{NoUnbilledTime, UnbilledTime, WipSummary};

pub use workforce::{
    AbsenceCaseClosed, AbsenceCaseOpened, AffectedWork, EmployeeAbsconded, QualificationExpired,
    QualificationExpiring, QualificationId,
};
/// The kernel's permissions (wl-kernel §6).
pub use workforce::{
    EMPLOYEE_SUBJECT_TYPE, EmployeeActivated, EmployeeAssignmentChanged, EmployeeDeceased,
    EmployeeExited, EmployeeId, EmployeeNoticeGiven, EmployeeNoticeWithdrawn,
    EmployeeOnboardingCancelled, EmployeeReinstated, FailedPayrollLine, LeaveApproved,
    LeaveCancelled, NoWorkforce, ONBOARDING_SUBJECT_TYPE, OnboardingCompleted, OnboardingId,
    OnboardingStarted, PAY_ACCOUNT_SUBJECT_TYPE, PAYROLL_RUN_SUBJECT_TYPE, PayAccountChanged,
    PayrollLinesFailed, PayrollRunId, PayrollRunPaid, PayrollRunReturned, SETTLEMENT_SUBJECT_TYPE,
    SettlementDisputed, SettlementId, SettlementPaid, StaffProfile, VISA_CASE_SUBJECT_TYPE,
    VisaCaseRejected, Workforce, provide_workforce, workforce,
};

/// The permission names the kernel declares.
pub mod permissions {
    /// See legal entities and scope–entity links.
    pub const LEGAL_ENTITIES_VIEW: &str = "legal_entities.view";
    /// Maintain legal entities and scope–entity links.
    pub const LEGAL_ENTITIES_MANAGE: &str = "legal_entities.manage";
    /// Pick a non-default issuing entity on a draft document.
    pub const LEGAL_ENTITIES_CHOOSE: &str = "legal_entities.choose";
    /// See all approval requests (approvers always see their own).
    pub const APPROVALS_VIEW: &str = "approvals.view";
    /// Run a failed rule run again.
    pub const AUTOMATIONS_RETRY: &str = "automations.retry";
    /// Void a record entered in error (a module may declare its own alias).
    pub const RECORDS_VOID: &str = "records.void";
    /// Approve a late entry beyond N hours or in a closed period (`PLT.ACL.10`).
    pub const LATE_ENTRIES_APPROVE: &str = "late_entries.approve";
    /// See the integration connections: their health, last error and queue.
    pub const INTEGRATIONS_VIEW: &str = "integrations.view";
    /// Disable and enable an integration connection.
    pub const INTEGRATIONS_MANAGE: &str = "integrations.manage";
}

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory.
const VIEWS: &[(&str, &str)] = &[
    (
        "wl-kernel/legal_entities.html",
        include_str!("../views/wl-kernel/legal_entities.html"),
    ),
    (
        block_guard::REFUSAL_TEMPLATE,
        include_str!("../views/wl-kernel/block_refusal.html"),
    ),
    (
        screening_guard::NOTICE_TEMPLATE,
        include_str!("../views/wl-kernel/screening_notice.html"),
    ),
    (
        approvals::web::CARD_TEMPLATE,
        include_str!("../views/wl-kernel/approval_card.html"),
    ),
    (
        voids::web::ACTION_TEMPLATE,
        include_str!("../views/wl-kernel/void_action.html"),
    ),
    (
        integrations::web::BANNER_TEMPLATE,
        include_str!("../views/wl-kernel/integration_banner.html"),
    ),
    (
        "wl-kernel/integrations.html",
        include_str!("../views/wl-kernel/integrations.html"),
    ),
    (
        "wl-kernel/integration.html",
        include_str!("../views/wl-kernel/integration.html"),
    ),
    (
        "wl-kernel/compliance_alerts.html",
        include_str!("../views/wl-kernel/compliance_alerts.html"),
    ),
    (
        late_entries::web::FIELDS_TEMPLATE,
        include_str!("../views/wl-kernel/late_entry_fields.html"),
    ),
    (
        late_entries::web::CARD_TEMPLATE,
        include_str!("../views/wl-kernel/late_entry_card.html"),
    ),
];

/// The kernel module. The host adds it with [`apply`], which also gives the
/// app the client's theme and languages.
pub struct WlKernel {
    config: Arc<ClientConfig>,
    manifest: Arc<Manifest>,
    calendar: Arc<BusinessCalendar>,
}

impl WlKernel {
    /// The kernel for this client. Fails when `client.toml` lists modules
    /// whose dependencies are not enabled.
    pub fn new(config: ClientConfig) -> Result<Self, ConfigError> {
        let manifest = config.manifest()?;
        let calendar = Arc::new(config.business_calendar());
        Ok(WlKernel {
            calendar,
            config: Arc::new(config),
            manifest: Arc::new(manifest),
        })
    }
}

/// Gives `app` this client's kernel, theme and languages.
pub fn apply(app: App, config: ClientConfig) -> Result<App, ConfigError> {
    let theme = branding::theme(&config.branding)?;
    let kernel = WlKernel::new(config)?;
    // Arabic pages are right-to-left already: Renox sets `app.dir` to `rtl` for `ar`.
    Ok(app.theme(theme).module(kernel))
}

/// What the legal entities page needs; `view:check` compares it with the
/// template.
#[derive(Serialize, ViewData)]
#[view(template = "wl-kernel/legal_entities.html")]
struct LegalEntitiesPage {
    heading: String,
    entities: Vec<LegalEntity>,
}

impl Module for WlKernel {
    fn name(&self) -> &'static str {
        "wl-kernel"
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .get("/legal-entities", legal_entities)
            .name("wl_kernel.legal_entities")
            .require_permission(permissions::LEGAL_ENTITIES_VIEW)
            // The approval card's buttons; who may decide is checked per request.
            .merge(approvals::web::routes())
            .merge(voids::web::routes())
            .merge(integrations::web::routes())
            .merge(compliance::web::routes())
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.provide(Arc::clone(&self.config))
            .provide(Arc::clone(&self.manifest))
            .provide(Arc::clone(&self.calendar))
            .model::<LegalEntity>()
            .model::<ScopeEntity>()
            .view_data::<LegalEntitiesPage>()
            .view_data::<integrations::web::IntegrationsPage>()
            .view_data::<integrations::web::IntegrationPage>()
            .view_data::<compliance::web::ComplianceAlertsPage>();
        approvals::register(app);
        automation::register(app);
        voids::register_models(app);
        late_entries::register_models(app);
        integrations::register_models(app);
        compliance::register_models(app);
        digest::register(app);
        // In the admin group of the staff sidebar; the label is the app's
        // `nav.legal_entities` text.
        app.nav(
            "app",
            NavLink::route("wl_kernel.legal_entities", "nav.legal_entities")
                .icon("building-2")
                .group("nav.admin")
                .order(30)
                .permission(permissions::LEGAL_ENTITIES_VIEW),
        );
        let config = Arc::clone(&self.config);
        app.command(
            "wl:install",
            "Create the legal entities client.toml lists and the approval policies the modules \
             declare, where the database lacks them",
            move |_args, state: AppState| {
                let config = Arc::clone(&config);
                async move {
                    let created = entities::install(&state.db, &config).await?;
                    println!("{created} legal entities created.");
                    let policies = approvals::install(&state.db).await?;
                    println!("{policies} approval policies created.");
                    let connections = integrations::install(&state.db).await?;
                    println!("{connections} integration connections created.");
                    let rules = compliance::install(&state.db).await?;
                    println!("{rules} compliance-critical rules stored.");
                    Ok(())
                }
            },
        );
        let config = Arc::clone(&self.config);
        app.seeder(move |state: AppState| {
            let config = Arc::clone(&config);
            async move {
                entities::install(&state.db, &config).await?;
                approvals::install(&state.db).await?;
                integrations::install(&state.db).await?;
                compliance::install(&state.db).await.map(|_| ())
            }
        });
        app.templates(|env| {
            for (name, source) in VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
    }
}

async fn legal_entities(State(db): State<Db>, lang: Lang) -> Result<View> {
    let entities = LegalEntity::query()
        .order_by_desc(LegalEntity::IS_DEFAULT)
        .order_by(LegalEntity::CODE)
        .get(&db)
        .await?;
    Ok(View::typed(LegalEntitiesPage {
        heading: texts::t(&lang, "wl_kernel.legal_entities"),
        entities,
    }))
}
