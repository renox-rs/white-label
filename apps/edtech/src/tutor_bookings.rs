//! The tutor of a one-to-one booking (10 §7.1 *Booking*, §9; BL-EDU-17).
//!
//! wl-scheduling asks the service type's own `required_qualifications` of the
//! person who delivers a booking (the one-to-one type asks the police
//! clearance, [`crate::seed::ONE_TO_ONE_QUALIFICATIONS`]) and then the
//! vertical's [`ResourceEligibility`]. This is 10's: a **freelance** tutor
//! also needs the private tutor permit or freelance visa, valid on the day of
//! the booking; an employed tutor needs no permit. The check runs for
//! `book`, `reassign`, `reschedule`, `check_in` and `start`, as the module
//! calls it, and judges the day the booking starts.
//!
//! The same permit and visa are one document kind
//! ([`crate::seed::TUTOR_PERMIT_KIND`]), so either one valid on the day passes.

use renox::prelude::*;
use wl_kernel::documents::dubai_date;
use wl_kernel::scheduling::{Eligibility, EligibilityQuery, ResourceEligibility, SchedulingFuture};

use crate::eligibility::{failures, is_freelance, kind_text, tutor_text};
use crate::seed::{ONE_TO_ONE_SERVICE, TUTOR_PERMIT_KIND};
use crate::texts::english;

/// 10's [`ResourceEligibility`] for the bookings of tutors.
#[derive(Debug, Clone, Copy, Default)]
pub struct TutorBookings;

impl ResourceEligibility for TutorBookings {
    fn check<'a>(
        &'a self,
        state: &'a AppState,
        query: EligibilityQuery,
    ) -> SchedulingFuture<'a, Eligibility> {
        Box::pin(async move {
            if query.service_type != ONE_TO_ONE_SERVICE {
                return Ok(Eligibility::Eligible);
            }
            let Some(employee) = query.employee_id else {
                return Ok(Eligibility::Eligible);
            };
            if !is_freelance(state, Some(employee)).await? {
                return Ok(Eligibility::Eligible);
            }
            let on = dubai_date(query.at);
            let missing = failures(state, Some(employee), &[TUTOR_PERMIT_KIND], &[on]).await?;
            if missing.is_empty() {
                return Ok(Eligibility::Eligible);
            }
            let name = tutor_text(state, Some(employee)).await;
            let kind = kind_text(TUTOR_PERMIT_KIND);
            Ok(Eligibility::Refused(english(
                "edtech.tutors.problem.not_eligible",
                &[
                    ("tutor", &name),
                    ("kind", &kind),
                    ("first", &on),
                    ("dates", &on),
                ],
            )))
        })
    }
}

/// Makes [`TutorBookings`] the app's resource eligibility.
pub(crate) fn register(app: &mut Registry) {
    wl_kernel::scheduling::provide_resource_eligibility(app, TutorBookings);
}
