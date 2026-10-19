#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod approvals;
pub mod automation;
pub mod block_effects;
pub mod block_guard;
pub mod branding;
pub mod broadcast;
pub mod calendar;
pub mod config;
pub mod consents;
pub mod delivery;
pub mod documents;
pub mod entities;
pub mod held_funds;
pub mod invoicing;
pub mod machines;
pub mod manifest;
pub mod notify;
pub mod numbering;
pub mod parties;
pub mod payments;
pub mod scopes;
pub mod screening_guard;
pub mod segments;
pub mod subject;
mod texts;
pub mod tickets;
pub mod voids;

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
    BroadcastId, BroadcastRequest, BroadcastRequesters, NoRequesters, RequesterCheck,
};
pub use calendar::BusinessCalendar;
pub use config::{ClientConfig, ConfigError, RolesConfig, owner_role};
pub use consents::{Consents, NoConsents};
pub use delivery::{
    DELIVERY_SUBJECT_TYPE, DeliveryDisputeClosed, DeliveryDisputed, DeliveryId, DeliveryLost,
    DeliveryReversed, ReviewOutcome,
};
pub use documents::{
    BlockCondition, BlockEntry, BlockFailure, BlockMode, BlockRefusal, BlockSubjects, BlockedGroup,
    CheckOutcome, DocumentAccepted, DocumentExpired, DocumentGuard, DocumentId, DocumentRejected,
    DocumentSuperseded, DocumentUploaded, FlagFailure, FlagReason, InvalidReason, NoDocuments,
    OverrideMode, PendingRenewal, PlannedError, PlannedItem, PlannedWarning, PlannedWarningLine,
    RecordFlag, SopId, SopPublished, SopReaderRef, SopReadersChanged, SopWithdrawn, Validity,
};
pub use entities::{IssuerSnapshot, LegalEntity, ScopeEntity};
pub use held_funds::{FundsSource, HeldFund, HeldFundsSource, held_for_all};
pub use invoicing::{
    CreditLine, CreditNoteId, CreditNoteIssued, CreditNoteReopened, CreditNoteStatus,
    CreditNoteSummary, DraftLine, InvoiceCredited, InvoiceDraft, InvoiceId, InvoiceIssued,
    InvoiceOverdue, InvoicePaid, InvoiceStatus, InvoiceSummary, InvoiceWrittenOff, Invoicer,
    NoInvoicer, VatCategory,
};
pub use machines::SubjectMachine;
pub use manifest::Manifest;
pub use notify::{
    GiveUpReaction, MessageFailed, MessageGaveUp, MessageId, MessageSuppressed, NoNotifier,
    NotificationTypeDef, Notifier, NotifyChannel, NotifyRecipient, NotifyRequest, PartyContacts,
};
pub use numbering::{Reset, Series};
pub use parties::{
    BillingIdentity, NEUTRAL_BLOCK_REASON, NoParties, PartyBlocked, PartyCreated, PartyDirectory,
    PartyId, PartyKind, PartyRestored, PartyStatus, PartySummary, PartyUnblocked,
    ScreeningCompleted, ScreeningFlagged, ScreeningId, ScreeningMatchConfirmed, ScreeningStatus,
};
pub use payments::{
    AllocationTarget, DEPOSIT_SUBJECT_TYPE, DepositId, Direction, NoPayments, PaymentId,
    PaymentMethod, PaymentReceived, PaymentRecorder, PaymentReversed, PaymentSource, PaymentStatus,
    PaymentSummary, RecordPayment, RefundAttemptRef, RefundFailed, RefundFailedFrom,
    RefundFailureKind, RefundId, RefundPaid, RefundRequest, RefundSource, ReversalReason,
};
pub use scopes::COMPANY_ADMIN_ROLE;
pub use segments::{NoSegments, SegmentId, Segments};
pub use subject::Subject;
pub use tickets::{
    Assignee, NoTickets, OpenTicket, Requester, TASK_CATEGORY, TaskComment, TaskCreator,
    TicketAssigned, TicketChannel, TicketClosed, TicketId, TicketRef, ticket_path,
};
pub use voids::{
    Effect, EffectKind, RecordVoided, Reverse, ReverseError, VoidRecord, Voidable, VoidableBuilder,
};

/// The kernel's permissions (wl-kernel §6).
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
            .view_data::<LegalEntitiesPage>();
        approvals::register(app);
        automation::register(app);
        voids::register_models(app);
        // In the admin group of the staff sidebar; the label is the app's
        // `nav.legal_entities` text.
        app.nav(
            "app",
            NavLink::route("wl_kernel.legal_entities", "nav.legal_entities")
                .icon("landmark")
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
                    Ok(())
                }
            },
        );
        let config = Arc::clone(&self.config);
        app.seeder(move |state: AppState| {
            let config = Arc::clone(&config);
            async move {
                entities::install(&state.db, &config).await?;
                approvals::install(&state.db).await.map(|_| ())
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
