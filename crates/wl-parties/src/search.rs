//! Parties in the global search and the Related panel (BL-AGT-04,
//! `PLT.SRC.01`, `PLT.REL.01`).
//!
//! A party is found by its name in English and Arabic, its code, its TRN and
//! its phone numbers and e-mail addresses (stored hashed, matched only on
//! the exact value). The Emirates ID of the spec is not indexed: the party
//! register keeps none (the identity documents are documents, found by their
//! own number).
//!
//! A party is global: it has no building or other scope, so only the holders
//! of `parties.view` in a global role find it.
//!
//! Its relations to the other modules' records (invoices, payments, leases,
//! tickets) are declared by those modules, which own the rows; its
//! documents come from the documents module's generic relation.

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::Subject;
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::search::{SearchDocument, Searchable, kinds};

use crate::contacts::{ContactPoint, ContactType};
use crate::details::CompanyDetails;
use crate::party::Party;
use crate::permissions;

/// Declares `parties.party` for search and keeps its document current when
/// its status moves.
pub(crate) fn register(app: &mut Registry) {
    Searchable::of::<Party>(PARTY_SUBJECT_TYPE)
        .permission(permissions::VIEW)
        .label("Parties")
        .link("/parties/{id}")
        .document_in(|tx, party| Box::pin(document_in(tx, party)))
        .register();
    // Block, unblock, archive...: the status on the result follows.
    app.listen(|event: Transitioned<Party>, state: AppState| async move {
        index(&state.db, event.key).await
    });
}

/// The search document of `party`, with its TRN and contact points read in
/// `tx`.
pub async fn document_in(tx: &mut Transaction, party: &Party) -> Result<SearchDocument> {
    let mut document = SearchDocument::new(party.display_name.clone())
        .secondary(party.code.clone())
        .keyword(party.legal_name_en.clone())
        .keyword(party.legal_name_ar.clone().unwrap_or_default())
        .identifier(kinds::NUMBER, party.code.clone())
        .status(party.status.as_str());
    let company = CompanyDetails::query()
        .where_eq("party_id", party.id)
        .first(&mut *tx)
        .await?;
    if let Some(trn) = company.and_then(|c| c.trn) {
        document = document.identifier(kinds::TRN, trn);
    }
    let contacts = ContactPoint::query()
        .where_eq("party_id", party.id)
        .order_by("id")
        .get(&mut *tx)
        .await?;
    for contact in contacts {
        document = match contact.contact_type {
            ContactType::Email => document.identifier(kinds::EMAIL, contact.value),
            ContactType::Mobile | ContactType::Landline => {
                document.identifier(kinds::PHONE, contact.value)
            }
            // An address is not an identifier and not searched.
            ContactType::Address => document,
        };
    }
    Ok(document)
}

/// Writes the document of party `id` again in its own transaction (after a
/// change that was not made in one, or after a transition committed).
pub async fn index(db: &Db, id: i64) -> Result {
    let mut tx = db.begin().await?;
    wl_kernel::search::index_in(&mut tx, &Subject::new(PARTY_SUBJECT_TYPE, id)).await?;
    tx.commit().await?;
    Ok(())
}
