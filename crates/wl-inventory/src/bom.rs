//! The Bill of materials (wl-inventory §4 *BOM*, `STK.BOM.01`; 07
//! Manufacturing §6 and §7.1 *BOM*): the components that make an item, in
//! versions.
//!
//! ```text
//! [*] --create--> draft --release--> released --obsolete--> obsolete
//! ```
//!
//! - A version belongs to one finished item and is numbered 1, 2, 3 ... per
//!   item. It makes `output_milli` of the item (1000 = one unit) at a
//!   `yield_bps` (10000 = every unit made is good) and takes
//!   `standard_milli_hours` of line time for that output (`MFG.CST.01`).
//! - A line is a component item, the quantity for the output quantity and a
//!   scrap allowance in basis points.
//! - A draft is edited; a released version is locked and a change is a new
//!   version ([`new_version`], `MFG.ACL.06`).
//! - `release` (`boms.release`): needs a line, every quantity above zero,
//!   components that exist and none that is the item made. Its effect rolls
//!   the cost of the materials of one unit up at the average cost of each
//!   component and makes the version the one in force; [`release`] then
//!   sends the released version it replaces to `obsolete`.
//! - `obsolete` (`boms.release`, or the system as `auto_obsolete`).
//!
//! The work order of 07 reads a version through the kernel's
//! [`BillsOfMaterials`] ([`InventoryBoms`]): the components of one unit
//! (the yield folded into the quantity, the scrap kept apart), the material
//! cost of one unit and the standard hours of one unit.
//!
//! Decisions where the specs are silent:
//!
//! - the BOM is shared master data without a scope: `boms.view`,
//!   `boms.manage` and `boms.release` count wherever the user holds them, so
//!   a production manager assigned to a plant can keep BOMs;
//! - only one draft per item at a time, so two people do not branch the next
//!   version;
//! - no sub-BOMs and no routing (the work order needs the standard hours
//!   only); a component may not be the item made, the only circular BOM
//!   there can be without sub-BOMs;
//! - the older released version becomes `obsolete` right after `release`, in
//!   its own transition; a work order already made from it goes on.

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::production::{
    BOM_SUBJECT_TYPE, BillsOfMaterials, BomComponent, BomVersion, ProductionFuture,
};

use crate::item::Item;
use crate::permissions;
use crate::texts::said;

/// The `release` transition's name.
pub const RELEASE: &str = "release";

/// The `obsolete` transition's name.
pub const OBSOLETE: &str = "obsolete";

/// The system's `obsolete`.
pub const AUTO_OBSOLETE: &str = "auto_obsolete";

/// Where a version stands (§4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BomStatus {
    /// Being defined; editable.
    #[default]
    Draft,
    /// In force; locked.
    Released,
    /// Replaced by a newer version (terminal).
    Obsolete,
}

/// One version of the bill of materials of an item.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_boms",
    state_machine,
    unique(item_id, version),
    index(status)
)]
pub struct Bom {
    /// The primary key.
    pub id: i64,
    /// The finished item.
    pub item_id: i64,
    /// The version of the item, from 1.
    pub version: i64,
    /// The lifecycle state; changed only by a transition.
    pub status: BomStatus,
    /// The quantity the components make, in thousandths.
    pub output_milli: i64,
    /// The yield in basis points.
    pub yield_bps: i64,
    /// The standard hours of the output quantity, in thousandths of an hour.
    pub standard_milli_hours: i64,
    /// The materials of one unit at release, in fils.
    pub material_cost_per_unit: i64,
    /// The user who released it.
    pub released_by: Option<i64>,
    /// When it was released.
    pub released_at: Option<DateTime>,
    /// When it became obsolete.
    pub obsoleted_at: Option<DateTime>,
    /// The user who made it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Bom {
    /// The version as a [`Subject`]: `inventory.bom:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(BOM_SUBJECT_TYPE, self.id)
    }
}

/// One component of a version. Quantities are thousandths of the base unit.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_inventory_bom_lines", unique(bom_id, position))]
pub struct BomLine {
    /// The primary key.
    pub id: i64,
    /// The version.
    pub bom_id: i64,
    /// The order of the line, from 1.
    pub position: i64,
    /// The component item.
    pub item_id: i64,
    /// The quantity for the output quantity of the version.
    pub quantity_milli: i64,
    /// The scrap allowance in basis points.
    pub scrap_bps: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// A user who keeps BOMs anywhere.
fn may_manage(user: &AuthUser) -> bool {
    user.has_permission_anywhere(permissions::BOMS_MANAGE)
}

/// A user who releases BOMs anywhere.
pub fn may_release(user: &AuthUser) -> bool {
    user.has_permission_anywhere(permissions::BOMS_RELEASE)
}

/// Whether `user` keeps (edits, makes) BOMs.
pub fn can_manage(user: &AuthUser) -> bool {
    may_manage(user)
}

/// Whether `user` may see BOMs.
pub fn can_view(user: &AuthUser) -> bool {
    [
        permissions::BOMS_VIEW,
        permissions::BOMS_MANAGE,
        permissions::BOMS_RELEASE,
    ]
    .iter()
    .any(|permission| user.has_permission_anywhere(permission))
}

/// The lines of version `bom_id`, in order.
///
/// # Errors
///
/// A database error.
pub async fn lines_of<'c>(db: impl renox::db::Executor<'c>, bom_id: i64) -> Result<Vec<BomLine>> {
    BomLine::query()
        .where_(BomLine::BOM_ID.eq(bom_id))
        .order_by(BomLine::POSITION)
        .get(db)
        .await
}

/// A version to prepare as a draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewBom {
    /// The finished item.
    pub item_id: i64,
    /// The quantity the components make, in thousandths (default 1000).
    pub output_milli: i64,
    /// The yield in basis points (default 10000).
    pub yield_bps: i64,
    /// The standard hours of the output quantity, in thousandths of an hour.
    pub standard_milli_hours: i64,
}

impl NewBom {
    /// A version of `item_id` that makes one unit at a full yield.
    pub fn of(item_id: i64) -> Self {
        NewBom {
            item_id,
            output_milli: 1_000,
            yield_bps: 10_000,
            standard_milli_hours: 0,
        }
    }
}

/// Checks the numbers of a header.
fn check_header(output_milli: i64, yield_bps: i64, hours: i64) -> Result {
    if output_milli <= 0 {
        return Err(refused(said("wl_inventory.bom.bad_output", &[])));
    }
    if !(1..=10_000).contains(&yield_bps) {
        return Err(refused(said("wl_inventory.bom.bad_yield", &[])));
    }
    if hours < 0 {
        return Err(refused(said("wl_inventory.bom.bad_hours", &[])));
    }
    Ok(())
}

/// Prepares a draft version for `new.item_id`, the next number of the item.
///
/// # Errors
///
/// `Forbidden` without `boms.manage`; 422 for an unknown item, a header out
/// of range or an item that has a draft already.
pub async fn create_draft(db: &Db, by: &AuthUser, new: NewBom) -> Result<Bom> {
    if !may_manage(by) {
        return Err(Error::Forbidden);
    }
    check_header(new.output_milli, new.yield_bps, new.standard_milli_hours)?;
    let item = Item::find(db, new.item_id)
        .await?
        .ok_or_else(|| refused(said("wl_inventory.bom.unknown_item", &[])))?;
    open_draft(db, by.id, &item, &new, &[]).await
}

/// Makes the draft of `item` with `lines` (item, quantity, scrap).
async fn open_draft(
    db: &Db,
    by: i64,
    item: &Item,
    new: &NewBom,
    lines: &[(i64, i64, i64)],
) -> Result<Bom> {
    let mut tx = db.begin().await?;
    let versions = Bom::query()
        .where_(Bom::ITEM_ID.eq(item.id))
        .order_by_desc(Bom::VERSION)
        .get(&mut tx)
        .await?;
    if versions.iter().any(|bom| bom.status == BomStatus::Draft) {
        return Err(refused(said(
            "wl_inventory.bom.draft_exists",
            &[("item", &item.code)],
        )));
    }
    let mut bom = Bom {
        item_id: item.id,
        version: versions.first().map_or(1, |bom| bom.version + 1),
        status: BomStatus::Draft,
        output_milli: new.output_milli,
        yield_bps: new.yield_bps,
        standard_milli_hours: new.standard_milli_hours,
        created_by: Some(by),
        ..Bom::default()
    };
    bom.insert(&mut tx).await?;
    for (position, (component, quantity_milli, scrap_bps)) in (1..).zip(lines) {
        BomLine {
            bom_id: bom.id,
            position,
            item_id: *component,
            quantity_milli: *quantity_milli,
            scrap_bps: *scrap_bps,
            ..BomLine::default()
        }
        .insert(&mut tx)
        .await?;
    }
    tx.commit().await?;
    Ok(bom)
}

/// A new draft from version `from_id` (any state): the same header and
/// lines, the next version number. This is how a released version is
/// changed (`MFG.ACL.06`).
///
/// # Errors
///
/// `Forbidden` without `boms.manage`; 422 when the item has a draft.
pub async fn new_version(db: &Db, by: &AuthUser, from_id: i64) -> Result<Bom> {
    if !may_manage(by) {
        return Err(Error::Forbidden);
    }
    let from = Bom::find_or_404(db, from_id).await?;
    let item = Item::find_or_404(db, from.item_id).await?;
    let lines: Vec<(i64, i64, i64)> = lines_of(db, from.id)
        .await?
        .iter()
        .map(|line| (line.item_id, line.quantity_milli, line.scrap_bps))
        .collect();
    let new = NewBom {
        item_id: from.item_id,
        output_milli: from.output_milli,
        yield_bps: from.yield_bps,
        standard_milli_hours: from.standard_milli_hours,
    };
    open_draft(db, by.id, &item, &new, &lines).await
}

/// The draft `id`, or a 422 once it is locked.
async fn editable(db: &Db, id: i64) -> Result<Bom> {
    let bom = Bom::find_or_404(db, id).await?;
    if bom.status != BomStatus::Draft {
        return Err(refused(said("wl_inventory.bom.locked", &[])));
    }
    Ok(bom)
}

/// Changes the output quantity, the yield and the standard hours of draft
/// `id`.
///
/// # Errors
///
/// `Forbidden` without `boms.manage`; 422 for a number out of range or a
/// version that is not a draft.
pub async fn update_draft(
    db: &Db,
    by: &AuthUser,
    id: i64,
    output_milli: i64,
    yield_bps: i64,
    standard_milli_hours: i64,
) -> Result<Bom> {
    if !may_manage(by) {
        return Err(Error::Forbidden);
    }
    let bom = editable(db, id).await?;
    check_header(output_milli, yield_bps, standard_milli_hours)?;
    Bom::query()
        .where_(Bom::ID.eq(bom.id))
        .update(
            db,
            &[
                ("output_milli", &output_milli),
                ("yield_bps", &yield_bps),
                ("standard_milli_hours", &standard_milli_hours),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Bom::find_or_404(db, id).await
}

/// Adds a component to draft `id`.
///
/// # Errors
///
/// `Forbidden` without `boms.manage`; 422 for a quantity that is not above
/// zero, a scrap outside 0 to 100%, an unknown component, the item made
/// itself, a component that is a line already or a version that is not a
/// draft.
pub async fn add_line(
    db: &Db,
    by: &AuthUser,
    id: i64,
    item_id: i64,
    quantity_milli: i64,
    scrap_bps: i64,
) -> Result<BomLine> {
    if !may_manage(by) {
        return Err(Error::Forbidden);
    }
    let bom = editable(db, id).await?;
    if quantity_milli <= 0 {
        return Err(refused(said("wl_inventory.bom.bad_quantity", &[])));
    }
    if !(0..=10_000).contains(&scrap_bps) {
        return Err(refused(said("wl_inventory.bom.bad_scrap", &[])));
    }
    let component = Item::find(db, item_id)
        .await?
        .ok_or_else(|| refused(said("wl_inventory.bom.unknown_component", &[])))?;
    if component.id == bom.item_id {
        return Err(refused(said(
            "wl_inventory.bom.own_component",
            &[("item", &component.code)],
        )));
    }
    let lines = lines_of(db, id).await?;
    if lines.iter().any(|line| line.item_id == component.id) {
        return Err(refused(said(
            "wl_inventory.bom.duplicate_component",
            &[("item", &component.code)],
        )));
    }
    let mut line = BomLine {
        bom_id: id,
        position: lines.iter().map(|line| line.position).max().unwrap_or(0) + 1,
        item_id: component.id,
        quantity_milli,
        scrap_bps,
        ..BomLine::default()
    };
    line.insert(db).await?;
    Ok(line)
}

/// Removes line `line_id` from draft `id`.
///
/// # Errors
///
/// `Forbidden` without `boms.manage`; 422 for a version that is not a
/// draft; 404 for a line that is not the version's.
pub async fn remove_line(db: &Db, by: &AuthUser, id: i64, line_id: i64) -> Result {
    if !may_manage(by) {
        return Err(Error::Forbidden);
    }
    editable(db, id).await?;
    let line = BomLine::find_or_404(db, line_id).await?;
    if line.bom_id != id {
        return Err(Error::NotFound);
    }
    BomLine::query()
        .where_(BomLine::ID.eq(line_id))
        .delete(db)
        .await?;
    Ok(())
}

/// The quantity of a component for one unit of the output, net of scrap,
/// with the yield folded in, in thousandths.
pub fn per_unit_quantity(bom: &Bom, line: &BomLine) -> i64 {
    let numerator = i128::from(line.quantity_milli) * 1_000 * 10_000;
    let denominator = i128::from(bom.output_milli) * i128::from(bom.yield_bps);
    i64::try_from((2 * numerator + denominator) / (2 * denominator)).unwrap_or(i64::MAX)
}

/// The cost of the materials of one unit in fils: every component with its
/// scrap at the average cost of its item.
///
/// # Errors
///
/// A database error.
pub async fn material_cost_per_unit(tx: &mut Transaction, bom: &Bom) -> Result<i64> {
    let mut total: i64 = 0;
    for line in lines_of(&mut *tx, bom.id).await? {
        let item = Item::find_or_404(&mut *tx, line.item_id).await?;
        // q x (1 + scrap) x average / (output x yield), rounded half up.
        let numerator = i128::from(line.quantity_milli)
            * i128::from(10_000 + line.scrap_bps)
            * i128::from(item.average_cost);
        let denominator = i128::from(bom.output_milli) * i128::from(bom.yield_bps);
        let cost = (2 * numerator + denominator) / (2 * denominator);
        total = total.saturating_add(i64::try_from(cost).unwrap_or(i64::MAX));
    }
    Ok(total)
}

/// What a `release` run carries to its guard.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReleaseInput {
    /// The user who releases it.
    pub by: Option<i64>,
    /// Every check that fails, one line each.
    pub problems: Vec<String>,
}

/// Every check that fails for releasing `bom`, one line each: no line, a
/// quantity not above zero, a component that does not exist or is the item
/// made.
///
/// # Errors
///
/// A database error.
pub async fn release_problems(db: &Db, bom: &Bom) -> Result<Vec<String>> {
    let mut problems = Vec::new();
    if Item::find(db, bom.item_id).await?.is_none() {
        problems.push(said("wl_inventory.bom.unknown_item", &[]));
    }
    let lines = lines_of(db, bom.id).await?;
    if lines.is_empty() {
        problems.push(said("wl_inventory.bom.no_lines", &[]));
    }
    for line in &lines {
        let position = line.position.to_string();
        if line.quantity_milli <= 0 {
            problems.push(said(
                "wl_inventory.bom.line_quantity",
                &[("position", &position)],
            ));
        }
        match Item::find(db, line.item_id).await? {
            None => problems.push(said(
                "wl_inventory.bom.line_unknown",
                &[("position", &position)],
            )),
            Some(item) if item.id == bom.item_id => problems.push(said(
                "wl_inventory.bom.own_component",
                &[("item", &item.code)],
            )),
            Some(_) => {}
        }
    }
    Ok(problems)
}

fn release_input(ctx: &TransitionCtx) -> Option<ReleaseInput> {
    ctx.input_as::<ReleaseInput>().ok()
}

/// The effect of `release`: the cost is rolled up and the version stamped.
async fn released_in(tx: &mut Transaction, bom: Bom, by: Option<i64>) -> Result {
    let cost = material_cost_per_unit(&mut *tx, &bom).await?;
    let now = renox::db::now();
    Bom::query()
        .where_(Bom::ID.eq(bom.id))
        .update(
            &mut *tx,
            &[
                ("material_cost_per_unit", &cost),
                ("released_by", &by),
                ("released_at", &Some(now)),
                ("updated_at", &now),
            ],
        )
        .await?;
    Ok(())
}

impl StateMachine for Bom {
    type State = BomStatus;
    const COLUMN: renox::db::Col<Bom, BomStatus> = Bom::STATUS;

    fn machine() -> Machine<Self> {
        use BomStatus::{Draft, Obsolete, Released};
        let release = Transition::new(RELEASE)
            .label("Release")
            .tone("primary")
            .from([Draft])
            .to(Released)
            .guard("complete", |_bom: &Bom, ctx| {
                let problems = release_input(ctx).map_or_else(
                    || vec![said("wl_inventory.bom.not_checked", &[])],
                    |input| input.problems,
                );
                Box::pin(async move {
                    if problems.is_empty() {
                        Ok(())
                    } else {
                        Err(problems.join("\n"))
                    }
                })
            })
            .effect(|bom: &Bom, ctx, tx| {
                let bom = bom.clone();
                let by = release_input(ctx).and_then(|input| input.by);
                Box::pin(async move { released_in(tx, bom, by).await })
            });
        let mut machine = Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Released)
            .tone("success")
            .state(Obsolete)
            .tone("neutral")
            .terminal()
            .transition(release);
        for (name, system) in [(OBSOLETE, false), (AUTO_OBSOLETE, true)] {
            let transition = Transition::new(name)
                .label("Make obsolete")
                .from([Released])
                .to(Obsolete)
                .effect(|bom: &Bom, _ctx, tx| {
                    let id = bom.id;
                    Box::pin(async move {
                        let now = renox::db::now();
                        Bom::query()
                            .where_(Bom::ID.eq(id))
                            .update(
                                &mut *tx,
                                &[("obsoleted_at", &Some(now)), ("updated_at", &now)],
                            )
                            .await?;
                        Ok(())
                    })
                });
            machine = machine.transition(if system {
                transition.system()
            } else {
                transition
            });
        }
        machine
    }
}

/// The message of a refused transition, else the transition's own error.
fn answer(error: TransitionError) -> Error {
    match error {
        TransitionError::Guard(message) => refused(message),
        other => other.into_error(),
    }
}

/// `by` releases draft `id`: the cost is rolled up, it is the version in
/// force and the one it replaces becomes obsolete.
///
/// # Errors
///
/// `Forbidden` without `boms.release`; a 422 listing every failing check, or
/// unless it is a draft.
pub async fn release(state: &AppState, by: &AuthUser, id: i64) -> Result<Bom> {
    if !may_release(by) {
        return Err(Error::Forbidden);
    }
    let bom = Bom::find_or_404(&state.db, id).await?;
    let input = ReleaseInput {
        by: Some(by.id),
        problems: if bom.status == BomStatus::Draft {
            release_problems(&state.db, &bom).await?
        } else {
            Vec::new()
        },
    };
    let outcome = bom
        .transition(RELEASE)
        .by(by)
        .input(&input)
        .run(state)
        .await
        .map_err(answer)?;
    outcome.emit(state).await?;
    for older in Bom::query()
        .where_(Bom::ITEM_ID.eq(bom.item_id))
        .where_(Bom::STATUS.eq(BomStatus::Released))
        .where_(Bom::ID.ne(id))
        .get(&state.db)
        .await?
    {
        older
            .transition(AUTO_OBSOLETE)
            .by_system()
            .run(state)
            .await
            .map_err(answer)?
            .emit(state)
            .await?;
    }
    Bom::find_or_404(&state.db, id).await
}

/// `by` makes released version `id` obsolete.
///
/// # Errors
///
/// `Forbidden` without `boms.release`; a 422 unless it is released.
pub async fn make_obsolete(state: &AppState, by: &AuthUser, id: i64) -> Result<Bom> {
    if !may_release(by) {
        return Err(Error::Forbidden);
    }
    let bom = Bom::find_or_404(&state.db, id).await?;
    bom.transition(OBSOLETE)
        .by(by)
        .run(state)
        .await
        .map_err(answer)?
        .emit(state)
        .await?;
    Bom::find_or_404(&state.db, id).await
}

/// The version as the work order reads it (see the module).
///
/// # Errors
///
/// A database error.
pub async fn version_of(db: &Db, id: i64) -> Result<Option<BomVersion>> {
    let Some(bom) = Bom::find(db, id).await? else {
        return Ok(None);
    };
    let components = lines_of(db, bom.id)
        .await?
        .iter()
        .map(|line| BomComponent {
            item: Subject::new(wl_kernel::inventory::ITEM_SUBJECT_TYPE, line.item_id),
            quantity_milli: per_unit_quantity(&bom, line),
            scrap_bps: line.scrap_bps,
        })
        .collect();
    let hours = i128::from(bom.standard_milli_hours) * 1_000;
    let per_unit_hours =
        (2 * hours + i128::from(bom.output_milli)) / (2 * i128::from(bom.output_milli));
    Ok(Some(BomVersion {
        bom: bom.subject(),
        item: Subject::new(wl_kernel::inventory::ITEM_SUBJECT_TYPE, bom.item_id),
        version: bom.version,
        released: bom.status == BomStatus::Released,
        components,
        material_cost_per_unit: bom.material_cost_per_unit,
        standard_milli_hours_per_unit: i64::try_from(per_unit_hours).unwrap_or(0),
    }))
}

/// The kernel's [`BillsOfMaterials`], provided by [`crate::WlInventory`].
#[derive(Debug, Clone, Copy, Default)]
pub struct InventoryBoms;

impl BillsOfMaterials for InventoryBoms {
    fn version<'a>(
        &'a self,
        state: &'a AppState,
        bom: &'a Subject,
    ) -> ProductionFuture<'a, Option<BomVersion>> {
        Box::pin(async move {
            if bom.subject_type != BOM_SUBJECT_TYPE {
                return Ok(None);
            }
            version_of(&state.db, bom.subject_id).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bom_is_drafted_released_and_made_obsolete() {
        let machine = Bom::machine();
        assert_eq!(machine.initial_states(), ["draft"]);
        let release = machine.find(RELEASE).expect("release");
        assert_eq!(release.from_states(), ["draft"]);
        assert_eq!(release.to_state(), Some("released"));
        assert_eq!(release.guard_names(), ["complete"]);
        for name in [OBSOLETE, AUTO_OBSOLETE] {
            let obsolete = machine.find(name).expect("obsolete");
            assert_eq!(obsolete.from_states(), ["released"]);
            assert_eq!(obsolete.to_state(), Some("obsolete"));
        }
    }

    #[test]
    fn the_yield_is_folded_into_the_quantity_of_one_unit() {
        let bom = Bom {
            output_milli: 2_000,
            yield_bps: 9_000,
            ..Bom::default()
        };
        let line = BomLine {
            quantity_milli: 3_600,
            ..BomLine::default()
        };
        // 3.6 kg for 2 units at 90% yield: 2 kg a good unit.
        assert_eq!(per_unit_quantity(&bom, &line), 2_000);
    }
}
