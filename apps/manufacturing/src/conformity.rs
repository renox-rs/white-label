//! Product conformity: regulated items leave the plant only with a valid
//! certificate, and *export only* items are not sold for domestic delivery
//! (07 Manufacturing §2, §3.3 `MFG.ACL.11`, §7.1 *Dispatch* row `pick`, §9
//! *Product conformity*; BL-MFG-11, #392; wl-documents `DOC.RUL.02`).
//!
//! **The flags.** An item is *regulated* (low-voltage electrical goods,
//! lighting, toys, paints, food-contact items), *export only* (single-use
//! plastics) or neither ([`ItemFlag`], set per item at onboarding by a holder
//! of `conformity.manage`, C-02). A regulated item names the certificate
//! kinds that count for it, its **accepted kinds**: the ECAS certificate, the
//! Emirates Quality Mark licence and the food-contact certificate; none named
//! means all three.
//!
//! **The block.** The document kinds list `dispatch.pick` and
//! `dispatch.dispatch` in their `blocks`, with the override `never`
//! (wl-documents; the host seeds them). This module registers what
//! wl-documents asks the guarded module for:
//!
//! - [`DispatchSubjects`], the `BlockSubjects` of both steps: a dispatch is
//!   the distinct regulated items on its lines, **for a kind only while no
//!   accepted kind of the item is valid today** - so any one valid kind among
//!   the accepted ones passes, the same kind expiring on the pick date still
//!   passes (a certificate is valid through its expiry date), a renewal that
//!   still waits for review does not (the refusal says *renewal pending
//!   review since* its date and the renewal goes to the top of the review
//!   queue), and an industrial licence or an ICV certificate blocks nothing
//!   because no kind of it lists the step;
//! - [`PlannedShipments`], the `FutureWork` of both steps (`DOC.RUL.02`, the
//!   nightly run at 01:00 Asia/Dubai): the confirmed sales order lines of the
//!   item, not yet picked, promised after the certificate expires, unless
//!   another accepted kind of the item is valid on the promised day. Sales
//!   and the planner get one list, and later runs list only new lines
//!   (wl-documents).
//!
//! **The flags of export.** [`ItemRules`] is the app's `DispatchGuard` (at
//! `pick` and again at `dispatch` an *export only* item on a dispatch of an
//! order delivered in the UAE is refused) and its `OrderLineRules` (the same
//! rule for a sales order line at `submit` and `amend`, which wl-sales asks).
//! A destination the sales module does not tell is not refused at the pick: a
//! deployment without the answer cannot prove the delivery domestic.
//!
//! Decisions: the date a certificate is judged on is today in Asia/Dubai (the
//! pick and the dispatch both run on the day), the refusal of a dispatch
//! leaves it as it is and the storekeeper can `cancel` it; no override is
//! offered for a conformity kind; which products are regulated is the
//! client's onboarding (C-02), not a rule here.

use renox::auth::permissions::Scope;
use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::documents::{
    BlockEntry, BlockSubjects, DocumentFuture, FutureWork, PlannedRecord, guard,
    register_block_subjects, register_future_work, subject_text, today,
};
use wl_kernel::inventory::{
    DispatchFacts, DispatchGuard, ITEM_SUBJECT_TYPE, TraceFuture, dispatch_directory,
};
use wl_kernel::sales::{OrderLineRules, SalesFuture, delivery_destination, promised_lines};

use crate::db::refused;
use crate::permissions;
use crate::texts::english;

/// The ECAS certificate (low-voltage electrical goods, lighting; one year).
pub const ECAS: &str = "ecas_certificate";
/// The Emirates Quality Mark licence (three years).
pub const EQM: &str = "eqm_licence";
/// The food-contact certificate.
pub const FOOD_CONTACT: &str = "food_contact_certificate";
/// The conformity kinds, in the order they are offered.
pub const KINDS: [&str; 3] = [ECAS, EQM, FOOD_CONTACT];

/// The guarded step `pick` of a dispatch.
pub const PICK: &str = "dispatch.pick";
/// The guarded step `dispatch` of a dispatch.
pub const DISPATCH: &str = "dispatch.dispatch";

/// The flags of an item.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "manufacturing_item_flags", unique(item_id))]
pub struct ItemFlag {
    /// The primary key.
    pub id: i64,
    /// The item, a wl-inventory item.
    pub item_id: i64,
    /// The item needs a conformity certificate to leave the plant.
    pub regulated: bool,
    /// The item is not sold for domestic delivery.
    pub export_only: bool,
    /// The certificate kinds that count, comma separated; empty: all three.
    pub accepted_kinds: String,
    /// The user who set the flags.
    pub set_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ItemFlag {
    /// The flags of `item_id`; `None` when the item has none.
    ///
    /// # Errors
    ///
    /// A database error.
    pub async fn of(db: &Db, item_id: i64) -> Result<Option<ItemFlag>> {
        ItemFlag::query()
            .where_(ItemFlag::ITEM_ID.eq(item_id))
            .first(db)
            .await
    }

    /// The kinds that count for the item: the ones it names, else all three.
    pub fn accepted(&self) -> Vec<String> {
        let named: Vec<String> = self
            .accepted_kinds
            .split(',')
            .map(str::trim)
            .filter(|kind| KINDS.contains(kind))
            .map(str::to_owned)
            .collect();
        if named.is_empty() {
            KINDS.iter().map(|kind| (*kind).to_owned()).collect()
        } else {
            named
        }
    }

    /// The item as a subject.
    pub fn subject(&self) -> Subject {
        Subject::new(ITEM_SUBJECT_TYPE, self.item_id)
    }
}

/// What `by` sets on an item.
#[derive(Debug, Clone, Default)]
pub struct FlagAsk {
    /// Regulated.
    pub regulated: bool,
    /// Export only.
    pub export_only: bool,
    /// The kinds that count; empty for all three.
    pub kinds: Vec<String>,
}

/// Whether `user` keeps the flags.
pub fn may_flag(user: &AuthUser) -> bool {
    user.has_permission(permissions::CONFORMITY_MANAGE)
}

/// Sets the flags of `item_id` as `by`.
///
/// # Errors
///
/// `Forbidden` without `conformity.manage`; a 422 for a kind that is not a
/// conformity kind.
pub async fn set_flags(
    state: &AppState,
    by: &AuthUser,
    item_id: i64,
    ask: FlagAsk,
) -> Result<ItemFlag> {
    if !may_flag(by) {
        return Err(Error::Forbidden);
    }
    if let Some(unknown) = ask
        .kinds
        .iter()
        .find(|kind| !KINDS.contains(&kind.as_str()))
    {
        return Err(refused(english(
            "manufacturing.conformity.kind",
            &[("kind", unknown)],
        )));
    }
    let kinds = ask.kinds.join(",");
    let now = renox::db::now();
    match ItemFlag::of(&state.db, item_id).await? {
        Some(found) => {
            ItemFlag::query()
                .where_(ItemFlag::ID.eq(found.id))
                .update(
                    &state.db,
                    &[
                        ("regulated", &ask.regulated),
                        ("export_only", &ask.export_only),
                        ("accepted_kinds", &kinds),
                        ("set_by", &Some(by.id)),
                        ("updated_at", &now),
                    ],
                )
                .await?;
        }
        None => {
            ItemFlag::create(
                &state.db,
                ItemFlag {
                    item_id,
                    regulated: ask.regulated,
                    export_only: ask.export_only,
                    accepted_kinds: kinds,
                    set_by: Some(by.id),
                    ..ItemFlag::default()
                },
            )
            .await?;
        }
    }
    ItemFlag::of(&state.db, item_id)
        .await?
        .ok_or(Error::NotFound)
}

/// Whether an accepted kind of `flag` is valid for the item on `day`.
async fn covered(state: &AppState, flag: &ItemFlag, day: NaiveDate) -> Result<bool> {
    for kind in flag.accepted() {
        if guard(state)
            .is_valid(&state.db, &flag.subject(), &kind, day)
            .await?
            .is_valid()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The distinct items on the lines of `facts`.
fn items_of(facts: &DispatchFacts) -> Vec<Subject> {
    let mut items: Vec<Subject> = Vec::new();
    for line in &facts.lines {
        if !items.contains(&line.item) {
            items.push(line.item.clone());
        }
    }
    items
}

/// Maps a dispatch to the regulated items whose certificate must be valid.
#[derive(Debug, Clone, Copy, Default)]
pub struct DispatchSubjects;

impl BlockSubjects for DispatchSubjects {
    fn subjects<'a>(
        &'a self,
        state: &'a AppState,
        entry: &'a BlockEntry,
        record_id: i64,
    ) -> DocumentFuture<'a, Vec<Subject>> {
        Box::pin(async move {
            let dispatch = Subject::new(wl_kernel::inventory::DISPATCH_SUBJECT_TYPE, record_id);
            let Some(facts) = dispatch_directory(state).facts(state, &dispatch).await? else {
                return Ok(Vec::new());
            };
            let day = today();
            let mut found = Vec::new();
            for item in items_of(&facts) {
                let Some(flag) = ItemFlag::of(&state.db, item.subject_id).await? else {
                    continue;
                };
                if flag.regulated
                    && flag.accepted().contains(&entry.kind)
                    && !covered(state, &flag, day).await?
                {
                    found.push(item);
                }
            }
            Ok(found)
        })
    }

    fn label<'a>(
        &'a self,
        state: &'a AppState,
        subject: &'a Subject,
    ) -> DocumentFuture<'a, Option<String>> {
        Box::pin(async move { Ok(Some(subject_text(&state.db, subject).await?)) })
    }

    fn scope<'a>(
        &'a self,
        _state: &'a AppState,
        _entry: &'a BlockEntry,
        _record_id: i64,
    ) -> DocumentFuture<'a, Option<Scope>> {
        Box::pin(async { Ok(None) })
    }
}

/// The sales order lines promised after a certificate expires (`DOC.RUL.02`).
#[derive(Debug, Clone, Copy, Default)]
pub struct PlannedShipments;

impl FutureWork for PlannedShipments {
    fn planned_from<'a>(
        &'a self,
        state: &'a AppState,
        _entry: &'a BlockEntry,
        subject: &'a Subject,
        from: NaiveDate,
    ) -> DocumentFuture<'a, Vec<PlannedRecord>> {
        Box::pin(async move {
            let Some(flag) = ItemFlag::of(&state.db, subject.subject_id).await? else {
                return Ok(Vec::new());
            };
            if !flag.regulated {
                return Ok(Vec::new());
            }
            let mut found = Vec::new();
            for line in promised_lines(state).of_item(state, subject, from).await? {
                // Another accepted kind that is valid on the promised day
                // covers the line.
                if covered(state, &flag, line.promised).await? {
                    continue;
                }
                let number = line
                    .number
                    .clone()
                    .unwrap_or_else(|| line.order.0.to_string());
                found.push(PlannedRecord {
                    record: line.order.subject(),
                    date: line.promised,
                    label: english(
                        "manufacturing.conformity.planned_line",
                        &[("order", &number), ("line", &line.line)],
                    ),
                    scope: None,
                });
            }
            Ok(found)
        })
    }
}

/// The words that refuse `item` for a domestic delivery.
async fn export_only_text(state: &AppState, item: &Subject) -> Result<String> {
    let name = subject_text(&state.db, item).await?;
    Ok(english(
        "manufacturing.conformity.export_only",
        &[("item", &name)],
    ))
}

/// The item flags of the app: *export only* items are not delivered in the
/// UAE (`MFG.ACL.11`), at `pick` and `dispatch` of a dispatch and at `submit`
/// and `amend` of an order line.
#[derive(Debug, Clone, Copy, Default)]
pub struct ItemRules;

impl DispatchGuard for ItemRules {
    fn blocked<'a>(
        &'a self,
        state: &'a AppState,
        dispatch: &'a DispatchFacts,
        _guarded: &'a str,
        _on: NaiveDate,
    ) -> TraceFuture<'a, Option<String>> {
        Box::pin(async move {
            let export = delivery_destination(state)
                .is_export(state, &dispatch.source)
                .await?;
            if export != Some(false) {
                return Ok(None);
            }
            for item in items_of(dispatch) {
                if ItemFlag::of(&state.db, item.subject_id)
                    .await?
                    .is_some_and(|flag| flag.export_only)
                {
                    return Ok(Some(export_only_text(state, &item).await?));
                }
            }
            Ok(None)
        })
    }
}

impl OrderLineRules for ItemRules {
    fn refuse_line<'a>(
        &'a self,
        state: &'a AppState,
        item: &'a Subject,
        domestic: bool,
    ) -> SalesFuture<'a, Option<String>> {
        Box::pin(async move {
            if domestic
                && ItemFlag::of(&state.db, item.subject_id)
                    .await?
                    .is_some_and(|flag| flag.export_only)
            {
                return Ok(Some(export_only_text(state, item).await?));
            }
            Ok(None)
        })
    }
}

/// Registers the providers wl-documents and wl-inventory ask for.
pub(crate) fn register(app: &mut Registry) {
    for guarded in [PICK, DISPATCH] {
        register_block_subjects(guarded, DispatchSubjects);
        register_future_work(guarded, PlannedShipments);
    }
    wl_kernel::inventory::provide_dispatch_guard(app, ItemRules);
    wl_kernel::sales::provide_order_line_rules(app, ItemRules);
}
