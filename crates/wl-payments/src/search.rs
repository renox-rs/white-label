//! Payments, cheques, deposits and refunds in the global search and the
//! Related panel (BL-AGT-04, `PLT.SRC.01`, `PLT.REL.01`).
//!
//! | Type | Title | Line | Identifiers |
//! |---|---|---|---|
//! | `payments.payment` | receipt or voucher number | payer | number, cheque number |
//! | `payments.cheque` | cheque number | drawer · bank | cheque number |
//! | `payments.deposit` | "Deposit #id" | party · what it is held for | — |
//! | `payments.refund` | "Refund #id" | party · what it returns | — |
//!
//! Cheque numbers repeat across banks: the identifier is the number alone
//! and the line shows the bank, so two cheques of one number are told apart
//! in the results. **Never indexed:** the bank account (free text that may
//! hold an IBAN), the card slip and the other method details but the cheque
//! number, a refund's payee details (an IBAN) and references.
//!
//! **Scope** (as `crate::scoped` lists them): a cheque or a deposit is in its
//! agreement's scope (a lease: its building); a payment in the scope of what
//! it was prepaid for, else of the first invoice it is allocated to, else of
//! the deposit it paid; a refund in its source's. The scope is read in the
//! indexing transaction: from the record's kernel machine when it has one
//! with a scope (a lease), else from its search document.
//!
//! The relations to a lease are declared by the vertical that owns leases.

use renox::auth::permissions::Scope;
use renox::db::{DbValue, Transaction, sql};
use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::Subject;
use wl_kernel::cheques::CHEQUE_SUBJECT_TYPE;
use wl_kernel::invoicing::INVOICE_SUBJECT_TYPE;
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::payments::{
    DEPOSIT_SUBJECT_TYPE, PAYMENT_SUBJECT_TYPE, PaymentMethod, REFUND_SUBJECT_TYPE,
};
use wl_kernel::related::Relation;
use wl_kernel::search::{SearchDocument, SearchDocumentRow, Searchable, kinds};

use crate::allocation::Allocation;
use crate::cheque::Cheque;
use crate::deposit::Deposit;
use crate::payment::Payment;
use crate::permissions;
use crate::refund::Refund;

/// The permission to see invoices, as the invoicing module names it.
const INVOICES_VIEW: &str = "invoices.view";

/// The permission to see parties, as the parties module names it.
const PARTIES_VIEW: &str = "parties.view";

/// Declares the four types for search, their relations, and keeps their
/// documents current after each transition.
pub(crate) fn register(app: &mut Registry) {
    Searchable::of::<Payment>(PAYMENT_SUBJECT_TYPE)
        .permission(permissions::VIEW)
        .label("Payments")
        .link("/payments/{id}")
        .document_in(|tx, payment| Box::pin(payment_document(tx, payment)))
        .register();
    Searchable::of::<Cheque>(CHEQUE_SUBJECT_TYPE)
        .permission(permissions::CHEQUES_VIEW)
        .label("Cheques")
        .link("/cheques/{id}")
        .document_in(|tx, cheque| Box::pin(cheque_document(tx, cheque)))
        .register();
    Searchable::of::<Deposit>(DEPOSIT_SUBJECT_TYPE)
        .permission(permissions::DEPOSITS_VIEW)
        .label("Deposits")
        .link("/deposits/{id}")
        .document_in(|tx, deposit| Box::pin(deposit_document(tx, deposit)))
        .register();
    Searchable::of::<Refund>(REFUND_SUBJECT_TYPE)
        .permission(permissions::VIEW)
        .label("Refunds")
        .link("/refunds/{id}")
        .document_in(|tx, refund| Box::pin(refund_document(tx, refund)))
        .register();
    relations();
    app.listen(|event: Transitioned<Payment>, state: AppState| async move {
        index(&state.db, &[Subject::new(PAYMENT_SUBJECT_TYPE, event.key)]).await
    });
    app.listen(|event: Transitioned<Cheque>, state: AppState| async move {
        index(&state.db, &[Subject::new(CHEQUE_SUBJECT_TYPE, event.key)]).await
    });
    app.listen(|event: Transitioned<Deposit>, state: AppState| async move {
        index(&state.db, &[Subject::new(DEPOSIT_SUBJECT_TYPE, event.key)]).await
    });
    app.listen(|event: Transitioned<Refund>, state: AppState| async move {
        index(&state.db, &[Subject::new(REFUND_SUBJECT_TYPE, event.key)]).await
    });
}

/// The search document of `payment`.
pub async fn payment_document(tx: &mut Transaction, payment: &Payment) -> Result<SearchDocument> {
    let payer = party_name_in(tx, payment.party_id).await?;
    let number = payment.number.clone().unwrap_or_default();
    let title = if number.is_empty() {
        format!("Payment #{}", payment.id)
    } else {
        number.clone()
    };
    let mut candidates = Vec::new();
    if let (Some(kind), Some(id)) = (&payment.prepaid_type, payment.prepaid_id) {
        candidates.push(Subject::new(kind, id));
    }
    let allocated = Allocation::query()
        .where_eq("payment_id", payment.id)
        .where_eq("target_type", INVOICE_SUBJECT_TYPE)
        .where_null("removed_at")
        .order_by("id")
        .first(&mut *tx)
        .await?;
    if let Some(allocation) = allocated {
        candidates.push(Subject::new(INVOICE_SUBJECT_TYPE, allocation.target_id));
    }
    let funded = Deposit::query()
        .where_eq("received_payment_id", payment.id)
        .first(&mut *tx)
        .await?;
    if let Some(deposit) = funded {
        candidates.push(deposit.source());
    }
    let scope = scope_in(tx, &candidates).await?;
    let mut document = SearchDocument::new(title)
        .secondary(payer)
        .identifier(kinds::NUMBER, number)
        .status(payment.status.as_str())
        .scope(scope)
        .legal_entity(payment.legal_entity_id);
    // Only the cheque number of the method details: a card slip or a
    // transfer's account is never indexed.
    if payment.method == PaymentMethod::Cheque
        && let Some(cheque) = payment.detail("number")
    {
        document = document.identifier(kinds::CHEQUE, cheque);
    }
    Ok(document)
}

/// The search document of `cheque`: its number, over the drawer and the
/// bank.
pub async fn cheque_document(tx: &mut Transaction, cheque: &Cheque) -> Result<SearchDocument> {
    let drawer = party_name_in(tx, cheque.party_id).await?;
    let mut candidates = Vec::new();
    if let (Some(kind), Some(id)) = (&cheque.agreement_type, cheque.agreement_id) {
        candidates.push(Subject::new(kind, id));
    }
    let scope = scope_in(tx, &candidates).await?;
    let line = [drawer.as_str(), cheque.bank.as_str()]
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    Ok(SearchDocument::new(cheque.number.clone())
        .secondary(line)
        .identifier(kinds::CHEQUE, cheque.number.clone())
        .status(cheque.status.as_str())
        .scope(scope)
        .legal_entity(cheque.legal_entity_id))
}

/// The search document of `deposit`: the party and what it is held for.
pub async fn deposit_document(tx: &mut Transaction, deposit: &Deposit) -> Result<SearchDocument> {
    let party = party_name_in(tx, deposit.party_id).await?;
    let source = deposit.source();
    let held_for = stored_in(tx, &source)
        .await?
        .map(|document| document.title)
        .unwrap_or_default();
    let scope = scope_in(tx, &[source]).await?;
    Ok(SearchDocument::new(format!("Deposit #{}", deposit.id))
        .secondary(joined(&party, &held_for))
        .keyword(held_for)
        .status(deposit.status.as_str())
        .scope(scope)
        .legal_entity(deposit.legal_entity_id))
}

/// The search document of `refund`: the party and what it returns.
pub async fn refund_document(tx: &mut Transaction, refund: &Refund) -> Result<SearchDocument> {
    let party = party_name_in(tx, refund.party_id).await?;
    let source = Subject::new(&refund.source_type, refund.source_id);
    let returns = stored_in(tx, &source)
        .await?
        .map(|document| document.title)
        .unwrap_or_default();
    let scope = scope_in(tx, &[source]).await?;
    Ok(SearchDocument::new(format!("Refund #{}", refund.id))
        .secondary(joined(&party, &returns))
        .keyword(returns)
        .status(refund.status.as_str())
        .scope(scope)
        .legal_entity(refund.legal_entity_id))
}

/// `a · b`, leaving out an empty part.
fn joined(a: &str, b: &str) -> String {
    [a, b]
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The relations of the four types, and a party's payments and an
/// invoice's cheques (this module owns the rows).
fn relations() {
    Relation::has_many(
        PARTY_SUBJECT_TYPE,
        "payments",
        PAYMENT_SUBJECT_TYPE,
        "wl_payments_payments",
        "party_id",
    )
    .label("Payments")
    .permission(permissions::VIEW)
    .register();
    Relation::new(INVOICE_SUBJECT_TYPE, "cheques", CHEQUE_SUBJECT_TYPE)
        .label("Cheques")
        .permission(permissions::CHEQUES_VIEW)
        .load(|db, ids| {
            Box::pin(async move {
                pairs(
                    db,
                    "SELECT subject_id AS from_id, cheque_id AS to_id FROM wl_payments_cheque_links \
                     WHERE subject_type = ? AND subject_id IN ({ids}) ORDER BY cheque_id",
                    vec![DbValue::Text(INVOICE_SUBJECT_TYPE.to_owned())],
                    ids,
                    INVOICE_SUBJECT_TYPE,
                    CHEQUE_SUBJECT_TYPE,
                )
                .await
            })
        })
        .register();
    // payments.payment
    Relation::new(PAYMENT_SUBJECT_TYPE, "invoices", INVOICE_SUBJECT_TYPE)
        .label("Invoices allocated")
        .permission(INVOICES_VIEW)
        .link("/invoices/{id}")
        .load(|db, ids| {
            Box::pin(async move {
                pairs(
                    db,
                    "SELECT payment_id AS from_id, target_id AS to_id FROM wl_payments_allocations \
                     WHERE target_type = ? AND removed_at IS NULL AND payment_id IN ({ids}) \
                     ORDER BY id",
                    vec![DbValue::Text(INVOICE_SUBJECT_TYPE.to_owned())],
                    ids,
                    PAYMENT_SUBJECT_TYPE,
                    INVOICE_SUBJECT_TYPE,
                )
                .await
            })
        })
        .register();
    party_of(PAYMENT_SUBJECT_TYPE, "wl_payments_payments", "Payer");
    // payments.cheque
    Relation::new(CHEQUE_SUBJECT_TYPE, "invoices", INVOICE_SUBJECT_TYPE)
        .label("Invoices it pays")
        .permission(INVOICES_VIEW)
        .link("/invoices/{id}")
        .load(|db, ids| {
            Box::pin(async move {
                pairs(
                    db,
                    "SELECT cheque_id AS from_id, subject_id AS to_id FROM wl_payments_cheque_links \
                     WHERE subject_type = ? AND cheque_id IN ({ids}) ORDER BY position",
                    vec![DbValue::Text(INVOICE_SUBJECT_TYPE.to_owned())],
                    ids,
                    CHEQUE_SUBJECT_TYPE,
                    INVOICE_SUBJECT_TYPE,
                )
                .await
            })
        })
        .register();
    party_of(CHEQUE_SUBJECT_TYPE, "wl_payments_cheques", "Drawer");
    // payments.deposit
    party_of(DEPOSIT_SUBJECT_TYPE, "wl_payments_deposits", "Party");
    // payments.refund
    party_of(REFUND_SUBJECT_TYPE, "wl_payments_refunds", "Party");
    for (name, label, to_type) in [
        ("payment", "Payment returned", PAYMENT_SUBJECT_TYPE),
        ("deposit", "Deposit returned", DEPOSIT_SUBJECT_TYPE),
    ] {
        Relation::new(REFUND_SUBJECT_TYPE, name, to_type)
            .label(label)
            .permission(permissions::VIEW)
            .load(move |db, ids| {
                Box::pin(async move {
                    pairs(
                        db,
                        "SELECT id AS from_id, source_id AS to_id FROM wl_payments_refunds \
                         WHERE source_type = ? AND id IN ({ids})",
                        vec![DbValue::Text(to_type.to_owned())],
                        ids,
                        REFUND_SUBJECT_TYPE,
                        to_type,
                    )
                    .await
                })
            })
            .register();
    }
}

/// `from_type` → its party: column `party_id` of `table`.
fn party_of(from_type: &str, table: &str, label: &str) {
    Relation::belongs_to(from_type, "party", PARTY_SUBJECT_TYPE, table, "party_id")
        .label(label)
        .permission(PARTIES_VIEW)
        .link("/parties/{id}")
        .register();
}

/// Runs `statement`, whose `{ids}` stands for one placeholder per id, with
/// `values` then `ids`, reading `from_id` and `to_id` as records of
/// `from_type` and `to_type`.
async fn pairs(
    db: &Db,
    statement: &str,
    mut values: Vec<DbValue>,
    ids: &[i64],
    from_type: &str,
    to_type: &str,
) -> Result<Vec<(Subject, Subject)>> {
    let placeholders = vec!["?"; ids.len()].join(", ");
    values.extend(ids.iter().map(|id| DbValue::Integer(*id)));
    let rows = sql(statement.replace("{ids}", &placeholders))
        .bind_all(values)
        .fetch_all(db)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let from: Option<i64> = row.try_get("from_id")?;
        let to: Option<i64> = row.try_get("to_id")?;
        if let (Some(from), Some(to)) = (from, to) {
            out.push((Subject::new(from_type, from), Subject::new(to_type, to)));
        }
    }
    Ok(out)
}

/// The stored search document of `subject`, read in `tx`.
async fn stored_in(tx: &mut Transaction, subject: &Subject) -> Result<Option<SearchDocumentRow>> {
    SearchDocumentRow::query()
        .where_eq("subject_type", subject.subject_type.as_str())
        .where_eq("subject_id", subject.subject_id)
        .first(&mut *tx)
        .await
}

/// The name party `id` is found by (the title of its search document), or
/// `""`.
async fn party_name_in(tx: &mut Transaction, id: i64) -> Result<String> {
    Ok(stored_in(tx, &Subject::new(PARTY_SUBJECT_TYPE, id))
        .await?
        .map(|document| document.title)
        .unwrap_or_default())
}

/// The scope of the first of `candidates` that has one: its kernel
/// machine's scope (read in `tx`), else the scope of its search document;
/// global when none has one.
async fn scope_in(tx: &mut Transaction, candidates: &[Subject]) -> Result<Scope> {
    for subject in candidates {
        if let Some(machine) =
            wl_kernel::machines::get(&subject.subject_type).filter(|m| m.has_scope())
        {
            // A record that is gone (or a source a test made up) has no
            // scope: the machine answers `NotFound`.
            match machine.scope_in(tx, subject.subject_id).await {
                Ok(scope) if !scope.is_global() => return Ok(scope),
                Ok(_) | Err(Error::NotFound) => {}
                Err(error) => return Err(error),
            }
        }
        if let Some(document) = stored_in(tx, subject).await?
            && !document.scope_type.is_empty()
        {
            return Ok(Scope::new(&document.scope_type, &document.scope_id));
        }
    }
    Ok(Scope::global())
}

/// Writes the documents of `subjects` again in their own transaction.
pub async fn index(db: &Db, subjects: &[Subject]) -> Result {
    let mut tx = db.begin().await?;
    for subject in subjects {
        wl_kernel::search::index_in(&mut tx, subject).await?;
    }
    tx.commit().await?;
    Ok(())
}
