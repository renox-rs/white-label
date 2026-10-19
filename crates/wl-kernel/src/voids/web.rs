//! The back office's part of voiding (wl-kernel §5.12, `PLT.ACL.09`): the
//! "Void" item under More and the route it posts to.
//!
//! A voidable record's page gives its template `void_action`
//! ([`action_for`], from the actions `available()` returned for the viewer)
//! and includes the partial next to its other actions:
//!
//! ```html
//! <rx-include template="wl-kernel/void_action.html"/>
//! ```
//!
//! The item is shown only when `void` is available, which already leaves
//! out states the module does not allow, records given to a third party and
//! users without the permission. The page should not draw `void` again in
//! its `<rx-transition-actions>`. A voided row in a list takes the class
//! [`VOIDED_CLASS`] ([`is_voided`](super::is_voided) says when).

use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::Available;
use serde::{Deserialize, Serialize};

use super::{VOID, VoidOutcome};
use crate::subject::Subject;
use crate::texts;

/// The template of the item, compiled into the kernel.
pub const ACTION_TEMPLATE: &str = "wl-kernel/void_action.html";

/// The class of a voided record's row; `app.css` strikes it through.
pub const VOIDED_CLASS: &str = "wl-voided";

/// The name of the void route.
pub const ROUTE_NAME: &str = "wl_kernel.voids.void";

/// What the partial needs for `subject`, or `None` when `void` is not among
/// the viewer's `actions` (the partial then shows nothing).
pub fn action_for(subject: &Subject, actions: &[Available], lang: &Lang) -> Option<Value> {
    if !actions.iter().any(|a| a.name == VOID) {
        return None;
    }
    let t = |key: &str| texts::t(lang, key);
    Some(json!({
        "id": format!("void-{}-{}", subject.subject_type.replace('.', "-"), subject.subject_id),
        "subject_type": subject.subject_type,
        "subject_id": subject.subject_id,
        "labels": {
            "more": t("wl_kernel.voids.more"),
            "void": t("wl_kernel.voids.action"),
            "title": t("wl_kernel.voids.title"),
            "reason": t("wl_kernel.voids.reason"),
            "hint": t("wl_kernel.voids.reason_hint"),
        },
    }))
}

/// The void route, for logged-in users; who may void is checked per
/// request (422 otherwise, as for any refused transition):
///
/// - `POST /voids/{subject_type}/{id}` (`reason` required, not blank): runs
///   [`voids::void`](super::void) as the user and goes back. The toast says
///   "Voided", or "Sent for approval" when an approval request waits.
pub fn routes() -> Routes {
    Routes::new()
        .post("/voids/{subject_type}/{id}", void_record)
        .name(ROUTE_NAME)
        .require_auth()
}

/// The form of the void action.
#[derive(Debug, Default, Deserialize, Serialize)]
struct VoidForm {
    #[serde(default)]
    reason: Option<String>,
}

impl Validate for VoidForm {
    fn rules(&self, _v: &mut Validator) {}
}

async fn void_record(
    State(state): State<AppState>,
    user: AuthUser,
    Path((subject_type, id)): Path<(String, i64)>,
    lang: Lang,
    back: Back,
    Valid(form): Valid<VoidForm>,
) -> Result<Response> {
    let reason = form.reason.as_deref().map_or("", str::trim);
    if reason.is_empty() {
        let mut errors = Errors::new();
        errors.add("reason", texts::t(&lang, "wl_kernel.voids.reason_required"));
        return Err(ValidationError::new(errors).with_input(&form).into());
    }
    let subject = Subject::new(&subject_type, id);
    let outcome = super::void(&state, &user, &subject, reason)
        .await
        .map_err(renox::state_machine::TransitionError::into_error)?;
    let toast = match outcome {
        VoidOutcome::Voided(_) => Toast::success(texts::t(&lang, "wl_kernel.voids.toast.voided")),
        VoidOutcome::Pending(_) => Toast::info(texts::t(&lang, "wl_kernel.approvals.sent")),
    };
    Ok((toast, back).into_response())
}
