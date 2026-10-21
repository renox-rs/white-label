//! An omission (02 Construction §4.5 row "An omission covers work already
//! done", §7.1 *Variation* `submit` and `approve`; BL-CON-12): a variation of
//! kind `omission` takes a quantity out of a BOQ line ([`crate::boq`]).
//!
//! - **`submit`** is refused when the omitted quantity is above the quantity
//!   not yet done on the line, taken from the latest recorded progress
//!   ([`crate::boq::progress_ipa`]); work already done stays in the valuation
//!   ([`submit_problem`]). The facts are read by [`prepare_submit`], because a
//!   guard cannot look them up.
//! - The price of an omission is negative. Without a price the omitted
//!   quantity at the line's rate is the price ([`priced`]).
//! - **`approve`** is allowed whatever the progress is since (the client
//!   decided), but when progress has overtaken the omission the excess is
//!   recorded on the variation and the QS is warned to record it as a claim
//!   ([`warn_after_approval`]).

use renox::db::Db;
use renox::prelude::*;
use serde::Serialize;

use crate::boq::{self, BoqLine};
use crate::notices::{holders, tell};
use crate::permissions;
use crate::project::Project;
use crate::texts::english;
use crate::variation::{NewVariation, Variation, VariationInput, VariationKind};

/// Why the omission `variation` cannot be submitted with `input`, if it
/// cannot: it names a BOQ line and a quantity above 0, and the quantity is not
/// above the quantity not yet done on the line. Other kinds are not checked.
pub fn submit_problem(variation: &Variation, input: &VariationInput) -> Option<String> {
    if variation.kind != VariationKind::Omission {
        return None;
    }
    let omitted = variation.omitted_quantity.filter(|quantity| *quantity > 0);
    if variation.boq_line_id.is_none() || omitted.is_none() {
        return Some(english("construction.variation.omission_line_needed", &[]));
    }
    let (Some(omitted), Some(left)) = (omitted, input.not_yet_done) else {
        return Some(english("construction.bad_input", &[]));
    };
    if omitted > left {
        return Some(english(
            "construction.variation.omission_above",
            &[("omitted", &omitted), ("left", &left)],
        ));
    }
    None
}

/// The input of `submit` for `variation`: for an omission the quantity not
/// yet done on its line, read from the latest recorded progress.
///
/// # Errors
///
/// A database error.
pub async fn prepare_submit(db: &Db, variation: &Variation) -> Result<VariationInput> {
    let mut input = VariationInput::default();
    if variation.kind == VariationKind::Omission
        && let Some(id) = variation.boq_line_id
        && let Some(line) = BoqLine::find(db, id).await?
    {
        input.not_yet_done = Some(boq::not_yet_done(db, &line).await?);
    }
    Ok(input)
}

/// `new` with the price of an omission that gave none: the omitted quantity
/// at the rate of its BOQ line.
///
/// # Errors
///
/// A database error.
pub async fn priced(db: &Db, new: &NewVariation) -> Result<NewVariation> {
    let mut priced = new.clone();
    if new.kind == VariationKind::Omission
        && new.priced_amount == 0
        && let (Some(id), Some(quantity)) = (new.boq_line_id, new.omitted_quantity)
        && let Some(line) = BoqLine::find(db, id).await?
    {
        priced.priced_amount = quantity * line.rate;
    }
    Ok(priced)
}

/// How much of the omission `variation` is above the quantity not yet done
/// now, 0 when none or when it is not an omission.
///
/// # Errors
///
/// A database error.
pub async fn excess_of(db: &Db, variation: &Variation) -> Result<i64> {
    if variation.kind != VariationKind::Omission {
        return Ok(0);
    }
    let (Some(id), Some(omitted)) = (variation.boq_line_id, variation.omitted_quantity) else {
        return Ok(0);
    };
    let Some(line) = BoqLine::find(db, id).await? else {
        return Ok(0);
    };
    let left = boq::not_yet_done(db, &line).await?;
    Ok((omitted - left).max(0))
}

/// What [`warn_after_approval`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct Warned {
    /// The quantity of the omission that progress has overtaken.
    pub excess: i64,
    /// Whether the QS was told.
    pub told: bool,
}

/// After `approve` of variation `id`: when progress has overtaken the
/// omission, the excess is recorded on the variation and the QS is warned to
/// record it as a claim. The approval stands.
///
/// # Errors
///
/// A database error and the notifier's errors.
pub async fn warn_after_approval(state: &AppState, id: i64) -> Result<Warned> {
    let variation = Variation::find_or_404(&state.db, id).await?;
    let excess = excess_of(&state.db, &variation).await?;
    if excess == 0 {
        return Ok(Warned::default());
    }
    Variation::query()
        .where_(Variation::ID.eq(id))
        .update(&state.db, &[("excess_quantity", &excess)])
        .await?;
    let project = Project::find_or_404(&state.db, variation.project_id).await?;
    let label = variation.label();
    let unit = match variation.boq_line_id {
        Some(line) => BoqLine::find(&state.db, line)
            .await?
            .map(|found| found.unit)
            .unwrap_or_default(),
        None => String::new(),
    };
    let title = english(
        "construction.variation.excess_title",
        &[("label", &label), ("project", &project.name)],
    );
    let body = english(
        "construction.variation.excess_body",
        &[("excess", &excess), ("unit", &unit)],
    );
    let users = holders(state, &project, permissions::QS_ROLE).await?;
    let told = tell(
        state,
        variation.subject(),
        &project.code,
        &users,
        &title,
        &body,
    )
    .await?;
    Ok(Warned { excess, told })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn omission(line: Option<i64>, quantity: Option<i64>) -> Variation {
        Variation {
            kind: VariationKind::Omission,
            boq_line_id: line,
            omitted_quantity: quantity,
            ..Variation::default()
        }
    }

    fn left(quantity: i64) -> VariationInput {
        VariationInput {
            not_yet_done: Some(quantity),
            ..VariationInput::default()
        }
    }

    #[test]
    fn an_omission_cannot_omit_more_than_is_not_yet_done() {
        let three_hundred = omission(Some(1), Some(300));
        assert_eq!(submit_problem(&three_hundred, &left(300)), None);
        assert_eq!(submit_problem(&three_hundred, &left(450)), None);
        let four_hundred = omission(Some(1), Some(400));
        let problem = submit_problem(&four_hundred, &left(300));
        assert!(problem.is_some_and(|p| p.contains("400") && p.contains("300")));
    }

    #[test]
    fn an_omission_names_its_line_and_a_quantity() {
        assert!(submit_problem(&omission(None, Some(5)), &left(10)).is_some());
        assert!(submit_problem(&omission(Some(1), None), &left(10)).is_some());
        assert!(submit_problem(&omission(Some(1), Some(0)), &left(10)).is_some());
        // The facts were not read.
        assert!(submit_problem(&omission(Some(1), Some(5)), &VariationInput::default()).is_some());
    }

    #[test]
    fn other_kinds_are_not_checked() {
        for kind in [VariationKind::Addition, VariationKind::Time] {
            let variation = Variation {
                kind,
                ..Variation::default()
            };
            assert_eq!(submit_problem(&variation, &VariationInput::default()), None);
        }
    }
}
