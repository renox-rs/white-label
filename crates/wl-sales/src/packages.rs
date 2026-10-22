//! The package services (wl-sales §4.4, §5.5, §6 `PackageLedger`; base for
//! BL-SAL-16): selling a package, linking its invoice, activating it when the
//! invoice is paid, consuming and reversing credits, extending, cancelling and
//! the daily expiry. Every status change is a transition of
//! [`crate::package_machine`]; the events follow the commit.
//!
//! Decisions:
//! - A package activates when its invoice is *paid in full* (`InvoicePaid`).
//!   Activation on the first instalment ("if allowed") needs the instalment
//!   plan of the invoice and is not built here.
//! - Consuming is idempotent per source: a source that consumed already (and
//!   did not give the credits back) changes nothing.
//! - The ledger takes the package that expires first, then the oldest.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transitioned};
use wl_kernel::invoicing::{InvoiceId, InvoicePaid, invoicer};
use wl_kernel::sales::{
    ConsumePackage, PackageActivated, PackageConsumed, PackageConsumption, PackageExpired,
    PackageExtended, PackageId, PackageLedger, PackageSales, SalesFuture, SellPackage,
};
use wl_kernel::{PartyId, Subject};

use crate::config::config_of;
use crate::package::{MovementKind, Package, PackageMovement, PackageStatus};
use crate::package_machine::{
    ACTIVATE, ActivateInput, CANCEL, CONSUME, CONSUME_LAST, CancelInput, ConsumeInput, EXPIRE,
    EXTEND, ExpireInput, ExtendInput, REVERSE, REVERSE_SYSTEM, ReverseInput,
};
use crate::package_reopen::record_refusal;
use crate::texts::english;

/// The scheduled task, and the command, of the daily expiry.
pub const TASK: &str = "sales:packages";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "00:10";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// A 422 with the English text `key`.
fn refused(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]))
}

/// A package to sell (§5.5 step 1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct NewPackage {
    /// Who uses the credits.
    pub holder: PartyId,
    /// Who pays, when not the holder.
    pub payer: Option<PartyId>,
    /// The code of the catalogue item.
    pub item_code: String,
    /// The name of the item as sold.
    pub item_name: String,
    /// `session`, `hour` or `item`.
    pub unit: String,
    /// The credits it holds.
    pub credits: i64,
    /// The price, in fils.
    pub price: i64,
    /// The months it is valid after activation; `None` for the setting.
    pub validity_months: Option<i64>,
    /// The invoice it is paid with, if drafted already.
    pub invoice: Option<InvoiceId>,
    /// The staff user who sells it.
    pub sold_by: Option<i64>,
}

impl NewPackage {
    /// `credits` sessions of `item_code` for `holder`.
    pub fn new(holder: PartyId, item_code: &str, credits: i64) -> Self {
        NewPackage {
            holder,
            payer: None,
            item_code: item_code.to_owned(),
            item_name: item_code.to_owned(),
            unit: "session".to_owned(),
            credits,
            price: 0,
            validity_months: None,
            invoice: None,
            sold_by: None,
        }
    }

    /// With the item's name as sold.
    pub fn item_name(mut self, name: &str) -> Self {
        self.item_name = name.to_owned();
        self
    }

    /// With the price in fils.
    pub fn price(mut self, fils: i64) -> Self {
        self.price = fils;
        self
    }

    /// Paid by `payer`.
    pub fn payer(mut self, payer: PartyId) -> Self {
        self.payer = Some(payer);
        self
    }

    /// Paid with `invoice`.
    pub fn invoice(mut self, invoice: InvoiceId) -> Self {
        self.invoice = Some(invoice);
        self
    }

    /// Sold by the staff user `user`.
    pub fn sold_by(mut self, user: i64) -> Self {
        self.sold_by = Some(user);
        self
    }

    /// Valid for `months` after activation.
    pub fn validity_months(mut self, months: i64) -> Self {
        self.validity_months = Some(months);
        self
    }
}

/// Records a package as `pending_payment`.
///
/// # Errors
///
/// 422 for fewer than one credit or a negative price.
pub async fn sell(state: &AppState, new: NewPackage) -> Result<Package> {
    if new.credits < 1 {
        return Err(refused("wl_sales.packages.problem.credits_total"));
    }
    if new.price < 0 {
        return Err(refused("wl_sales.packages.problem.price"));
    }
    let months = new
        .validity_months
        .unwrap_or_else(|| config_of(state).package_validity_months);
    if months < 1 {
        return Err(refused("wl_sales.packages.problem.validity"));
    }
    Package::create(
        &state.db,
        Package {
            holder_party_id: new.holder.0,
            payer_party_id: new.payer.map(|p| p.0),
            item_code: new.item_code,
            item_name: new.item_name,
            unit: new.unit,
            credits_total: new.credits,
            price_paid: new.price,
            currency: wl_kernel::invoicing::DEFAULT_CURRENCY.to_owned(),
            validity_months: months,
            invoice_id: new.invoice.map(|i| i.0),
            sold_by: new.sold_by,
            ..Package::default()
        },
    )
    .await
}

/// Links `invoice` to a package still `pending_payment`.
pub async fn link_invoice(state: &AppState, id: i64, invoice: InvoiceId) -> Result<Package> {
    let package = Package::find_or_404(&state.db, id).await?;
    if package.status != PackageStatus::PendingPayment {
        return Err(refused("wl_sales.packages.problem.not_pending"));
    }
    Package::query()
        .where_(Package::ID.eq(id))
        .update(&state.db, &[("invoice_id", &Some(invoice.0))])
        .await?;
    Package::find_or_404(&state.db, id).await
}

/// The package paid with `invoice`, if any.
pub async fn of_invoice(db: &Db, invoice: InvoiceId) -> Result<Option<Package>> {
    Package::query()
        .where_(Package::INVOICE_ID.eq(invoice.0))
        .order_by(Package::ID)
        .first(db)
        .await
}

/// `activate` on `on`: the package is paid. Returns the package as it is
/// after.
pub async fn activate(state: &AppState, id: i64, on: NaiveDate) -> Result<Package> {
    let package = Package::find_or_404(&state.db, id).await?;
    let _ = package
        .transition(ACTIVATE)
        .by_system()
        .input(&ActivateInput { on, paid: true })
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    Package::find_or_404(&state.db, id).await
}

/// The net credits `source` consumed from package `id`: consumed less given
/// back.
async fn consumed_by(db: &Db, id: i64, source: &Subject) -> Result<i64> {
    let movements = PackageMovement::query()
        .where_(PackageMovement::PACKAGE_ID.eq(id))
        .where_(PackageMovement::SOURCE_TYPE.eq(source.subject_type.clone()))
        .where_(PackageMovement::SOURCE_ID.eq(source.subject_id))
        .get(db)
        .await?;
    Ok(movements
        .iter()
        .map(|m| match m.kind {
            MovementKind::Consume => m.quantity,
            MovementKind::Reverse => -m.quantity,
            _ => 0,
        })
        .sum())
}

/// Uses `quantity` credits of package `id` for `source` (`consume`, or
/// `consume_last` when it empties the package).
///
/// # Errors
///
/// 422 when the package is not `active`, expired by date, short of credits or
/// `quantity` is not positive.
pub async fn consume(
    state: &AppState,
    id: i64,
    quantity: i64,
    source: Subject,
) -> Result<PackageConsumption> {
    let package = Package::find_or_404(&state.db, id).await?;
    if consumed_by(&state.db, id, &source).await? > 0 {
        return Ok(PackageConsumption::Already {
            package: package.package_id(),
        });
    }
    // BL-SAL-16: with the setting on, an unpaid reopened balance refuses the
    // credit. The refusal is recorded for the seller's task; the source
    // records "credit not consumed: unpaid" and decides what follows.
    let suspend = config_of(state).suspend_package_consumption_while_unpaid;
    if suspend && package.has_unpaid_balance() && package.status == PackageStatus::Active {
        record_refusal(state, &package, &source, quantity).await?;
        return Ok(PackageConsumption::Unpaid {
            package: package.package_id(),
        });
    }
    let name = if quantity == package.remaining() {
        CONSUME_LAST
    } else {
        CONSUME
    };
    let input = ConsumeInput {
        quantity,
        source,
        today: wl_kernel::documents::today(),
        suspend_while_unpaid: suspend,
    };
    let _ = package
        .transition(name)
        .by_system()
        .input(&input)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    let after = Package::find_or_404(&state.db, id).await?;
    Ok(PackageConsumption::Consumed {
        package: after.package_id(),
        remaining: after.remaining(),
    })
}

/// Gives back `quantity` credits of package `id`: by `by` (needs
/// `packages.adjust` and a reason), or by the system when the source was
/// cancelled.
pub async fn reverse(
    state: &AppState,
    id: i64,
    quantity: i64,
    source: Option<Subject>,
    by: Option<(&renox::AuthUser, &str)>,
) -> Result<Package> {
    let package = Package::find_or_404(&state.db, id).await?;
    let input = ReverseInput { quantity, source };
    let run = match by {
        Some((user, reason)) => package
            .transition(REVERSE)
            .by(user)
            .reason(reason.to_owned()),
        None => package.transition(REVERSE_SYSTEM).by_system(),
    };
    let _ = run
        .input(&input)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    Package::find_or_404(&state.db, id).await
}

/// Moves the end date of package `id` to `new_end` (`extend`).
pub async fn extend(
    state: &AppState,
    by: &renox::AuthUser,
    id: i64,
    new_end: NaiveDate,
    reason: &str,
) -> Result<Package> {
    let package = Package::find_or_404(&state.db, id).await?;
    let input = ExtendInput {
        new_end,
        today: wl_kernel::documents::today(),
        limit: config_of(state).package_extension_limit,
    };
    let mut run = package.transition(EXTEND).by(by).input(&input);
    if !reason.trim().is_empty() {
        run = run.reason(reason.trim().to_owned());
    }
    let _ = run.run(state).await.map_err(|e| e.into_error())?;
    Package::find_or_404(&state.db, id).await
}

/// Cancels package `id` while nothing was paid on its invoice (`cancel`).
pub async fn cancel(state: &AppState, by: &renox::AuthUser, id: i64) -> Result<Package> {
    let package = Package::find_or_404(&state.db, id).await?;
    let nothing_paid = match package.invoice_id {
        None => true,
        Some(invoice) => invoicer(state)
            .find(&state.db, InvoiceId(invoice))
            .await?
            .is_none_or(|i| i.balance >= i.gross_total),
    };
    let _ = package
        .transition(CANCEL)
        .by(by)
        .input(&CancelInput { nothing_paid })
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    Package::find_or_404(&state.db, id).await
}

/// Expires the `active` packages whose last day is before `today`. Returns
/// how many. A package another run expired first is skipped.
pub async fn run_expiry(state: &AppState, today: NaiveDate) -> Result<usize> {
    let due = Package::query()
        .where_(Package::STATUS.eq(PackageStatus::Active))
        .where_(Package::VALID_TO.lt(today))
        .order_by(Package::ID)
        .get(&state.db)
        .await?;
    let mut expired = 0;
    for package in due {
        match package
            .transition(EXPIRE)
            .by_system()
            .input(&ExpireInput { today })
            .run(state)
            .await
        {
            Ok(_) => expired += 1,
            Err(renox::state_machine::TransitionError::NotFrom(_)) => {}
            Err(error) => return Err(error.into_error()),
        }
    }
    Ok(expired)
}

/// The kernel's [`PackageSales`] for the verticals: an enrolment for a package
/// of sessions sells one (10 `EDU.ENR.01`).
pub(crate) struct Sales;

impl PackageSales for Sales {
    fn sell<'a>(
        &'a self,
        state: &'a AppState,
        sale: SellPackage,
    ) -> SalesFuture<'a, Option<PackageId>> {
        Box::pin(async move {
            let mut new = NewPackage::new(sale.holder, &sale.item, sale.credits)
                .item_name(&sale.item_name)
                .price(sale.price);
            if let Some(payer) = sale.payer {
                new = new.payer(payer);
            }
            if let Some(months) = sale.validity_months {
                new = new.validity_months(months);
            }
            if let Some(invoice) = sale.invoice {
                new = new.invoice(InvoiceId(invoice));
            }
            if let Some(user) = sale.sold_by {
                new = new.sold_by(user);
            }
            let package = sell(state, new).await?;
            Ok(Some(package.package_id()))
        })
    }
}

/// The kernel's [`PackageLedger`] for the verticals and `wl-scheduling`.
pub(crate) struct Ledger;

impl PackageLedger for Ledger {
    fn consume<'a>(
        &'a self,
        state: &'a AppState,
        request: ConsumePackage,
    ) -> SalesFuture<'a, PackageConsumption> {
        Box::pin(async move {
            let held = Package::query()
                .where_(Package::HOLDER_PARTY_ID.eq(request.party.0))
                .where_(Package::ITEM_CODE.eq(request.item.clone()))
                .order_by(Package::ID)
                .get(&state.db)
                .await?;
            for package in &held {
                if consumed_by(&state.db, package.id, &request.source).await? > 0 {
                    return Ok(PackageConsumption::Already {
                        package: package.package_id(),
                    });
                }
            }
            let today = wl_kernel::documents::today();
            let mut usable: Vec<&Package> = held
                .iter()
                .filter(|p| {
                    p.status == PackageStatus::Active
                        && p.remaining() >= request.quantity
                        && !p.is_past_validity(today)
                })
                .collect();
            usable.sort_by_key(|p| (p.valid_to, p.id));
            let Some(package) = usable.first() else {
                return Ok(PackageConsumption::NoCredit);
            };
            consume(state, package.id, request.quantity, request.source).await
        })
    }

    fn reverse<'a>(&'a self, state: &'a AppState, source: Subject) -> SalesFuture<'a, bool> {
        Box::pin(async move {
            let consumed = PackageMovement::query()
                .where_(PackageMovement::KIND.eq(MovementKind::Consume))
                .where_(PackageMovement::SOURCE_TYPE.eq(source.subject_type.clone()))
                .where_(PackageMovement::SOURCE_ID.eq(source.subject_id))
                .order_by(PackageMovement::ID)
                .get(&state.db)
                .await?;
            let mut gave_back = false;
            for movement in consumed {
                let net = consumed_by(&state.db, movement.package_id, &source).await?;
                if net > 0 {
                    reverse(state, movement.package_id, net, Some(source.clone()), None).await?;
                    gave_back = true;
                }
            }
            Ok(gave_back)
        })
    }

    fn balance<'a>(&'a self, db: &'a Db, party: PartyId, item: &'a str) -> SalesFuture<'a, i64> {
        Box::pin(async move {
            let today = wl_kernel::documents::today();
            let held = Package::query()
                .where_(Package::HOLDER_PARTY_ID.eq(party.0))
                .where_(Package::ITEM_CODE.eq(item.to_owned()))
                .where_(Package::STATUS.eq(PackageStatus::Active))
                .get(db)
                .await?;
            Ok(held
                .iter()
                .filter(|p| !p.is_past_validity(today))
                .map(Package::remaining)
                .sum())
        })
    }
}

/// The package paid with a paid invoice activates.
async fn on_invoice_paid(state: &AppState, event: &InvoicePaid) -> Result {
    let Some(package) = of_invoice(&state.db, event.invoice).await? else {
        return Ok(());
    };
    if package.status == PackageStatus::PendingPayment {
        activate(state, package.id, wl_kernel::documents::today()).await?;
    }
    Ok(())
}

/// Emits the kernel events after a package transition committed.
async fn announce(state: &AppState, event: &Transitioned<Package>) -> Result {
    let name = event.transition.as_str();
    if ![ACTIVATE, CONSUME, CONSUME_LAST, EXTEND, EXPIRE].contains(&name) {
        return Ok(());
    }
    let Some(package) = Package::find(&state.db, event.key).await? else {
        return Ok(());
    };
    let (id, party) = (package.package_id(), package.holder());
    match name {
        ACTIVATE => {
            state
                .emit(PackageActivated {
                    package: id,
                    party,
                    credits: package.credits_total,
                    valid_to: package.valid_to,
                })
                .await
        }
        EXTEND => {
            let Some(valid_to) = package.valid_to else {
                return Ok(());
            };
            state
                .emit(PackageExtended {
                    package: id,
                    party,
                    remaining: package.remaining(),
                    valid_to,
                })
                .await
        }
        EXPIRE => {
            state
                .emit(PackageExpired {
                    package: id,
                    party,
                    remaining: package.remaining(),
                })
                .await
        }
        _ => {
            let last = PackageMovement::query()
                .where_(PackageMovement::PACKAGE_ID.eq(package.id))
                .where_(PackageMovement::KIND.eq(MovementKind::Consume))
                .order_by_desc(PackageMovement::ID)
                .first(&state.db)
                .await?;
            let Some(movement) = last else {
                return Ok(());
            };
            let Some(source) = movement.source() else {
                return Ok(());
            };
            state
                .emit(PackageConsumed {
                    package: id,
                    party,
                    quantity: movement.quantity,
                    remaining: package.remaining(),
                    source,
                })
                .await
        }
    }
}

/// Registers the listeners, the ledger and the daily expiry.
pub(crate) fn register(app: &mut Registry) {
    renox::state_machine::register::<Package>();
    wl_kernel::sales::provide_package_ledger(app, Ledger);
    wl_kernel::sales::provide_package_sales(app, Sales);
    app.listen(|event: Transitioned<Package>, state: AppState| async move {
        announce(&state, &event).await
    });
    app.listen(|event: InvoicePaid, state: AppState| async move {
        on_invoice_paid(&state, &event).await
    });
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run_expiry(&state, wl_kernel::documents::today())
                .await
                .map(|_| ())
        })
        .timezone(TASK_ZONE);
    app.command(
        TASK,
        "Expire the active packages whose last day has passed",
        |_args, state: AppState| async move {
            let expired = run_expiry(&state, wl_kernel::documents::today()).await?;
            println!("{expired} packages expired.");
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::sales::PackageId;

    #[test]
    fn the_builder_sets_the_sale() {
        let new = NewPackage::new(PartyId(7), "physio_10", 10)
            .item_name("Physio, 10 sessions")
            .price(150_000)
            .sold_by(3)
            .validity_months(3);
        assert_eq!(new.credits, 10);
        assert_eq!(new.price, 150_000);
        assert_eq!(new.sold_by, Some(3));
        assert_eq!(new.validity_months, Some(3));
        assert_eq!(new.item_name, "Physio, 10 sessions");
    }

    #[test]
    fn the_expiry_runs_after_midnight_dubai() {
        assert_eq!(TASK, "sales:packages");
        assert_eq!(TASK_AT, "00:10");
        assert_eq!(PackageId(3).subject(), Subject::new("sales.package", 3));
    }
}
