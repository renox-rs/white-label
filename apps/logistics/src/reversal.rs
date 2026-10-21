//! A delivered shipment whose delivery was reversed (03 Logistics §4.1 row
//! *Recipient denies the delivery, or it was marked by mistake*, §4.4 step 3,
//! §7.1 row `reverse_delivery`, `WRK.ACL.10`, `LOG.ACL.12`; BL-LOG-10).
//!
//! The reversal, its approval by a second operations manager, the liability
//! decision and the cash treatment are wl-delivery's (#161). The shipment only
//! follows the `DeliveryReversed` event ([`crate::events`]): the system runs
//! `reverse_delivery` (back to `attempted`) or `reverse_delivery_on_road`
//! (back to `out_for_delivery`), as the event says the delivery went; no user
//! ever runs them. The original e-POD is never edited: wl-delivery keeps it
//! and marks it reversed.
//!
//! - **In the transition** ([`record`]): the [`Reversal`] row keeps the cause,
//!   the liability, the cash treatment and the amounts; the shipment is
//!   *reversed* (out of billing until delivered again), forgets the cash
//!   recorded at the stop, and expects no COD any more when the cash was
//!   refunded to the payer.
//! - **After the commit** ([`settled`]): the COD comes off a draft or approved
//!   remittance, or, when it was already paid to the client, *refund to payer*
//!   drafts a negative adjustment line (source `delivery_reversal`, awaiting
//!   the approval of `LOG.ACL.12`) and *keep for the next attempt* marks the
//!   COD already remitted so it is never remitted twice; an invoiced shipment
//!   gets a credit due and the invoice owner a task ([`crate::credit`]); the
//!   company-liable cases (*driver*, *business*) open a claim ticket
//!   ([`crate::claim`]); *driver* gives the operations manager a task naming
//!   the amount and the driver, *business* records the loss; the client is
//!   told. *Client* and *none* post nothing: no claim, no task.
//! - **The operations manager's task** ([`raise_shortage`], [`open_hr_case`]):
//!   *raise shortage* asks wl-payments, through the kernel's
//!   [`ShortageRaiser`](wl_kernel::custody::ShortageRaiser), for a later
//!   shortage handover against the driver with the shipment as its source;
//!   *open HR case* opens a task for the HR role. Either closes the task.
//! - **Refunds to a payer who is no party:** the shipment's recipient is a
//!   person, not a party record, so the refund of a COD is paid by hand: this
//!   module tells wl-delivery so ([`ShipmentRefunds`]) and wl-delivery opens a
//!   task instead of refusing the reversal.

use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::custody::LaterShortage;
use wl_kernel::delivery::{DeliveryReversed, ManualRefunds};
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::tickets::{self, TicketId};

use crate::adjustment::{AdjustmentLine, AdjustmentStatus, source};
use crate::claim;
use crate::credit;
use crate::notices::tell_client;
use crate::permissions;
use crate::rates::aed;
use crate::remittance::{Remittance, RemittanceLine, RemittanceStatus};
use crate::shipment::{SUBJECT_TYPE, Shipment, ShipmentStatus, track};
use crate::shipment_machine::{REVERSE_DELIVERY, REVERSE_DELIVERY_ON_ROAD, ShipmentInput};
use crate::texts::english;

/// The task of the operations manager is waiting for a decision.
pub const TASK_OPEN: &str = "open";
/// *Raise shortage* was chosen.
pub const TASK_SHORTAGE: &str = "shortage_raised";
/// *Open HR case* was chosen.
pub const TASK_HR_CASE: &str = "hr_case";
/// Liability *driver* gives the operations manager a task; the others none.
pub const TASK_NONE: &str = "none";

/// The cash treatment *refund to payer* as wl-delivery names it.
pub const REFUNDED: &str = "refunded";

/// One reversal of a shipment's delivery, with what became of it.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_reversals", index(shipment_id), index(task_status))]
pub struct Reversal {
    /// The primary key.
    pub id: i64,
    /// The shipment.
    #[model(references = "logistics_shipments")]
    pub shipment_id: i64,
    /// The delivery that was reversed.
    pub delivery_id: i64,
    /// Why, in words.
    pub reason: String,
    /// `driver`, `business`, `client` or `none`.
    pub liability: String,
    /// `removed`, `refunded` or `kept`; none when no cash was recorded.
    pub cash_treatment: Option<String>,
    /// The COD the shipment expected when it was reversed, in fils.
    pub cod_amount: i64,
    /// The cash recorded as collected, in fils.
    pub cash_amount: i64,
    /// The driver of the run.
    pub driver_user_id: Option<i64>,
    /// The operations manager who approved the reversal.
    pub approver_id: Option<i64>,
    /// The state the shipment went back to.
    pub target_state: String,
    /// Whether the COD was on a paid remittance.
    pub remitted: bool,
    /// The negative adjustment line drafted for a refund of remitted COD.
    pub adjustment_line_id: Option<i64>,
    /// The charge to credit on the invoice, in fils.
    pub credit_due: i64,
    /// What the business bears when it is liable, in fils.
    pub loss_amount: i64,
    /// The claim ticket opened for a company-liable reversal.
    pub claim_ticket_id: Option<i64>,
    /// The operations manager's task: [`TASK_NONE`], [`TASK_OPEN`],
    /// [`TASK_SHORTAGE`] or [`TASK_HR_CASE`].
    pub task_status: String,
    /// The ticket of that task.
    pub task_ticket_id: Option<i64>,
    /// The later shortage handover raised from the task.
    pub shortage_handover_id: Option<i64>,
    /// The HR case ticket opened from the task.
    pub hr_ticket_id: Option<i64>,
    /// The user who decided the task.
    pub decided_by: Option<i64>,
    /// When.
    pub decided_at: Option<DateTime>,
    /// Whether what follows the commit was done.
    pub settled: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// What the system's `reverse_delivery` carries, from `DeliveryReversed`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReversalInput {
    /// The delivery.
    pub delivery: i64,
    /// Why it was reversed.
    pub reason: String,
    /// `driver`, `business`, `client` or `none`.
    pub liability: String,
    /// `removed`, `refunded` or `kept`.
    pub cash_treatment: Option<String>,
    /// The cash recorded as collected, in fils.
    pub cash_amount: i64,
    /// The driver of the run, if it was on one.
    pub driver: Option<i64>,
    /// The approver.
    pub approver: Option<i64>,
    /// `out_for_delivery` or `attempted`.
    pub target_state: String,
}

/// The transition's effect, inside its transaction: the reversal is kept, the
/// shipment forgets the cash recorded at the stop and leaves billing.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn record(
    tx: &mut Transaction,
    shipment: &Shipment,
    input: &ReversalInput,
) -> Result {
    let refunded = input.cash_treatment.as_deref() == Some(REFUNDED);
    let mut reversal = Reversal {
        shipment_id: shipment.id,
        delivery_id: input.delivery,
        reason: input.reason.clone(),
        liability: input.liability.clone(),
        cash_treatment: input.cash_treatment.clone(),
        cod_amount: shipment.cod_amount,
        cash_amount: if input.cash_amount > 0 {
            input.cash_amount
        } else {
            shipment.cod_collected
        },
        driver_user_id: input.driver,
        approver_id: input.approver,
        target_state: input.target_state.clone(),
        task_status: TASK_NONE.to_owned(),
        ..Reversal::default()
    };
    reversal.insert(&mut *tx).await?;
    // Refunded to the payer: no COD is expected any more, the delivery is
    // still due.
    let expected = if refunded { 0 } else { shipment.cod_amount };
    let released = if refunded {
        shipment.cod_released + shipment.cod_amount
    } else {
        shipment.cod_released
    };
    let no_time: Option<DateTime> = None;
    let no_text: Option<String> = None;
    let no_cash: i64 = 0;
    let liability = Some(input.liability.clone());
    Shipment::query()
        .where_(Shipment::ID.eq(shipment.id))
        .update(
            &mut *tx,
            &[
                ("delivered_at", &no_time),
                ("cod_collected", &no_cash),
                ("cod_difference", &no_cash),
                ("cod_difference_reason", &no_text),
                ("cod_absorbed_by", &no_text),
                ("cod_amount", &expected),
                ("cod_released", &released),
                ("liability", &liability),
                ("cash_treatment", &input.cash_treatment),
                ("reversed", &true),
            ],
        )
        .await?;
    track(
        &mut *tx,
        shipment.id,
        "reversed",
        &input.reason,
        input.approver,
    )
    .await
}

/// Runs the system's `reverse_delivery` of `shipment` for `event`: back to
/// `out_for_delivery` when the delivery is out on the road again, else to
/// `attempted`. A shipment that is not `delivered` or `under_review` is left
/// alone (a second event finds it moved).
///
/// # Errors
///
/// Database errors and the engine's.
pub(crate) async fn follow(
    state: &AppState,
    shipment: Shipment,
    event: &DeliveryReversed,
) -> Result {
    if !matches!(
        shipment.status,
        ShipmentStatus::Delivered | ShipmentStatus::UnderReview
    ) {
        return Ok(());
    }
    let on_road = event.target_state.as_deref() == Some("out_for_delivery");
    let input = ReversalInput {
        delivery: event.delivery.0,
        reason: event.reason.clone(),
        liability: event.liability.clone(),
        cash_treatment: event.cash_treatment.clone(),
        cash_amount: event.cash_amount,
        driver: event.driver,
        approver: event.approver,
        target_state: if on_road {
            "out_for_delivery".to_owned()
        } else {
            "attempted".to_owned()
        },
    };
    let name = if on_road {
        REVERSE_DELIVERY_ON_ROAD
    } else {
        REVERSE_DELIVERY
    };
    let _outcome = shipment
        .transition(name)
        .by_system()
        .input(&ShipmentInput {
            reversal: Some(input),
            ..ShipmentInput::default()
        })
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(())
}

/// The line of `shipment` on a paid remittance, with that remittance. A line
/// on a draft or an approved remittance is taken off (an approved one goes
/// back to a draft and the accountants are told).
async fn paid_line(
    state: &AppState,
    shipment: &Shipment,
) -> Result<Option<(RemittanceLine, Remittance)>> {
    let Some(line) = RemittanceLine::query()
        .where_(RemittanceLine::SHIPMENT_ID.eq(shipment.id))
        .first(&state.db)
        .await?
    else {
        return Ok(None);
    };
    let remittance = Remittance::find_or_404(&state.db, line.remittance_id).await?;
    match remittance.status {
        RemittanceStatus::Paid | RemittanceStatus::PaymentReturned => Ok(Some((line, remittance))),
        RemittanceStatus::Draft | RemittanceStatus::Approved => {
            crate::review::take_off_remittance(state, shipment).await?;
            Ok(None)
        }
    }
}

/// Drafts the negative adjustment line that takes back the COD already paid to
/// the client: on its draft remittance when it has one, else waiting for the
/// next weekly run. It is pending until another user approves it
/// (`LOG.ACL.12`); no user prepared it.
async fn draft_refund_line(
    state: &AppState,
    shipment: &Shipment,
    line: &RemittanceLine,
    remittance: &Remittance,
) -> Result<i64> {
    let draft = Remittance::draft_of(&state.db, shipment.client()).await?;
    let mut adjustment = AdjustmentLine {
        client_party_id: shipment.client_party_id,
        remittance_id: draft.map(|draft| draft.id),
        amount: -line.collected_for_client(),
        reason: english("logistics.reversal.refund_reason", &[]),
        source: source::DELIVERY_REVERSAL.to_owned(),
        shipment_id: Some(shipment.id),
        corrects_remittance_id: Some(remittance.id),
        prepared_by: None,
        status: AdjustmentStatus::Pending,
        ..AdjustmentLine::default()
    };
    adjustment.insert(&state.db).await?;
    Ok(adjustment.id)
}

/// The name of user `id`, else `#id`.
async fn user_name(db: &Db, id: i64) -> String {
    renox::auth::User::find(db, id)
        .await
        .ok()
        .flatten()
        .map_or_else(|| format!("#{id}"), |user| user.name)
}

/// What follows the commit of `reverse_delivery` for shipment `id`: the COD,
/// the billing, the liability and the client's notice (see the module docs).
///
/// # Errors
///
/// Database errors and the ticketing and notifier ones.
pub(crate) async fn settled(state: &AppState, id: i64) -> Result {
    let Some(reversal) = Reversal::query()
        .where_(Reversal::SHIPMENT_ID.eq(id))
        .where_(Reversal::SETTLED.eq(false))
        .order_by_desc(Reversal::ID)
        .first(&state.db)
        .await?
    else {
        return Ok(());
    };
    let shipment = Shipment::find_or_404(&state.db, id).await?;

    // The COD: off a draft or approved remittance; already paid, it is taken
    // back by a negative line when refunded, and never remitted twice when it
    // is kept for the next attempt.
    let mut remitted = false;
    let mut adjustment = None;
    if reversal.cod_amount > 0
        && let Some((line, remittance)) = paid_line(state, &shipment).await?
    {
        remitted = true;
        if reversal.cash_treatment.as_deref() == Some(REFUNDED) {
            adjustment = Some(draft_refund_line(state, &shipment, &line, &remittance).await?);
        } else {
            Shipment::query()
                .where_(Shipment::ID.eq(id))
                .update(&state.db, &[("cod_already_remitted", &true)])
                .await?;
        }
    }

    // The billing: an invoiced shipment gets a credit due.
    let credit_due = credit::due(state, &shipment, "reversed", reversal.id).await?;

    // The liability.
    let company_liable = matches!(reversal.liability.as_str(), "driver" | "business");
    let mut claim_ticket = None;
    if company_liable {
        let title = english(
            "logistics.reversal.claim_title",
            &[("number", &shipment.number)],
        );
        let description = english(
            "logistics.reversal.claim_description",
            &[
                ("number", &shipment.number),
                ("reason", &reversal.reason),
                ("liability", &reversal.liability),
            ],
        );
        claim_ticket = claim::open(
            state,
            &shipment,
            &format!("logistics.reversal_claim:{}", reversal.id),
            &title,
            &description,
        )
        .await?
        .map(|ticket| ticket.id.0);
    }
    let mut task_status = TASK_NONE;
    let mut task_ticket = None;
    if reversal.liability == "driver" {
        task_status = TASK_OPEN;
        let driver = match reversal.driver_user_id {
            Some(driver) => user_name(&state.db, driver).await,
            None => english("logistics.reversal.no_driver", &[]),
        };
        let title = english(
            "logistics.reversal.task_title",
            &[
                ("number", &shipment.number),
                ("amount", &aed(reversal.cash_amount)),
                ("driver", &driver),
            ],
        );
        let description = english(
            "logistics.reversal.task_description",
            &[
                ("number", &shipment.number),
                ("amount", &aed(reversal.cash_amount)),
                ("driver", &driver),
                ("reason", &reversal.reason),
            ],
        );
        let task = RoleTask::new(
            shipment.subject(),
            permissions::OPERATIONS_MANAGER_ROLE,
            &title,
            &format!("logistics.reversal_task:{}", reversal.id),
        )
        .description(&description);
        task_ticket = role_tasks::open(state, task).await?.map(|t| t.id.0);
    }
    let loss = if reversal.liability == "business" {
        reversal.cash_amount.max(reversal.cod_amount)
    } else {
        0
    };

    let task_status = task_status.to_owned();
    Reversal::query()
        .where_(Reversal::ID.eq(reversal.id))
        .update(
            &state.db,
            &[
                ("remitted", &remitted),
                ("adjustment_line_id", &adjustment),
                ("credit_due", &credit_due),
                ("loss_amount", &loss),
                ("claim_ticket_id", &claim_ticket),
                ("task_status", &task_status),
                ("task_ticket_id", &task_ticket),
                ("settled", &true),
            ],
        )
        .await?;

    // The client is told.
    let title = english("logistics.reversal.title", &[("number", &shipment.number)]);
    let body = english(
        "logistics.reversal.body",
        &[("number", &shipment.number), ("reason", &reversal.reason)],
    );
    tell_client(state, &shipment, &title, &body).await
}

/// A 422 with `message`.
fn refused(message: String) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)
}

/// The open task of the reversal `id`, for the operations manager `user`.
async fn open_task(state: &AppState, user: &AuthUser, id: i64) -> Result<Reversal> {
    if !user.has_permission(permissions::SHIPMENTS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let reversal = Reversal::find_or_404(&state.db, id).await?;
    if reversal.task_status != TASK_OPEN {
        return Err(refused(english("logistics.reversal.task_closed", &[])));
    }
    Ok(reversal)
}

/// Records the decision on the task and closes its ticket.
async fn decided(
    state: &AppState,
    user: &AuthUser,
    reversal: &Reversal,
    status: &str,
    handover: Option<i64>,
    hr_ticket: Option<i64>,
    note: &str,
) -> Result<Reversal> {
    let by = Some(user.id);
    let at = Some(renox::db::now());
    let status_value = status.to_owned();
    Reversal::query()
        .where_(Reversal::ID.eq(reversal.id))
        .update(
            &state.db,
            &[
                ("task_status", &status_value),
                ("shortage_handover_id", &handover),
                ("hr_ticket_id", &hr_ticket),
                ("decided_by", &by),
                ("decided_at", &at),
            ],
        )
        .await?;
    track(&state.db, reversal.shipment_id, status, note, by).await?;
    if let Some(ticket) = reversal.task_ticket_id {
        tickets::task_creator(state)
            .complete(state, TicketId(ticket), note)
            .await?;
    }
    Reversal::find_or_404(&state.db, reversal.id).await
}

/// `user` (`shipments.manage`, and `cash.count` in wl-payments) raises a later
/// shortage against the driver for the cash of the reversal `id`, with the
/// shipment as its source (`FIN.ACL.05`: never the driver's own).
///
/// # Errors
///
/// `Forbidden` without the permissions; 422 when the task is not open, the
/// reversal has no driver or no cash, or wl-payments refuses.
pub async fn raise_shortage(state: &AppState, user: &AuthUser, id: i64) -> Result<Reversal> {
    let reversal = open_task(state, user, id).await?;
    let Some(driver) = reversal.driver_user_id else {
        return Err(refused(english("logistics.reversal.no_driver", &[])));
    };
    if reversal.cash_amount <= 0 {
        return Err(refused(english("logistics.reversal.no_cash", &[])));
    }
    let shipment = Shipment::find_or_404(&state.db, reversal.shipment_id).await?;
    let shortage = LaterShortage {
        holder_user: driver,
        amount: reversal.cash_amount,
        reason: english(
            "logistics.reversal.shortage_reason",
            &[("number", &shipment.number), ("reason", &reversal.reason)],
        ),
        source: Subject::new(SUBJECT_TYPE, shipment.id),
    };
    let handover = wl_kernel::custody::shortage_raiser(state)
        .raise(state, user, shortage)
        .await?;
    let note = english(
        "logistics.reversal.shortage_raised",
        &[("amount", &aed(reversal.cash_amount))],
    );
    decided(
        state,
        user,
        &reversal,
        TASK_SHORTAGE,
        Some(handover.0),
        None,
        &note,
    )
    .await
}

/// `user` (`shipments.manage`) opens an HR case for the driver of the
/// reversal `id`: a task for the HR role.
///
/// # Errors
///
/// `Forbidden` without the permission; 422 when the task is not open.
pub async fn open_hr_case(state: &AppState, user: &AuthUser, id: i64) -> Result<Reversal> {
    let reversal = open_task(state, user, id).await?;
    let shipment = Shipment::find_or_404(&state.db, reversal.shipment_id).await?;
    let driver = match reversal.driver_user_id {
        Some(driver) => user_name(&state.db, driver).await,
        None => english("logistics.reversal.no_driver", &[]),
    };
    let title = english(
        "logistics.reversal.hr_title",
        &[("number", &shipment.number), ("driver", &driver)],
    );
    let description = english(
        "logistics.reversal.hr_description",
        &[
            ("number", &shipment.number),
            ("amount", &aed(reversal.cash_amount)),
            ("driver", &driver),
            ("reason", &reversal.reason),
        ],
    );
    let task = RoleTask::new(
        shipment.subject(),
        permissions::HR_ROLE,
        &title,
        &format!("logistics.reversal_hr:{}", reversal.id),
    )
    .description(&description);
    let ticket = role_tasks::open(state, task).await?.map(|t| t.id.0);
    let note = english("logistics.reversal.hr_opened", &[]);
    decided(state, user, &reversal, TASK_HR_CASE, None, ticket, &note).await
}

/// The reversals whose task still waits for the operations manager, oldest
/// first.
///
/// # Errors
///
/// Database errors.
pub async fn open_tasks(db: &Db) -> Result<Vec<Reversal>> {
    Reversal::query()
        .where_(Reversal::TASK_STATUS.eq(TASK_OPEN))
        .order_by(Reversal::ID)
        .get(db)
        .await
}

/// The reversals of shipment `id`, oldest first.
///
/// # Errors
///
/// Database errors.
pub async fn of_shipment(db: &Db, id: i64) -> Result<Vec<Reversal>> {
    Reversal::query()
        .where_(Reversal::SHIPMENT_ID.eq(id))
        .order_by(Reversal::ID)
        .get(db)
        .await
}

/// A refund of a COD to a recipient of a shipment is paid by hand: the
/// recipient is not a party, so wl-payments cannot pay it back
/// ([`ManualRefunds`]).
#[derive(Debug, Clone, Copy, Default)]
pub struct ShipmentRefunds;

impl ManualRefunds for ShipmentRefunds {
    fn accepts(&self, source: &Subject) -> bool {
        source.subject_type == SUBJECT_TYPE
    }
}
