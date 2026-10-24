//! The vertical's notification types and who is told (01 Property §7.2),
//! sent through the kernel [`Notifier`](wl_kernel::notify::Notifier). Types
//! that go only to staff are registered with the reaction `none`: the item
//! stays on the user's task list when a message gives up.

use std::collections::BTreeSet;

use renox::auth::permissions::users_with_role_in;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::document_store::WithAttachments;
use wl_kernel::notify::{
    self, Category, GROUP_INVOICES, GROUP_PORTAL, GROUP_SIGNATURES, GROUP_STATEMENTS,
    GiveUpReaction, NotificationTypeDef, NotifyChannel, NotifyRequest,
};

use crate::lease::Lease;
use crate::permissions::{ACCOUNTANT_ROLE, LEASING_AGENT_ROLE, PROPERTY_MANAGER_ROLE};
use crate::texts::english;

/// The leasing agent and the property manager are reminded that a lease is
/// still waiting for its signature (`PRP.RUL.12`, at N days).
pub const LEASE_UNSIGNED: &str = "property.lease_unsigned";

/// The property manager is told that the lease entered its renewal window
/// (`PRP.RUL.01`).
pub const RENEWAL_DUE: &str = "property.renewal_due";

/// The property manager is warned that the notice-of-changed-terms deadline
/// is near and the renewal decision or the notice is missing (`PRP.RUL.09`,
/// 14 and 3 days before).
pub const RENEWAL_DEADLINE: &str = "property.renewal_deadline";

/// The property manager is told that the deadline passed and the lease can
/// now be renewed on the current terms only (`PRP.RUL.09`).
pub const CURRENT_TERMS_ONLY: &str = "property.current_terms_only";

/// The property managers are told that the tenant holds over after the end
/// date (`PRP.RUL.03`).
pub const HOLDING_OVER: &str = "property.holding_over";

/// The tenant is told that the lease entered its renewal window: a normal
/// message, not a statutory notice (`PRP.RUL.01`).
pub const RENEWAL_WINDOW: &str = "lease.renewal_window";

/// The statutory notice of changed terms to the tenant (`PRP.RUL.09`,
/// `PRP.ACL.17`). The property manager sends it; its delivery is the
/// provider's receipt or the proof staff record when it gave up.
pub const NOTICE_OF_CHANGED_TERMS: &str = "lease.notice_of_changed_terms";

/// The statutory notice of a landlord who ends the lease (`record_notice`
/// with party landlord). Its delivery is the provider's receipt or the proof
/// staff record when it gave up; the planned move-out date and the deadlines
/// that depend on the notice start from it.
pub const LANDLORD_NOTICE: &str = "lease.landlord_notice";

/// The accountant is told that the dispute window of the published deposit
/// deductions ended with no dispute, so the settlement may proceed
/// (`PRP.RUL.11`).
pub const DEPOSIT_READY: &str = "property.deposit_ready";

/// The property manager's manager is told that a deductions dispute was left
/// unanswered (`PRP.RUL.11`).
pub const DEPOSIT_DISPUTE_UNANSWERED: &str = "property.deposit_dispute_unanswered";

/// The statutory formal payment demand to the tenant (`PRP.RUL.10`). The
/// property manager sends it from the task; the M-day clock of the legal-case
/// task starts at its proof of delivery, never at sending.
pub const PAYMENT_DEMAND: &str = "lease.payment_demand";

/// The property managers are told that a terminated lease still has its
/// Ejari or Tawtheeq registration to cancel (`PRP.RUL.13`).
pub const EJARI_CANCELLATION: &str = "property.ejari_cancellation";

/// The accountants are told that the draft owner statements of a month were
/// generated (`PRP.RUL.05`).
pub const OWNER_STATEMENT_DRAFT: &str = "property.owner_statement_draft";

/// The preparer of an owner statement is told that the property manager
/// returned it with a reason.
pub const OWNER_STATEMENT_RETURNED: &str = "property.owner_statement_returned";

/// The owner receives a published statement (the e-mail carries the statement;
/// the PDF comes with `DOC.GEN.01`).
pub const OWNER_STATEMENT: &str = "owner_statement.published";

/// The owner is told that a corrected statement follows a revision.
pub const OWNER_STATEMENT_REVISED: &str = "owner_statement.revision_follows";

/// The owner is told how the dispute of a statement line was answered.
pub const OWNER_STATEMENT_DISPUTE_OUTCOME: &str = "owner_statement.dispute_outcome";

/// The property managers and accountants are told that notice was given on a
/// management agreement and the handover pack is started (BL-PRP-22).
pub const AGREEMENT_NOTICE: &str = "property.agreement_notice";

/// The tenant is told who the new payee is after the management agreement
/// ended or the unit left it: a normal message (BL-PRP-22, BL-PRP-23).
pub const PAYEE_CHANGED: &str = "lease.new_payee";

/// The tenant receives the contract to sign (a portal invitation or signature
/// request, §7.2 *Notification types*). Sent with the e-signature of the
/// contract (`DOC.GEN.01`, later); the type is registered now so its give-up
/// reaction is the group's.
pub const CONTRACT_FOR_SIGNATURE: &str = "lease.contract_for_signature";

/// The tenant is told that the notice to end the lease was withdrawn
/// (`PRP.ACL.20`).
pub const NOTICE_WITHDRAWN: &str = "lease.notice_withdrawn";

/// The company admin is told that a compliance certificate of a building
/// expired and the building is flagged non-compliant (BL-PRP-25).
pub const CERTIFICATE_EXPIRED: &str = "property.certificate_expired";

/// An e-mail reminder type of the vertical with English texts.
fn english_reminder(key: &str, texts: &str) -> NotificationTypeDef {
    let subject = english(&format!("property.notices.{texts}_subject"), &[]);
    let body = english(&format!("property.notices.{texts}_body"), &[]);
    NotificationTypeDef::new(key, "property")
        .group("property")
        .category(Category::Reminder)
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
}

/// A type that goes only to staff: the item stays on the user's task list when
/// a message gives up, so the reaction is `none` (§7.2 *Notification types*).
fn staff_reminder(key: &str, texts: &str) -> NotificationTypeDef {
    english_reminder(key, texts).give_up(&[GiveUpReaction::None])
}

/// A message to a party with English texts, in the notification group `group`
/// of wl-notify §5.4. `staff_role` is the role that gets the task when the
/// group's reaction ends in a `staff_task`: the property manager for the
/// portal and signature groups and the statutory notices (the request's owner
/// is the lease's property manager), the accountant for invoices and
/// statements (the collector).
fn party_message(key: &str, texts: &str, group: &str, staff_role: &str) -> NotificationTypeDef {
    let subject = english(&format!("property.notices.{texts}_subject"), &[]);
    let body = english(&format!("property.notices.{texts}_body"), &[]);
    NotificationTypeDef::new(key, "property")
        .group(group)
        .staff_role(staff_role)
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
}

/// A message to the tenant with English texts; `statutory` for a notice with
/// legal effect, whose give-up the platform handles with the proof task for
/// the property manager (`statutory_notice`, `NTF.RUL.01`).
fn tenant_message(key: &str, texts: &str, statutory: bool) -> NotificationTypeDef {
    let def = party_message(key, texts, GROUP_PORTAL, PROPERTY_MANAGER_ROLE);
    if statutory {
        def.group("statutory").statutory(true)
    } else {
        def
    }
}

/// Registers the vertical's notification types with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(staff_reminder(LEASE_UNSIGNED, "lease_unsigned"));
    notify::register_type(staff_reminder(RENEWAL_DUE, "renewal_due"));
    notify::register_type(staff_reminder(RENEWAL_DEADLINE, "renewal_deadline"));
    notify::register_type(staff_reminder(CURRENT_TERMS_ONLY, "current_terms_only"));
    notify::register_type(staff_reminder(HOLDING_OVER, "holding_over"));
    notify::register_type(tenant_message(RENEWAL_WINDOW, "renewal_window", false));
    notify::register_type(party_message(
        CONTRACT_FOR_SIGNATURE,
        "contract_for_signature",
        GROUP_SIGNATURES,
        PROPERTY_MANAGER_ROLE,
    ));
    notify::register_type(tenant_message(NOTICE_WITHDRAWN, "notice_withdrawn", false));
    notify::register_type(tenant_message(
        NOTICE_OF_CHANGED_TERMS,
        "changed_terms",
        true,
    ));
    notify::register_type(tenant_message(LANDLORD_NOTICE, "landlord_notice", true));
    notify::register_type(staff_reminder(EJARI_CANCELLATION, "ejari_cancellation"));
    notify::register_type(tenant_message(PAYMENT_DEMAND, "payment_demand", true));
    notify::register_type(staff_reminder(DEPOSIT_READY, "deposit_ready"));
    notify::register_type(staff_reminder(
        DEPOSIT_DISPUTE_UNANSWERED,
        "deposit_dispute_unanswered",
    ));
    notify::register_type(staff_reminder(AGREEMENT_NOTICE, "agreement_notice"));
    notify::register_type(staff_reminder(CERTIFICATE_EXPIRED, "certificate_expired"));
    notify::register_type(party_message(
        PAYEE_CHANGED,
        "payee_changed",
        GROUP_INVOICES,
        ACCOUNTANT_ROLE,
    ));
    notify::register_type(staff_reminder(
        OWNER_STATEMENT_DRAFT,
        "owner_statement_draft",
    ));
    notify::register_type(staff_reminder(
        OWNER_STATEMENT_RETURNED,
        "owner_statement_returned",
    ));
    notify::register_type(party_message(
        OWNER_STATEMENT,
        "owner_statement",
        GROUP_STATEMENTS,
        ACCOUNTANT_ROLE,
    ));
    notify::register_type(party_message(
        OWNER_STATEMENT_REVISED,
        "owner_statement_revised",
        GROUP_STATEMENTS,
        ACCOUNTANT_ROLE,
    ));
    notify::register_type(party_message(
        OWNER_STATEMENT_DISPUTE_OUTCOME,
        "owner_statement_dispute_outcome",
        GROUP_STATEMENTS,
        ACCOUNTANT_ROLE,
    ));
}

/// The users who hold `role` in the building of `lease`, or everywhere.
async fn holders(state: &AppState, role: &str, lease: &Lease) -> Result<Vec<i64>> {
    Ok(users_with_role_in(&state.db, role, &lease.scope())
        .await?
        .iter()
        .map(|user| user.id)
        .collect())
}

/// The leasing agent of `lease` (who drafted it, else the holders of the
/// leasing agent role in its building) and its property managers.
pub async fn leasing_team(state: &AppState, lease: &Lease) -> Result<Vec<i64>> {
    let mut ids = BTreeSet::new();
    match lease.created_by {
        Some(agent) => {
            ids.insert(agent);
        }
        None => ids.extend(holders(state, LEASING_AGENT_ROLE, lease).await?),
    }
    ids.extend(holders(state, PROPERTY_MANAGER_ROLE, lease).await?);
    Ok(ids.into_iter().collect())
}

/// Reminds the leasing agent and the property managers that `lease` has been
/// waiting `days` days for its signature. Nothing is sent to the tenant.
pub async fn lease_unsigned(
    state: &AppState,
    lease: &Lease,
    days: i64,
    since: NaiveDate,
) -> Result<Vec<i64>> {
    let users = leasing_team(state, lease).await?;
    if users.is_empty() {
        return Ok(users);
    }
    let mut request = NotifyRequest::new(LEASE_UNSIGNED, lease.subject())
        .subject_label(&lease.number_with_version())
        .data(json!({
            "lease": lease.number_with_version(),
            "days": days,
            "since": since.format("%-d %b %Y").to_string(),
        }));
    for user in &users {
        request = request.to_user(*user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(users)
}

/// The users who hold the property manager role in the building of `lease`,
/// or everywhere.
pub async fn managers(state: &AppState, lease: &Lease) -> Result<Vec<i64>> {
    holders(state, PROPERTY_MANAGER_ROLE, lease).await
}

/// Tells the property managers of `lease` with a message of `type_key`; no
/// message when nobody holds the role. Returns who was told.
async fn tell_managers(
    state: &AppState,
    lease: &Lease,
    type_key: &str,
    data: renox::serde_json::Value,
) -> Result<Vec<i64>> {
    let users = managers(state, lease).await?;
    if users.is_empty() {
        return Ok(users);
    }
    let mut request = NotifyRequest::new(type_key, lease.subject())
        .subject_label(&lease.number_with_version())
        .data(data);
    for user in &users {
        request = request.to_user(*user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(users)
}

/// A date as the messages show it: `2 Oct 2027`.
fn shown(day: NaiveDate) -> String {
    day.format("%-d %b %Y").to_string()
}

/// `lease` entered its renewal window (`PRP.RUL.01`): the property managers
/// are told with the notice deadline, and the tenant gets a normal message.
pub async fn renewal_due(state: &AppState, lease: &Lease, deadline: NaiveDate) -> Result {
    let data = json!({
        "lease": lease.number_with_version(),
        "end_date": shown(lease.end_date),
        "deadline": shown(deadline),
    });
    tell_managers(state, lease, RENEWAL_DUE, data.clone()).await?;
    // An open legal case: the tenant gets no automatic message.
    if crate::legal_case::has_open(&state.db, lease.root_id).await? {
        return Ok(());
    }
    let request = NotifyRequest::new(RENEWAL_WINDOW, lease.subject())
        .subject_label(&lease.number_with_version())
        .to_party(lease.tenant())
        .data(data);
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Warns the property managers that the notice deadline of `lease` is `days`
/// days away and `problem` is not settled (`PRP.RUL.09`).
pub async fn renewal_deadline(
    state: &AppState,
    lease: &Lease,
    days: i64,
    deadline: NaiveDate,
    problem: &str,
) -> Result {
    let data = json!({
        "lease": lease.number_with_version(),
        "days": days,
        "deadline": shown(deadline),
        "problem": problem,
    });
    tell_managers(state, lease, RENEWAL_DEADLINE, data).await?;
    Ok(())
}

/// Tells the property managers that the deadline of `lease` passed without a
/// timely notice, so only the current terms can be renewed (`PRP.RUL.09`).
pub async fn current_terms_only(state: &AppState, lease: &Lease, deadline: NaiveDate) -> Result {
    let data = json!({
        "lease": lease.number_with_version(),
        "deadline": shown(deadline),
    });
    tell_managers(state, lease, CURRENT_TERMS_ONLY, data).await?;
    Ok(())
}

/// Sends the statutory notice of changed terms of `lease` to the tenant.
pub async fn changed_terms(state: &AppState, lease: &Lease, deadline: NaiveDate) -> Result {
    let request = NotifyRequest::new(NOTICE_OF_CHANGED_TERMS, lease.subject())
        .subject_label(&lease.number_with_version())
        .to_party(lease.tenant())
        .data(json!({
            "lease": lease.number_with_version(),
            "end_date": shown(lease.end_date),
            "deadline": shown(deadline),
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Sends the statutory notice of the landlord to end `lease` to the tenant:
/// `notice_date` is the date of the notice and `ground` the landlord's ground
/// as text.
pub async fn landlord_notice(
    state: &AppState,
    lease: &Lease,
    notice_date: NaiveDate,
    ground: &str,
) -> Result {
    let request = NotifyRequest::new(LANDLORD_NOTICE, lease.subject())
        .subject_label(&lease.number_with_version())
        .to_party(lease.tenant())
        .data(json!({
            "lease": lease.number_with_version(),
            "end_date": shown(lease.end_date),
            "notice_date": shown(notice_date),
            "ground": ground,
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the property managers that the Ejari or Tawtheeq registration of the
/// terminated `lease` is not cancelled yet (`PRP.RUL.13`). Returns who was
/// told.
pub async fn ejari_cancellation(state: &AppState, lease: &Lease) -> Result<Vec<i64>> {
    let data = json!({
        "lease": lease.number_with_version(),
        "ejari_number": lease.ejari_number.clone().unwrap_or_default(),
        "terminated_on": lease.terminated_on.map(shown).unwrap_or_default(),
    });
    tell_managers(state, lease, EJARI_CANCELLATION, data).await
}

/// Tells the property managers that `lease` is holding over until the end of
/// its statutory renewal period (`PRP.RUL.03`).
pub async fn holding_over(state: &AppState, lease: &Lease) -> Result {
    let data = json!({
        "lease": lease.number_with_version(),
        "end_date": shown(lease.end_date),
        "until": lease.holding_over_until.map(shown).unwrap_or_default(),
    });
    tell_managers(state, lease, HOLDING_OVER, data).await?;
    Ok(())
}

/// Sends the formal payment demand `demand` to the tenant of `lease` as a
/// statutory notice listing the overdue invoices in `rows`, with the demand
/// period of `days` days.
pub async fn payment_demand(
    state: &AppState,
    lease: &Lease,
    demand: &crate::arrears::ArrearsDemand,
    rows: &[crate::arrears::DemandInvoice],
    days: i64,
) -> Result {
    let request = NotifyRequest::new(PAYMENT_DEMAND, demand.subject())
        .subject_label(&lease.number_with_version())
        .to_party(lease.tenant())
        .data(crate::arrears::notice_data(lease, rows, days));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the accountants of the building of `lease` that the dispute window of
/// version `version` of the deductions, published on `published_on`, ended
/// with no dispute (`PRP.RUL.11`). Returns who was told.
pub async fn deposit_may_proceed(
    state: &AppState,
    lease: &Lease,
    version: i32,
    published_on: NaiveDate,
) -> Result<Vec<i64>> {
    let users = holders(state, ACCOUNTANT_ROLE, lease).await?;
    if users.is_empty() {
        return Ok(users);
    }
    let mut request = NotifyRequest::new(DEPOSIT_READY, lease.subject())
        .subject_label(&lease.number())
        .data(json!({
            "lease": lease.number(),
            "version": version,
            "published_on": shown(published_on),
        }));
    for user in &users {
        request = request.to_user(*user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(users)
}

/// Tells `users` (the property manager's manager) that the dispute of the
/// deposit deductions of `lease`, opened on `opened_on`, has had no answer for
/// `days` days (`PRP.RUL.11`).
pub async fn dispute_unanswered(
    state: &AppState,
    lease: &Lease,
    users: &[i64],
    opened_on: NaiveDate,
    days: i64,
) -> Result {
    if users.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(DEPOSIT_DISPUTE_UNANSWERED, lease.subject())
        .subject_label(&lease.number())
        .data(json!({
            "lease": lease.number(),
            "opened_on": shown(opened_on),
            "days": days,
        }));
    for user in users {
        request = request.to_user(*user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells `users` (the accountants) that the draft statement `statement` was
/// generated (`PRP.RUL.05`).
pub async fn statement_drafted(
    state: &AppState,
    statement: &crate::statement::OwnerStatement,
    users: &[i64],
) -> Result {
    if users.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(OWNER_STATEMENT_DRAFT, statement.subject())
        .subject_label(&statement.number())
        .data(json!({
            "statement": statement.number(),
            "period": statement_period(statement),
            "balance": crate::money_text(statement.balance()),
        }));
    for user in users {
        request = request.to_user(*user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells `users` (the preparer) that `statement` was returned with `reason`.
pub async fn statement_returned(
    state: &AppState,
    statement: &crate::statement::OwnerStatement,
    users: &[i64],
    reason: &str,
) -> Result {
    if users.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(OWNER_STATEMENT_RETURNED, statement.subject())
        .subject_label(&statement.number())
        .data(json!({
            "statement": statement.number(),
            "period": statement_period(statement),
            "reason": reason,
        }));
    for user in users {
        request = request.to_user(*user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// `3 Jan 2027 - 31 Jan 2027`: the period of a statement as messages show it.
fn statement_period(statement: &crate::statement::OwnerStatement) -> String {
    format!(
        "{} - {}",
        shown(statement.period_start),
        shown(statement.period_end)
    )
}

/// Sends the tenant of the submitted `lease` its generated `contract` for
/// signature (BL-PRP-31), attached to the e-mail (BL-NTF-10).
pub async fn contract_for_signature(
    state: &AppState,
    lease: &Lease,
    contract: &crate::generated::GeneratedDocument,
) -> Result {
    let request = NotifyRequest::new(CONTRACT_FOR_SIGNATURE, lease.subject())
        .subject_label(&lease.number_with_version())
        .to_party(lease.tenant())
        .data(json!({
            "lease": lease.number_with_version(),
            "contract": contract.title,
            "document": contract.document_id,
        }))
        .attach_document(
            wl_kernel::documents::DocumentId(contract.document_id),
            &format!("{}.pdf", contract.title),
        );
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Sends the published `statement` to its owner, with the lines `lines` as
/// text and its generated PDF, when there is one (BL-PRP-31).
pub async fn statement_published(
    state: &AppState,
    statement: &crate::statement::OwnerStatement,
    lines: &str,
    pdf: Option<&crate::generated::GeneratedDocument>,
) -> Result {
    let request = NotifyRequest::new(OWNER_STATEMENT, statement.subject())
        .subject_label(&statement.number())
        .to_party(wl_kernel::PartyId(statement.owner_party_id))
        .data(json!({
            "statement": statement.number(),
            "period": statement_period(statement),
            "brought_forward": crate::money_text(statement.brought_forward),
            "collected": crate::money_text(statement.collected),
            "expenses": crate::money_text(statement.expenses),
            "fee": crate::money_text(statement.fee),
            "adjustments": crate::money_text(statement.adjustments),
            "balance": crate::money_text(statement.balance()),
            "lines": lines,
            "document": pdf.map(|pdf| pdf.document_id),
        }));
    let request = match pdf {
        Some(pdf) => request.attach_document(
            wl_kernel::documents::DocumentId(pdf.document_id),
            &format!("{}.pdf", pdf.title),
        ),
        None => request,
    };
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the owner that the statement `statement` is replaced by a corrected
/// one, revision `revision`.
pub async fn statement_revised(
    state: &AppState,
    statement: &crate::statement::OwnerStatement,
    revision: i32,
) -> Result {
    let request = NotifyRequest::new(OWNER_STATEMENT_REVISED, statement.subject())
        .subject_label(&statement.number())
        .to_party(wl_kernel::PartyId(statement.owner_party_id))
        .data(json!({
            "statement": statement.number(),
            "period": statement_period(statement),
            "revision": revision,
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the owner of `statement` how the dispute of one of its lines was
/// answered: `outcome` in words.
pub async fn dispute_outcome(
    state: &AppState,
    statement: &crate::statement::OwnerStatement,
    outcome: &str,
) -> Result {
    let request = NotifyRequest::new(OWNER_STATEMENT_DISPUTE_OUTCOME, statement.subject())
        .subject_label(&statement.number())
        .to_party(wl_kernel::PartyId(statement.owner_party_id))
        .data(json!({
            "statement": statement.number(),
            "period": statement_period(statement),
            "outcome": outcome,
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the property managers and the accountants that notice was given on
/// `agreement` and the handover pack is started (BL-PRP-22).
pub async fn agreement_notice(
    state: &AppState,
    agreement: &crate::agreement::ManagementAgreement,
) -> Result {
    let mut users: BTreeSet<i64> = BTreeSet::new();
    for role in [PROPERTY_MANAGER_ROLE, ACCOUNTANT_ROLE] {
        for user in renox::auth::permissions::users_with_role(&state.db, role).await? {
            users.insert(user.id);
        }
    }
    if users.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(AGREEMENT_NOTICE, agreement.subject())
        .subject_label(&agreement.number())
        .data(json!({
            "agreement": agreement.number(),
            "party": agreement.notice_party.clone().unwrap_or_default(),
            "notice_on": agreement.notice_on.map(shown).unwrap_or_default(),
            "handover": agreement.handover_date.map(shown).unwrap_or_default(),
        }));
    for user in users {
        request = request.to_user(user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the tenant of `lease` that `payee` is paid from the day it was handed
/// over (BL-PRP-22, BL-PRP-23).
pub async fn payee_changed(state: &AppState, lease: &Lease, payee: &str) -> Result {
    let request = NotifyRequest::new(PAYEE_CHANGED, lease.subject())
        .subject_label(&lease.number_with_version())
        .to_party(lease.tenant())
        .data(json!({
            "lease": lease.number(),
            "payee": payee,
            "handover": lease.handed_over_on.map(shown).unwrap_or_default(),
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells `users` (the company admin) that the certificate `kind` of
/// `building`, valid through `expired_on`, expired (BL-PRP-25).
pub async fn certificate_expired(
    state: &AppState,
    building: &crate::building::Building,
    kind: &str,
    expired_on: NaiveDate,
    users: &[i64],
) -> Result {
    if users.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(
        CERTIFICATE_EXPIRED,
        wl_kernel::Subject::new(crate::building::BUILDING_SUBJECT_TYPE, building.id),
    )
    .subject_label(&building.code)
    .data(json!({
        "building": building.name_en.clone(),
        "kind": kind,
        "expired_on": shown(expired_on),
    }));
    for user in users {
        request = request.to_user(*user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}
