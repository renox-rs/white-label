//! The amounts of a final settlement (wl-hr §3 *Final settlement*: unpaid
//! salary, leave encashment, gratuity, notice pay, other dues, recoveries;
//! §4 `dispute`/`resolve`: *revised*; issues #212, #213).
//!
//! The preparer (`settlements.prepare`) enters the figures while the
//! settlement is a `draft`, and again, as the **revised** amounts, while it is
//! `disputed`. Nothing else changes them. The user who enters them is the
//! preparer of `HRM.ACL.04`: `approve` (draft) and `resolve` (revised) are
//! run by another user. Every change is audited.
//!
//! Decision: the figures are typed by the preparer; deriving the gratuity and
//! the leave encashment from the contract is not in these issues. The
//! overpayment recoveries that supplementary runs add are not typed
//! ([`crate::supplementary`]); the other recovery types are.

use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};

use crate::permissions;
use crate::settlement::{FinalSettlement, PAYEE_ESTATE, SettlementStatus, refuse};
use crate::settlement_offset::recalculate_in;

/// The figures of a settlement, AED fils.
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq, Eq)]
#[serde(default)]
pub struct AmountsInput {
    /// The last working day.
    pub last_working_day: Option<NaiveDate>,
    /// Unpaid salary.
    pub unpaid_salary: i64,
    /// Leave encashment.
    pub leave_encashment: i64,
    /// Gratuity.
    pub gratuity: i64,
    /// Notice pay.
    pub notice_pay: i64,
    /// Other dues.
    pub other_dues: i64,
    /// Advances and loans to recover.
    pub advance_recoveries: i64,
    /// Fines to recover.
    pub fine_recoveries: i64,
    /// Court debts to recover.
    pub court_debt_recoveries: i64,
    /// Damage to recover.
    pub damage_recoveries: i64,
    /// The estate's representative (payee *estate*).
    pub representative: Option<String>,
    /// The document giving the representative the right (payee *estate*).
    pub right_document: Option<String>,
}

impl AmountsInput {
    fn figures(&self) -> [i64; 9] {
        [
            self.unpaid_salary,
            self.leave_encashment,
            self.gratuity,
            self.notice_pay,
            self.other_dues,
            self.advance_recoveries,
            self.fine_recoveries,
            self.court_debt_recoveries,
            self.damage_recoveries,
        ]
    }
}

fn clean(text: &Option<String>) -> Option<String> {
    text.as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

/// `by` (a holder of `settlements.prepare`) enters the amounts of settlement
/// `id`, which is `draft` (the preparer) or `disputed` (the revised amounts).
/// The offset within the caps is worked out again.
///
/// # Errors
///
/// `Forbidden` without `settlements.prepare`; 422 in another state or for a
/// negative figure.
pub async fn set_amounts(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    input: AmountsInput,
) -> Result<FinalSettlement> {
    if !by.has_permission(permissions::SETTLEMENTS_PREPARE) {
        return Err(Error::Forbidden);
    }
    if input.figures().iter().any(|figure| *figure < 0) {
        return Err(refuse("wl_hr.settlement.amount_negative"));
    }
    let mut tx = state.db.begin().await?;
    let settlement = FinalSettlement::load_in(&mut tx, id).await?;
    let revised = match settlement.status {
        SettlementStatus::Draft => false,
        SettlementStatus::Disputed => true,
        _ => return Err(refuse("wl_hr.settlement.amounts_state")),
    };
    FinalSettlement::query()
        .where_(FinalSettlement::ID.eq(id))
        .update(
            &mut tx,
            &[
                (
                    "last_working_day",
                    &input.last_working_day.or(settlement.last_working_day),
                ),
                ("unpaid_salary", &input.unpaid_salary),
                ("leave_encashment", &input.leave_encashment),
                ("gratuity", &input.gratuity),
                ("notice_pay", &input.notice_pay),
                ("other_dues", &input.other_dues),
                ("advance_recoveries", &input.advance_recoveries),
                ("fine_recoveries", &input.fine_recoveries),
                ("court_debt_recoveries", &input.court_debt_recoveries),
                ("damage_recoveries", &input.damage_recoveries),
                (
                    "representative",
                    &clean(&input.representative).or(settlement.representative.clone()),
                ),
                (
                    "right_document",
                    &clean(&input.right_document).or(settlement.right_document.clone()),
                ),
                (
                    "prepared_by",
                    &if revised {
                        settlement.prepared_by
                    } else {
                        Some(by.id)
                    },
                ),
                (
                    "revised_by",
                    &if revised {
                        Some(by.id)
                    } else {
                        settlement.revised_by
                    },
                ),
                ("updated_at", &now()),
            ],
        )
        .await?;
    let saved = recalculate_in(&mut tx, id).await?;
    tx.commit().await?;
    renox::audit::record(
        &state.db,
        renox::audit::Entry::new(if revised {
            "hr.settlement_amounts_revised"
        } else {
            "hr.settlement_amounts_entered"
        })
        .user(by.id)
        .subject(FinalSettlement::TABLE, id)
        .data(json!({
            "dues": saved.dues(),
            "recoveries": saved.recoveries_total(),
            "offset": saved.recoveries_offset,
            "net": saved.net,
            "owed_by_employee": saved.owed_by_employee,
            "estate": saved.payee == PAYEE_ESTATE,
        })),
    )
    .await?;
    Ok(saved)
}
