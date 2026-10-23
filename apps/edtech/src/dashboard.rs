//! The figures of the home dashboard that belong to the vertical: the sessions
//! today, the enrolments waiting for their first payment, the batches open for
//! enrolment, the certificates drafted and waiting to be issued, and the sessions
//! flagged (a tutor who cannot teach). Each is `None` when the user lacks the
//! permission of the page that lists it, so the dashboard hides the tile; a tutor
//! counts the sessions they teach only (`EDU.ACL.03`).

use renox::chrono::NaiveDate;
use renox::prelude::*;

use crate::batch::{Batch, BatchStatus};
use crate::certificate::{Certificate, CertificateStatus};
use crate::enrolment::{Enrolment, EnrolmentStatus};
use crate::permissions;
use crate::session::{Session, SessionStatus};
use crate::sessions;

/// What the dashboard shows of the vertical.
#[derive(Debug, Clone, Default)]
pub struct Figures {
    /// Sessions of `today` still to be taught (`sessions.view`).
    pub sessions_today: Option<u64>,
    /// Sessions from today on that are flagged (`sessions.view`).
    pub sessions_flagged: Option<u64>,
    /// Enrolments waiting for their first payment (`enrolments.view`).
    pub enrolments_waiting: Option<u64>,
    /// Batches open for enrolment (`batches.view`).
    pub batches_open: Option<u64>,
    /// Certificates in draft (`certificates.view`).
    pub certificates_draft: Option<u64>,
}

/// The figures `user` may see, as of `today`.
///
/// # Errors
///
/// The database's errors and the workforce contract's.
pub async fn figures(state: &AppState, user: &AuthUser, today: NaiveDate) -> Result<Figures> {
    let mut out = Figures::default();
    if user.has_permission(permissions::SESSIONS_VIEW) {
        let own = sessions::assigned_scope(state, user).await?;
        let coming = Session::query()
            .where_(Session::SESSION_DATE.gte(today))
            .get(&state.db)
            .await?;
        let mine: Vec<&Session> = coming
            .iter()
            .filter(|s| own.is_none_or(|employee| s.tutor_employee_id == Some(employee)))
            .collect();
        out.sessions_today = Some(
            mine.iter()
                .filter(|s| s.session_date == today && s.status == SessionStatus::Scheduled)
                .count() as u64,
        );
        out.sessions_flagged = Some(mine.iter().filter(|s| s.is_flagged()).count() as u64);
    }
    if user.has_permission(permissions::ENROLMENTS_VIEW) {
        out.enrolments_waiting = Some(
            Enrolment::query()
                .where_(Enrolment::STATUS.eq(EnrolmentStatus::PendingPayment))
                .count(&state.db)
                .await?,
        );
    }
    if user.has_permission(permissions::BATCHES_VIEW) {
        out.batches_open = Some(
            Batch::query()
                .where_(Batch::STATUS.eq(BatchStatus::Open))
                .count(&state.db)
                .await?,
        );
    }
    if user.has_permission(permissions::CERTIFICATES_VIEW) {
        out.certificates_draft = Some(
            Certificate::query()
                .where_(Certificate::STATUS.eq(CertificateStatus::Draft))
                .count(&state.db)
                .await?,
        );
    }
    Ok(out)
}
