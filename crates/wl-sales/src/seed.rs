//! The idempotent installer of the nine lead sources (wl-sales §3, #138 A5).
//! [`install`] adds the sources the database lacks and keeps every row that
//! is there, so a client's renames and deactivations survive a re-run.

use renox::auth::User;
use renox::prelude::*;

use crate::lead_source::{DEFAULTS, LeadSource};

/// What [`install`] created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Installed {
    /// Lead sources created.
    pub lead_sources: usize,
}

impl Installed {
    /// The number of rows of every kind created.
    pub fn total(&self) -> usize {
        self.lead_sources
    }
}

/// Stores the lead sources the database lacks, in one transaction.
pub async fn install(db: &Db) -> Result<Installed> {
    let mut tx = db.begin().await?;
    let mut installed = Installed::default();
    for (code, name) in DEFAULTS {
        if LeadSource::by_code(&mut tx, code).await?.is_none() {
            LeadSource::create(
                &mut tx,
                LeadSource {
                    code: code.to_owned(),
                    name_en: name.to_owned(),
                    active: true,
                    seeded: true,
                    ..LeadSource::default()
                },
            )
            .await?;
            installed.lead_sources += 1;
        }
    }
    tx.commit().await?;
    Ok(installed)
}

/// Stores two demo campaigns as drafts, where their codes are free: an
/// acquisition campaign on Meta with a budget and a promo code, and a
/// retention-style e-mail campaign without a budget. The owner is the first
/// user, if there is one. A re-run adds nothing. Returns how many it made.
pub async fn demo_campaigns(db: &Db) -> Result<usize> {
    use renox::chrono::NaiveDate;

    use crate::campaign::{Campaign, MarketingChannel, Purpose};

    let owner = User::query().order_by("id").first(db).await?.map(|u| u.id);
    let day = |m, d| NaiveDate::from_ymd_opt(2026, m, d);
    let demos = [
        Campaign {
            name: "Spring offer".into(),
            code: "SPRING26".into(),
            purpose: Some(Purpose::Acquisition),
            marketing_channel: Some(MarketingChannel::Meta),
            owner_id: owner,
            start_date: day(3, 1),
            end_date: day(3, 31),
            budget: Some(500_000),
            promo_code: Some("SPRING26".into()),
            ..Campaign::default()
        },
        Campaign {
            name: "Summer newsletter".into(),
            code: "SUMMER26".into(),
            purpose: Some(Purpose::Acquisition),
            marketing_channel: Some(MarketingChannel::Email),
            owner_id: owner,
            start_date: day(6, 1),
            end_date: day(8, 31),
            ..Campaign::default()
        },
    ];
    let mut made = 0;
    for demo in demos {
        if Campaign::by_code(db, &demo.code).await?.is_none() && demo.store(db).await?.is_ok() {
            made += 1;
        }
    }
    Ok(made)
}

/// Stores three demo orders for the customer `party` so a person can click
/// through the online-payment stories (BL-SAL-01 to BL-SAL-03): a website
/// order that waits for the gateway (`awaiting_payment`, ten minutes left),
/// a website order cancelled because its payment failed, and an aggregator
/// order `TB-555` on channel `talabat` whose number a second import is
/// refused. Each is found by channel and external number, so a re-run adds
/// nothing. Returns how many it made.
pub async fn demo_online_orders(db: &Db, party: i64) -> Result<usize> {
    use renox::db::now;
    use wl_kernel::documents::today;

    use crate::sale_line::{NewLine, SaleLine, totals};
    use crate::sales_order::{
        ORDER_SERIES, OrderPaymentStatus, PAYMENT_ONLINE, SalesOrder, SalesOrderStatus,
    };
    use crate::sales_orders::external_key;

    let demos = [
        (
            "website",
            "WEB-1001",
            SalesOrderStatus::AwaitingPayment,
            PAYMENT_ONLINE,
            OrderPaymentStatus::Pending,
        ),
        (
            "website",
            "WEB-1002",
            SalesOrderStatus::Cancelled,
            PAYMENT_ONLINE,
            OrderPaymentStatus::Failed,
        ),
        (
            "talabat",
            "TB-555",
            SalesOrderStatus::Confirmed,
            "on_account",
            OrderPaymentStatus::NotRequired,
        ),
    ];
    let mut made = 0;
    for (channel, external, status, mode, payment_status) in demos {
        let key = external_key(external);
        if let Some(key) = &key
            && SalesOrder::of_external(db, channel, key).await?.is_some()
        {
            continue;
        }
        let number = ORDER_SERIES
            .next_allowing_gaps(db, None, None, today())
            .await?;
        let mut order = SalesOrder {
            number,
            party_id: party,
            channel: channel.to_owned(),
            payment_mode: mode.to_owned(),
            currency: "AED".to_owned(),
            revision: 1,
            external_number: Some(external.to_owned()),
            external_key: key,
            payment_status,
            ..SalesOrder::default()
        };
        match status {
            SalesOrderStatus::AwaitingPayment => {
                order.payment_due_at = Some(now() + renox::chrono::TimeDelta::minutes(10));
            }
            SalesOrderStatus::Cancelled => {
                order.reason = Some(crate::texts::english(
                    "wl_sales.orders.payment_failed_reason",
                    &[],
                ));
            }
            _ => order.confirmed_at = Some(now()),
        }
        let mut tx = db.begin().await?;
        order.insert_in_state(&mut tx, status, "demo").await?;
        tx.commit().await?;
        let order = SalesOrder::find_or_404(db, order.id).await?;
        SaleLine::add(db, &order.subject(), NewLine::new("MEAL", 10_000)).await?;
        let lines = SaleLine::of(db, &order.subject()).await?;
        let t = totals(&lines);
        SalesOrder::query()
            .where_(SalesOrder::ID.eq(order.id))
            .update(
                db,
                &[
                    ("net_total", &t.net),
                    ("vat_total", &t.vat),
                    ("gross_total", &t.gross),
                ],
            )
            .await?;
        made += 1;
    }
    Ok(made)
}

/// Stores three demo orders for the customer `party` so a person can click
/// through the stories of changing a placed order (BL-SAL-05, BL-SAL-06,
/// BL-SAL-08, BL-SAL-09). Each is found by its external number, so a re-run
/// adds nothing. Returns how many it made.
///
/// - `AMD-1001`: a `confirmed` order on account at revision 2 (10 → 12 meals),
///   revision 1 kept read-only, with its change record.
/// - `AMD-1002`: a prepaid website order at revision 2 whose payment link for
///   the added side dish expired unpaid: the side dish is held, the owner has
///   *resend the link* and *amend back*.
/// - `AMD-1003`: a prepaid order `fulfilled` after its remaining line was
///   cancelled with the outcome *scrap* and a fee, with the refund requested.
///
/// A change that waits for a manager is made by clicking: it needs the
/// approval policies of the running app.
pub async fn demo_order_changes(db: &Db, party: i64) -> Result<usize> {
    use renox::db::now;
    use wl_kernel::documents::today;

    use crate::amend::AmendDetail;
    use crate::cancel_lines::{CancelDetail, CancelledLine};
    use crate::order_change::{
        KIND_AMEND, KIND_CANCEL_REMAINING, OrderChange, OrderRevision, OrderSettlement,
        RevisionLine, SETTLE_LINK, SETTLE_REFUND, STATUS_EXPIRED, STATUS_REQUESTED,
    };
    use crate::sale_line::{NewLine, SaleLine, totals};
    use crate::sales_order::{
        ORDER_SERIES, OrderPaymentStatus, PAYMENT_ONLINE, SalesOrder, SalesOrderStatus,
    };
    use crate::sales_orders::external_key;
    use wl_kernel::sales::{AmendedLine, LineOutcomeKind};

    /// The demo order `external` on `channel`, with `lines` (code, quantity
    /// in thousandths, price), in `status`; `None` when it exists already.
    #[allow(clippy::too_many_arguments)]
    async fn order(
        db: &Db,
        party: i64,
        channel: &str,
        external: &str,
        mode: &str,
        paid: OrderPaymentStatus,
        status: SalesOrderStatus,
        revision: i64,
        lines: &[(&str, i64, i64)],
    ) -> Result<Option<SalesOrder>> {
        let key = external_key(external);
        if let Some(key) = &key
            && SalesOrder::of_external(db, channel, key).await?.is_some()
        {
            return Ok(None);
        }
        let number = ORDER_SERIES
            .next_allowing_gaps(db, None, None, today())
            .await?;
        let mut order = SalesOrder {
            number,
            party_id: party,
            channel: channel.to_owned(),
            payment_mode: mode.to_owned(),
            currency: "AED".to_owned(),
            revision,
            external_number: Some(external.to_owned()),
            external_key: key,
            payment_status: paid,
            confirmed_at: Some(now()),
            ..SalesOrder::default()
        };
        let mut tx = db.begin().await?;
        order.insert_in_state(&mut tx, status, "demo").await?;
        tx.commit().await?;
        let order = SalesOrder::find_or_404(db, order.id).await?;
        for (code, quantity, price) in lines {
            SaleLine::add(
                db,
                &order.subject(),
                NewLine::new(code, *price).quantity_milli(*quantity),
            )
            .await?;
        }
        Ok(Some(order))
    }

    /// Stores the totals of the order's lines on the order.
    async fn total(db: &Db, order: &SalesOrder) -> Result<SalesOrder> {
        let lines = SaleLine::of(db, &order.subject()).await?;
        let t = totals(&lines);
        SalesOrder::query()
            .where_(SalesOrder::ID.eq(order.id))
            .update(
                db,
                &[
                    ("net_total", &t.net),
                    ("vat_total", &t.vat),
                    ("gross_total", &t.gross),
                ],
            )
            .await?;
        SalesOrder::find_or_404(db, order.id).await
    }

    /// The revision `revision` of `order` kept as `lines` (code, quantity,
    /// price) were.
    async fn keep(
        db: &Db,
        order: &SalesOrder,
        revision: i64,
        reason: &str,
        lines: &[SaleLine],
        quantities: &[(i64, i64)],
    ) -> Result {
        let mut kept = Vec::new();
        let mut net = 0;
        let mut gross = 0;
        for (line, quantity) in lines.iter().zip(quantities.iter().map(|q| q.1)) {
            let as_was = SaleLine {
                quantity_milli: quantity,
                ..line.clone()
            };
            net += as_was.net();
            gross += as_was.gross();
            kept.push(RevisionLine {
                line: line.id,
                line_no: line.line_no,
                item_code: line.item_code.clone(),
                description: line.description.clone(),
                quantity_milli: quantity,
                unit_price: line.unit_price,
                discount: line.discount,
                gross: as_was.gross(),
            });
        }
        OrderRevision::create(
            db,
            OrderRevision {
                order_id: order.id,
                revision,
                reason: reason.to_owned(),
                net_total: net,
                vat_total: gross - net,
                gross_total: gross,
                lines: renox::serde_json::to_string(&kept).unwrap_or_default(),
                created_at: Some(now()),
                ..OrderRevision::default()
            },
        )
        .await?;
        Ok(())
    }

    let mut made = 0;

    // AMD-1001: 10 meals became 12 on an order on account.
    if let Some(placed) = order(
        db,
        party,
        "phone",
        "AMD-1001",
        "on_account",
        OrderPaymentStatus::NotRequired,
        SalesOrderStatus::Confirmed,
        2,
        &[("MEAL", 12_000, 40_000)],
    )
    .await?
    {
        let placed = total(db, &placed).await?;
        let lines = SaleLine::of(db, &placed.subject()).await?;
        keep(db, &placed, 1, "Two more meals for the guests", &lines, &[(lines[0].id, 10_000)]).await?;
        let detail = AmendDetail {
            revision_before: 1,
            revision_after: 2,
            value_before: 420_000,
            value_after: placed.gross_total,
            lines: vec![AmendedLine {
                line: lines[0].id,
                quantity_before_milli: 10_000,
                quantity_after_milli: 12_000,
                unit_price_before: 40_000,
                unit_price_after: 40_000,
                held_milli: 0,
            }],
            no_money: false,
        };
        OrderChange {
            order_id: placed.id,
            kind: KIND_AMEND.to_owned(),
            reason: "Two more meals for the guests".to_owned(),
            detail: renox::serde_json::to_string(&detail).unwrap_or_default(),
            announced_at: Some(now()),
            ..OrderChange::default()
        }
        .insert(db)
        .await?;
        made += 1;
    }

    // AMD-1002: a prepaid increase whose payment link expired unpaid.
    if let Some(placed) = order(
        db,
        party,
        "website",
        "AMD-1002",
        PAYMENT_ONLINE,
        OrderPaymentStatus::Received,
        SalesOrderStatus::Confirmed,
        2,
        &[("MEAL", 4_000, 5_000), ("SIDE", 2_000, 1_000)],
    )
    .await?
    {
        let placed = total(db, &placed).await?;
        let lines = SaleLine::of(db, &placed.subject()).await?;
        keep(
            db,
            &placed,
            1,
            "Add two side dishes",
            &lines,
            &[(lines[0].id, 4_000), (lines[1].id, 0)],
        )
        .await?;
        SaleLine::query()
            .where_(SaleLine::ID.eq(lines[1].id))
            .update(db, &[("held_milli", &2_000_i64)])
            .await?;
        OrderSettlement::create(
            db,
            OrderSettlement {
                order_id: placed.id,
                revision: 2,
                kind: SETTLE_LINK.to_owned(),
                amount: 2_100,
                status: STATUS_EXPIRED.to_owned(),
                due_at: Some(now() - renox::chrono::TimeDelta::minutes(5)),
                ..OrderSettlement::default()
            },
        )
        .await?;
        made += 1;
    }

    // AMD-1003: the remaining line cancelled as scrap with a fee.
    if let Some(placed) = order(
        db,
        party,
        "website",
        "AMD-1003",
        PAYMENT_ONLINE,
        OrderPaymentStatus::Received,
        SalesOrderStatus::Fulfilled,
        1,
        &[("MEAL", 2_000, 4_000), ("DESSERT", 3_000, 2_000)],
    )
    .await?
    {
        let placed = total(db, &placed).await?;
        let lines = SaleLine::of(db, &placed.subject()).await?;
        SaleLine::query()
            .where_(SaleLine::ID.eq(lines[0].id))
            .update(db, &[("fulfilled_milli", &2_000_i64)])
            .await?;
        let share = lines[1].share_of(3_000);
        let fee = share / 10;
        SaleLine::query()
            .where_(SaleLine::ID.eq(lines[1].id))
            .update(
                db,
                &[
                    ("cancelled_milli", &3_000_i64),
                    ("cancel_outcome", &Some(LineOutcomeKind::Scrap.as_str().to_owned())),
                    ("cancel_fee", &fee),
                    ("cancel_reason", &Some("The dessert was prepared and the guests left".to_owned())),
                ],
            )
            .await?;
        let detail = CancelDetail {
            lines: vec![CancelledLine {
                line: lines[1].id,
                quantity_milli: 3_000,
                started: true,
                outcome: Some(LineOutcomeKind::Scrap),
                fee,
                fee_waived: false,
                share,
            }],
            complete: true,
            whole: false,
        };
        OrderChange {
            order_id: placed.id,
            kind: KIND_CANCEL_REMAINING.to_owned(),
            reason: "The dessert was prepared and the guests left".to_owned(),
            detail: renox::serde_json::to_string(&detail).unwrap_or_default(),
            announced_at: Some(now()),
            ..OrderChange::default()
        }
        .insert(db)
        .await?;
        OrderSettlement::create(
            db,
            OrderSettlement {
                order_id: placed.id,
                revision: 1,
                kind: SETTLE_REFUND.to_owned(),
                amount: share - fee,
                status: STATUS_REQUESTED.to_owned(),
                ..OrderSettlement::default()
            },
        )
        .await?;
        made += 1;
    }
    Ok(made)
}

/// Stores demo orders for the customer `party` so a person can click through
/// the stories of cancelling an order after fulfilment started (BL-SAL-07)
/// and of a refund after completion with nothing returned (BL-SAL-14). Each is
/// found by its external number, so a re-run adds nothing. Returns how many it
/// made.
///
/// Lines are zero-rated so the figures are the stories' (AED 120, AED 90).
///
/// - `CXL-1001`: a prepaid `confirmed` order of AED 120 (meal 80, side 40) to
///   cancel from its page: with the kitchen's `FulfilmentProgress` marking the
///   meal as started, it asks for an outcome and waits for a manager.
/// - `CXL-1002`: a prepaid order `cancelled` after the meal was prepared:
///   outcome *scrap*, fee AED 8, refund AED 112 requested.
/// - `RFD-1001`: a `fulfilled` order of AED 90 (pizza 60, cola 30) to raise a
///   refund after completion from (AED 60 waits for a manager, AED 30 does
///   not).
/// - `ORD-1001`: a `fulfilled` Talabat order of AED 25 whose customer the
///   aggregator refunded itself: a no goods return *refunded by the channel*,
///   to reverse from its page.
/// - `RFD-1002`: a `fulfilled` order of AED 90 with a refund of AED 30 for a
///   missing cola resolved by the business (amount refunded AED 30).
pub async fn demo_order_cancellations(db: &Db, party: i64) -> Result<usize> {
    use renox::db::now;
    use renox::state_machine::StateMachine;
    use wl_kernel::documents::today;

    use crate::cancel_lines::{CancelDetail, CancelledLine};
    use crate::order_change::{
        KIND_CANCEL, OrderChange, OrderSettlement, SETTLE_FEE_INVOICE, SETTLE_REFUND,
        STATUS_REQUESTED,
    };
    use crate::sale_line::{NewLine, SaleLine, totals};
    use crate::sales_order::{
        ORDER_SERIES, OrderPaymentStatus, PAYMENT_ONLINE, SalesOrder, SalesOrderStatus,
    };
    use crate::sales_orders::external_key;
    use crate::sales_return::{
        RETURN_SERIES, ReturnKind, ReturnResolution, ReturnStatus, SalesReturn,
        SalesReturnLine,
    };
    use wl_kernel::invoicing::VatCategory;
    use wl_kernel::sales::LineOutcomeKind;

    /// The demo order `external` on `channel` in `status` with `lines` (code,
    /// quantity in thousandths, price), totals stored; `None` when it exists.
    #[allow(clippy::too_many_arguments)]
    async fn order(
        db: &Db,
        party: i64,
        channel: &str,
        external: &str,
        mode: &str,
        paid: OrderPaymentStatus,
        status: SalesOrderStatus,
        lines: &[(&str, i64, i64)],
        fulfilled: bool,
    ) -> Result<Option<(SalesOrder, Vec<SaleLine>)>> {
        let key = external_key(external);
        if let Some(key) = &key
            && SalesOrder::of_external(db, channel, key).await?.is_some()
        {
            return Ok(None);
        }
        let number = ORDER_SERIES
            .next_allowing_gaps(db, None, None, today())
            .await?;
        let mut order = SalesOrder {
            number,
            party_id: party,
            channel: channel.to_owned(),
            payment_mode: mode.to_owned(),
            currency: "AED".to_owned(),
            revision: 1,
            external_number: Some(external.to_owned()),
            external_key: key,
            payment_status: paid,
            confirmed_at: Some(now()),
            fulfilled_at: fulfilled.then(now),
            ..SalesOrder::default()
        };
        let mut tx = db.begin().await?;
        order.insert_in_state(&mut tx, status, "demo").await?;
        tx.commit().await?;
        let order = SalesOrder::find_or_404(db, order.id).await?;
        for (code, quantity, price) in lines {
            SaleLine::add(
                db,
                &order.subject(),
                NewLine::new(code, *price)
                    .quantity_milli(*quantity)
                    .vat_category(VatCategory::ZeroRated),
            )
            .await?;
        }
        let rows = SaleLine::of(db, &order.subject()).await?;
        let t = totals(&rows);
        SalesOrder::query()
            .where_(SalesOrder::ID.eq(order.id))
            .update(
                db,
                &[
                    ("net_total", &t.net),
                    ("vat_total", &t.vat),
                    ("gross_total", &t.gross),
                ],
            )
            .await?;
        if fulfilled {
            for row in &rows {
                SaleLine::query()
                    .where_(SaleLine::ID.eq(row.id))
                    .update(db, &[("fulfilled_milli", &row.quantity_milli)])
                    .await?;
            }
        }
        let order = SalesOrder::find_or_404(db, order.id).await?;
        let rows = SaleLine::of(db, &order.subject()).await?;
        Ok(Some((order, rows)))
    }

    /// A resolved no goods return of `value` fils on `line` of `order` with
    /// `resolution`; raises the order's amount refunded.
    #[allow(clippy::too_many_arguments)]
    async fn resolved_return(
        db: &Db,
        order: &SalesOrder,
        line: &SaleLine,
        value: i64,
        resolution: ReturnResolution,
        category: &str,
        party: &str,
        reason: &str,
    ) -> Result<SalesReturn> {
        let number = RETURN_SERIES
            .next_allowing_gaps(db, None, None, today())
            .await?;
        let mut made = SalesReturn {
            number,
            sale_type: wl_kernel::sales::SALES_ORDER_SUBJECT_TYPE.to_owned(),
            sale_id: order.id,
            party_id: Some(order.party_id),
            kind: ReturnKind::NoGoods,
            resolution: Some(resolution),
            reason: reason.to_owned(),
            net_value: value,
            gross_value: value,
            complaint_category: Some(category.to_owned()),
            responsible_party: Some(party.to_owned()),
            resolved_at: Some(now()),
            ..SalesReturn::default()
        };
        let mut tx = db.begin().await?;
        made.insert_in_state(&mut tx, ReturnStatus::Resolved, "demo")
            .await?;
        tx.commit().await?;
        SalesReturnLine::create(
            db,
            SalesReturnLine {
                return_id: made.id,
                sale_line_id: line.id,
                refund_value: value,
                ..SalesReturnLine::default()
            },
        )
        .await?;
        SalesOrder::query()
            .where_(SalesOrder::ID.eq(order.id))
            .update(db, &[("refunded_amount", &value)])
            .await?;
        SalesReturn::find_or_404(db, made.id).await
    }

    let mut made = 0;

    // CXL-1001: waiting to be cancelled.
    if order(
        db,
        party,
        "phone",
        "CXL-1001",
        PAYMENT_ONLINE,
        OrderPaymentStatus::Received,
        SalesOrderStatus::Confirmed,
        &[("MEAL", 1_000, 80_00), ("SIDE", 1_000, 40_00)],
        false,
    )
    .await?
    .is_some()
    {
        made += 1;
    }

    // CXL-1002: cancelled after the meal was prepared.
    if let Some((cancelled, lines)) = order(
        db,
        party,
        "phone",
        "CXL-1002",
        PAYMENT_ONLINE,
        OrderPaymentStatus::Received,
        SalesOrderStatus::Cancelled,
        &[("MEAL", 1_000, 80_00), ("SIDE", 1_000, 40_00)],
        false,
    )
    .await?
    {
        let reason = "The guests left before the meal was served";
        let fee = lines[0].share_of(1_000) / 10;
        let mut details = Vec::new();
        for (n, line) in lines.iter().enumerate() {
            let started = n == 0;
            SaleLine::query()
                .where_(SaleLine::ID.eq(line.id))
                .update(
                    db,
                    &[
                        ("cancelled_milli", &1_000_i64),
                        (
                            "cancel_outcome",
                            &started.then(|| LineOutcomeKind::Scrap.as_str().to_owned()),
                        ),
                        ("cancel_fee", &(if started { fee } else { 0 })),
                        ("cancel_reason", &Some(reason.to_owned())),
                    ],
                )
                .await?;
            details.push(CancelledLine {
                line: line.id,
                quantity_milli: 1_000,
                started,
                outcome: started.then_some(LineOutcomeKind::Scrap),
                fee: if started { fee } else { 0 },
                fee_waived: false,
                share: line.share_of(1_000),
            });
        }
        SalesOrder::query()
            .where_(SalesOrder::ID.eq(cancelled.id))
            .update(db, &[("reason", &Some(reason.to_owned()))])
            .await?;
        let detail = CancelDetail {
            lines: details,
            complete: false,
            whole: true,
        };
        OrderChange {
            order_id: cancelled.id,
            kind: KIND_CANCEL.to_owned(),
            reason: reason.to_owned(),
            detail: renox::serde_json::to_string(&detail).unwrap_or_default(),
            announced_at: Some(now()),
            ..OrderChange::default()
        }
        .insert(db)
        .await?;
        for (kind, amount) in [
            (SETTLE_REFUND, cancelled.gross_total - fee),
            (SETTLE_FEE_INVOICE, fee),
        ] {
            OrderSettlement::create(
                db,
                OrderSettlement {
                    order_id: cancelled.id,
                    revision: 1,
                    kind: kind.to_owned(),
                    amount,
                    status: STATUS_REQUESTED.to_owned(),
                    ..OrderSettlement::default()
                },
            )
            .await?;
        }
        made += 1;
    }

    // RFD-1001: completed, to refund from.
    if order(
        db,
        party,
        "phone",
        "RFD-1001",
        "on_account",
        OrderPaymentStatus::NotRequired,
        SalesOrderStatus::Fulfilled,
        &[("PIZZA", 1_000, 60_00), ("COLA", 1_000, 30_00)],
        true,
    )
    .await?
    .is_some()
    {
        made += 1;
    }

    // ORD-1001: the aggregator refunded its customer itself.
    if let Some((talabat, lines)) = order(
        db,
        party,
        "talabat",
        "ORD-1001",
        "on_account",
        OrderPaymentStatus::NotRequired,
        SalesOrderStatus::Fulfilled,
        &[("WRAP", 1_000, 25_00)],
        true,
    )
    .await?
    {
        resolved_return(
            db,
            &talabat,
            &lines[0],
            talabat.gross_total,
            ReturnResolution::RefundedByChannel,
            "cold",
            "business",
            "Talabat refunded the customer",
        )
        .await?;
        made += 1;
    }

    // RFD-1002: a missing cola refunded by the business.
    if let Some((done, lines)) = order(
        db,
        party,
        "phone",
        "RFD-1002",
        "on_account",
        OrderPaymentStatus::NotRequired,
        SalesOrderStatus::Fulfilled,
        &[("PIZZA", 1_000, 60_00), ("COLA", 1_000, 30_00)],
        true,
    )
    .await?
    {
        resolved_return(
            db,
            &done,
            &lines[1],
            3_000,
            ReturnResolution::Refund,
            "missing_item",
            "business",
            "The cola was missing",
        )
        .await?;
        OrderSettlement::create(
            db,
            OrderSettlement {
                order_id: done.id,
                revision: 1,
                kind: SETTLE_REFUND.to_owned(),
                amount: 3_000,
                status: STATUS_REQUESTED.to_owned(),
                ..OrderSettlement::default()
            },
        )
        .await?;
        made += 1;
    }
    Ok(made)
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::testing::TestApp;

    #[renox::test]
    async fn installing_twice_leaves_nine_and_keeps_edits() {
        let app = TestApp::new(App::new().module(crate::WlSales::new())).await;
        let db = app.db();
        assert_eq!(install(db).await.unwrap(), Installed { lead_sources: 9 });
        assert!(
            LeadSource::ordered(db)
                .await
                .unwrap()
                .iter()
                .all(|s| s.active && s.seeded)
        );

        let mut walk_in = LeadSource::by_code(db, "walk_in").await.unwrap().unwrap();
        walk_in.name_en = "Walk in".into();
        walk_in.active = false;
        walk_in.save(db).await.unwrap();

        assert_eq!(install(db).await.unwrap(), Installed::default());
        assert_eq!(LeadSource::query().count(db).await.unwrap(), 9);
        let walk_in = LeadSource::by_code(db, "walk_in").await.unwrap().unwrap();
        assert_eq!(walk_in.name_en, "Walk in");
        assert!(!walk_in.active);
    }

    #[renox::test]
    async fn the_demo_cancellations_and_refunds_are_made_once() {
        use crate::sales_order::SalesOrder;
        let app = TestApp::new(
            App::new()
                .module(renox::auth::Auth::new())
                .module(crate::WlSales::new()),
        )
        .await;
        let db = app.db();
        assert_eq!(demo_order_cancellations(db, 7).await.unwrap(), 5);
        assert_eq!(demo_order_cancellations(db, 7).await.unwrap(), 0);
        let key = crate::sales_orders::external_key("ORD-1001").unwrap();
        let talabat = SalesOrder::of_external(db, "talabat", &key)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(talabat.refunded_amount, 2_500);
        assert_eq!(talabat.gross_total, 2_500);
    }

    #[renox::test]
    async fn the_demo_campaigns_are_two_drafts_made_once() {
        use crate::campaign::{Campaign, CampaignStatus};
        let app = TestApp::new(
            App::new()
                .module(renox::auth::Auth::new())
                .module(crate::WlSales::new()),
        )
        .await;
        let db = app.db();
        assert_eq!(demo_campaigns(db).await.unwrap(), 2);
        assert_eq!(demo_campaigns(db).await.unwrap(), 0);
        let spring = Campaign::by_code(db, "spring26").await.unwrap().unwrap();
        assert_eq!(spring.status, CampaignStatus::Draft);
        assert_eq!(spring.budget, Some(500_000));
        assert_eq!(Campaign::query().count(db).await.unwrap(), 2);
    }

    #[renox::test]
    async fn a_source_the_client_added_is_not_seeded_and_survives() {
        let app = TestApp::new(App::new().module(crate::WlSales::new())).await;
        let db = app.db();
        install(db).await.unwrap();
        LeadSource::add(db, "TV_Ad", "TV ad", "")
            .await
            .unwrap()
            .unwrap();
        install(db).await.unwrap();
        let tv = LeadSource::by_code(db, "tv_ad").await.unwrap().unwrap();
        assert!(tv.active && !tv.seeded);
        assert_eq!(LeadSource::query().count(db).await.unwrap(), 10);
    }
}
