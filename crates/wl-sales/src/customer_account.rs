//! The customer account (wl-sales §3 *Customer account*, *Credit group*;
//! BL-SAL-10, BL-SAL-12): the commercial terms of a buying party that the
//! credit control reads, and the sources that keep it on hold.
//!
//! Decisions:
//! - A customer without an account is not credit-checked: the limit is a
//!   term of the account, and an account is made on purpose.
//! - A credit group is named here by its owner party. `wl-parties` exposes
//!   no company group to the kernel yet, so the accounts name the group they
//!   share a combined limit in; the bounced cheque rule resolves "the whole
//!   group" from it ([`CustomerAccount::group_members`]).
//! - The on-hold flag is not a column: an account is on hold while at least
//!   one [`AccountHoldSource`] is open, so a second bounced cheque and the
//!   clearing of the first need no bookkeeping beyond the rows.
//! - Setting the terms is a plain action behind `credit.limits.approve`; the
//!   proposal and approval of a limit change (`credit.limits.propose`,
//!   `CreditLimitChanged`) is not part of the credit check issues.

use renox::db::{DateTime, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::{PartyId, Subject};

use crate::texts::english;

/// The subject type of a customer account.
pub const ACCOUNT_SUBJECT_TYPE: &str = "sales.customer_account";

/// The reason of a hold source: a bounced received cheque.
pub const REASON_BOUNCED_CHEQUE: &str = "bounced_cheque";

/// A 422 with the English text `key`.
fn refused(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]))
}

/// Companies under one owner that share a combined limit.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_sales_credit_groups", unique(party_id))]
pub struct CreditGroup {
    /// The primary key.
    pub id: i64,
    /// The owner of the companies.
    pub party_id: i64,
    /// What the group is called.
    pub name: String,
    /// The combined limit, in fils; `None`: none.
    pub combined_limit: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The commercial terms of a buying party.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_customer_accounts",
    unique(party_id),
    index(credit_group_id)
)]
pub struct CustomerAccount {
    /// The primary key.
    pub id: i64,
    /// The buying party.
    pub party_id: i64,
    /// The credit limit, in fils; `None`: no limit.
    pub credit_limit: Option<i64>,
    /// The payment terms, in days.
    pub payment_terms_days: i64,
    /// The days past terms before an overdue invoice fails the check; `None`:
    /// the module setting.
    pub grace_days: Option<i64>,
    /// The credit group it shares a combined limit in.
    #[model(references = "wl_sales_credit_groups")]
    pub credit_group_id: Option<i64>,
    /// The user who sells to the customer and is told of a hold.
    pub sales_rep_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// One open or cleared reason an account is on hold.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_account_hold_sources",
    unique(account_id, source_type, source_id)
)]
pub struct AccountHoldSource {
    /// The primary key.
    pub id: i64,
    /// The account.
    #[model(references = "wl_sales_customer_accounts")]
    pub account_id: i64,
    /// The type of the subject that caused it (`payments.cheque`).
    pub source_type: String,
    /// The id of the subject that caused it.
    pub source_id: i64,
    /// Why: `bounced_cheque`.
    pub reason: String,
    /// When it was cleared; `None` while open.
    pub cleared_at: Option<DateTime>,
    /// The user who approved the clearing.
    pub cleared_by: Option<i64>,
    /// The clearing request that removed it.
    pub clearing_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl AccountHoldSource {
    /// The subject that caused the hold.
    pub fn source(&self) -> Subject {
        Subject::new(&self.source_type, self.source_id)
    }

    /// Whether the source still holds the account.
    pub fn is_open(&self) -> bool {
        self.cleared_at.is_none()
    }
}

impl CustomerAccount {
    /// The account as a kernel subject.
    pub fn subject(&self) -> Subject {
        Subject::new(ACCOUNT_SUBJECT_TYPE, self.id)
    }

    /// The buying party.
    pub fn party(&self) -> PartyId {
        PartyId(self.party_id)
    }

    /// The account of `party`, if it has one.
    pub async fn of_party(db: &Db, party: i64) -> Result<Option<CustomerAccount>> {
        CustomerAccount::query()
            .where_(CustomerAccount::PARTY_ID.eq(party))
            .first(db)
            .await
    }

    /// The grace days: the account's own, else `default`.
    pub fn grace(&self, default: i64) -> i64 {
        self.grace_days.unwrap_or(default)
    }

    /// The group this account belongs to, if any.
    pub async fn group(&self, db: &Db) -> Result<Option<CreditGroup>> {
        match self.credit_group_id {
            Some(id) => CreditGroup::find(db, id).await,
            None => Ok(None),
        }
    }

    /// Every account of the credit group (this one included); just this one
    /// when it has no group.
    pub async fn group_members(&self, db: &Db) -> Result<Vec<CustomerAccount>> {
        match self.credit_group_id {
            Some(id) => {
                CustomerAccount::query()
                    .where_(CustomerAccount::CREDIT_GROUP_ID.eq(id))
                    .order_by(CustomerAccount::ID)
                    .get(db)
                    .await
            }
            None => Ok(vec![self.clone()]),
        }
    }

    /// The sources that hold the account now, oldest first.
    pub async fn open_sources(&self, db: &Db) -> Result<Vec<AccountHoldSource>> {
        AccountHoldSource::query()
            .where_(AccountHoldSource::ACCOUNT_ID.eq(self.id))
            .where_(AccountHoldSource::CLEARED_AT.is_null())
            .order_by(AccountHoldSource::ID)
            .get(db)
            .await
    }

    /// Every source of the account, open or cleared, newest first.
    pub async fn all_sources(&self, db: &Db) -> Result<Vec<AccountHoldSource>> {
        AccountHoldSource::query()
            .where_(AccountHoldSource::ACCOUNT_ID.eq(self.id))
            .order_by_desc(AccountHoldSource::ID)
            .get(db)
            .await
    }

    /// Whether the on-hold flag is set: a source is open.
    pub async fn is_on_hold(&self, db: &Db) -> Result<bool> {
        Ok(!self.open_sources(db).await?.is_empty())
    }
}

/// What the terms of an account are set to.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Terms {
    /// The credit limit, in fils; `None`: no limit.
    pub credit_limit: Option<i64>,
    /// The payment terms, in days.
    pub payment_terms_days: i64,
    /// The grace days past terms; `None`: the module setting.
    pub grace_days: Option<i64>,
    /// The credit group, by its id.
    pub credit_group: Option<i64>,
    /// The sales rep, by user id.
    pub sales_rep: Option<i64>,
}

impl Default for Terms {
    fn default() -> Self {
        Terms {
            credit_limit: None,
            payment_terms_days: 30,
            grace_days: None,
            credit_group: None,
            sales_rep: None,
        }
    }
}

impl Terms {
    /// Terms with this limit in fils.
    pub fn limit(mut self, fils: i64) -> Self {
        self.credit_limit = Some(fils);
        self
    }

    /// With this many grace days past terms.
    pub fn grace_days(mut self, days: i64) -> Self {
        self.grace_days = Some(days);
        self
    }

    /// In this credit group.
    pub fn group(mut self, group: i64) -> Self {
        self.credit_group = Some(group);
        self
    }

    /// Sold to by this user.
    pub fn rep(mut self, user: i64) -> Self {
        self.sales_rep = Some(user);
        self
    }
}

/// Makes (or changes) the account of `party` with `terms`.
///
/// # Errors
///
/// 422 when the limit is negative, the terms or grace days are negative, or
/// the group does not exist.
pub async fn save(state: &AppState, party: PartyId, terms: Terms) -> Result<CustomerAccount> {
    if terms.credit_limit.is_some_and(|limit| limit < 0) {
        return Err(refused("wl_sales.credit.problem.limit"));
    }
    if terms.payment_terms_days < 0 || terms.grace_days.is_some_and(|days| days < 0) {
        return Err(refused("wl_sales.credit.problem.days"));
    }
    if let Some(group) = terms.credit_group
        && CreditGroup::find(&state.db, group).await?.is_none()
    {
        return Err(refused("wl_sales.credit.problem.group"));
    }
    match CustomerAccount::of_party(&state.db, party.0).await? {
        Some(existing) => {
            CustomerAccount::query()
                .where_(CustomerAccount::ID.eq(existing.id))
                .update(
                    &state.db,
                    &[
                        ("credit_limit", &terms.credit_limit),
                        ("payment_terms_days", &terms.payment_terms_days),
                        ("grace_days", &terms.grace_days),
                        ("credit_group_id", &terms.credit_group),
                        ("sales_rep_id", &terms.sales_rep),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
            CustomerAccount::find_or_404(&state.db, existing.id).await
        }
        None => {
            CustomerAccount::create(
                &state.db,
                CustomerAccount {
                    party_id: party.0,
                    credit_limit: terms.credit_limit,
                    payment_terms_days: terms.payment_terms_days,
                    grace_days: terms.grace_days,
                    credit_group_id: terms.credit_group,
                    sales_rep_id: terms.sales_rep,
                    ..CustomerAccount::default()
                },
            )
            .await
        }
    }
}

/// Makes (or renames) the credit group owned by `owner` with a combined
/// limit in fils.
///
/// # Errors
///
/// 422 when the limit is negative.
pub async fn save_group(
    state: &AppState,
    owner: PartyId,
    name: &str,
    combined_limit: Option<i64>,
) -> Result<CreditGroup> {
    if combined_limit.is_some_and(|limit| limit < 0) {
        return Err(refused("wl_sales.credit.problem.limit"));
    }
    let existing = CreditGroup::query()
        .where_(CreditGroup::PARTY_ID.eq(owner.0))
        .first(&state.db)
        .await?;
    match existing {
        Some(group) => {
            CreditGroup::query()
                .where_(CreditGroup::ID.eq(group.id))
                .update(
                    &state.db,
                    &[
                        ("name", &name.trim().to_owned()),
                        ("combined_limit", &combined_limit),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
            CreditGroup::find_or_404(&state.db, group.id).await
        }
        None => {
            CreditGroup::create(
                &state.db,
                CreditGroup {
                    party_id: owner.0,
                    name: name.trim().to_owned(),
                    combined_limit,
                    ..CreditGroup::default()
                },
            )
            .await
        }
    }
}

/// Adds the hold source `source` (a bounced cheque) to `account`. A source
/// already recorded for the account changes nothing, open or cleared: the
/// same `ChequeBounced` twice adds one source. Returns whether a row was
/// added.
pub async fn add_source(
    db: &Db,
    account: &CustomerAccount,
    source: &Subject,
    reason: &str,
) -> Result<bool> {
    let known = AccountHoldSource::query()
        .where_(AccountHoldSource::ACCOUNT_ID.eq(account.id))
        .where_(AccountHoldSource::SOURCE_TYPE.eq(source.subject_type.clone()))
        .where_(AccountHoldSource::SOURCE_ID.eq(source.subject_id))
        .first(db)
        .await?;
    if known.is_some() {
        return Ok(false);
    }
    AccountHoldSource::create(
        db,
        AccountHoldSource {
            account_id: account.id,
            source_type: source.subject_type.clone(),
            source_id: source.subject_id,
            reason: reason.to_owned(),
            ..AccountHoldSource::default()
        },
    )
    .await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms_default_to_thirty_days_and_no_limit() {
        let terms = Terms::default();
        assert_eq!(terms.credit_limit, None);
        assert_eq!(terms.payment_terms_days, 30);
        let terms = Terms::default().limit(5_000_000).grace_days(10).rep(4);
        assert_eq!(
            (terms.credit_limit, terms.grace_days, terms.sales_rep),
            (Some(5_000_000), Some(10), Some(4))
        );
    }

    #[test]
    fn the_grace_is_the_accounts_own_else_the_setting() {
        let account = CustomerAccount::default();
        assert_eq!(account.grace(30), 30);
        let own = CustomerAccount {
            grace_days: Some(10),
            ..CustomerAccount::default()
        };
        assert_eq!(own.grace(30), 10);
    }

    #[test]
    fn a_source_is_open_until_cleared() {
        let mut source = AccountHoldSource {
            source_type: "payments.cheque".into(),
            source_id: 3,
            ..AccountHoldSource::default()
        };
        assert!(source.is_open());
        assert_eq!(source.source(), Subject::new("payments.cheque", 3));
        source.cleared_at = Some(now());
        assert!(!source.is_open());
    }
}
