//! What surrounds a supplier warranty claim (08 §4.6 step 4, §7.2 `AUT.RUL.07`
//! and `AUT.RUL.08`, `AUT.WAR.02`; BL-AUT-11).
//!
//! - **`AUT.RUL.07`** ([`on_ticket_closed`]): when a warranty ticket is
//!   `closed` (never when it is only `resolved`, D-07) the purchaser gets a task
//!   to raise the supplier claim. For a job card the task is made only when a
//!   part was fitted on it, the part to claim. Whether the ticket's *warranty
//!   decision* is accepted is not readable through the kernel (see
//!   `DEFERRED.md`): the purchaser confirms it when raising, and the claim is
//!   refused when the decision was not accepted.
//! - **Raising** ([`after_raise`]): the claim document is emailed to the
//!   supplier (`auto.supplier_claim`; the purchaser gets a task when it cannot
//!   be sent), and the failed part, when it was kept, goes on a draft Return to
//!   supplier with outcome *credit* through the kernel's `SupplierReturnDrafts`
//!   (wl-inventory), kept for collection at the store of the job card's
//!   workshop; its dispatch and credit are BL-STK-03 and BL-STK-05.
//! - **A refusal** ([`ask_to_write_off`]): a Return credited by hand with the
//!   reason *supplier refused* does not recover the claim; the purchaser gets a
//!   task to write it off.
//! - **`AUT.RUL.08`** ([`chase`]): a claim still `raised` 60 calendar days
//!   (Asia/Dubai) after it was raised tells the purchaser and the branch
//!   manager, once.
//!
//! Decision (D-06, D-07): the rules used the wrong trigger (`resolved`) and a
//! state that does not exist (`supplier_claimed`); here the trigger is `closed`
//! and the only states are `raised`, `recovered` and `written_off`.

use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::documents::{dubai_date, today};
use wl_kernel::inventory::{DraftReturn, stock_service, supplier_return_drafts};
use wl_kernel::notify::{
    self, GROUP_SIGNATURES, GiveUpReaction, NotificationTypeDef, NotifyChannel, NotifyRequest,
    notifier,
};
use wl_kernel::parties::directory;
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::tickets::{TicketClosed, TicketId, task_creator};
use wl_kernel::{PartyId, Subject};

use crate::estimate::Estimate;
use crate::job_card::{JOB_CARD_SUBJECT_TYPE, JobCard};
use crate::labour_line::LineStatus;
use crate::permissions;
use crate::supplier_claim::{SupplierClaim, SupplierClaimStatus};
use crate::supplier_claims::WARRANTY_CATEGORY;
use crate::texts::english;

/// The rule that makes the task.
pub const RULE_TASK: &str = "AUT.RUL.07";
/// The rule that tells at N days.
pub const RULE_CHASE: &str = "AUT.RUL.08";
/// N: the days after which an unrecovered claim is chased.
pub const CHASE_DAYS: i64 = 60;
/// The email with the claim document to the supplier.
pub const SUPPLIER_CLAIM_TYPE: &str = "auto.supplier_claim";
/// The scheduled task and command of the daily chase.
pub const TASK: &str = "auto:supplier-chase";
/// The time of day it runs, Asia/Dubai.
pub const TASK_AT: &str = "08:30";
/// The time zone of [`TASK_AT`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// The notification type of the claim document: an email to the supplier; when
/// it gives up the purchaser gets a task.
pub fn supplier_claim_type() -> NotificationTypeDef {
    let subject = english("auto.supplier_mail.subject", &[]);
    let body = english("auto.supplier_mail.body", &[]);
    NotificationTypeDef::new(SUPPLIER_CLAIM_TYPE, "auto")
        .group(GROUP_SIGNATURES)
        .give_up(&[
            GiveUpReaction::MarkContactInvalid,
            GiveUpReaction::StaffTask,
        ])
        .staff_role(permissions::PURCHASER_ROLE)
        .channels(&[NotifyChannel::Email])
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
}

/// Opens a task for `role` about `subject`, found by `key` while open.
async fn task(
    state: &AppState,
    subject: Subject,
    role: &str,
    title: &str,
    body: &str,
    key: &str,
) -> Result {
    let task = RoleTask::new(subject, role, title, key).description(body);
    role_tasks::open(state, task).await?;
    Ok(())
}

/// `AUT.RUL.07`: the warranty ticket `ticket` was closed.
///
/// # Errors
///
/// The database's and the ticketing contract's errors.
pub async fn on_ticket_closed(state: &AppState, ticket: TicketId) -> Result {
    let Some(about) = task_creator(state).ticket_about(state, ticket).await? else {
        return Ok(());
    };
    if about.category_key != WARRANTY_CATEGORY || about.ticket.status != "closed" {
        return Ok(());
    }
    if SupplierClaim::of_ticket(&state.db, ticket.0)
        .await?
        .is_some()
    {
        return Ok(());
    }
    if about.subject.subject_type == JOB_CARD_SUBJECT_TYPE {
        let estimate = Estimate::load(&state.db, about.subject.subject_id).await?;
        let fitted = estimate
            .parts
            .iter()
            .any(|part| part.approval == LineStatus::Approved && part.fitted_milli > 0);
        if !fitted {
            return Ok(());
        }
    }
    let (title, body) = {
        let number = about.ticket.number.clone();
        let params: [(&str, &dyn std::fmt::Display); 1] = [("ticket", &number)];
        (
            english("auto.supplier.task_raise_title", &params),
            english("auto.supplier.task_raise_body", &params),
        )
    };
    task(
        state,
        about.subject.clone(),
        permissions::PURCHASER_ROLE,
        &title,
        &body,
        &format!("auto.supplier_claim_task:{}", ticket.0),
    )
    .await
}

/// The store of the workshop of the job card the claim is about, where the kept
/// part waits for the supplier.
async fn store_of(state: &AppState, claim: &SupplierClaim) -> Result<Option<Subject>> {
    let card_id = match claim.about() {
        Some(about) if about.subject_type == JOB_CARD_SUBJECT_TYPE => Some(about.subject_id),
        _ => claim.fitted_job_card_id,
    };
    let Some(card_id) = card_id else {
        return Ok(None);
    };
    let Some(card) = JobCard::find(&state.db, card_id).await? else {
        return Ok(None);
    };
    let Some(workshop) = card.workshop() else {
        return Ok(None);
    };
    stock_service(state)
        .store_of(state, &workshop.subject_type, workshop.subject_id)
        .await
}

/// What follows a claim that was raised: the claim document to the supplier
/// and the draft Return to supplier of the kept part.
///
/// # Errors
///
/// The database's and the inventory contract's errors; a notifier that refuses
/// the email gives the purchaser a task instead.
pub async fn after_raise(state: &AppState, claim_id: i64) -> Result {
    let claim = SupplierClaim::find_or_404(&state.db, claim_id).await?;
    let supplier_name = directory(state)
        .display_name(&state.db, PartyId(claim.supplier_party_id), "en")
        .await?
        .unwrap_or_default();
    let number = claim.number();
    let request = NotifyRequest::new(SUPPLIER_CLAIM_TYPE, claim.subject())
        .subject_label(&number)
        .to_party(PartyId(claim.supplier_party_id))
        .data(json!({
            "claim": number,
            "supplier": supplier_name,
            "ticket": claim.ticket_number,
            "part": claim.part_description,
            "item": claim.item_code.clone().unwrap_or_default(),
            "quantity": crate::number_text(claim.quantity_milli / 1_000),
            "amount": crate::money_text(claim.claim_amount),
            "purchased": claim.purchase_date.to_string(),
            "failed": claim.failure_date.to_string(),
        }));
    let queued = notifier(state).notify(state, request).await.is_ok();
    if queued {
        SupplierClaim::query()
            .where_(SupplierClaim::ID.eq(claim.id))
            .update(
                &state.db,
                &[
                    ("claim_sent_at", &Some(now())),
                    (
                        "claim_document",
                        &Some(format!("{SUPPLIER_CLAIM_TYPE}:{number}")),
                    ),
                    ("updated_at", &now()),
                ],
            )
            .await?;
    } else {
        let (title, body) = {
            let params: [(&str, &dyn std::fmt::Display); 1] = [("claim", &number)];
            (
                english("auto.supplier.task_send_title", &params),
                english("auto.supplier.task_send_body", &params),
            )
        };
        task(
            state,
            claim.subject(),
            permissions::PURCHASER_ROLE,
            &title,
            &body,
            &format!("auto.supplier_claim_send:{}", claim.id),
        )
        .await?;
    }
    if claim.part_kept {
        draft_return(state, &claim).await?;
    }
    Ok(())
}

/// Drafts the Return to supplier of the kept part, or gives the purchaser a task
/// to do it by hand when the part has no item code or the workshop has no store.
async fn draft_return(state: &AppState, claim: &SupplierClaim) -> Result {
    let drafted = match (&claim.item_code, store_of(state, claim).await?) {
        (Some(code), Some(location)) => {
            supplier_return_drafts(state)
                .draft_credit(
                    state,
                    DraftReturn {
                        supplier: PartyId(claim.supplier_party_id),
                        source: claim.subject(),
                        item_code: code.clone(),
                        quantity_milli: claim.quantity_milli,
                        reason: format!("{}: {}", claim.number(), claim.part_description),
                        location,
                    },
                )
                .await?
        }
        _ => None,
    };
    match drafted {
        Some(subject) => {
            SupplierClaim::query()
                .where_(SupplierClaim::ID.eq(claim.id))
                .update(
                    &state.db,
                    &[
                        ("supplier_return_id", &Some(subject.subject_id)),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
        }
        None => {
            let (title, body) = {
                let number = claim.number();
                let params: [(&str, &dyn std::fmt::Display); 1] = [("claim", &number)];
                (
                    english("auto.supplier.task_return_title", &params),
                    english("auto.supplier.task_return_body", &params),
                )
            };
            task(
                state,
                claim.subject(),
                permissions::PURCHASER_ROLE,
                &title,
                &body,
                &format!("auto.supplier_claim_return:{}", claim.id),
            )
            .await?;
        }
    }
    Ok(())
}

/// The Return of the claim was credited by hand with the reason *supplier
/// refused*: the claim stays raised and the purchaser gets a task to write it
/// off.
///
/// # Errors
///
/// The ticketing contract's errors.
pub async fn ask_to_write_off(state: &AppState, claim: &SupplierClaim) -> Result {
    let (title, body) = {
        let number = claim.number();
        let params: [(&str, &dyn std::fmt::Display); 1] = [("claim", &number)];
        (
            english("auto.supplier.task_write_off_title", &params),
            english("auto.supplier.task_write_off_body", &params),
        )
    };
    task(
        state,
        claim.subject(),
        permissions::PURCHASER_ROLE,
        &title,
        &body,
        &format!("auto.supplier_claim_write_off:{}", claim.id),
    )
    .await
}

/// `AUT.RUL.08` on `on`: tells the purchaser and the branch manager of every
/// claim still raised [`CHASE_DAYS`] days after it was raised, once. Answers how
/// many claims were chased.
///
/// # Errors
///
/// The database's and the ticketing contract's errors.
pub async fn chase(state: &AppState, on: NaiveDate) -> Result<usize> {
    let open = SupplierClaim::query()
        .where_(SupplierClaim::STATUS.eq(SupplierClaimStatus::Raised))
        .where_(SupplierClaim::OVERDUE_NOTIFIED_AT.is_null())
        .order_by(SupplierClaim::ID)
        .get(&state.db)
        .await?;
    let mut chased = 0;
    for claim in open {
        let Some(raised) = claim.raised_at else {
            continue;
        };
        if (on - dubai_date(raised)).num_days() < CHASE_DAYS {
            continue;
        }
        let (title, body) = {
            let number = claim.number();
            let params: [(&str, &dyn std::fmt::Display); 2] =
                [("claim", &number), ("days", &CHASE_DAYS)];
            (
                english("auto.supplier.chase_title", &params),
                english("auto.supplier.chase_body", &params),
            )
        };
        for (role, who) in [
            (permissions::PURCHASER_ROLE, "purchaser"),
            (permissions::BRANCH_MANAGER_ROLE, "manager"),
        ] {
            task(
                state,
                claim.subject(),
                role,
                &title,
                &body,
                &format!("auto.supplier_chase:{}:{who}", claim.id),
            )
            .await?;
        }
        SupplierClaim::query()
            .where_(SupplierClaim::ID.eq(claim.id))
            .update(
                &state.db,
                &[
                    ("overdue_notified_at", &Some(now())),
                    ("updated_at", &now()),
                ],
            )
            .await?;
        chased += 1;
    }
    Ok(chased)
}

/// Registers the notification type, the listener of a closed ticket, the daily
/// chase and its command.
pub(crate) fn register(app: &mut Registry) {
    notify::register_type(supplier_claim_type());
    app.listen(|event: TicketClosed, state: AppState| async move {
        on_ticket_closed(&state, event.ticket).await
    });
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            chase(&state, today()).await.map(|_| ())
        })
        .timezone(TASK_ZONE);
    app.command(
        TASK,
        "Tell the purchaser and the branch manager about supplier warranty claims not recovered in 60 days",
        |_args, state: AppState| async move {
            let chased = chase(&state, today()).await?;
            println!("{chased} supplier claims chased.");
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_claim_document_is_an_email_to_the_supplier() {
        let def = supplier_claim_type();
        assert_eq!(def.channels, [NotifyChannel::Email]);
        assert!(def.template_for(NotifyChannel::Email, "en").is_some());
        assert_eq!(def.staff_role.as_deref(), Some("procurement"));
    }
}
