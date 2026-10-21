//! Destroying goods on the client's written instruction (03 Logistics §4.1
//! row *Parcel damaged at the hub*, §7.1 row `dispose`, §7.2 `LOG.RUL.07`,
//! §3.3 `LOG.ACL.14`; BL-LOG-06).
//!
//! - **The request** ([`request`]): a user with `shipments.manage` asks to
//!   dispose of a shipment that is `on_hold`, with a reason, at least one
//!   photo, the reference of the client's written instruction (not needed
//!   when the goods are *dangerous*) and a liability (*client*, *business*
//!   or *none*). The kernel's policy `logistics.dispose` (`LOG.ACL.14`) defers
//!   it as an approval request that a holder of
//!   `shipments.approve_disposal` other than the requester decides; the
//!   shipment stays `on_hold` while it waits, and after a rejection. Even
//!   dangerous goods wait for the approval.
//! - **The effects** ([`record`], [`settled`]): the shipment is `disposed`
//!   and expects no COD. *Client*: a disposal charge from the client's prices
//!   (AED 0 and a note, and a task for the accountant, when it has no
//!   disposal line) goes on the next monthly draft with the shipment, which is
//!   billed because the client caused it. *Business*: a claim ticket opens
//!   (the one the hold opened is the one linked), and the shipment is not
//!   billed. *None*: no charge, no claim, not billed. The liability is stored
//!   on the shipment. The open delivery is cancelled and the client told.
//! - **After a settled claim** ([`crate::loss::choose_keep_and_dispose`]) the
//!   client's choice *keep the settlement* may be carried out as a disposal
//!   instead of a return, with the same request and the same approval.
//!
//! The approver may not change the liability on the approval card: the
//! kernel's generic card has no field for it (recorded in the deferred list).
//! Stock or remains are not tracked in the MVP.

use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{Outcome, StateMachine, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{PolicyDefault, announce, submit::REQUEST_KEY};
use wl_kernel::role_tasks::{self, RoleTask};

use crate::claim;
use crate::hold::NewHold;
use crate::notices::tell_client;
use crate::pages::{Fields, checked, field};
use crate::permissions;
use crate::rates;
use crate::shipment::{Shipment, ShipmentStatus, charge_kind, track};
use crate::shipment_machine::{DISPOSE, ShipmentInput};
use crate::texts::{english, t};
use renox::db::Transaction;

/// The approval policy's action key of a request to dispose (`LOG.ACL.14`).
pub const ACTION: &str = "logistics.dispose";

/// Hours an approval request to dispose may wait.
pub const EXPIRY_HOURS: i64 = 72;

/// Who bears a disposal: the client (its packing or contents caused it), the
/// business, or no one.
pub const LIABILITIES: [&str; 3] = ["client", "business", "none"];

/// What a disposal carries: the user's request, which is also the approval
/// request's payload.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisposalInput {
    /// Why the goods are destroyed.
    #[serde(default)]
    pub reason: String,
    /// The references of the photos.
    #[serde(default)]
    pub photos: Vec<String>,
    /// The reference of the client's written instruction (e-mail, letter).
    #[serde(default)]
    pub instruction: Option<String>,
    /// Whether the hub classed the goods as dangerous.
    #[serde(default)]
    pub dangerous: bool,
    /// Who bears the disposal: one of [`LIABILITIES`].
    #[serde(default)]
    pub liability: String,
}

/// The guard of `dispose`: a reason, a photo, the client's written
/// instruction unless the goods are dangerous, and a liability.
pub(crate) fn check(input: Option<&DisposalInput>) -> std::result::Result<(), String> {
    let Some(input) = input else {
        return Err(english("logistics.disposal.not_prepared", &[]));
    };
    if input.reason.trim().is_empty() {
        return Err(english("logistics.disposal.reason_needed", &[]));
    }
    if input.photos.iter().all(|photo| photo.trim().is_empty()) {
        return Err(english("logistics.disposal.photo_needed", &[]));
    }
    let instructed = input
        .instruction
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty());
    if !instructed && !input.dangerous {
        return Err(english("logistics.disposal.instruction_needed", &[]));
    }
    if !LIABILITIES.contains(&input.liability.trim()) {
        return Err(english("logistics.disposal.liability_needed", &[]));
    }
    Ok(())
}

/// The effect of `dispose`, inside the transition: what was entered is
/// stored, the liability too, no COD is expected any more, and the disposal
/// charge is recorded when the client caused it.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn record(
    tx: &mut Transaction,
    shipment: &Shipment,
    input: &DisposalInput,
    actor: Option<i64>,
) -> Result {
    let at = Some(renox::db::now());
    let reason = Some(input.reason.trim().to_owned());
    let photos = Some(
        input
            .photos
            .iter()
            .map(|photo| photo.trim())
            .filter(|photo| !photo.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
    );
    let instruction = input
        .instruction
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned);
    let liability = Some(input.liability.trim().to_owned());
    let expected: i64 = 0;
    Shipment::query()
        .where_(Shipment::ID.eq(shipment.id))
        .update(
            &mut *tx,
            &[
                ("disposal_reason", &reason),
                ("disposal_photos", &photos),
                ("disposal_instruction", &instruction),
                ("disposal_dangerous", &input.dangerous),
                ("disposed_at", &at),
                ("liability", &liability),
                ("cod_released", &shipment.cod_amount),
                ("cod_amount", &expected),
            ],
        )
        .await?;
    if input.liability.trim() == "client" {
        // Charged only when the client caused it; a client without a disposal
        // line pays AED 0 and the accountant is told after the commit.
        match rates::price(&mut *tx, shipment.client(), charge_kind::DISPOSAL).await? {
            Some(amount) => {
                rates::record_charge(&mut *tx, shipment, charge_kind::DISPOSAL, amount, "").await?;
            }
            None => {
                let note = english(
                    "logistics.disposal.no_price",
                    &[("client", &shipment.client_party_id)],
                );
                rates::record_charge(&mut *tx, shipment, charge_kind::DISPOSAL, 0, &note).await?;
            }
        }
    }
    track(
        &mut *tx,
        shipment.id,
        "disposed",
        reason.as_deref().unwrap_or_default(),
        actor,
    )
    .await
}

/// What follows the commit of a disposal of shipment `id`: the open delivery
/// is cancelled, the claim ticket opens for *business*, the accountant is
/// told when the client was charged AED 0, and the client is told.
///
/// # Errors
///
/// Database errors and the ticketing, notifier and role task ones.
pub(crate) async fn settled(state: &AppState, id: i64) -> Result {
    let shipment = Shipment::find_or_404(&state.db, id).await?;
    crate::events::cancel_delivery(
        state,
        &shipment,
        &english("logistics.disposal.delivery_reason", &[]),
    )
    .await?;
    match shipment.liability.as_deref() {
        Some("business") if shipment.claim_ticket_id.is_none() => {
            let title = english(
                "logistics.disposal.claim_title",
                &[("number", &shipment.number)],
            );
            let description = english(
                "logistics.disposal.claim_description",
                &[
                    ("number", &shipment.number),
                    ("reason", &shipment.disposal_reason.clone().unwrap_or_default()),
                ],
            );
            claim::open(
                state,
                &shipment,
                &format!("logistics.disposal_claim:{}", shipment.id),
                &title,
                &description,
            )
            .await?;
        }
        Some("client") => {
            let charge = shipment
                .charges(&state.db)
                .await?
                .into_iter()
                .find(|charge| charge.kind == charge_kind::DISPOSAL);
            if charge.is_some_and(|charge| charge.amount == 0) {
                let title = english(
                    "logistics.disposal.accountant_title",
                    &[("number", &shipment.number)],
                );
                let description = english(
                    "logistics.disposal.accountant_description",
                    &[("number", &shipment.number)],
                );
                let task = RoleTask::new(
                    shipment.subject(),
                    permissions::INVOICE_OWNER_ROLE,
                    &title,
                    &format!("logistics.disposal_price:{}", shipment.id),
                )
                .description(&description);
                role_tasks::open(state, task).await?;
            }
        }
        _ => {}
    }
    let shipment = Shipment::find_or_404(&state.db, id).await?;
    let title = english("logistics.disposal.title", &[("number", &shipment.number)]);
    let body = english("logistics.disposal.body", &[("number", &shipment.number)]);
    tell_client(state, &shipment, &title, &body).await
}

/// What a request to dispose came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisposeRequest {
    /// No approval was needed: the shipment is disposed.
    Disposed,
    /// An approval request (its id) waits for an operations manager or the
    /// company admin other than the requester.
    Requested(i64),
}

/// `user` (`shipments.manage`) asks to dispose of the held shipment `id`.
/// The request is announced after the commit.
///
/// # Errors
///
/// `Forbidden` without `shipments.manage`; `NotFrom` unless the shipment is
/// `on_hold`; `Guard` (422) without a reason, a photo, the client's
/// instruction (unless dangerous) or a known liability, and when a request is
/// already pending.
pub async fn request(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    input: &DisposalInput,
) -> std::result::Result<DisposeRequest, TransitionError> {
    let shipment = Shipment::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    let outcome = shipment
        .transition(DISPOSE)
        .by(user)
        .input(&ShipmentInput {
            disposal: Some(input.clone()),
            ..ShipmentInput::default()
        })
        .run(state)
        .await?;
    announce(state, &outcome)
        .await
        .map_err(TransitionError::Failed)?;
    Ok(match &outcome {
        Outcome::Pending(pending) => match pending.data.get(REQUEST_KEY).and_then(Value::as_i64) {
            Some(id) => DisposeRequest::Requested(id),
            None => DisposeRequest::Disposed,
        },
        _ => DisposeRequest::Disposed,
    })
}

/// Registers the approval policy of `LOG.ACL.14` for disposal.
pub(crate) fn register() {
    PolicyDefault::new(ACTION)
        .step_permission(permissions::SHIPMENTS_APPROVE_DISPOSAL)
        .expiry_hours(Some(EXPIRY_HOURS))
        .register();
}

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .post("/shipments/{id}/dispose", dispose)
        .name("logistics.shipment_dispose")
        .require_auth()
}

/// `POST /shipments/{id}/dispose`: `reason`, `photos` (references, one per
/// line), `instruction` (the reference of the client's written instruction),
/// `dangerous` (ticked) and `liability` (`client`, `business` or `none`). The
/// request waits for an operations manager other than the user.
async fn dispose(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let input = DisposalInput {
        reason: field(&form, "reason"),
        photos: NewHold::photos_in(&field(&form, "photos")),
        instruction: Some(field(&form, "instruction")),
        dangerous: checked(&form, "dangerous"),
        liability: field(&form, "liability"),
    };
    let asked = request(&state, &user, id, &input)
        .await
        .map_err(TransitionError::into_error)?;
    let key = match asked {
        DisposeRequest::Disposed => "logistics.disposed",
        DisposeRequest::Requested(_) => "logistics.disposal_requested",
    };
    Ok((
        Toast::success(t(&lang, key)),
        Redirect::route("logistics.shipment", &[&id])?,
    )
        .into_response())
}

/// The states a shipment can be disposed of from.
pub fn requestable(status: ShipmentStatus) -> bool {
    status == ShipmentStatus::OnHold
}
