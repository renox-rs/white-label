//! The vendor document flags (wl-ticketing §3, §4.6 *Vendor document
//! expired*, §5.6 step 7, issue #152): what follows when a vendor's document
//! that guards the categories of its service contracts expires, and when a
//! renewed one is accepted.
//!
//! **`DocumentExpired`** (from `wl-documents`, once, on the first nightly run
//! after the expiry date) for a vendor party whose `blocks` guard
//! `ticket.resolve` or `service_contract.activate` ([`crate::vendor_guard`]):
//! every `active` or `expiring` contract of that vendor with a covered
//! category the entries guard
//!
//! - carries the **vendor document flag** (with the kind that expired);
//! - has its open planned visits flagged (the vendor's, and the unassigned
//!   ones); `resolve` is refused on them by the guard itself;
//! - gets the compliance flag `vendor_document_expired` on each covered subject
//!   whose category is compliance-critical (one open per subject, category and
//!   reason: a repeated event raises nothing twice);
//! - tells the coordinator and the manager in scope, once.
//!
//! The vendor may still attend; a visit it completes is refused by the guard.
//! **Clearing.** Everything clears on **`DocumentAccepted`** for a renewed
//! document of that kind (valid today, [`accepted`]): the contract flag, the
//! visit flags and the compliance flags, cleared as "document renewed", unless
//! another flagged contract still covers the subject and category. Otherwise
//! the coordinator reassigns a visit (`assign` to someone else, which clears
//! *that visit's* flag, [`clear_on_reassign`]) or terminates the contract. A
//! subject's flag also clears when a replacement contract becomes `active`
//! ([`ServiceContracts::activate`](crate::ServiceContracts::activate)), and
//! when the contract ends `WRK.RUL.18` clears it and raises `no_contract`
//! instead. Nothing else clears a flag. A look-ahead (`DocumentExpiring`)
//! raises no flag.

use renox::prelude::*;
use renox::serde_json::{self, json};
use renox::state_machine::Transitioned;
use wl_kernel::documents::{BlockEntry, DocumentAccepted, DocumentExpired, today};
use wl_kernel::notify::{self, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::parties::{PARTY_SUBJECT_TYPE, directory};
use wl_kernel::tickets::ComplianceFlagReason;
use wl_kernel::PartyId;

use crate::assign::{ASSIGN, AUTO_ASSIGN, AssignKind, AssignMeta};
use crate::config::Category;
use crate::contract::{ContractStatus, ServiceContract};
use crate::flags::{self, DOCUMENT_RENEWED, Raise};
use crate::texts::english;
use crate::ticket::Ticket;
use crate::vendor_guard::{ACTIVATE, RESOLVE};
use crate::visit_check::{holders_in, settings};

/// The notice to the coordinator and the manager in scope.
pub const NOTIFICATION_TYPE: &str = "ticketing.vendor_document_expired";

/// The reassignment reason that goes with it.
pub const REASSIGN_REASON: &str = "vendor_document_expired";

/// What [`expired`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Flagged {
    /// The contracts that got the flag now (not those that had it).
    pub contracts: Vec<i64>,
    /// The visits flagged now.
    pub visits: usize,
    /// The compliance flags raised.
    pub flags: usize,
}

/// The notification type [`NOTIFICATION_TYPE`].
pub fn notification_type() -> NotificationTypeDef {
    let subject = english("wl_ticketing.notice.vendor_document_expired.subject").to_owned();
    let body = english("wl_ticketing.notice.vendor_document_expired.body").to_owned();
    NotificationTypeDef::new(NOTIFICATION_TYPE, "ticketing")
        .group("tickets")
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
}

/// The vendor of a document `subject`, when it is a party.
fn vendor_of(subject: &wl_kernel::Subject) -> Option<PartyId> {
    (subject.subject_type == PARTY_SUBJECT_TYPE).then(|| PartyId(subject.subject_id))
}

/// The entries of `blocks` that guard what this module reacts to.
fn guarding(blocks: &[BlockEntry]) -> Vec<&BlockEntry> {
    blocks
        .iter()
        .filter(|b| b.guarded == RESOLVE || b.guarded == ACTIVATE)
        .collect()
}

/// Flags the contracts, visits and subjects of a vendor whose document of
/// `event.kind` expired.
///
/// # Errors
///
/// The database's errors, or a notice that failed.
pub async fn expired(state: &AppState, event: &DocumentExpired) -> Result<Flagged> {
    let mut done = Flagged::default();
    let Some(vendor) = vendor_of(&event.subject) else {
        return Ok(done);
    };
    let entries = guarding(&event.blocks);
    if entries.is_empty() {
        return Ok(done);
    }
    let at = renox::db::now();
    for contract in ServiceContract::in_force_of(&state.db, vendor).await? {
        let mut guarded = Vec::new();
        for row in contract.coverage(&state.db).await? {
            let Some(category) = Category::find(&state.db, row.category_id).await? else {
                continue;
            };
            if entries.iter().any(|e| e.applies_to(Some(category.key.as_str()))) {
                guarded.push((row, category));
            }
        }
        if guarded.is_empty() {
            continue;
        }
        let newly = ServiceContract::query()
            .where_(ServiceContract::ID.eq(contract.id))
            .where_(ServiceContract::VENDOR_DOCUMENT_FLAG.eq(false))
            .update(
                &state.db,
                &[
                    ("vendor_document_flag", &true),
                    ("vendor_document_kind", &Some(event.kind.clone())),
                ],
            )
            .await?
            > 0;
        for visit in contract.visits(&state.db).await? {
            let theirs = visit.assignee_party_id.is_none_or(|party| party == vendor.0)
                && visit.assignee_user_id.is_none();
            if visit.is_open() && theirs && !visit.vendor_document_flag {
                Ticket::query()
                    .where_(Ticket::ID.eq(visit.id))
                    .update(&state.db, &[("vendor_document_flag", &true)])
                    .await?;
                done.visits += 1;
            }
        }
        for (row, category) in &guarded {
            let raised = flags::raise(
                state,
                Raise {
                    subject: row.subject(),
                    category: category.key.clone(),
                    reason: ComplianceFlagReason::VendorDocumentExpired,
                    contract: Some(contract.id),
                    ticket: None,
                },
                at,
            )
            .await?;
            done.flags += usize::from(raised.is_some());
        }
        if newly {
            done.contracts.push(contract.id);
            tell(state, &contract, event).await?;
        }
    }
    Ok(done)
}

/// Tells the coordinator and the manager in scope of the contract.
async fn tell(state: &AppState, contract: &ServiceContract, event: &DocumentExpired) -> Result<()> {
    let settings = settings(state);
    let subject = contract.subject();
    let mut recipients = holders_in(state, &subject, &settings.coordinator_role).await?;
    recipients.extend(holders_in(state, &subject, &settings.manager_role).await?);
    recipients.sort_unstable();
    recipients.dedup();
    let vendor = directory(state)
        .display_name(&state.db, contract.vendor(), "en")
        .await?
        .unwrap_or_else(|| format!("#{}", contract.vendor_party_id));
    for user in recipients {
        let request = NotifyRequest::new(NOTIFICATION_TYPE, subject.clone())
            .to_user(user)
            .data(json!({
                "contract": contract.number,
                "vendor": vendor,
                "kind": event.kind,
                "expires_on": event.expires_on.to_string(),
            }));
        notify::notifier(state).notify(state, request).await?;
    }
    Ok(())
}

/// Clears the flags of the contracts of the vendor whose document of
/// `event.kind` was renewed: the contract, its visits and its subjects'
/// compliance flags. Returns the contracts cleared. A document that is
/// already expired today is no renewal and clears nothing.
///
/// # Errors
///
/// The database's errors.
pub async fn accepted(state: &AppState, event: &DocumentAccepted) -> Result<Vec<i64>> {
    let Some(vendor) = vendor_of(&event.subject) else {
        return Ok(Vec::new());
    };
    if event.expires_on.is_some_and(|expires| expires < today()) {
        return Ok(Vec::new());
    }
    let flagged = ServiceContract::query()
        .where_(ServiceContract::VENDOR_PARTY_ID.eq(vendor.0))
        .where_(ServiceContract::VENDOR_DOCUMENT_FLAG.eq(true))
        .order_by(ServiceContract::ID)
        .get(&state.db)
        .await?;
    let contracts: Vec<ServiceContract> = flagged
        .into_iter()
        .filter(|c| {
            c.vendor_document_kind
                .as_deref()
                .is_none_or(|kind| kind == event.kind)
        })
        .collect();
    let at = renox::db::now();
    for contract in &contracts {
        ServiceContract::query()
            .where_(ServiceContract::ID.eq(contract.id))
            .update(
                &state.db,
                &[
                    ("vendor_document_flag", &false),
                    ("vendor_document_kind", &None::<String>),
                ],
            )
            .await?;
        Ticket::query()
            .where_(Ticket::CONTRACT_ID.eq(contract.id))
            .where_(Ticket::VENDOR_DOCUMENT_FLAG.eq(true))
            .update(&state.db, &[("vendor_document_flag", &false)])
            .await?;
    }
    let cleared: Vec<i64> = contracts.iter().map(|c| c.id).collect();
    for contract in &contracts {
        for row in contract.coverage(&state.db).await? {
            // Another contract that is still flagged keeps the subject's flag.
            if still_flagged(state, &row.subject(), row.category_id).await? {
                continue;
            }
            flags::clear_matching(
                state,
                &row.subject(),
                row.category_id,
                &[ComplianceFlagReason::VendorDocumentExpired],
                DOCUMENT_RENEWED,
                at,
            )
            .await?;
        }
    }
    Ok(cleared)
}

/// Whether a contract that still carries the vendor document flag covers
/// `subject` for the category.
async fn still_flagged(
    state: &AppState,
    subject: &wl_kernel::Subject,
    category_id: i64,
) -> Result<bool> {
    let rows = crate::contract::Coverage::query()
        .where_(crate::contract::Coverage::SUBJECT_TYPE.eq(subject.subject_type.as_str()))
        .where_(crate::contract::Coverage::SUBJECT_ID.eq(subject.subject_id))
        .where_(crate::contract::Coverage::CATEGORY_ID.eq(category_id))
        .get(&state.db)
        .await?;
    for row in rows {
        if let Some(contract) = ServiceContract::find(&state.db, row.contract_id).await?
            && contract.vendor_document_flag
            && matches!(
                contract.status,
                ContractStatus::Active | ContractStatus::Expiring
            )
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Clears the flag of a visit that was reassigned to someone else.
///
/// # Errors
///
/// The database's errors.
pub async fn clear_on_reassign(state: &AppState, ticket: i64) -> Result<()> {
    Ticket::query()
        .where_(Ticket::ID.eq(ticket))
        .where_(Ticket::VENDOR_DOCUMENT_FLAG.eq(true))
        .update(&state.db, &[("vendor_document_flag", &false)])
        .await?;
    Ok(())
}

/// Registers the notice and the listeners for `DocumentExpired`,
/// `DocumentAccepted` and a reassignment.
pub(crate) fn register(app: &mut Registry) {
    notify::register_type(notification_type());
    app.listen(|event: DocumentExpired, state: AppState| async move {
        expired(&state, &event).await.map(|_| ())
    });
    app.listen(|event: DocumentAccepted, state: AppState| async move {
        accepted(&state, &event).await.map(|_| ())
    });
    app.listen(|event: Transitioned<Ticket>, state: AppState| async move {
        if event.transition != ASSIGN && event.transition != AUTO_ASSIGN {
            return Ok(());
        }
        let Ok(meta) = serde_json::from_value::<AssignMeta>(event.meta.clone()) else {
            return Ok(());
        };
        if meta.kind() == Some(AssignKind::Reassign) {
            clear_on_reassign(&state, event.key).await?;
        }
        Ok(())
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::documents::{BlockMode, OverrideMode};

    fn entry(guarded: &str) -> BlockEntry {
        BlockEntry {
            kind: "civil_defence_approval".into(),
            guarded: guarded.into(),
            mode: BlockMode::Block,
            override_mode: OverrideMode::Never,
            condition: None,
            locked: true,
        }
    }

    #[test]
    fn only_the_entries_of_resolve_and_activate_count() {
        let blocks = vec![
            entry("ticket.resolve"),
            entry("deployment.activate"),
            entry("service_contract.activate"),
        ];
        let kept: Vec<_> = guarding(&blocks).iter().map(|e| e.guarded.as_str()).collect();
        assert_eq!(kept, ["ticket.resolve", "service_contract.activate"]);
    }

    #[test]
    fn only_a_party_has_a_vendor() {
        assert_eq!(
            vendor_of(&wl_kernel::Subject::new("parties.party", 7)),
            Some(PartyId(7))
        );
        assert_eq!(vendor_of(&wl_kernel::Subject::new("hr.employee", 7)), None);
    }

    #[test]
    fn the_notice_has_an_english_text() {
        let def = notification_type();
        let template = def
            .template_for(NotifyChannel::Email, "en")
            .expect("a text");
        assert!(!template.body.starts_with("wl_ticketing."));
        assert_eq!(REASSIGN_REASON, "vendor_document_expired");
    }
}
