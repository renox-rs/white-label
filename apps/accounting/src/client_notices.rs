//! What the client is told about a restricted suspension or end
//! (`ACC.ACL.14`): nothing automatically; the MLRO (or deputy) chooses a
//! neutral template, and the choice is recorded on the engagement.
//!
//! The two neutral templates ([`NeutralTemplate`]) say nothing about a block,
//! a screening or a reason: "Your engagement is on hold, please contact your
//! partner" and "Our engagement has ended", in English and Arabic. They are
//! statutory notices of 04 §9 (*suspension and end notices*), sent through
//! the kernel's notifier to the client party. Only holders of
//! `parties.block_reason.view` send them ([`send`]).

use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};
use wl_kernel::notify::{self, NotificationTypeDef, NotifyChannel, NotifyRequest, notifier};

use crate::db::refused;
use crate::engagement::{Engagement, EngagementStatus};
use crate::permissions::{self, MLRO_ROLE};
use crate::texts::english;

/// The notification type of the neutral "on hold" template.
pub const ON_HOLD_NOTICE: &str = "accounting.engagement_on_hold";
/// The notification type of the neutral "ended" template.
pub const ENDED_NOTICE: &str = "accounting.engagement_ended";

/// The notification type of the neutral "not proceeding" template.
pub const DECLINED_NOTICE: &str = "accounting.engagement_declined";
/// The notification type of the neutral "unable to proceed" template.
pub const UNABLE_NOTICE: &str = "accounting.engagement_unable";

/// The module that sends the notices.
const MODULE: &str = "accounting";

/// A neutral template the MLRO may send.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NeutralTemplate {
    /// "Your engagement is on hold, please contact your partner": for a
    /// suspended engagement.
    #[default]
    OnHold,
    /// "Our engagement has ended": for an ended engagement.
    Ended,
    /// "We are not able to proceed with the proposed engagement": for a
    /// declined engagement whose reason is not *CDD rejected* (the letter was
    /// not signed, the client withdrew, another reason).
    Declined,
    /// "We are unable to proceed with an engagement at this time": for a
    /// declined engagement whose reason is *CDD rejected*; it never says
    /// why.
    Unable,
}

impl NeutralTemplate {
    /// The notification type it is sent as.
    pub fn type_key(self) -> &'static str {
        match self {
            NeutralTemplate::OnHold => ON_HOLD_NOTICE,
            NeutralTemplate::Ended => ENDED_NOTICE,
            NeutralTemplate::Declined => DECLINED_NOTICE,
            NeutralTemplate::Unable => UNABLE_NOTICE,
        }
    }

    /// The engagement status it fits.
    pub fn fits(self, status: EngagementStatus) -> bool {
        matches!(
            (self, status),
            (NeutralTemplate::OnHold, EngagementStatus::Suspended)
                | (NeutralTemplate::Ended, EngagementStatus::Ended)
                | (
                    NeutralTemplate::Declined | NeutralTemplate::Unable,
                    EngagementStatus::Declined
                )
        )
    }

    /// The template of a decline for `reason` (`DECLINE_REASONS`): the one
    /// template of the reason's group. *CDD rejected* has its own, which
    /// never mentions due diligence.
    pub fn for_decline(reason: &str) -> NeutralTemplate {
        if reason.trim() == crate::reasons::NOT_ACCEPTED {
            NeutralTemplate::Unable
        } else {
            NeutralTemplate::Declined
        }
    }

    /// The template named `text` (`on_hold`, `ended`).
    pub fn parse(text: &str) -> Option<NeutralTemplate> {
        NeutralTemplate::ALL
            .iter()
            .copied()
            .find(|t| t.as_str() == text.trim())
    }
}

/// A neutral template sent to the client of an engagement.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "accounting_client_notices", index(engagement_id))]
pub struct ClientNotice {
    /// The primary key.
    pub id: i64,
    /// The engagement.
    #[model(references = "accounting_engagements")]
    pub engagement_id: i64,
    /// The template chosen.
    pub template: NeutralTemplate,
    /// The MLRO or deputy who sent it.
    pub sent_by: i64,
    /// When it was sent.
    pub sent_at: DateTime,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ClientNotice {
    /// The notices of engagement `id`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_engagement(db: &Db, id: i64) -> Result<Vec<ClientNotice>> {
        ClientNotice::query()
            .where_(ClientNotice::ENGAGEMENT_ID.eq(id))
            .order_by(ClientNotice::ID)
            .get(db)
            .await
    }
}

/// A notification type of a neutral template with its English and Arabic
/// texts: a statutory notice to the client (04 §9), whose give-up is the
/// MLRO's task.
fn neutral_type(
    key: &str,
    texts: &str,
    arabic_subject: &str,
    arabic_body: &str,
) -> NotificationTypeDef {
    let subject = english(&format!("accounting.client_notices.{texts}_subject"), &[]);
    let body = english(&format!("accounting.client_notices.{texts}_body"), &[]);
    NotificationTypeDef::new(key, MODULE)
        .group("statutory")
        .statutory(true)
        .staff_role(MLRO_ROLE)
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
        .template(
            NotifyChannel::Email,
            "ar",
            Some(arabic_subject),
            arabic_body,
        )
}

/// Registers the neutral templates with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(neutral_type(
        ON_HOLD_NOTICE,
        "on_hold",
        "ارتباطكم {{ number }}",
        "ارتباطكم معلّق مؤقتًا، يرجى التواصل مع الشريك المسؤول عنكم.",
    ));
    notify::register_type(neutral_type(
        DECLINED_NOTICE,
        "declined",
        "ارتباطكم {{ number }}",
        "لا يمكننا المضي في الارتباط المقترح، يرجى التواصل مع الشريك المسؤول عنكم.",
    ));
    notify::register_type(neutral_type(
        UNABLE_NOTICE,
        "unable",
        "ارتباطكم {{ number }}",
        "لا يمكننا المضي في ارتباط في الوقت الحالي.",
    ));
    notify::register_type(neutral_type(
        ENDED_NOTICE,
        "ended",
        "ارتباطكم {{ number }}",
        "انتهى ارتباطنا.",
    ));
}

/// `by` (the MLRO or deputy) tells the client of engagement `id` with the
/// neutral `template`: the notice goes to the client party and is recorded
/// on the engagement.
///
/// Decision: the choice is recorded before the notice is queued, so a
/// notice is never sent without its record.
///
/// # Errors
///
/// `Forbidden` without `parties.block_reason.view`; a 422 when the template
/// does not fit the engagement's status (on hold for `suspended`, ended for
/// `ended`, declined or unable for `declined`); the notifier's errors.
pub async fn send(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    template: NeutralTemplate,
) -> Result<ClientNotice> {
    if !by.has_permission(permissions::BLOCK_REASON_VIEW) {
        return Err(Error::Forbidden);
    }
    let engagement = Engagement::find_or_404(&state.db, id).await?;
    send_template(state, &engagement, template, by.id).await
}

/// Records and queues the neutral `template` for the client of
/// `engagement`, as user `by`; the permission is the caller's to check
/// (the MLRO's [`send`], or the manager who declines).
///
/// # Errors
///
/// A 422 when the template does not fit the engagement's status; the
/// notifier's errors.
pub(crate) async fn send_template(
    state: &AppState,
    engagement: &Engagement,
    template: NeutralTemplate,
    by: i64,
) -> Result<ClientNotice> {
    if !template.fits(engagement.status) {
        return Err(refused(english(
            "accounting.client_notices.wrong_template",
            &[],
        )));
    }
    let mut notice = ClientNotice {
        engagement_id: engagement.id,
        template,
        sent_by: by,
        sent_at: now(),
        ..ClientNotice::default()
    };
    notice.insert(&state.db).await?;
    let request = NotifyRequest::new(template.type_key(), engagement.subject())
        .to_party(engagement.client())
        .subject_label(&engagement.number())
        .data(json!({ "number": engagement.number() }));
    notifier(state).notify(state, request).await?;
    Ok(notice)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_template_fits_its_status_only() {
        assert!(NeutralTemplate::OnHold.fits(EngagementStatus::Suspended));
        assert!(!NeutralTemplate::OnHold.fits(EngagementStatus::Ended));
        assert!(NeutralTemplate::Ended.fits(EngagementStatus::Ended));
        assert!(!NeutralTemplate::Ended.fits(EngagementStatus::Active));
        assert!(NeutralTemplate::Declined.fits(EngagementStatus::Declined));
        assert!(NeutralTemplate::Unable.fits(EngagementStatus::Declined));
        assert!(!NeutralTemplate::Unable.fits(EngagementStatus::Ended));
        assert_eq!(
            NeutralTemplate::for_decline("not_accepted"),
            NeutralTemplate::Unable
        );
        assert_eq!(
            NeutralTemplate::for_decline("letter_not_signed"),
            NeutralTemplate::Declined
        );
        assert_eq!(
            NeutralTemplate::parse("ended"),
            Some(NeutralTemplate::Ended)
        );
        assert_eq!(NeutralTemplate::parse("sanctions"), None);
        assert_eq!(NeutralTemplate::OnHold.as_str(), "on_hold");
    }

    #[test]
    fn the_templates_say_nothing_about_a_block() {
        for key in [
            "accounting.client_notices.on_hold_body",
            "accounting.client_notices.ended_body",
            "accounting.client_notices.declined_body",
            "accounting.client_notices.unable_body",
        ] {
            let body = english(key, &[]).to_lowercase();
            for word in [
                "sanction",
                "block",
                "aml",
                "screening",
                "match",
                "diligence",
                "cdd",
                "kyc",
            ] {
                assert!(!body.contains(word), "{key}: {body}");
            }
        }
        assert_eq!(
            english("accounting.client_notices.on_hold_body", &[]),
            "Your engagement is on hold, please contact your partner."
        );
    }
}
