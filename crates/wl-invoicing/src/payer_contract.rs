//! The module's [`PayerBills`]: what a vertical may do with payer bills
//! without depending on this crate (wl-invoicing §6 `PayerBilling`). Every
//! action is the service of the same name run as the signed-in user, or as
//! the system when the vertical gives none; a refusal of the machine goes
//! back as the 403 or 422 of [`TransitionError::into_error`].

use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_kernel::Subject;
use wl_kernel::payer_bills::{
    NewPayerBill, PayerAnswer, PayerBillFuture, PayerBillId, PayerBillStatus, PayerBillSummary,
    PayerBills, Recoupment, RejectionDeadline, Remittance, SubmissionError,
};

use crate::credit_notes::By;
use crate::payer_bill::PayerBill;
use crate::payer_issue;
use crate::payer_recoupment;
use crate::payer_rejection;
use crate::payer_remittance;
use crate::payer_split;

/// The module's [`PayerBills`].
#[derive(Debug, Clone, Copy, Default)]
pub struct PayerBilling;

/// `user` as the actor of a run, the system when none.
fn by_of(user: Option<&AuthUser>) -> By<'_> {
    match user {
        Some(user) => By::User(user),
        None => By::System(None),
    }
}

impl PayerBills for PayerBilling {
    fn find<'a>(
        &'a self,
        db: &'a Db,
        id: PayerBillId,
    ) -> PayerBillFuture<'a, Option<PayerBillSummary>> {
        Box::pin(async move { Ok(PayerBill::find(db, id.0).await?.map(|bill| bill.summary())) })
    }

    fn of_source<'a>(
        &'a self,
        db: &'a Db,
        source: &'a Subject,
    ) -> PayerBillFuture<'a, Vec<PayerBillSummary>> {
        Box::pin(async move {
            Ok(PayerBill::query()
                .where_(PayerBill::SOURCE_TYPE.eq(source.subject_type.clone()))
                .where_(PayerBill::SOURCE_ID.eq(source.subject_id))
                .where_op("status", "!=", PayerBillStatus::Voided.as_str())
                .order_by(PayerBill::ID)
                .get(db)
                .await?
                .iter()
                .map(PayerBill::summary)
                .collect())
        })
    }

    fn create<'a>(
        &'a self,
        state: &'a AppState,
        request: NewPayerBill,
        by: Option<&'a AuthUser>,
    ) -> PayerBillFuture<'a, PayerBillId> {
        Box::pin(async move {
            let bill = payer_split::create(state, request, by.map(|user| user.id)).await?;
            Ok(bill.payer_bill_id())
        })
    }

    fn correct_line<'a>(
        &'a self,
        state: &'a AppState,
        id: PayerBillId,
        line: i64,
        claimed: i64,
    ) -> PayerBillFuture<'a, ()> {
        Box::pin(async move {
            payer_split::correct_line(state, id.0, line, claimed)
                .await
                .map(|_| ())
        })
    }

    fn issue<'a>(
        &'a self,
        state: &'a AppState,
        id: PayerBillId,
        by: Option<&'a AuthUser>,
    ) -> PayerBillFuture<'a, ()> {
        Box::pin(async move {
            payer_issue::issue(state, by_of(by), id.0)
                .await
                .map(|_| ())
                .map_err(TransitionError::into_error)
        })
    }

    fn submission_rejected<'a>(
        &'a self,
        state: &'a AppState,
        id: PayerBillId,
        errors: Vec<SubmissionError>,
        by: Option<&'a AuthUser>,
    ) -> PayerBillFuture<'a, RejectionDeadline> {
        Box::pin(async move {
            let (_, deadline) =
                payer_rejection::submission_rejected(state, by_of(by), id.0, errors)
                    .await
                    .map_err(TransitionError::into_error)?;
            Ok(RejectionDeadline { deadline })
        })
    }

    fn resubmit<'a>(
        &'a self,
        state: &'a AppState,
        id: PayerBillId,
        corrections: &'a str,
        by: Option<&'a AuthUser>,
    ) -> PayerBillFuture<'a, ()> {
        Box::pin(async move {
            payer_issue::resubmit(state, by_of(by), id.0, corrections)
                .await
                .map(|_| ())
                .map_err(TransitionError::into_error)
        })
    }

    fn record_remittance<'a>(
        &'a self,
        state: &'a AppState,
        id: PayerBillId,
        remittance: Remittance,
        by: &'a AuthUser,
    ) -> PayerBillFuture<'a, PayerAnswer> {
        Box::pin(async move {
            payer_remittance::record(state, by, id.0, remittance)
                .await
                .map(|(_, answer)| answer)
                .map_err(TransitionError::into_error)
        })
    }

    fn record_recoupment<'a>(
        &'a self,
        state: &'a AppState,
        id: PayerBillId,
        recoupment: Recoupment,
        by: &'a AuthUser,
    ) -> PayerBillFuture<'a, PayerAnswer> {
        Box::pin(async move {
            payer_recoupment::record(state, by, id.0, recoupment)
                .await
                .map(|(_, answer)| answer)
                .map_err(TransitionError::into_error)
        })
    }
}
