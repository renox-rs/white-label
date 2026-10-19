//! The cheque pages (wl-payments §4 *Cheque*, §6, A10):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /cheques` | `wl_payments.cheques.index` | `cheques.view` |
//! | `GET /cheques/{id}` | `wl_payments.cheques.show` | `cheques.view` |
//! | `POST /cheques/{id}/deposit` | `wl_payments.cheques.deposit` | `cheques.deposit` |
//! | `POST /cheques/{id}/clear` | `wl_payments.cheques.clear` | `cheques.deposit` |
//! | `POST /cheques/{id}/bounce` | `wl_payments.cheques.bounce` | `cheques.deposit` (fields `reason`, `certificate`, `honoured`) |
//! | `POST /cheques/{id}/represent` | `wl_payments.cheques.represent` | `cheques.deposit` (fields `date`, `funds_confirmed`, `note`) |
//! | `POST /cheques/{id}/settle` | `wl_payments.cheques.settle` | `cheques.manage` (fields `method`, `amount`, `reference`) |
//! | `POST /cheques/{id}/replacements` | `wl_payments.cheques.replacements` | `cheques.manage` (fields `kind`, `ref_id`) |
//! | `POST /cheques/{id}/replace` | `wl_payments.cheques.replace` | `cheques.manage` |
//! | `POST /cheques/{id}/write_off` | `wl_payments.cheques.write_off` | `cheques.manage` (field `reason`) |
//! | `POST /cheques/{id}/refer` | `wl_payments.cheques.refer` | `cheques.legal` (fields `case_type`, `case_owner`) |
//!
//! Every action has its own route: the transitions take input (a return
//! reason and certificate, a date, an amount) that the kernel's transition
//! route does not carry. A refused guard answers 422 under its field.

use std::collections::BTreeMap;

use renox::auth::User;
use renox::auth::permissions::{Scope, users_with_role_in};
use renox::chrono::NaiveDate;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{self, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::cheques::ChequeStatus;
use wl_kernel::invoicing::INVOICE_SUBJECT_TYPE;
use wl_kernel::payments::{Direction, PaymentMethod};

use crate::cheque::{Cheque, ChequeReturn};
use crate::cheque_bounce::{self, BounceRequest};
use crate::cheque_deposit;
use crate::cheque::CaseType;
use crate::cheque_legal::{self, ReferRequest};
use crate::cheque_represent::{self, RepresentRequest};
use crate::cheque_settle::{
    self, ChequeReplacement, ReplacementKind, SETTLE_METHODS, SettleRequest,
};
use crate::cheque_write_off::{self, ChequeWriteOff};
use crate::payment::Payment;
use crate::followup::{ACCOUNTANT_ROLE, FollowupTask, money_text};
use crate::pages::{
    Text, Texts, filled, invalid, options, parse_amount, party_name, texts_in,
};
use crate::permissions;
use crate::texts;

/// The statuses of a cheque, in life-cycle order.
const STATUSES: [ChequeStatus; 9] = [
    ChequeStatus::Received,
    ChequeStatus::DueForDeposit,
    ChequeStatus::Deposited,
    ChequeStatus::Cleared,
    ChequeStatus::Bounced,
    ChequeStatus::LegalCase,
    ChequeStatus::Replaced,
    ChequeStatus::WrittenOff,
    ChequeStatus::Returned,
];

/// The bank return reasons offered, as `(code, the text stored)`. The text
/// is compared without case against the criminal-grounds list of the
/// settings.
pub(crate) const RETURN_REASONS: [(&str, &str); 5] = [
    ("insufficient_funds", "insufficient funds"),
    ("account_closed", "account closed"),
    ("payment_stopped", "payment stopped by the drawer"),
    ("signature_irregular", "signature or alteration irregular"),
    ("other", "other"),
];

/// The reason text of the code `code`, if it is one offered.
pub(crate) fn reason_of(code: &str) -> Option<&'static str> {
    RETURN_REASONS
        .iter()
        .find(|(c, _)| *c == code.trim())
        .map(|(_, text)| *text)
}

/// The module's cheque routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/cheques", index)
                .name("wl_payments.cheques.index")
                .get("/cheques/{id}", show)
                .name("wl_payments.cheques.show")
                .require_permission(permissions::CHEQUES_VIEW),
        )
        .merge(
            Routes::new()
                .post("/cheques/{id}/deposit", store_deposit)
                .name("wl_payments.cheques.deposit")
                .post("/cheques/{id}/clear", store_clear)
                .name("wl_payments.cheques.clear")
                .post("/cheques/{id}/bounce", store_bounce)
                .name("wl_payments.cheques.bounce")
                .post("/cheques/{id}/represent", store_represent)
                .name("wl_payments.cheques.represent")
                .require_permission(permissions::CHEQUES_DEPOSIT),
        )
        .merge(
            Routes::new()
                .post("/cheques/{id}/settle", store_settle)
                .name("wl_payments.cheques.settle")
                .post("/cheques/{id}/replacements", store_replacement)
                .name("wl_payments.cheques.replacements")
                .post("/cheques/{id}/replace", store_replace)
                .name("wl_payments.cheques.replace")
                .post("/cheques/{id}/write_off", store_write_off)
                .name("wl_payments.cheques.write_off")
                .require_permission(permissions::CHEQUES_MANAGE),
        )
        .merge(
            Routes::new()
                .post("/cheques/{id}/refer", store_refer)
                .name("wl_payments.cheques.refer")
                .require_permission(permissions::CHEQUES_LEGAL),
        )
}

/// The badge colour of a status.
pub(crate) fn status_kind(status: ChequeStatus) -> &'static str {
    match status {
        ChequeStatus::Received => "info",
        ChequeStatus::DueForDeposit => "warning",
        ChequeStatus::Deposited => "info",
        ChequeStatus::Cleared => "success",
        ChequeStatus::Bounced | ChequeStatus::LegalCase => "error",
        ChequeStatus::Replaced => "success",
        ChequeStatus::WrittenOff => "warning",
        ChequeStatus::Returned => "info",
    }
}

/// The cheques grid: party, number, bank, date, amounts, direction and status.
pub fn grid(lang: &Lang) -> Grid {
    grid_with(&texts_in(lang))
}

/// [`grid`] with the texts of `t`.
fn grid_with(t: Text<'_>) -> Grid {
    let codes: Vec<&str> = STATUSES.iter().map(|s| s.as_str()).collect();
    Grid::new("wl-payment-cheques")
        .title(&t("cheques.title"))
        .column(
            Column::number("id", "#")
                .frozen()
                .mobile()
                .link("/cheques/{id}"),
        )
        .column(
            Column::text("party_name", &t("fields.party"))
                .computed(
                    "(SELECT \"wl_parties_parties\".\"display_name\" FROM \"wl_parties_parties\" \
                     WHERE \"wl_parties_parties\".\"id\" = {T}.\"party_id\")",
                )
                .mobile()
                .searchable(),
        )
        .column(Column::text("number", &t("cheques.number")).searchable().mobile())
        .column(Column::text("bank", &t("cheques.bank")).searchable())
        .column(Column::date("cheque_date", &t("cheques.date")))
        .column(Column::money("amount", &t("fields.amount")).currency_from("currency"))
        .column(Column::money("open_amount", &t("cheques.open")).currency_from("currency"))
        .column(Column::select(
            "direction",
            &t("fields.direction"),
            options(&t, "cheques.direction", &["in", "out"]),
        ))
        .column(
            Column::select(
                "status",
                &t("fields.status"),
                options(&t, "cheques.status", &codes),
            )
            .badges(&[
                ("received", "info"),
                ("due_for_deposit", "warning"),
                ("deposited", "info"),
                ("cleared", "success"),
                ("bounced", "danger"),
                ("legal_case", "danger"),
                ("replaced", "neutral"),
                ("written_off", "neutral"),
                ("returned", "neutral"),
            ])
            .mobile(),
        )
        .column(Column::datetime("created_at", &t("fields.created_at")).hidden())
        .sort_by("-cheque_date")
        .row_url("/cheques/{id}")
        .cards_on_mobile()
        .empty_state(&t("cheques.empty"), Some(&t("cheques.empty_hint")))
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-payments/cheques.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    cheques: GridPage<Cheque>,
}

/// `GET /cheques`: the grid.
async fn index(lang: Lang, request: GridRequest) -> Result<View> {
    let cheques = grid(&lang).page(Cheque::query(), &request).await?;
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        cheques,
    }))
}

/// One document a cheque pays.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct LinkRow {
    label: String,
    link: Option<String>,
}

/// One bank return of a cheque.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ReturnRow {
    id: i64,
    presentation: i32,
    reason: String,
    certificate: String,
    honoured: String,
    fee: String,
    returned_on: String,
}

/// One follow-up task of a cheque.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ChequeTaskRow {
    id: i64,
    text: String,
    status: String,
    open: bool,
}

/// One replacement linked to a bounced cheque.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ReplacementRow {
    id: i64,
    kind: String,
    label: String,
    link: String,
    amount: String,
}

/// One invoice write-off a cheque asked wl-invoicing for.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct WriteOffRow {
    id: i64,
    label: String,
    link: String,
    amount: String,
    status: String,
    status_kind: &'static str,
    failure: String,
}

/// What the cheque page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-payments/cheque.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    cheque: Cheque,
    title: String,
    status_label: String,
    status_kind: &'static str,
    party_name: String,
    direction_label: String,
    /// Amount, open, honoured, settled and fees, formatted.
    amounts: BTreeMap<&'static str, String>,
    agreement_label: String,
    linked: Vec<LinkRow>,
    returns: Vec<ReturnRow>,
    tasks: Vec<ChequeTaskRow>,
    timeline: Vec<Value>,
    can_deposit: bool,
    can_clear: bool,
    can_bounce: bool,
    /// Whether "Present again" is offered: bounced, received, nothing paid by
    /// the bank, presentations left.
    can_represent: bool,
    /// Today, the earliest new presentation date.
    today: String,
    /// Whether "Settle part" is offered: a received cheque still owed.
    can_settle: bool,
    /// Whether linking a replacement and "Replace" are offered: owed.
    can_replace: bool,
    /// Whether "Write off" is offered: a received cheque still owed.
    can_write_off: bool,
    /// The invoice write-offs asked for, with where each stands.
    write_offs: Vec<WriteOffRow>,
    /// Whether "Refer to legal" is offered: a bounced received cheque with
    /// its return certificate.
    can_refer: bool,
    /// The kinds of legal case, `(code, label)`.
    case_types: Vec<(String, String)>,
    /// The users who can own the case, `(id, name)`, and the one chosen first.
    owners: Vec<(String, String)>,
    owner_default: String,
    /// The legal case, as text, and its owner's name.
    case_label: String,
    case_owner_name: String,
    /// The methods a part payment is made by, `(code, label)`.
    settle_methods: Vec<(String, String)>,
    /// The kinds of replacement, `(code, label)`.
    replacement_kinds: Vec<(String, String)>,
    /// The replacements linked, and what they cover against what is open.
    replacements: Vec<ReplacementRow>,
    covered: String,
    covers_open_amount: bool,
    /// The return reasons offered, `(code, label)`.
    reasons: Vec<(String, String)>,
    /// Whether a certificate is required (a received cheque).
    certificate_required: bool,
    bounce_description: String,
}

/// The documents `cheque` pays as rows.
async fn link_rows(db: &Db, cheque: &Cheque) -> Result<Vec<LinkRow>> {
    Ok(cheque
        .linked(db)
        .await?
        .iter()
        .map(|document| LinkRow {
            label: document.to_string(),
            link: (document.subject_type == INVOICE_SUBJECT_TYPE)
                .then(|| format!("/invoices/{}", document.subject_id)),
        })
        .collect())
}

/// The return rows of `cheque`.
async fn return_rows(db: &Db, cheque: &Cheque) -> Result<Vec<ReturnRow>> {
    Ok(cheque
        .returns(db)
        .await?
        .iter()
        .map(|r: &ChequeReturn| ReturnRow {
            id: r.id,
            presentation: r.presentation,
            reason: r.reason.clone(),
            certificate: r.certificate.clone().unwrap_or_default(),
            honoured: money_text(&cheque.currency, r.honoured_amount),
            fee: if r.fee_amount > 0 {
                money_text(&cheque.currency, r.fee_amount)
            } else {
                String::new()
            },
            returned_on: r.returned_on.format("%Y-%m-%d").to_string(),
        })
        .collect())
}

/// The task rows of `cheque`.
async fn task_rows(db: &Db, t: Text<'_>, cheque: &Cheque) -> Result<Vec<ChequeTaskRow>> {
    Ok(FollowupTask::of_cheque(db, cheque.id)
        .await?
        .iter()
        .map(|task| ChequeTaskRow {
            id: task.id,
            text: task.text.clone(),
            status: t(if task.is_open() {
                "tasks.open"
            } else {
                "tasks.finished"
            }),
            open: task.is_open(),
        })
        .collect())
}

/// The users who can own a legal case, `(id, name)`: the holders of the
/// accountant role and `user`.
async fn case_owners(state: &AppState, user: &AuthUser) -> Result<Vec<(String, String)>> {
    let mut owners: Vec<(String, String)> =
        users_with_role_in(&state.db, ACCOUNTANT_ROLE, &Scope::global())
            .await?
            .into_iter()
            .map(|owner| (owner.id.to_string(), owner.name))
            .collect();
    if !owners.iter().any(|(id, _)| *id == user.id.to_string()) {
        owners.insert(0, (user.id.to_string(), user.name.clone()));
    }
    Ok(owners)
}

/// The replacement rows of `cheque`.
async fn replacement_rows(db: &Db, t: Text<'_>, cheque: &Cheque) -> Result<Vec<ReplacementRow>> {
    let mut rows = Vec::new();
    for link in ChequeReplacement::of_cheque(db, cheque.id).await? {
        let (label, url) = match link.kind {
            ReplacementKind::Cheque => {
                let number = Cheque::find(db, link.ref_id)
                    .await?
                    .map_or_else(|| format!("#{}", link.ref_id), |c| c.number);
                (
                    format!("{} {number}", t("cheques.heading")),
                    format!("/cheques/{}", link.ref_id),
                )
            }
            ReplacementKind::Payment => {
                let number = Payment::find(db, link.ref_id)
                    .await?
                    .and_then(|p| p.number)
                    .unwrap_or_else(|| format!("#{}", link.ref_id));
                (
                    format!("{} {number}", t("cheques.replacement.payment")),
                    format!("/payments/{}", link.ref_id),
                )
            }
        };
        rows.push(ReplacementRow {
            id: link.id,
            kind: t(&format!("cheques.replacement.{}", link.kind.as_str())),
            label,
            link: url,
            amount: money_text(&cheque.currency, link.amount),
        });
    }
    Ok(rows)
}

/// The invoice write-off rows of `cheque`.
async fn write_off_rows(
    state: &AppState,
    t: Text<'_>,
    cheque: &Cheque,
) -> Result<Vec<WriteOffRow>> {
    let mut rows = Vec::new();
    for request in ChequeWriteOff::of_cheque(&state.db, cheque.id).await? {
        let invoice = wl_kernel::invoicing::invoicer(state)
            .find(&state.db, wl_kernel::invoicing::InvoiceId(request.invoice_id))
            .await?;
        let label = invoice
            .and_then(|i| i.number)
            .unwrap_or_else(|| format!("#{}", request.invoice_id));
        rows.push(WriteOffRow {
            id: request.id,
            label,
            link: format!("/invoices/{}", request.invoice_id),
            amount: money_text(&cheque.currency, request.amount),
            status: t(&format!(
                "cheques.write_off.status.{}",
                request.status.as_str()
            )),
            status_kind: match request.status {
                cheque_write_off::WriteOffStatus::Pending => "warning",
                cheque_write_off::WriteOffStatus::WrittenOff => "success",
                cheque_write_off::WriteOffStatus::Failed => "error",
            },
            failure: request.failure.clone().unwrap_or_default(),
        });
    }
    Ok(rows)
}

/// `GET /cheques/{id}`: the card, the documents it pays, its bank returns,
/// its tasks, the actions and the history.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(cheque): Found<Cheque>,
) -> Result<View> {
    let t = texts_in(&lang);
    let db = state.db.clone();
    let money = |fils: i64| money_text(&cheque.currency, fils);
    let amounts = BTreeMap::from([
        ("amount", money(cheque.amount)),
        ("open", money(cheque.open_amount)),
        ("honoured", money(cheque.honoured_amount)),
        ("settled", money(cheque.settled_amount)),
        ("fees", money(cheque.bounce_fee_total)),
    ]);
    let history = state_machine::history(&db, Cheque::TABLE, cheque.id).await?;
    let mut timeline =
        state_machine::timeline(&history, Some(&state_machine::info_of::<Cheque>()));
    timeline.sort_by(|a, b| a["time"].as_str().cmp(&b["time"].as_str()));
    let deposit_rights = user.has_permission(permissions::CHEQUES_DEPOSIT);
    let can_deposit = deposit_rights
        && matches!(
            cheque.status,
            ChequeStatus::Received | ChequeStatus::DueForDeposit
        );
    let can_clear = deposit_rights && cheque.status == ChequeStatus::Deposited;
    let can_bounce = can_clear;
    let config = crate::config_of(&state);
    let can_represent = deposit_rights
        && cheque.status == ChequeStatus::Bounced
        && cheque.is_received()
        && cheque.honoured_amount == 0
        && i64::from(cheque.presentations) < config.cheque_max_presentations;
    let reason_codes: Vec<&str> = RETURN_REASONS.iter().map(|(code, _)| *code).collect();
    let manage_rights = user.has_permission(permissions::CHEQUES_MANAGE);
    let can_replace = manage_rights && cheque.status.is_owed();
    let can_settle = can_replace && cheque.is_received();
    let can_write_off = can_settle;
    let write_offs = write_off_rows(&state, &t, &cheque).await?;
    let can_refer = user.has_permission(permissions::CHEQUES_LEGAL)
        && cheque.status == ChequeStatus::Bounced
        && cheque.is_received()
        && cheque.return_certificate.is_some();
    let owners = if can_refer {
        case_owners(&state, &user).await?
    } else {
        Vec::new()
    };
    let case_owner_name = match cheque.case_owner_id {
        Some(id) => User::find(&db, id)
            .await?
            .map_or_else(|| format!("#{id}"), |owner| owner.name),
        None => String::new(),
    };
    let replacements = replacement_rows(&db, &t, &cheque).await?;
    let covered = cheque_settle::covered_by_replacements(&db, &cheque).await?;
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        title: format!("{} {}", t("cheques.heading"), cheque.number),
        status_label: t(&format!("cheques.status.{}", cheque.status.as_str())),
        status_kind: status_kind(cheque.status),
        party_name: party_name(&state, cheque.party()).await?,
        direction_label: t(&format!("cheques.direction.{}", cheque.direction.as_str())),
        amounts,
        agreement_label: cheque
            .agreement()
            .map(|a| a.to_string())
            .unwrap_or_default(),
        linked: link_rows(&db, &cheque).await?,
        returns: return_rows(&db, &cheque).await?,
        tasks: task_rows(&db, &t, &cheque).await?,
        timeline,
        can_deposit,
        can_clear,
        can_bounce,
        can_represent,
        can_refer,
        case_types: options(&t, "cheques.case", &["civil_execution", "criminal_complaint"]),
        owners,
        owner_default: user.id.to_string(),
        case_label: cheque
            .case_type
            .map(|case| t(&format!("cheques.case.{}", case.as_str())))
            .unwrap_or_default(),
        case_owner_name,
        can_settle,
        can_replace,
        can_write_off,
        write_offs,
        settle_methods: options(
            &t,
            "method",
            &SETTLE_METHODS.iter().map(|m| m.as_str()).collect::<Vec<_>>(),
        ),
        replacement_kinds: options(&t, "cheques.replacement", &["cheque", "payment"]),
        replacements,
        covered: money(covered),
        covers_open_amount: covered >= cheque.open_amount && cheque.open_amount > 0,
        today: wl_kernel::documents::today().format("%Y-%m-%d").to_string(),
        reasons: options(&t, "cheques.reason", &reason_codes),
        certificate_required: cheque.direction == Direction::In,
        bounce_description: t(if cheque.direction == Direction::In {
            "cheques.bounce.description"
        } else {
            "cheques.bounce.description_issued"
        }),
        cheque,
    }))
}

/// `POST /cheques/{id}/deposit`: deposits the cheque and reloads the page.
async fn store_deposit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(cheque): Found<Cheque>,
) -> Result<Response> {
    cheque_deposit::deposit(&state, &user, &cheque)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(texts::t(&lang, "wl_payments.cheques.deposit.done")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /cheques/{id}/clear`: clears the cheque (records its payment) and
/// reloads the page.
async fn store_clear(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(cheque): Found<Cheque>,
) -> Result<Response> {
    cheque_deposit::clear(&state, &user, &cheque)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(texts::t(&lang, "wl_payments.cheques.clear.done")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The bounce sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct BounceForm {
    /// The reason code, one of [`RETURN_REASONS`].
    #[validate(max = 32)]
    pub reason: Option<String>,
    /// The reference of the bank's return certificate.
    #[validate(max = 200)]
    pub certificate: Option<String>,
    /// What the bank paid, in dirhams like `6000.00`; empty for nothing.
    #[validate(max = 32)]
    pub honoured: Option<String>,
}

/// The honoured amount in `form` in fils: 0 when blank; `None` when it is
/// not an amount.
fn honoured_of(form: &BounceForm) -> Option<i64> {
    match filled(&form.honoured) {
        None => Some(0),
        Some(text) if text.trim_matches(|c| c == '0' || c == '.' || c == ',').is_empty() => {
            Some(0)
        }
        Some(text) => parse_amount(text),
    }
}

/// `POST /cheques/{id}/bounce`: records the bank's return and reloads the
/// page. A refused guard (no certificate, a honoured amount that is not below
/// the cheque) answers 422 under its field.
async fn store_bounce(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(cheque): Found<Cheque>,
    Valid(form): Valid<BounceForm>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let Some(reason) = filled(&form.reason).and_then(reason_of) else {
        return Err(invalid("reason", t("cheques.bounce.reason_required"), &form));
    };
    let Some(honoured) = honoured_of(&form) else {
        return Err(invalid("honoured", t("cheques.bounce.honoured_invalid"), &form));
    };
    let request = BounceRequest {
        reason: reason.to_owned(),
        certificate: form.certificate.clone(),
        honoured,
    };
    match cheque_bounce::bounce(&state, &user, &cheque, &request).await {
        Ok(_) => Ok((
            Toast::success(t("cheques.bounce.done")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::Guard(message)) => {
            let field = if message.contains("certificate") {
                "certificate"
            } else if message.contains("honoured") {
                "honoured"
            } else {
                "reason"
            };
            Err(invalid(field, message, &form))
        }
        Err(other) => Err(other.into_error()),
    }
}

/// The re-present sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct RepresentForm {
    /// The new presentation date.
    #[validate(required)]
    pub date: Option<NaiveDate>,
    /// Set when the party confirmed there are funds.
    #[serde(default)]
    pub funds_confirmed: bool,
    /// A note, e.g. how the funds were confirmed.
    #[validate(max = 2000)]
    pub note: Option<String>,
}

/// `POST /cheques/{id}/represent`: presents the bounced cheque again and
/// reloads the page. A refused guard (no confirmation or note, a past date,
/// too many presentations) answers 422 under its field.
async fn store_represent(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(cheque): Found<Cheque>,
    Valid(form): Valid<RepresentForm>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let Some(date) = form.date else {
        return Err(invalid("date", t("cheques.represent.date_required"), &form));
    };
    let request = RepresentRequest {
        date,
        funds_confirmed: form.funds_confirmed,
        note: form.note.clone(),
    };
    match cheque_represent::represent(&state, &user, &cheque, &request).await {
        Ok(_) => Ok((
            Toast::success(t("cheques.represent.done")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::Guard(message)) => {
            let field = if message.contains("date") {
                "date"
            } else {
                "note"
            };
            Err(invalid(field, message, &form))
        }
        Err(other) => Err(other.into_error()),
    }
}

/// The part-payment sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct SettleForm {
    /// The method code: `cash`, `transfer` or `card`.
    #[validate(max = 32)]
    pub method: Option<String>,
    /// The amount in dirhams, like `4000.00`.
    #[validate(max = 32)]
    pub amount: Option<String>,
    /// The transfer's or card slip's reference.
    #[validate(max = 200)]
    pub reference: Option<String>,
}

/// The method with code `code`, if a part payment can be made by it.
fn settle_method_of(code: &str) -> Option<PaymentMethod> {
    SETTLE_METHODS
        .iter()
        .copied()
        .find(|method| method.as_str() == code.trim())
}

/// `POST /cheques/{id}/settle`: records the party's part payment and reloads
/// the page. A refused guard (not below the open amount, a transfer without a
/// reference) answers 422 under its field.
async fn store_settle(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(cheque): Found<Cheque>,
    Valid(form): Valid<SettleForm>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let Some(method) = filled(&form.method).and_then(settle_method_of) else {
        return Err(invalid("method", t("cheques.settle.method_required"), &form));
    };
    let Some(amount) = filled(&form.amount).and_then(parse_amount) else {
        return Err(invalid("amount", t("cheques.settle.amount_required"), &form));
    };
    let request = SettleRequest {
        method,
        amount,
        reference: form.reference.clone(),
    };
    match cheque_settle::settle_part(&state, &user, &cheque, &request).await {
        Ok(_) => Ok((Toast::success(t("cheques.settle.done")), renox::HxRefresh).into_response()),
        Err(TransitionError::Guard(message)) => {
            let field = if message.contains("reference") {
                "reference"
            } else if message.contains("method") || message.contains("cash") {
                "method"
            } else {
                "amount"
            };
            Err(invalid(field, message, &form))
        }
        Err(other) => Err(other.into_error()),
    }
}

/// The replacement sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct ReplacementForm {
    /// `cheque` or `payment`.
    #[validate(max = 32)]
    pub kind: Option<String>,
    /// The id of the replacement cheque or payment.
    #[validate(required)]
    pub ref_id: Option<i64>,
}

/// The replacement kind with code `code`.
fn replacement_kind_of(code: &str) -> Option<ReplacementKind> {
    ReplacementKind::ALL
        .iter()
        .copied()
        .find(|kind| kind.as_str() == code.trim())
}

/// `POST /cheques/{id}/replacements`: links a cheque or a payment as the
/// replacement and reloads the page. A refusal (another party, already
/// linked) answers 422 under the id.
async fn store_replacement(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(cheque): Found<Cheque>,
    Valid(form): Valid<ReplacementForm>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let Some(kind) = filled(&form.kind).and_then(replacement_kind_of) else {
        return Err(invalid("kind", t("cheques.replacement.kind_required"), &form));
    };
    let Some(ref_id) = form.ref_id else {
        return Err(invalid("ref_id", t("cheques.replacement.id_required"), &form));
    };
    match cheque_settle::link_replacement(&state, &user, &cheque, kind, ref_id).await {
        Ok(_) => Ok((
            Toast::success(t("cheques.replacement.done")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::Guard(message)) => Err(invalid("ref_id", message, &form)),
        Err(other) => Err(other.into_error()),
    }
}

/// `POST /cheques/{id}/replace`: marks the cheque replaced and reloads the
/// page. Replacements that do not cover the open amount answer 422.
async fn store_replace(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(cheque): Found<Cheque>,
) -> Result<Response> {
    cheque_settle::replace(&state, &user, &cheque)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(texts::t(&lang, "wl_payments.cheques.replace.done")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The write-off sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct WriteOffForm {
    /// Why the open amount is written off.
    #[validate(max = 2000)]
    pub reason: Option<String>,
}

/// `POST /cheques/{id}/write_off`: asks wl-invoicing to write off the linked
/// invoices and reloads the page. A cheque whose invoices were all written off
/// at once is `written_off`; otherwise it stays while the approvals wait. A
/// refused guard (no reason, balances that differ, a disputed invoice) answers
/// 422 under its field.
async fn store_write_off(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(cheque): Found<Cheque>,
    Valid(form): Valid<WriteOffForm>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let Some(reason) = filled(&form.reason) else {
        return Err(invalid("reason", t("cheques.write_off.reason_required"), &form));
    };
    match cheque_write_off::write_off(&state, &user, &cheque, reason).await {
        Ok(done) => {
            let text = if done.status == ChequeStatus::WrittenOff {
                t("cheques.write_off.done")
            } else {
                t("cheques.write_off.requested")
            };
            Ok((Toast::success(text), renox::HxRefresh).into_response())
        }
        Err(TransitionError::Guard(message)) => Err(invalid("reason", message, &form)),
        Err(other) => Err(other.into_error()),
    }
}

/// The refer sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct ReferForm {
    /// `civil_execution` (the default) or `criminal_complaint`.
    #[validate(max = 32)]
    pub case_type: Option<String>,
    /// The case owner's user id; the user who refers when empty.
    pub case_owner: Option<i64>,
}

/// The case type with code `code`.
fn case_type_of(code: &str) -> Option<CaseType> {
    CaseType::ALL
        .iter()
        .copied()
        .find(|case| case.as_str() == code.trim())
}

/// `POST /cheques/{id}/refer`: refers the bounced cheque to a legal case and
/// reloads the page. A refused guard (a criminal complaint for "insufficient
/// funds", no certificate) answers 422 under its field.
async fn store_refer(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(cheque): Found<Cheque>,
    Valid(form): Valid<ReferForm>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let case_type = match filled(&form.case_type) {
        None => CaseType::CivilExecution,
        Some(code) => match case_type_of(code) {
            Some(case) => case,
            None => return Err(invalid("case_type", t("cheques.refer.case_required"), &form)),
        },
    };
    let request = ReferRequest {
        case_type,
        case_owner: form.case_owner,
    };
    match cheque_legal::refer_to_legal(&state, &user, &cheque, &request).await {
        Ok(_) => Ok((Toast::success(t("cheques.refer.done")), renox::HxRefresh).into_response()),
        Err(TransitionError::Guard(message)) => {
            let field = if message.contains("owner") {
                "case_owner"
            } else {
                "case_type"
            };
            Err(invalid(field, message, &form))
        }
        Err(other) => Err(other.into_error()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn english_text(key: &str) -> String {
        texts::english(&format!("wl_payments.{key}")).to_owned()
    }

    #[test]
    fn the_grid_lists_the_cheque_columns() {
        let grid = grid_with(&english_text);
        let keys: Vec<&str> = grid.columns().iter().map(Column::key).collect();
        assert_eq!(
            keys,
            [
                "id",
                "party_name",
                "number",
                "bank",
                "cheque_date",
                "amount",
                "open_amount",
                "direction",
                "status",
                "created_at"
            ]
        );
    }

    #[test]
    fn every_status_and_reason_has_a_label() {
        for status in STATUSES {
            let key = format!("cheques.status.{}", status.as_str());
            assert_ne!(english_text(&key), format!("wl_payments.{key}"), "{key}");
        }
        for (code, _) in RETURN_REASONS {
            let key = format!("cheques.reason.{code}");
            assert_ne!(english_text(&key), format!("wl_payments.{key}"), "{key}");
        }
        for direction in ["in", "out"] {
            let key = format!("cheques.direction.{direction}");
            assert_ne!(english_text(&key), format!("wl_payments.{key}"), "{key}");
        }
    }

    #[test]
    fn a_reason_code_gives_the_text_stored() {
        assert_eq!(reason_of("insufficient_funds"), Some("insufficient funds"));
        assert_eq!(reason_of(" account_closed "), Some("account closed"));
        assert_eq!(reason_of("whatever"), None);
        assert_eq!(reason_of(""), None);
    }

    #[test]
    fn the_honoured_amount_is_read_as_fils_and_blank_is_nothing() {
        let form = |text: Option<&str>| BounceForm {
            reason: None,
            certificate: None,
            honoured: text.map(str::to_owned),
        };
        assert_eq!(honoured_of(&form(None)), Some(0));
        assert_eq!(honoured_of(&form(Some("  "))), Some(0));
        assert_eq!(honoured_of(&form(Some("0"))), Some(0));
        assert_eq!(honoured_of(&form(Some("0.00"))), Some(0));
        assert_eq!(honoured_of(&form(Some("6000"))), Some(600_000));
        assert_eq!(honoured_of(&form(Some("6,000.50"))), Some(600_050));
        assert_eq!(honoured_of(&form(Some("six"))), None);
    }

    #[test]
    fn a_part_payment_is_made_in_cash_by_transfer_or_by_card() {
        for code in ["cash", "transfer", "card"] {
            assert!(settle_method_of(code).is_some(), "{code}");
        }
        assert_eq!(settle_method_of(" cash "), Some(PaymentMethod::Cash));
        for code in ["cheque", "gateway", ""] {
            assert_eq!(settle_method_of(code), None, "{code}");
        }
        assert_eq!(replacement_kind_of("cheque"), Some(ReplacementKind::Cheque));
        assert_eq!(replacement_kind_of("payment"), Some(ReplacementKind::Payment));
        assert_eq!(replacement_kind_of("loan"), None);
    }

    #[test]
    fn a_case_is_a_civil_execution_or_a_criminal_complaint() {
        assert_eq!(case_type_of("civil_execution"), Some(CaseType::CivilExecution));
        assert_eq!(case_type_of(" criminal_complaint "), Some(CaseType::CriminalComplaint));
        assert_eq!(case_type_of("arbitration"), None);
        for case in CaseType::ALL {
            let key = format!("cheques.case.{}", case.as_str());
            assert_ne!(english_text(&key), format!("wl_payments.{key}"), "{key}");
        }
    }

    #[test]
    fn every_status_has_a_badge_colour() {
        for status in STATUSES {
            assert!(
                ["info", "success", "warning", "error"].contains(&status_kind(status)),
                "{status:?}"
            );
        }
    }
}
