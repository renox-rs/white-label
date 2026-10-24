//! The approvals waiting for a person (UX §2.1 "Needs your action", the
//! *Approvals* source): the pending requests whose current step the person
//! may decide, and a registry through which a module names the page of its
//! subject (the kernel does not know the modules' routes).

use std::collections::BTreeMap;
use std::sync::{PoisonError, RwLock};

use renox::prelude::*;
use serde::Serialize;

use super::notify;
use super::request::{ApprovalRequest, ApprovalStatus};
use crate::texts;

/// A module's page for a subject: its id to the URL.
pub type SubjectLink = fn(i64) -> String;

static LINKS: RwLock<BTreeMap<String, SubjectLink>> = RwLock::new(BTreeMap::new());

/// Registers the page of a module's subject type, e.g.
/// `register_subject_link("documents.block_override", |id| format!("/documents/overrides/{id}"))`.
/// A second registration of the same type replaces the first.
pub fn register_subject_link(subject_type: &str, link: SubjectLink) {
    LINKS
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(subject_type.to_owned(), link);
}

/// The page of the subject `subject_type:id`, when its module registered one.
pub fn subject_url(subject_type: &str, id: i64) -> Option<String> {
    let link = *LINKS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(subject_type)?;
    Some(link(id))
}

/// The pending requests whose current step `user` may decide, oldest first
/// (the step's approvers hold its role or permission in the subject's scope;
/// with SoD on, the requester is not one).
pub async fn awaiting(db: &Db, user: &User) -> Result<Vec<ApprovalRequest>> {
    let pending = ApprovalRequest::query()
        .where_(ApprovalRequest::STATUS.eq(ApprovalStatus::Pending))
        .order_by(ApprovalRequest::ID)
        .get(db)
        .await?;
    let mut mine = Vec::new();
    for request in pending {
        let approvers = notify::step_approvers(db, &request).await?;
        if approvers.iter().any(|approver| approver.id == user.id) {
            mine.push(request);
        }
    }
    Ok(mine)
}

/// One approval waiting for the user, as the dashboard lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AwaitingItem {
    /// The pending request.
    pub request_id: i64,
    /// What to decide, in the viewer's language.
    pub title: String,
    /// The record the request is about: its type.
    pub subject_type: String,
    /// The record's id.
    pub subject_id: i64,
    /// The subject's page, when its module registered one.
    pub url: Option<String>,
}

/// The actions the platform's modules ask approval for, in words.
const ACTION_LABELS: &[(&str, &str)] = &[
    ("accounting.resume", "Resume an engagement"),
    ("accounting.return_file", "File a return"),
    ("accounting.tax_profile_change", "Change a tax profile"),
    ("accounting.wip_write_off", "Write off work in progress"),
    (
        "auto.job_card_cancel",
        "Cancel a job card: a bill below the work done or a refused signature",
    ),
    (
        "campaigns.cost_over_budget",
        "Approve a campaign cost over its budget",
    ),
    ("clinics.claim_void", "Void a claim"),
    (
        "construction.deduction_accept",
        "Accept a deduction above the limit",
    ),
    (
        "construction.po_subcontractor_registration",
        "Order from a subcontractor without a valid registration",
    ),
    ("construction.project_terminate", "Terminate a project"),
    (
        "construction.retention_deduction_accept",
        "Accept a deduction from the retention above the limit",
    ),
    ("construction.variation_abandon", "Abandon a variation"),
    (
        "delivery.deliver_short",
        "Approve a delivery with a cash difference",
    ),
    ("delivery.discard", "Approve discarding returned goods"),
    ("delivery.mark_lost", "Mark a delivery as lost"),
    (
        "delivery.reverse_delivery",
        "Approve reversing a delivered order",
    ),
    ("documents.block_override", "Override a document block"),
    ("edtech.terminate", "Terminate a running batch"),
    (
        "edtech.withdraw",
        "Withdraw an enrolment and refund the student",
    ),
    ("fnb.batch_discard", "Approve discarding a production batch"),
    ("fnb.payout_void", "Void a payout statement"),
    ("hr.cancel_onboarding", "Cancel an onboarding"),
    (
        "hr.record_death",
        "Approve a non-standard exit: the death of a worker",
    ),
    ("hr.reinstate", "Reinstate a worker who left"),
    (
        "hr.report_absconding",
        "Approve a non-standard exit: a worker reported as absconded",
    ),
    ("hr.waive_cooling", "Waive the pay account cooling period"),
    ("hr.withdraw_notice", "Withdraw a resignation notice"),
    ("inventory.goods_receipt_post", "Post an over-receipt"),
    ("inventory.hold_discard", "Discard stock on hold"),
    ("inventory.hold_release", "Release stock on hold"),
    (
        "inventory.landed_cost_adjust",
        "Post a landed cost adjustment",
    ),
    (
        "inventory.pending_issue_clear",
        "Clear a pending stock issue",
    ),
    ("inventory.purchase_order_revise", "Revise a purchase order"),
    (
        "inventory.supplier_return_credit",
        "Credit a return to a supplier",
    ),
    (
        "inventory.supplier_return_dispatch",
        "Dispatch a return to a supplier",
    ),
    ("invoicing.budget_approval", "Approve the company budget"),
    ("invoicing.dispute_credit", "Credit a disputed invoice"),
    ("invoicing.owner_drawing", "Approve an owner drawing"),
    ("invoicing.payer_bill_void", "Void a payer bill"),
    ("invoicing.payer_money", "Approve a payer money movement"),
    ("invoicing.payer_write_off", "Write off a payer balance"),
    (
        "invoicing.rate_card_past_dated",
        "Approve a past-dated rate card",
    ),
    ("invoicing.void", "Void an invoice"),
    ("invoicing.write_off", "Write off an invoice"),
    ("kernel.late_entry", "Approve a late entry"),
    (
        "loyalty.adjustment_over_limit",
        "Adjust loyalty points above the limit",
    ),
    (
        "manpower.release",
        "Release a client contract from a credit hold",
    ),
    ("manpower.terminate", "Terminate a client contract"),
    (
        "notifications.broadcast_approval",
        "Send a broadcast to its audience",
    ),
    (
        "parties.block_exception",
        "Allow an exception for a blocked party",
    ),
    (
        "parties.confirm_match",
        "Confirm a possible screening match",
    ),
    ("payments.accept_chargeback", "Accept a chargeback"),
    (
        "payments.authorise_deduction",
        "Authorise a deduction from pay",
    ),
    ("payments.cash_write_off", "Write off a cash shortage"),
    ("payments.pay", "Pay out"),
    ("payments.refund", "Pay a refund"),
    ("payments.reverse", "Reverse a payment"),
    ("property.lease_cancel", "Cancel a lease"),
    ("property.lease_force_terminate", "End a lease by force"),
    ("property.owner_invoice", "Invoice an owner"),
    ("property.owner_payout", "Pay an owner"),
    ("sales.order_amend", "Approve a change to an order"),
    ("sales.order_cancel", "Approve cancelling an order"),
    (
        "sales.order_cancel_line",
        "Approve cancelling a line of an order",
    ),
    (
        "sales.order_cancel_remaining",
        "Approve cancelling the rest of an order",
    ),
    ("sales.return_refund", "Approve a refund after completion"),
    ("sales.void", "Void a sale"),
];

/// The action key of a request in words: the app's text
/// `wl_kernel.approvals.action_<key with underscores>` when it has one, else
/// the platform's. An action nobody gave words shows *Approval needed* and its
/// key goes to the log, never to the page.
pub fn action_label(lang: &Lang, action: &str) -> String {
    let key = format!("wl_kernel.approvals.action_{}", action.replace('.', "_"));
    let found = lang.t(&key, &[]);
    if found != key {
        return found;
    }
    match platform_label(action) {
        Some(label) => label.to_owned(),
        None => {
            eprintln!("approvals: the action `{action}` has no words; shown as `Approval needed`");
            texts::t(lang, "wl_kernel.approvals.action_unknown")
        }
    }
}

/// The platform's words for the approval action `action`, if it has any.
pub(crate) fn platform_label(action: &str) -> Option<&'static str> {
    ACTION_LABELS
        .iter()
        .find(|(known, _)| *known == action)
        .map(|(_, label)| *label)
}

/// Whether the platform has words for the approval action `action` (an app
/// may have its own, as [`action_label`] reads them first).
#[must_use]
pub fn action_is_labelled(action: &str) -> bool {
    ACTION_LABELS.iter().any(|(known, _)| *known == action)
}

/// The dashboard's approvals for `user` ([`awaiting`]), with the subject's
/// page from the registry.
pub async fn awaiting_items(db: &Db, user: &User, lang: &Lang) -> Result<Vec<AwaitingItem>> {
    Ok(awaiting(db, user)
        .await?
        .into_iter()
        .map(|request| AwaitingItem {
            request_id: request.id,
            title: texts::t_with(
                lang,
                "wl_kernel.approvals.awaiting.item",
                &[("action", &action_label(lang, &request.action))],
            ),
            url: subject_url(&request.subject_type, request.subject_id),
            subject_type: request.subject_type,
            subject_id: request.subject_id,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_is_labelled_once() {
        let mut keys: Vec<&str> = ACTION_LABELS.iter().map(|(key, _)| *key).collect();
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "an action is labelled twice");
    }

    #[test]
    fn the_manpower_and_hr_actions_have_words() {
        for action in ["manpower.release", "manpower.terminate", "hr.waive_cooling"] {
            assert!(
                ACTION_LABELS.iter().any(|(key, _)| *key == action),
                "{action} shows as a raw key"
            );
        }
    }

    #[test]
    fn a_registered_link_names_the_subjects_page() {
        register_subject_link("test.awaiting_thing", |id| format!("/things/{id}"));
        assert_eq!(
            subject_url("test.awaiting_thing", 7).as_deref(),
            Some("/things/7")
        );
        assert_eq!(subject_url("test.awaiting_unknown", 7), None);
    }
}
