//! The KPI pack machine (04 §7.1 *KPI pack*; BL-ACC-20, BL-ACC-21).
//!
//! | Transition | From | To | By |
//! |---|---|---|---|
//! | [`PUBLISH`] | draft | published | `kpi.publish` (manager) |
//! | [`SUPERSEDE`] | published | superseded | the system, after the `publish` of a newer version |
//! | [`WITHDRAW`] | published | withdrawn | `kpi.publish` |
//!
//! The services ([`crate::kpi_packs`]) gather what the guards need into a
//! [`KpiInput`]: the accounts with no KPI category (`publish` names them) and
//! whether the commentary is there when the template requires it. `withdraw`
//! needs one of the two reasons ([`WITHDRAW_REASONS`]). What reaches other
//! modules after the commit (the supersede of the previous version, the
//! client notices, the privacy incident task) runs in a listener
//! ([`crate::kpi_packs::after`]), never in an effect.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;

use crate::kpi::{KpiPack, KpiStatus};
use crate::permissions;
use crate::texts::english;

/// The `publish` transition's name.
pub const PUBLISH: &str = "publish";
/// The `supersede` transition's name.
pub const SUPERSEDE: &str = "supersede";
/// The `withdraw` transition's name.
pub const WITHDRAW: &str = "withdraw";

/// The pack went to the wrong client.
pub const WRONG_CLIENT: &str = "wrong_client";
/// The pack has wrong figures.
pub const WRONG_FIGURES: &str = "wrong_figures";
/// The reasons a published pack is withdrawn with.
pub const WITHDRAW_REASONS: [&str; 2] = [WRONG_CLIENT, WRONG_FIGURES];

/// What a run carries to the guards and effects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KpiInput {
    /// The codes of the accounts with no KPI category (`publish`).
    #[serde(default)]
    pub unmapped: Vec<String>,
    /// Whether the commentary is there, or the template does not require it
    /// (`publish`).
    #[serde(default)]
    pub commentary_ok: bool,
    /// The reason of `withdraw`.
    #[serde(default)]
    pub reason: Option<String>,
    /// The business day of the run (Asia/Dubai); today when empty.
    #[serde(default)]
    pub on: Option<NaiveDate>,
}

/// The input of a run; the default without one (`available`).
fn input_of(ctx: &TransitionCtx) -> KpiInput {
    ctx.input_as::<KpiInput>().unwrap_or_default()
}

impl StateMachine for KpiPack {
    type State = KpiStatus;
    const COLUMN: renox::db::Col<KpiPack, KpiStatus> = KpiPack::STATUS;

    fn machine() -> Machine<Self> {
        use KpiStatus::{Draft, Published, Superseded, Withdrawn};
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Published)
            .tone("success")
            .state(Superseded)
            .tone("neutral")
            .terminal()
            .state(Withdrawn)
            .tone("error")
            .terminal()
            .transition(publish())
            .transition(supersede())
            .transition(withdraw())
    }
}

/// `publish`: every account is mapped and the commentary is there; the day
/// and the user are kept. The previous version is superseded in the same
/// transaction and the client told after the commit.
fn publish() -> Transition<KpiPack> {
    Transition::new(PUBLISH)
        .label("Publish")
        .tone("primary")
        .permission(permissions::KPI_PUBLISH)
        .from([KpiStatus::Draft])
        .to(KpiStatus::Published)
        .guard("all_mapped", |_p: &KpiPack, ctx| {
            let unmapped = input_of(ctx).unmapped;
            Box::pin(async move {
                if unmapped.is_empty() {
                    Ok(())
                } else {
                    Err(english(
                        "accounting.kpi.problem.unmapped",
                        &[("accounts", &unmapped.join(", "))],
                    ))
                }
            })
        })
        .guard("commentary_given", |_p: &KpiPack, ctx| {
            let given = input_of(ctx).commentary_ok;
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err(english("accounting.kpi.problem.commentary", &[]))
                }
            })
        })
        .effect(|p: &KpiPack, ctx, tx| {
            let id = p.id;
            let client = p.client_party_id;
            let period = p.period_end;
            let version = p.version;
            let on = input_of(ctx).on.unwrap_or_else(today);
            let by = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                // Decision: the earlier published version of the period is
                // superseded in the same transaction, so two published
                // versions never coexist (BL-ACC-20).
                let stamp = renox::db::now();
                KpiPack::query()
                    .where_(KpiPack::CLIENT_PARTY_ID.eq(client))
                    .where_(KpiPack::PERIOD_END.eq(period))
                    .where_(KpiPack::STATUS.eq(KpiStatus::Published))
                    .where_(KpiPack::VERSION.lt(version))
                    .update(
                        &mut *tx,
                        &[
                            (
                                "status",
                                &KpiStatus::Superseded as &(dyn renox::db::ToDbValue + Sync),
                            ),
                            ("updated_at", &stamp),
                        ],
                    )
                    .await?;
                crate::db::set_kpi_pack(
                    &mut *tx,
                    id,
                    &[("published_on", &Some(on)), ("published_by", &by)],
                )
                .await
            })
        })
}

/// `supersede`: the system, when a newer version of the period is published.
fn supersede() -> Transition<KpiPack> {
    Transition::new(SUPERSEDE)
        .label("Supersede")
        .system()
        .from([KpiStatus::Published])
        .to(KpiStatus::Superseded)
}

/// `withdraw`: a reason (wrong client, wrong figures) is given; the pack
/// leaves the portal at once and stays readable by staff.
fn withdraw() -> Transition<KpiPack> {
    Transition::new(WITHDRAW)
        .label("Withdraw")
        .tone("danger")
        .permission(permissions::KPI_PUBLISH)
        .from([KpiStatus::Published])
        .to(KpiStatus::Withdrawn)
        .guard("reason_given", |_p: &KpiPack, ctx| {
            let reason = input_of(ctx).reason;
            let valid = reason
                .as_deref()
                .is_some_and(|r| WITHDRAW_REASONS.contains(&r));
            Box::pin(async move {
                if valid {
                    Ok(())
                } else {
                    Err(english("accounting.kpi.problem.reason", &[]))
                }
            })
        })
        .effect(|p: &KpiPack, ctx, tx| {
            let id = p.id;
            let input = input_of(ctx);
            let on = input.on.unwrap_or_else(today);
            let by = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                crate::db::set_kpi_pack(
                    &mut *tx,
                    id,
                    &[
                        ("withdraw_reason", &input.reason),
                        ("withdrawn_on", &Some(on)),
                        ("withdrawn_by", &by),
                    ],
                )
                .await
            })
        })
}

/// Registers the machine for `rnx state:diagram`. Decision: no approval
/// policy; `kpi.publish` holders act directly.
pub(crate) fn register() {
    renox::state_machine::register::<KpiPack>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_every_state_and_transition() {
        let machine = KpiPack::machine();
        let states: Vec<(&str, Option<&str>, bool)> = machine
            .states()
            .iter()
            .map(|s| (s.name.as_str(), s.tone.as_deref(), s.terminal))
            .collect();
        assert_eq!(
            states,
            [
                ("draft", Some("neutral"), false),
                ("published", Some("success"), false),
                ("superseded", Some("neutral"), true),
                ("withdrawn", Some("error"), true),
            ]
        );
        let expected = [
            (PUBLISH, "draft", "published", false),
            (SUPERSEDE, "published", "superseded", true),
            (WITHDRAW, "published", "withdrawn", false),
        ];
        assert_eq!(machine.transitions().len(), expected.len());
        for (name, from, to, system) in expected {
            let transition = machine.find(name).unwrap();
            assert_eq!(transition.from_states(), [from], "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert_eq!(transition.is_system(), system, "{name}");
        }
        assert_eq!(
            machine.find(PUBLISH).unwrap().guard_names(),
            ["all_mapped", "commentary_given"]
        );
        assert_eq!(
            machine.find(WITHDRAW).unwrap().guard_names(),
            ["reason_given"]
        );
    }
}
