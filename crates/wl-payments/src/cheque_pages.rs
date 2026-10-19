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
//!
//! Every action has its own route: the transitions take input (a return
//! reason and certificate, a date, an amount) that the kernel's transition
//! route does not carry. A refused guard answers 422 under its field.

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{self, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::cheques::ChequeStatus;
use wl_kernel::invoicing::INVOICE_SUBJECT_TYPE;
use wl_kernel::payments::Direction;

use crate::cheque::{Cheque, ChequeReturn};
use crate::cheque_bounce::{self, BounceRequest};
use crate::cheque_deposit;
use crate::cheque_represent::{self, RepresentRequest};
use crate::followup::{FollowupTask, money_text};
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
    fn every_status_has_a_badge_colour() {
        for status in STATUSES {
            assert!(
                ["info", "success", "warning", "error"].contains(&status_kind(status)),
                "{status:?}"
            );
        }
    }
}
