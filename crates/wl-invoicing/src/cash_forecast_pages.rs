//! The cash-flow forecast pages (wl-invoicing §5.14, §6, `FIN.CFL.01`):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /cash-forecast` | `wl_invoicing.cash_forecast` | `cashflow.view` |
//! | `GET /cash-forecast/cell` | `wl_invoicing.cash_forecast_cell` | `cashflow.view` |
//! | `POST /cash-balances` | `wl_invoicing.cash_balances.store` | `cashflow.manage` |
//! | `POST /cash-balances/{id}/void` | `wl_invoicing.cash_balances.void` | `cashflow.manage` |
//!
//! The grid is read only and computed when opened: the weeks across, the
//! opening balance, the receipts and payments, the projected balance and the
//! memo lines. `?entity=` limits it to one legal entity (empty: all the
//! user's entities); a user sees the entities whose scope grants
//! `cashflow.view`, and an entity outside it is 403. Each receipts or
//! payments cell opens the lines behind it (`/cash-forecast/cell`); the owner
//! drawings and the totals a source marks show their amount only, unless the
//! user holds `drawings.approve` (`FIN.ACL.22`). A row per budget category
//! shows the budget fill (`side=fill:<category code>` opens its lines); the
//! receipts moved beyond the last week by *adjust by days late* are listed
//! under the grid as *after week 13* (`side=after`). The Excel export of the grid
//! as a sensitive report export is not built yet. The templates are compiled
//! in and an app can replace them.

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::validation::{Errors, ValidationError};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::parties::directory;
use wl_kernel::payments::Direction;
use wl_kernel::{LegalEntity, PartyId, Subject};

use crate::allocate::amount_text;
use crate::cash_balance::{self, EnterBalance, OpeningCashBalance};
use crate::cash_forecast::{self, BalanceSource, Forecast, Line, LineKind, MemoKind, Warning};
use crate::cash_forecast_load;
use crate::config_of;
use crate::permissions;
use crate::texts;

/// The texts a template reads as `tx['cash.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The module's cash-flow routes; the handlers check the permissions against
/// the user's entity scope.
pub fn routes() -> Routes {
    Routes::new()
        .get("/cash-forecast", index)
        .name("wl_invoicing.cash_forecast")
        .get("/cash-forecast/cell", cell)
        .name("wl_invoicing.cash_forecast_cell")
        .post("/cash-balances", store_balance)
        .name("wl_invoicing.cash_balances.store")
        .post("/cash-balances/{id}/void", void_balance)
        .name("wl_invoicing.cash_balances.void")
}

/// `1,234` AED for `fils`, rounded to whole dirhams (half away from zero):
/// the grid shows whole AED while totals are computed in fils.
pub(crate) fn whole_text(fils: i64) -> String {
    let sign = if fils < 0 { "-" } else { "" };
    let aed = (fils.unsigned_abs() + 50) / 100;
    let digits = aed.to_string();
    let mut grouped = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    if aed == 0 {
        return "0".to_owned();
    }
    format!("{sign}{grouped}")
}

/// The links to `documents` with a name for people each (`INV-2026-0003`),
/// for those that have a page.
async fn document_links(
    db: &Db,
    user: &AuthUser,
    documents: &[Subject],
) -> Result<Vec<(String, String)>> {
    let mut links = Vec::new();
    for document in documents {
        let href = document_link(document);
        // A bill is a link only for who may open it.
        let openable = document.subject_type != "invoicing.supplier_bill"
            || user.has_permission_anywhere(permissions::SUPPLIER_BILLS_VIEW);
        if !href.is_empty() && openable {
            links.push((
                wl_kernel::documents::subject_text(db, document).await?,
                href,
            ));
        }
    }
    Ok(links)
}

/// The page of the document a line stands for, as far as it has one.
pub(crate) fn document_link(document: &Subject) -> String {
    match document.subject_type.as_str() {
        "invoicing.supplier_bill" => format!("/supplier-bills/{}", document.subject_id),
        "invoicing.invoice" => format!("/invoices/{}", document.subject_id),
        "invoicing.vat_summary" => format!("/vat-summaries/{}", document.subject_id),
        "payments.cheque" => format!("/cheques/{}", document.subject_id),
        "payments.refund" => format!("/refunds/{}", document.subject_id),
        _ => String::new(),
    }
}

/// The query of the pages.
#[derive(Deserialize, Default, Debug)]
pub(crate) struct Filter {
    /// The legal entity id; empty or missing for all.
    entity: Option<String>,
    /// The week, from 1 (the cell page).
    week: Option<usize>,
    /// `in`, `out` or a memo code (the cell page).
    side: Option<String>,
}

impl Filter {
    fn entity_id(&self) -> Option<i64> {
        self.entity.as_deref().and_then(|e| e.trim().parse().ok())
    }
}

/// The entities a user may open, and the one asked for.
async fn entities_for(
    state: &AppState,
    user: &AuthUser,
    permission: &str,
    requested: Option<i64>,
) -> Result<(Vec<LegalEntity>, Vec<LegalEntity>)> {
    if !user.has_permission_anywhere(permission) {
        return Err(Error::Forbidden);
    }
    let scope = user.scopes_with::<LegalEntity>(permission);
    let all: Vec<LegalEntity> = LegalEntity::query()
        .order_by(LegalEntity::CODE)
        .get(&state.db)
        .await?
        .into_iter()
        .filter(|e| scope.contains(&e.id))
        .collect();
    if requested.is_some_and(|id| !all.iter().any(|e| e.id == id)) {
        return Err(Error::Forbidden);
    }
    let chosen = all
        .iter()
        .filter(|e| requested.is_none_or(|id| e.id == id))
        .cloned()
        .collect();
    Ok((all, chosen))
}

/// One cell of the grid.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cell {
    text: String,
    href: String,
}

/// One row of the grid.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    name: String,
    /// `normal`, `memo` or `total`.
    look: &'static str,
    cells: Vec<Cell>,
}

/// An opening balance of the account table.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpeningView {
    entity: String,
    account: String,
    balance: String,
    as_of: String,
    source: String,
    movements: String,
    opening: String,
}

/// A manual balance of the history.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct HistoryView {
    id: i64,
    entity: String,
    account: String,
    amount: String,
    as_of: String,
    note: String,
    voided: bool,
    void_reason: String,
    can_void: bool,
}

/// What the grid page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/cash_forecast.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    head: Vec<String>,
    rows: Vec<Row>,
    entities: Vec<(String, String)>,
    entity: String,
    warnings: Vec<String>,
    openings: Vec<OpeningView>,
    history: Vec<HistoryView>,
    can_manage: bool,
    form_entities: Vec<(i64, String)>,
    today: String,
    later: Vec<LineView>,
    later_total: String,
    later_caption: String,
}

fn kind_text(t: &dyn Fn(&str) -> String, kind: MemoKind) -> String {
    t(&format!("cash.memo.{}", kind.code()))
}

/// The text of a warning.
pub(crate) fn warning_text(
    t: &dyn Fn(&str) -> String,
    codes: &BTreeMap<i64, String>,
    warning: &Warning,
) -> String {
    let code = |id: &i64| codes.get(id).cloned().unwrap_or_default();
    match warning {
        Warning::NoBalance {
            legal_entity_id,
            account,
        } => format!(
            "{} {} - {}",
            code(legal_entity_id),
            account,
            t("cash.warning.no_balance")
        ),
        Warning::Stale {
            legal_entity_id,
            account,
            as_of,
        } => format!(
            "{} {} - {} {}",
            code(legal_entity_id),
            account,
            t("cash.warning.stale"),
            as_of.format("%Y-%m-%d")
        ),
        Warning::Unpriced { document } => {
            format!("{document} - {}", t("cash.warning.unpriced"))
        }
        Warning::VatPastDue {
            legal_entity_id,
            period_end,
        } => format!(
            "{} {} - {}",
            code(legal_entity_id),
            period_end.format("%Y-%m-%d"),
            t("cash.warning.vat_past_due")
        ),
        Warning::Source {
            legal_entity_id,
            message,
            ..
        } => format!("{} - {message}", code(legal_entity_id)),
        Warning::NoBudget {
            legal_entity_id,
            months,
        } => format!(
            "{} - {} {}",
            code(legal_entity_id),
            t("cash.warning.no_budget"),
            months
                .iter()
                .map(|m| m.format("%Y-%m").to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// The rows of the grid for `forecast`.
pub(crate) fn rows_of(forecast: &Forecast, entity: &str, t: &dyn Fn(&str) -> String) -> Vec<Row> {
    let count = forecast.weeks.len();
    let query = |week: usize, side: &str| {
        let entity = if entity.is_empty() {
            String::new()
        } else {
            format!("entity={entity}&")
        };
        format!(
            "{}?{entity}week={}&side={side}",
            "/cash-forecast/cell",
            week + 1
        )
    };
    let plain = |text: String| Cell {
        text,
        href: String::new(),
    };
    let mut rows = vec![Row {
        name: t("cash.row.opening"),
        look: "normal",
        cells: (0..count)
            .map(|w| {
                plain(if w == 0 {
                    whole_text(forecast.opening_total())
                } else {
                    String::new()
                })
            })
            .collect(),
    }];
    rows.push(Row {
        name: t("cash.row.receipts"),
        look: "normal",
        cells: (0..count)
            .map(|w| Cell {
                text: whole_text(forecast.receipts(w)),
                href: if forecast.receipts(w) != 0 {
                    query(w, "in")
                } else {
                    String::new()
                },
            })
            .collect(),
    });
    rows.push(Row {
        name: t("cash.row.payments"),
        look: "normal",
        cells: (0..count)
            .map(|w| Cell {
                text: whole_text(forecast.payments(w)),
                href: if forecast.payments(w) != 0 {
                    query(w, "out")
                } else {
                    String::new()
                },
            })
            .collect(),
    });
    rows.push(Row {
        name: t("cash.row.balance"),
        look: "total",
        cells: (0..count)
            .map(|w| plain(whole_text(forecast.balance(w))))
            .collect(),
    });
    // The budget fill, a row per category (counted in the rows above).
    let mut fills: Vec<(&str, &str)> = Vec::new();
    for line in forecast
        .lines
        .iter()
        .filter(|l| l.kind == LineKind::BudgetFill)
    {
        if !fills.iter().any(|(code, _)| *code == line.category) {
            fills.push((line.category.as_str(), line.label.as_str()));
        }
    }
    for (code, name) in fills {
        let in_week = |w: usize| -> i64 {
            forecast
                .lines
                .iter()
                .filter(|l| l.kind == LineKind::BudgetFill && l.category == code && l.week == w)
                .map(|l| l.amount)
                .sum()
        };
        rows.push(Row {
            name: format!("{}: {}", t("cash.row.fill"), name),
            look: "normal",
            cells: (0..count)
                .map(|w| Cell {
                    text: whole_text(in_week(w)),
                    href: if in_week(w) != 0 {
                        query(w, &format!("fill:{code}"))
                    } else {
                        String::new()
                    },
                })
                .collect(),
        });
    }
    for kind in MemoKind::ALL {
        if forecast.memos.iter().all(|m| m.kind != kind) {
            continue;
        }
        rows.push(Row {
            name: format!("{} ({})", kind_text(t, kind), t("cash.row.memo")),
            look: "memo",
            cells: (0..count)
                .map(|w| Cell {
                    text: whole_text(forecast.memo(kind, w)),
                    href: if forecast.memo(kind, w) != 0 {
                        query(w, kind.code())
                    } else {
                        String::new()
                    },
                })
                .collect(),
        });
    }
    rows
}

/// `GET /cash-forecast`: the 13-week grid.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(filter): Query<Filter>,
) -> Result<View> {
    let t = |key: &str| texts::t(&lang, &format!("wl_invoicing.{key}"));
    let requested = filter.entity_id();
    let (all, chosen) = entities_for(&state, &user, permissions::CASHFLOW_VIEW, requested).await?;
    let config = config_of(&state);
    let inputs = cash_forecast_load::load(&state, &config, &chosen, today()).await?;
    let forecast = cash_forecast::assemble(&inputs);
    let entity = requested.map(|e| e.to_string()).unwrap_or_default();
    let rows = rows_of(&forecast, &entity, &t);
    let codes: BTreeMap<i64, String> = all.iter().map(|e| (e.id, e.code.clone())).collect();
    let warnings = forecast
        .warnings
        .iter()
        .map(|w| warning_text(&t, &codes, w))
        .collect();
    let head = std::iter::once(String::new())
        .chain(forecast.weeks.iter().map(|w| {
            format!(
                "{} {} ({} - {})",
                t("cash.week"),
                w.number,
                w.start.format("%d %b"),
                w.end.format("%d %b")
            )
        }))
        .collect();
    let openings = forecast
        .openings
        .iter()
        .map(|o| OpeningView {
            entity: codes.get(&o.legal_entity_id).cloned().unwrap_or_default(),
            account: o.account.clone(),
            balance: o
                .balance
                .map_or_else(|| "-".to_owned(), |b| whole_text(b.amount)),
            as_of: o
                .balance
                .map_or_else(String::new, |b| b.as_of.format("%Y-%m-%d").to_string()),
            source: t(match o.balance.map(|b| b.source) {
                Some(BalanceSource::Statement) => "cash.source.statement",
                Some(BalanceSource::Manual) => "cash.source.manual",
                _ => "cash.source.none",
            }),
            movements: whole_text(o.movements),
            opening: whole_text(o.amount),
        })
        .collect();
    let manage_scope = user.scopes_with::<LegalEntity>(permissions::CASHFLOW_MANAGE);
    let can_manage = user.has_permission_anywhere(permissions::CASHFLOW_MANAGE);
    let mut history = Vec::new();
    for e in &chosen {
        for row in cash_balance::history(&state.db, e.id).await? {
            history.push(HistoryView {
                id: row.id,
                entity: e.code.clone(),
                account: row.bank_account.clone(),
                amount: whole_text(row.amount),
                as_of: row.as_of.format("%Y-%m-%d").to_string(),
                note: row.note.clone().unwrap_or_default(),
                voided: row.is_voided(),
                void_reason: row.void_reason.clone().unwrap_or_default(),
                can_void: !row.is_voided() && manage_scope.contains(&e.id),
            });
        }
    }
    let mut entities = vec![(String::new(), t("cash.all_entities"))];
    entities.extend(
        all.iter()
            .map(|e| (e.id.to_string(), format!("{} - {}", e.code, e.name_en))),
    );
    let form_entities = all
        .iter()
        .filter(|e| manage_scope.contains(&e.id))
        .map(|e| (e.id, format!("{} - {}", e.code, e.name_en)))
        .collect();
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        head,
        rows,
        entities,
        entity,
        warnings,
        openings,
        history,
        can_manage,
        form_entities,
        today: today().format("%Y-%m-%d").to_string(),
        later: forecast
            .after
            .iter()
            .map(|line| LineView {
                label: line.label.clone(),
                kind: t(&format!("cash.kind.{}", line.kind.code())),
                party: String::new(),
                date: line.date.format("%Y-%m-%d").to_string(),
                amount: amount_text("AED", line.amount),
                flags: vec![t("cash.flag.not_counted")],
                links: line
                    .documents
                    .iter()
                    .map(|d| (d.to_string(), document_link(d)))
                    .filter(|(_, href)| !href.is_empty())
                    .filter(|(_, href)| {
                        !href.starts_with("/supplier-bills/")
                            || user.has_permission_anywhere(permissions::SUPPLIER_BILLS_VIEW)
                    })
                    .collect(),
            })
            .collect(),
        later_total: amount_text("AED", forecast.after.iter().map(|l| l.amount).sum()),
        later_caption: format!(
            "{} {}",
            t("cash.after"),
            forecast.weeks.last().map_or(0, |w| w.number)
        ),
    }))
}

/// One line behind a cell.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct LineView {
    label: String,
    kind: String,
    party: String,
    date: String,
    amount: String,
    flags: Vec<String>,
    links: Vec<(String, String)>,
}

/// What the cell page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/cash_forecast_cell.html")]
pub(crate) struct CellPage {
    tx: Texts,
    title: String,
    back: String,
    lines: Vec<LineView>,
    total: String,
}

/// Whether `user` may see what a line stands for: a line marked as a total
/// shows its party and documents only for owner drawings and only to a holder
/// of `drawings.approve` in every entity asked for (`FIN.ACL.22`).
fn may_open(line: &Line, drawings_ok: bool) -> bool {
    !line.total_only || (matches!(line.kind, LineKind::Bill | LineKind::BudgetFill) && drawings_ok)
}

/// `GET /cash-forecast/cell`: the lines behind a cell.
async fn cell(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(filter): Query<Filter>,
) -> Result<View> {
    let t = |key: &str| texts::t(&lang, &format!("wl_invoicing.{key}"));
    let requested = filter.entity_id();
    let (_, chosen) = entities_for(&state, &user, permissions::CASHFLOW_VIEW, requested).await?;
    let config = config_of(&state);
    let inputs = cash_forecast_load::load(&state, &config, &chosen, today()).await?;
    let forecast = cash_forecast::assemble(&inputs);
    let week = filter.week.unwrap_or(1).saturating_sub(1);
    let side = filter.side.clone().unwrap_or_default();
    let drawings_scope = user.scopes_with::<LegalEntity>(permissions::DRAWINGS_APPROVE);
    let drawings_ok = !chosen.is_empty() && chosen.iter().all(|e| drawings_scope.contains(&e.id));
    let directory = directory(&state);
    let mut views = Vec::new();
    let mut total = 0;
    let party_name = |party: Option<PartyId>| {
        let directory = directory.clone();
        let db = state.db.clone();
        async move {
            match party {
                Some(id) => Ok::<String, Error>(
                    directory
                        .find(&db, id)
                        .await?
                        .map(|p| p.display_name)
                        .unwrap_or_default(),
                ),
                None => Ok(String::new()),
            }
        }
    };
    let direction = match side.as_str() {
        "in" => Some(Direction::In),
        "out" => Some(Direction::Out),
        _ => None,
    };
    let fill = side.strip_prefix("fill:").map(str::to_owned);
    let later = side == "after";
    if direction.is_some() || fill.is_some() || later {
        let selected: Vec<&Line> = if later {
            forecast.after.iter().collect()
        } else if let Some(code) = &fill {
            forecast
                .lines
                .iter()
                .filter(|l| l.week == week && l.kind == LineKind::BudgetFill && &l.category == code)
                .collect()
        } else {
            forecast
                .lines
                .iter()
                .filter(|l| l.week == week && Some(l.direction) == direction)
                .collect()
        };
        for line in selected {
            let open = may_open(line, drawings_ok);
            let mut flags = Vec::new();
            if line.not_approved {
                flags.push(t("cash.flag.not_approved"));
            }
            if line.estimate {
                flags.push(t("cash.flag.estimate"));
            }
            if later {
                flags.push(t("cash.flag.not_counted"));
            }
            total += line.amount;
            let label = if matches!(line.kind, LineKind::Bill | LineKind::BudgetFill)
                && line.total_only
                && !open
            {
                t("cash.drawings")
            } else {
                line.label.clone()
            };
            views.push(LineView {
                label,
                kind: line
                    .source
                    .clone()
                    .unwrap_or_else(|| t(&format!("cash.kind.{}", line.kind.code()))),
                party: if open {
                    party_name(line.party).await?
                } else {
                    String::new()
                },
                date: line.date.format("%Y-%m-%d").to_string(),
                amount: amount_text("AED", line.amount),
                flags,
                links: if open {
                    document_links(&state.db, &user, &line.documents).await?
                } else {
                    Vec::new()
                },
            });
        }
    } else if let Some(kind) = MemoKind::ALL.iter().find(|k| k.code() == side) {
        for memo in forecast
            .memos
            .iter()
            .filter(|m| m.week == week && m.kind == *kind)
        {
            total += memo.amount;
            views.push(LineView {
                label: memo.label.clone(),
                kind: kind_text(&t, *kind),
                party: party_name(memo.party).await?,
                date: String::new(),
                amount: amount_text("AED", memo.amount),
                flags: vec![t("cash.flag.not_counted")],
                links: document_links(&state.db, &user, &memo.documents).await?,
            });
        }
    }
    let heading = match (direction, forecast.weeks.get(week)) {
        _ if later => format!(
            "{} {}",
            t("cash.after"),
            forecast.weeks.last().map_or(0, |w| w.number)
        ),
        (Some(Direction::In), Some(w)) => t("cash.row.receipts") + &week_text(&t, w),
        (Some(Direction::Out), Some(w)) => t("cash.row.payments") + &week_text(&t, w),
        (None, Some(w)) if fill.is_some() => t("cash.row.fill") + &week_text(&t, w),
        (None, Some(w)) => {
            kind_text(
                &t,
                MemoKind::ALL
                    .iter()
                    .copied()
                    .find(|k| k.code() == side)
                    .unwrap_or(MemoKind::Disputed),
            ) + &week_text(&t, w)
        }
        _ => t("cash.title"),
    };
    Ok(View::typed(CellPage {
        tx: texts::all(&lang),
        title: heading,
        back: match requested {
            Some(id) => format!("/cash-forecast?entity={id}"),
            None => "/cash-forecast".to_owned(),
        },
        lines: views,
        total: amount_text("AED", total),
    }))
}

fn week_text(t: &dyn Fn(&str) -> String, week: &cash_forecast::Week) -> String {
    format!(
        ", {} {} ({} - {})",
        t("cash.week"),
        week.number,
        week.start.format("%Y-%m-%d"),
        week.end.format("%Y-%m-%d")
    )
}

/// The form that enters an opening cash balance.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct BalanceForm {
    /// The legal entity's id.
    #[validate(required)]
    pub legal_entity_id: Option<i64>,
    /// The bank account label.
    #[validate(required, max = 100)]
    pub bank_account: Option<String>,
    /// The balance in dirhams, like `60000.00` (may be negative).
    #[validate(required, max = 32)]
    pub amount: Option<String>,
    /// The day it is as of.
    #[validate(required)]
    pub as_of: Option<NaiveDate>,
    /// A note.
    #[validate(max = 500)]
    pub note: Option<String>,
}

/// The form that voids a balance.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct VoidForm {
    /// Why it is voided.
    #[validate(required, max = 500)]
    pub reason: Option<String>,
}

fn invalid<F: Serialize>(field: &str, message: String, form: &F) -> Error {
    let mut errors = Errors::new();
    errors.add(field, message);
    Error::Validation(ValidationError::new(errors).with_input(form))
}

/// `POST /cash-balances`: enters an opening cash balance.
async fn store_balance(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Valid(form): Valid<BalanceForm>,
) -> Result<Response> {
    let t = |key: &str| texts::t(&lang, &format!("wl_invoicing.{key}"));
    let (Some(entity), Some(as_of)) = (form.legal_entity_id, form.as_of) else {
        return Err(invalid(
            "legal_entity_id",
            t("cash.balances.bad_entity"),
            &form,
        ));
    };
    if !user
        .scopes_with::<LegalEntity>(permissions::CASHFLOW_MANAGE)
        .contains(&entity)
    {
        return Err(Error::Forbidden);
    }
    let Some(amount) = form.amount.as_deref().and_then(cash_balance::parse_balance) else {
        return Err(invalid("amount", t("cash.balances.bad_amount"), &form));
    };
    let balance = EnterBalance {
        legal_entity_id: entity,
        bank_account: form.bank_account.clone().unwrap_or_default(),
        amount,
        as_of,
        note: form.note.clone(),
    };
    if cash_balance::enter_problem(&balance, today()).is_some() {
        return Err(invalid("as_of", t("cash.balances.bad_date"), &form));
    }
    cash_balance::enter(&state.db, Some(user.id), &balance).await?;
    Ok((
        Toast::success(t("cash.balances.entered_toast")),
        Redirect::route("wl_invoicing.cash_forecast", &[])?,
    )
        .into_response())
}

/// `POST /cash-balances/{id}/void`: voids a balance with a reason.
async fn void_balance(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<VoidForm>,
) -> Result<Response> {
    let t = |key: &str| texts::t(&lang, &format!("wl_invoicing.{key}"));
    let row = OpeningCashBalance::find_or_404(&state.db, id).await?;
    if !user
        .scopes_with::<LegalEntity>(permissions::CASHFLOW_MANAGE)
        .contains(&row.legal_entity_id)
    {
        return Err(Error::Forbidden);
    }
    let reason = form.reason.clone().unwrap_or_default();
    if reason.trim().is_empty() {
        return Err(invalid("reason", t("cash.balances.reason_required"), &form));
    }
    cash_balance::void(&state.db, id, Some(user.id), &reason).await?;
    Ok((
        Toast::success(t("cash.balances.voided_toast")),
        Redirect::route("wl_invoicing.cash_forecast", &[])?,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::texts::english;

    fn english_text(key: &str) -> String {
        english(&format!("wl_invoicing.{key}")).to_owned()
    }

    #[test]
    fn the_grid_shows_whole_dirhams() {
        assert_eq!(whole_text(0), "0");
        assert_eq!(whole_text(49), "0");
        assert_eq!(whole_text(50), "1");
        assert_eq!(whole_text(1_050_000), "10,500");
        assert_eq!(whole_text(-1_234_550), "-12,346");
        assert_eq!(whole_text(123_456_789), "1,234,568");
    }

    #[test]
    fn a_document_opens_on_its_own_page() {
        assert_eq!(
            document_link(&Subject::new("invoicing.invoice", 4)),
            "/invoices/4"
        );
        assert_eq!(
            document_link(&Subject::new("payments.cheque", 7)),
            "/cheques/7"
        );
        assert_eq!(document_link(&Subject::new("property.lease", 1)), "");
    }

    #[test]
    fn a_total_line_opens_only_for_drawings_and_a_holder() {
        let mut line = Line {
            legal_entity_id: 1,
            kind: LineKind::Bill,
            label: "SB-1".into(),
            category: "bill".into(),
            source: None,
            party: None,
            date: NaiveDate::from_ymd_opt(2027, 1, 1).unwrap(),
            week: 0,
            amount: 1,
            direction: Direction::Out,
            documents: vec![],
            not_approved: false,
            estimate: false,
            total_only: true,
        };
        assert!(may_open(&line, true));
        assert!(!may_open(&line, false));
        line.kind = LineKind::Source;
        assert!(!may_open(&line, true));
        line.total_only = false;
        assert!(may_open(&line, false));
    }

    #[test]
    fn every_memo_kind_and_line_kind_has_a_label() {
        for kind in MemoKind::ALL {
            assert_ne!(
                english_text(&format!("cash.memo.{}", kind.code())),
                format!("cash.memo.{}", kind.code())
            );
        }
        for kind in [
            LineKind::Invoice,
            LineKind::ChequeIn,
            LineKind::Bill,
            LineKind::ChequeOut,
            LineKind::Refund,
            LineKind::Vat,
            LineKind::Source,
            LineKind::BudgetFill,
        ] {
            let key = format!("cash.kind.{}", kind.code());
            assert_ne!(english_text(&key), key);
        }
    }
}
