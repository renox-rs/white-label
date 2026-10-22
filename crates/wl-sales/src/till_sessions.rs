//! The till session services (wl-sales §4.3, `SAL.POS.01`; base for
//! BL-FNB-15 and BL-PAY-19): open a session with a float, count and close it,
//! reopen and review it, and announce `TillSessionClosed` after the commit,
//! which is what makes `wl-payments` create the till's cash handover.
//!
//! The cash the till should hold is the opening float plus the cash tenders of
//! the counter sales rung up on the session. A counter sale is rung up on the
//! cashier's open session when it names none ([`crate::counter_sales::open`]).
//!
//! The kernel's [`TillDesk`] is provided here so a vertical (06's daily
//! closing) opens, counts and reopens sessions without depending on this crate.

use renox::prelude::*;
use renox::state_machine::{StateMachine, Transitioned};
use wl_kernel::sales::{OpenTill, SalesFuture, TillDesk, TillSessionClosed, TillView};

use crate::counter_sale::CounterSale;
use crate::counter_sale_machine::METHOD_ACCOUNT;
use crate::till_session::{
    CLOSE, CLOSE_SYSTEM, REOPEN, REOPEN_SYSTEM, REVIEW, REVIEW_SYSTEM, START_COUNT,
    START_COUNT_SYSTEM, TillInput, TillSession, TillSessionStatus,
};
use crate::permissions;
use crate::texts::english;

/// A 422 with the English text `key`.
fn refused(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]))
}

/// 403 unless `by` (when there is one) holds `permission`.
fn require(by: Option<&AuthUser>, permission: &str) -> Result {
    match by {
        Some(user) if !user.has_permission(permission) => Err(Error::Forbidden),
        _ => Ok(()),
    }
}

/// The cash and the card payments of the completed counter sales on the
/// session `id`, in fils.
///
/// # Errors
///
/// Database errors.
pub async fn takings(db: &Db, id: i64) -> Result<(i64, i64)> {
    let sales = CounterSale::query()
        .where_(CounterSale::TILL_SESSION_ID.eq(id))
        .get(db)
        .await?;
    let (mut cash, mut card) = (0, 0);
    for sale in sales.iter().filter(|sale| !sale.is_open()) {
        for payment in sale.payments(db).await? {
            match payment.method.as_str() {
                "cash" => cash += payment.amount,
                m if m == METHOD_ACCOUNT => {}
                _ => card += payment.amount,
            }
        }
    }
    Ok((cash, card))
}

/// The session as a vertical reads it.
///
/// # Errors
///
/// Database errors.
pub async fn view_of(db: &Db, session: &TillSession) -> Result<TillView> {
    let (cash_sales, card_sales) = takings(db, session.id).await?;
    Ok(TillView {
        id: session.id,
        till: session.till.clone(),
        location: session.location.clone(),
        business_date: session
            .business_date
            .unwrap_or_else(wl_kernel::documents::today),
        status: session.status.as_str().to_owned(),
        cashier_user: session.cashier_user_id,
        float: session.float_amount,
        cash_sales,
        card_sales,
        expected: match session.status {
            TillSessionStatus::Closed | TillSessionStatus::Reviewed => session.expected_cash,
            _ => session.float_amount + cash_sales,
        },
        counted: session.counted_cash,
        variance: session.variance,
    })
}

/// Opens a session for the till of `new`.
///
/// # Errors
///
/// 403 without `pos.open_till` (for a user); 422 for a negative float or a
/// till that has a session for that day already.
pub async fn open(state: &AppState, by: Option<&AuthUser>, new: OpenTill) -> Result<TillSession> {
    require(by, permissions::POS_OPEN_TILL)?;
    if new.float < 0 {
        return Err(refused("wl_sales.till.problem.float"));
    }
    let till = new.till.trim().to_owned();
    if till.is_empty() {
        return Err(refused("wl_sales.till.problem.till"));
    }
    if TillSession::of_day(&state.db, &till, new.business_date)
        .await?
        .is_some()
    {
        return Err(refused("wl_sales.till.problem.exists"));
    }
    let mut session = TillSession {
        till,
        location: new.location,
        legal_entity_id: new.legal_entity_id,
        business_date: Some(new.business_date),
        cashier_user_id: new.cashier_user,
        float_amount: new.float,
        opened_at: Some(renox::db::now()),
        ..TillSession::default()
    };
    let mut tx = state.db.begin().await?;
    session
        .insert_in_state(
            &mut tx,
            TillSessionStatus::Open,
            &english("wl_sales.till.history.opened", &[]),
        )
        .await?;
    tx.commit().await?;
    TillSession::find_or_404(&state.db, session.id).await
}

/// Counts the session `id` and closes it with the `counted` cash:
/// `start_count`, then `close`. `TillSessionClosed` is emitted after the
/// commit.
///
/// # Errors
///
/// 403 without `pos.close_till` (for a user); 422 for a session that is not
/// open or a negative count.
pub async fn submit_count(
    state: &AppState,
    by: Option<&AuthUser>,
    id: i64,
    counted: i64,
) -> Result<TillSession> {
    require(by, permissions::POS_CLOSE_TILL)?;
    if counted < 0 {
        return Err(refused("wl_sales.till.problem.not_counted"));
    }
    let session = TillSession::find_or_404(&state.db, id).await?;
    if session.status != TillSessionStatus::Open {
        return Err(refused("wl_sales.till.problem.not_open"));
    }
    let builder = session.transition(if by.is_some() {
        START_COUNT
    } else {
        START_COUNT_SYSTEM
    });
    let outcome = match by {
        Some(user) => builder.by(user).run(state).await,
        None => builder.by_system().run(state).await,
    };
    let _ = outcome.map_err(|e| e.into_error())?;
    let session = TillSession::find_or_404(&state.db, id).await?;
    let (cash, _card) = takings(&state.db, id).await?;
    let input = TillInput {
        counted: Some(counted),
        expected: session.float_amount + cash,
    };
    let builder = session.transition(if by.is_some() { CLOSE } else { CLOSE_SYSTEM });
    let outcome = match by {
        Some(user) => builder.by(user).input(&input).run(state).await,
        None => builder.by_system().input(&input).run(state).await,
    };
    let _ = outcome.map_err(|e| e.into_error())?;
    TillSession::find_or_404(&state.db, id).await
}

/// Reopens the closed session `id` for a correction.
///
/// # Errors
///
/// 403 without `pos.reopen_till` (for a user); 422 without a reason or for a
/// session that is not closed.
pub async fn reopen(
    state: &AppState,
    by: Option<&AuthUser>,
    id: i64,
    reason: &str,
) -> Result<TillSession> {
    require(by, permissions::POS_REOPEN_TILL)?;
    let session = TillSession::find_or_404(&state.db, id).await?;
    let builder = session
        .transition(if by.is_some() { REOPEN } else { REOPEN_SYSTEM })
        .reason(reason.trim().to_owned());
    let outcome = match by {
        Some(user) => builder.by(user).run(state).await,
        None => builder.by_system().run(state).await,
    };
    let _ = outcome.map_err(|e| e.into_error())?;
    TillSession::find_or_404(&state.db, id).await
}

/// Marks the closed session `id` reviewed.
///
/// # Errors
///
/// 403 without `pos.review` (for a user); 422 for a session that is not
/// closed.
pub async fn review(state: &AppState, by: Option<&AuthUser>, id: i64) -> Result<TillSession> {
    require(by, permissions::POS_REVIEW)?;
    let session = TillSession::find_or_404(&state.db, id).await?;
    let builder = session.transition(if by.is_some() { REVIEW } else { REVIEW_SYSTEM });
    let outcome = match by {
        Some(user) => builder.by(user).run(state).await,
        None => builder.by_system().run(state).await,
    };
    let _ = outcome.map_err(|e| e.into_error())?;
    TillSession::find_or_404(&state.db, id).await
}

/// Emits [`TillSessionClosed`] after the commit of `close`.
async fn announce(state: &AppState, event: &Transitioned<TillSession>) -> Result {
    if event.transition != CLOSE && event.transition != CLOSE_SYSTEM {
        return Ok(());
    }
    let Some(session) = TillSession::find(&state.db, event.key).await? else {
        return Ok(());
    };
    state
        .emit(TillSessionClosed {
            session: session.id,
            till: session.till.clone(),
            cashier_user: session.cashier_user_id,
            date: session
                .business_date
                .unwrap_or_else(wl_kernel::documents::today),
            expected: session.expected_cash,
            counted: session.counted_cash.unwrap_or_default(),
            location: session.location.clone(),
            legal_entity_id: session.legal_entity_id,
        })
        .await
}

/// The desk `wl-sales` gives the verticals.
struct Desk;

impl TillDesk for Desk {
    fn open<'a>(
        &'a self,
        state: &'a AppState,
        by: Option<&'a AuthUser>,
        till: OpenTill,
    ) -> SalesFuture<'a, Option<TillView>> {
        Box::pin(async move {
            let session = open(state, by, till).await?;
            Ok(Some(view_of(&state.db, &session).await?))
        })
    }

    fn find<'a>(&'a self, state: &'a AppState, id: i64) -> SalesFuture<'a, Option<TillView>> {
        Box::pin(async move {
            match TillSession::find(&state.db, id).await? {
                Some(session) => Ok(Some(view_of(&state.db, &session).await?)),
                None => Ok(None),
            }
        })
    }

    fn of_day<'a>(
        &'a self,
        state: &'a AppState,
        till: &'a str,
        date: renox::chrono::NaiveDate,
    ) -> SalesFuture<'a, Option<TillView>> {
        Box::pin(async move {
            match TillSession::of_day(&state.db, till, date).await? {
                Some(session) => Ok(Some(view_of(&state.db, &session).await?)),
                None => Ok(None),
            }
        })
    }

    fn submit_count<'a>(
        &'a self,
        state: &'a AppState,
        by: Option<&'a AuthUser>,
        id: i64,
        counted: i64,
    ) -> SalesFuture<'a, TillView> {
        Box::pin(async move {
            let session = submit_count(state, by, id, counted).await?;
            view_of(&state.db, &session).await
        })
    }

    fn reopen<'a>(
        &'a self,
        state: &'a AppState,
        by: Option<&'a AuthUser>,
        id: i64,
        reason: &'a str,
    ) -> SalesFuture<'a, TillView> {
        Box::pin(async move {
            let session = reopen(state, by, id, reason).await?;
            view_of(&state.db, &session).await
        })
    }

    fn review<'a>(
        &'a self,
        state: &'a AppState,
        by: Option<&'a AuthUser>,
        id: i64,
    ) -> SalesFuture<'a, TillView> {
        Box::pin(async move {
            let session = review(state, by, id).await?;
            view_of(&state.db, &session).await
        })
    }
}

/// Registers the machine, the listener and the kernel's `TillDesk`.
pub(crate) fn register(app: &mut Registry) {
    renox::state_machine::register::<TillSession>();
    wl_kernel::sales::provide_till_desk(app, Desk);
    app.listen(
        |event: Transitioned<TillSession>, state: AppState| async move {
            announce(&state, &event).await
        },
    );
}
