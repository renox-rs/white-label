//! The accounting records in the global search and the *Related* panel
//! (BL-AGT-12, decision 0006): a partner finds an engagement by its number,
//! the client's name or TRN, a tax specialist a return by its number, its
//! period or the authority's reference, each with its related records.
//!
//! **Confidentiality.** An AML case is the MLRO's alone (`ACC.ACL.05`): its
//! type needs `aml.manage`, which neither the company admin nor any other
//! role holds, so nobody else finds it, counts it or sees it in a panel; its
//! document holds the case number, the kind and the goAML reference only,
//! never the facts. A CDD file needs `cdd.view` and holds its number and the
//! client's name only: the risk factors and EDD notes are never indexed. No
//! relation leads from an engagement, a return or a party to an AML case.
//!
//! The client's name is read from its party row (`wl_parties_parties`, as
//! the lists' client column reads it) in the transaction that indexes.

use renox::db::{DbValue, Transaction, sql};
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transitioned};
use wl_kernel::Subject;
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::related::{LoadFuture, Relation};
use wl_kernel::search::{SearchDocument, Searchable, index_in, kinds};

use crate::aml::{AML_CASE_SUBJECT_TYPE, AmlCase};
use crate::cdd::{CDD_FILE_SUBJECT_TYPE, CddFile};
use crate::engagement::{ENGAGEMENT_SUBJECT_TYPE, Engagement};
use crate::permissions;
use crate::tax_profile::TaxProfile;
use crate::tax_return::{RETURN_SUBJECT_TYPE, Return, ReturnStatus};

const PARTIES_VIEW: &str = "parties.view";

/// The kind of identifier of a return's period (`2026-03-31`, `2026-03`,
/// `VAT 2026-03`).
pub const PERIOD: &str = "period";

/// The display name of party `id`, read in `tx`; empty when unknown.
async fn client_name_in(tx: &mut Transaction, id: i64) -> Result<String> {
    Ok(
        sql("SELECT display_name FROM wl_parties_parties WHERE id = ?")
            .bind(id)
            .scalar_optional::<String>(&mut *tx)
            .await?
            .unwrap_or_default(),
    )
}

/// The TRNs the client's tax profiles have held, newest first.
async fn trns_in(tx: &mut Transaction, client: i64) -> Result<Vec<String>> {
    Ok(TaxProfile::query()
        .where_eq("client_party_id", client)
        .order_by_desc("effective_date")
        .get(&mut *tx)
        .await?
        .into_iter()
        .filter_map(|profile| profile.trn)
        .filter(|trn| !trn.trim().is_empty())
        .collect())
}

async fn engagement_document(
    tx: &mut Transaction,
    engagement: &Engagement,
) -> Result<SearchDocument> {
    let client = client_name_in(tx, engagement.client_party_id).await?;
    let number = engagement.number();
    let mut document = SearchDocument::new(format!("{number} · {}", engagement.title))
        .secondary(&client)
        .keyword(&engagement.title)
        .identifier(kinds::NUMBER, &number)
        .status(engagement.status.as_str());
    for trn in trns_in(tx, engagement.client_party_id).await? {
        document = document.identifier(kinds::TRN, trn);
    }
    Ok(document)
}

async fn return_document(tx: &mut Transaction, ret: &Return) -> Result<SearchDocument> {
    let client = client_name_in(tx, ret.client_party_id).await?;
    let number = ret.number();
    let kind = ret.return_type.as_str().to_uppercase();
    let month = ret.period_end.format("%Y-%m").to_string();
    let mut document = SearchDocument::new(format!("{number} · {kind} {month}"))
        .secondary(&client)
        .identifier(kinds::NUMBER, &number)
        .identifier(PERIOD, ret.period_end.to_string())
        .identifier(PERIOD, &month)
        .identifier(PERIOD, format!("{kind} {month}"))
        .status(ret.status.as_str())
        .voided(ret.status == ReturnStatus::Cancelled);
    for reference in [&ret.acknowledgement, &ret.correction_reference]
        .into_iter()
        .flatten()
    {
        document = document.identifier(kinds::NUMBER, reference);
    }
    for trn in trns_in(tx, ret.client_party_id).await? {
        document = document.identifier(kinds::TRN, trn);
    }
    Ok(document)
}

async fn cdd_document(tx: &mut Transaction, file: &CddFile) -> Result<SearchDocument> {
    let client = client_name_in(tx, file.client_party_id).await?;
    let number = file.number();
    Ok(SearchDocument::new(&number)
        .secondary(&client)
        .identifier(kinds::NUMBER, &number)
        .status(file.status.as_str()))
}

async fn aml_document(tx: &mut Transaction, case: &AmlCase) -> Result<SearchDocument> {
    let client = client_name_in(tx, case.client_party_id).await?;
    let number = case.number();
    let mut document = SearchDocument::new(&number)
        .secondary(&client)
        .keyword(case.kind.as_str())
        .identifier(kinds::NUMBER, &number)
        .status(case.status.as_str());
    if let Some(reference) = &case.goaml_reference {
        document = document.identifier(kinds::NUMBER, reference);
    }
    Ok(document)
}

/// The `(from, to)` pairs of the records of `from_table` and `to_table`
/// that share a client (`client_party_id`), for the records `ids` of
/// `from_table`.
fn same_client<'a>(
    db: &'a Db,
    ids: &'a [i64],
    from_table: &'static str,
    to_table: &'static str,
    from_type: &'static str,
    to_type: &'static str,
) -> LoadFuture<'a> {
    Box::pin(async move {
        let placeholders = vec!["?"; ids.len()].join(", ");
        let rows = sql(format!(
            "SELECT a.id AS from_id, b.id AS to_id FROM {from_table} a \
             JOIN {to_table} b ON b.client_party_id = a.client_party_id \
             WHERE a.id IN ({placeholders}) ORDER BY b.id DESC"
        ))
        .bind_all(ids.iter().map(|id| DbValue::Integer(*id)))
        .fetch_all(db)
        .await?;
        let mut pairs = Vec::with_capacity(rows.len());
        for row in &rows {
            let from: i64 = row.try_get("from_id")?;
            let to: i64 = row.try_get("to_id")?;
            pairs.push((Subject::new(from_type, from), Subject::new(to_type, to)));
        }
        Ok(pairs)
    })
}

/// Declares the searchable types and their relations.
pub(crate) fn register() {
    Searchable::of::<Engagement>(ENGAGEMENT_SUBJECT_TYPE)
        .permission(permissions::ENGAGEMENTS_VIEW)
        .label("Engagements")
        .link("/accounting/engagements/{id}")
        .document_in(|tx, engagement| Box::pin(engagement_document(tx, engagement)))
        .register();
    Searchable::of::<Return>(RETURN_SUBJECT_TYPE)
        .permission(permissions::RETURNS_VIEW)
        .label("Tax returns")
        .link("/accounting/returns/{id}")
        .document_in(|tx, ret| Box::pin(return_document(tx, ret)))
        .register();
    Searchable::of::<CddFile>(CDD_FILE_SUBJECT_TYPE)
        .permission(permissions::CDD_VIEW)
        .label("CDD files")
        .link("/accounting/cdd/{id}")
        .document_in(|tx, file| Box::pin(cdd_document(tx, file)))
        .register();
    // The MLRO's alone (`ACC.ACL.05`).
    Searchable::of::<AmlCase>(AML_CASE_SUBJECT_TYPE)
        .permission(permissions::AML_MANAGE)
        .label("AML cases")
        .link("/accounting/aml-cases/{id}")
        .document_in(|tx, case| Box::pin(aml_document(tx, case)))
        .register();

    // An engagement: its returns, the client's CDD files and the client.
    Relation::has_many(
        ENGAGEMENT_SUBJECT_TYPE,
        "returns",
        RETURN_SUBJECT_TYPE,
        "accounting_returns",
        "engagement_id",
    )
    .label("Tax returns")
    .register();
    Relation::new(ENGAGEMENT_SUBJECT_TYPE, "cdd_files", CDD_FILE_SUBJECT_TYPE)
        .label("CDD files")
        .load(|db, ids| {
            same_client(
                db,
                ids,
                "accounting_engagements",
                "accounting_cdd_files",
                ENGAGEMENT_SUBJECT_TYPE,
                CDD_FILE_SUBJECT_TYPE,
            )
        })
        .register();
    Relation::belongs_to(
        ENGAGEMENT_SUBJECT_TYPE,
        "client",
        PARTY_SUBJECT_TYPE,
        "accounting_engagements",
        "client_party_id",
    )
    .label("Client")
    .permission(PARTIES_VIEW)
    .link("/parties/{id}")
    .register();

    // A return: its engagement, the version it supersedes, its correction.
    Relation::belongs_to(
        RETURN_SUBJECT_TYPE,
        "engagement",
        ENGAGEMENT_SUBJECT_TYPE,
        "accounting_returns",
        "engagement_id",
    )
    .label("Engagement")
    .register();
    Relation::belongs_to(
        RETURN_SUBJECT_TYPE,
        "supersedes",
        RETURN_SUBJECT_TYPE,
        "accounting_returns",
        "supersedes_id",
    )
    .label("Supersedes")
    .register();
    Relation::belongs_to(
        RETURN_SUBJECT_TYPE,
        "correction",
        RETURN_SUBJECT_TYPE,
        "accounting_returns",
        "correction_return_id",
    )
    .label("Correction")
    .register();

    // A CDD file: the client's engagements and the client.
    Relation::new(
        CDD_FILE_SUBJECT_TYPE,
        "engagements",
        ENGAGEMENT_SUBJECT_TYPE,
    )
    .label("Engagements")
    .load(|db, ids| {
        same_client(
            db,
            ids,
            "accounting_cdd_files",
            "accounting_engagements",
            CDD_FILE_SUBJECT_TYPE,
            ENGAGEMENT_SUBJECT_TYPE,
        )
    })
    .register();
    Relation::belongs_to(
        CDD_FILE_SUBJECT_TYPE,
        "client",
        PARTY_SUBJECT_TYPE,
        "accounting_cdd_files",
        "client_party_id",
    )
    .label("Client")
    .permission(PARTIES_VIEW)
    .link("/parties/{id}")
    .register();

    // An AML case: its client only (seen by the MLRO, who sees the case).
    Relation::belongs_to(
        AML_CASE_SUBJECT_TYPE,
        "client",
        PARTY_SUBJECT_TYPE,
        "accounting_aml_cases",
        "client_party_id",
    )
    .label("Client")
    .permission(PARTIES_VIEW)
    .link("/parties/{id}")
    .register();

    // A client company: its engagements, returns and CDD files (a partner
    // who finds the client by name sees them in its panel).
    Relation::has_many(
        PARTY_SUBJECT_TYPE,
        "engagements",
        ENGAGEMENT_SUBJECT_TYPE,
        "accounting_engagements",
        "client_party_id",
    )
    .label("Engagements")
    .register();
    Relation::has_many(
        PARTY_SUBJECT_TYPE,
        "tax_returns",
        RETURN_SUBJECT_TYPE,
        "accounting_returns",
        "client_party_id",
    )
    .label("Tax returns")
    .register();
    Relation::has_many(
        PARTY_SUBJECT_TYPE,
        "cdd_files",
        CDD_FILE_SUBJECT_TYPE,
        "accounting_cdd_files",
        "client_party_id",
    )
    .label("CDD files")
    .register();
}

/// Writes the document of `subject` again in a transaction of its own, for
/// a record saved outside one.
///
/// # Errors
///
/// A database error.
pub async fn index(db: &Db, subject: &Subject) -> Result {
    let mut tx = db.begin().await?;
    index_in(&mut tx, subject).await?;
    tx.commit().await?;
    Ok(())
}

/// Writes the documents of the client `client`'s engagements and returns
/// again (its TRN changed).
///
/// # Errors
///
/// A database error.
pub async fn index_client_in(tx: &mut Transaction, client: i64) -> Result {
    for engagement in Engagement::query()
        .where_eq("client_party_id", client)
        .get(&mut *tx)
        .await?
    {
        index_in(tx, &Subject::new(ENGAGEMENT_SUBJECT_TYPE, engagement.id)).await?;
    }
    for ret in Return::query()
        .where_eq("client_party_id", client)
        .get(&mut *tx)
        .await?
    {
        index_in(tx, &Subject::new(RETURN_SUBJECT_TYPE, ret.id)).await?;
    }
    Ok(())
}

/// Writes the document of a record of `M` again once a transition run with
/// `run` has committed (its status changed).
fn reindex_after<M>(app: &mut Registry, subject_type: &'static str)
where
    M: StateMachine<Key = i64> + Send + Sync + 'static,
{
    app.listen(move |event: Transitioned<M>, state: AppState| async move {
        index(&state.db, &Subject::new(subject_type, event.key)).await
    });
}

/// Keeps the documents current after transitions.
pub(crate) fn listen(app: &mut Registry) {
    reindex_after::<Engagement>(app, ENGAGEMENT_SUBJECT_TYPE);
    reindex_after::<Return>(app, RETURN_SUBJECT_TYPE);
    reindex_after::<CddFile>(app, CDD_FILE_SUBJECT_TYPE);
    reindex_after::<AmlCase>(app, AML_CASE_SUBJECT_TYPE);
}
