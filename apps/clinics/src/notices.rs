//! The notification types 05 registers besides the ones its flows own (05
//! §7.2 *Notification types registered by 05*; BL-CLN-15).
//!
//! The four types are `clinics.card_upload_request` (document requests:
//! [`crate::card_requests`]), `clinics.prior_auth_denied` (booking messages:
//! [`crate::authorisations`]), `clinics.product_recall_notice` (urgent, no
//! group: [`crate::recalls`]) and `clinics.recall_invite`, here.
//!
//! Decision: the keys carry the app's name, `clinics.`, not the spec's
//! `clinic.`: the texts and the subject types of the app do the same, and the
//! kernel names a notification type by `<module>.<name>`.
//!
//! `clinics.recall_invite` is the type `CLN.RUL.02` sends (a follow-up recall,
//! such as a dental check-up): the booking messages group, normal urgency, no
//! task when it gives up (the group's default). The rule that sends it is
//! `CLN.RUL.02`, which is not built yet (see `DEFERRED-b6.md`).

use wl_kernel::notify::{self, GROUP_BOOKINGS, NotificationTypeDef, NotifyChannel};

use crate::texts::english;

/// The notification type of the recall invitation (`CLN.RUL.02`).
pub const RECALL_INVITE: &str = "clinics.recall_invite";

/// The Arabic subject of the invitation.
const SUBJECT_AR: &str = "حان وقت زيارتك القادمة";

/// The Arabic body of the invitation: the clinic, the time and a way to book,
/// nothing clinical.
const BODY_AR: &str = "حان وقت حجز زيارتك القادمة. اتصل بنا أو استخدم الرابط لاختيار موعد.";

/// The types of this module.
pub fn notification_types() -> Vec<NotificationTypeDef> {
    let subject = english("clinics.recall_invite.subject", &[]);
    let body = english("clinics.recall_invite.body", &[]);
    vec![
        NotificationTypeDef::new(RECALL_INVITE, "clinics")
            .group(GROUP_BOOKINGS)
            .template(NotifyChannel::Email, "en", Some(&subject), &body)
            .template(NotifyChannel::Email, "ar", Some(SUBJECT_AR), BODY_AR),
    ]
}

/// Registers the types.
pub(crate) fn register() {
    for def in notification_types() {
        notify::register_type(def);
    }
}
