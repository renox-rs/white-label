//! The enrolment pages (10 `EDU.ENR.01`, §4.1, §7.1 *Enrolment*).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /edtech/enrolments` | `edtech.enrolments` | `enrolments.view` |
//! | `POST /edtech/enrolments` | `edtech.enrolment_create` | `enrolments.manage` |
//! | `GET /edtech/enrolments/{id}` | `edtech.enrolment` | `enrolments.view` |
//! | `GET /edtech/parties/options` | `edtech.party_options` | `enrolments.manage` |
//! | `POST /edtech/enrolments/{id}/terms` | `edtech.enrolment_terms` | `enrolments.manage` |
//! | `POST /edtech/enrolments/{id}/discount` | `edtech.enrolment_discount` | `enrolments.approve` |
//! | `POST /edtech/enrolments/{id}/activate` | `edtech.enrolment_activate` | `enrolments.manage` |
//! | `POST /edtech/enrolments/{id}/freeze` | `edtech.enrolment_freeze` | `enrolments.manage` |
//! | `POST /edtech/enrolments/{id}/unfreeze` | `edtech.enrolment_unfreeze` | `enrolments.manage` |
//! | `POST /edtech/enrolments/{id}/withdraw` | `edtech.enrolment_withdraw` | `enrolments.manage` |
//! | `POST /edtech/enrolments/{id}/transfer` | `edtech.enrolment_transfer` | `enrolments.manage` |
//! | `POST /edtech/enrolments/{id}/settle` | `edtech.enrolment_settle` | `enrolments.manage` |
//! | `GET /edtech/enrolments/{id}/collect` | `edtech.enrolment_collect` | `enrolments.view` |
//! | `POST /edtech/enrolments/{id}/payment-link` | `edtech.enrolment_payment_link` | `enrolments.manage` |
//!
//! The list shows each enrolment with the student, what it is for, the fee and
//! its status in words. The new enrolment form picks the student, the guardian
//! and the payer with the party picker (`PartyDirectory::search`; a code such
//! as `P-00023` is accepted) and the batch from the open batches with their
//! free seats. The transitions are the machine's: the pages only read the
//! form and call [`crate::enrolments`].

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::approvals::{self, ApprovalCard};
use wl_kernel::documents::dubai_text;
use wl_kernel::parties::directory;

use super::{
    Texts, optional_party_of, parse_date, parse_money, parse_number, party_of, require, text,
    ticked,
};
use crate::batch::{Batch, BatchStatus};
use crate::certificate::Certificate;
use crate::course::Course;
use crate::enrolment::{
    Enrolment, EnrolmentFreeze, EnrolmentKind, EnrolmentStatus, FreezeCause, Instalment,
    InstalmentStatus,
};
use crate::enrolments::{self, NewEnrolment, party_name};
use crate::money_text;
use crate::permissions;
use crate::seats;
use crate::texts;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/edtech/enrolments", index)
        .name("edtech.enrolments")
        .post("/edtech/enrolments", create)
        .name("edtech.enrolment_create")
        .get("/edtech/parties/options", party_options)
        .name("edtech.party_options")
        .get("/edtech/enrolments/{id}", show)
        .name("edtech.enrolment")
        .post("/edtech/enrolments/{id}/terms", terms)
        .name("edtech.enrolment_terms")
        .post("/edtech/enrolments/{id}/discount", discount)
        .name("edtech.enrolment_discount")
        .post("/edtech/enrolments/{id}/activate", activate)
        .name("edtech.enrolment_activate")
        .post("/edtech/enrolments/{id}/freeze", freeze)
        .name("edtech.enrolment_freeze")
        .post("/edtech/enrolments/{id}/unfreeze", unfreeze)
        .name("edtech.enrolment_unfreeze")
        .post("/edtech/enrolments/{id}/withdraw", withdraw)
        .name("edtech.enrolment_withdraw")
        .post("/edtech/enrolments/{id}/transfer", transfer)
        .name("edtech.enrolment_transfer")
        .post("/edtech/enrolments/{id}/settle", settle)
        .name("edtech.enrolment_settle")
        .get("/edtech/enrolments/{id}/collect", collect)
        .name("edtech.enrolment_collect")
        .post("/edtech/enrolments/{id}/payment-link", payment_link)
        .name("edtech.enrolment_payment_link")
        .require_auth()
}

/// The status as staff read it.
pub(crate) fn status_text(lang: &Lang, status: EnrolmentStatus) -> String {
    text(lang, &format!("enrolments.status.{}", status.as_str()))
}

/// The badge kind of a status.
pub(crate) fn status_kind(status: EnrolmentStatus) -> &'static str {
    match status {
        EnrolmentStatus::PendingPayment => "warning",
        EnrolmentStatus::Active | EnrolmentStatus::Completed => "success",
        EnrolmentStatus::Frozen
        | EnrolmentStatus::Withdrawn
        | EnrolmentStatus::Transferred
        | EnrolmentStatus::Lapsed => "info",
    }
}

/// The kind as staff read it.
fn kind_text(lang: &Lang, kind: EnrolmentKind) -> String {
    text(lang, &format!("enrolments.kind.{}", kind.as_str()))
}

/// What the enrolment is for: the batch number, or the item.
async fn subject_text(state: &AppState, enrolment: &Enrolment) -> String {
    if let Some(batch) = enrolment.batch_id
        && let Ok(Some(found)) = Batch::find(&state.db, batch).await
    {
        return found.number();
    }
    enrolment.item_name.clone().unwrap_or_default()
}

/// One enrolment of the list.
#[derive(Serialize, Debug, Clone)]
pub struct EnrolmentRow {
    /// The primary key.
    pub id: i64,
    /// `ENR-0007`.
    pub number: String,
    /// The student's name.
    pub student: String,
    /// Course, package or membership.
    pub kind: String,
    /// The batch or the item.
    pub subject: String,
    /// The fee after the discount, `AED 2,520.00`.
    pub fee: String,
    /// The status.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "edtech/enrolments.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    enrolments: Vec<EnrolmentRow>,
    kind_options: Vec<[String; 2]>,
    batch_options: Vec<[String; 2]>,
    batch_selected: String,
    can_manage: bool,
}

/// `GET /edtech/enrolments`.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    require(&user, permissions::ENROLMENTS_VIEW)?;
    let all = Enrolment::query()
        .order_by_desc(Enrolment::ID)
        .get(&state.db)
        .await?;
    let mut rows = Vec::new();
    for e in &all {
        rows.push(EnrolmentRow {
            id: e.id,
            number: e.number(),
            student: party_name(&state, &lang.locale, e.student_party_id).await,
            kind: kind_text(&lang, e.kind),
            subject: subject_text(&state, e).await,
            fee: money_text(e.fee()),
            status: status_text(&lang, e.status),
            status_kind: status_kind(e.status),
        });
    }
    let courses = Course::query().get(&state.db).await?;
    let open = Batch::query()
        .where_(Batch::STATUS.eq(BatchStatus::Open))
        .order_by(Batch::ID)
        .get(&state.db)
        .await?;
    let batch_options: Vec<[String; 2]> = open
        .iter()
        .map(|b| {
            let course = courses
                .iter()
                .find(|c| c.id == b.course_id)
                .map_or_else(String::new, Course::title);
            [
                b.id.to_string(),
                format!(
                    "{} · {} · {} {}",
                    b.number(),
                    course,
                    seats::free(b),
                    text(&lang, "enrolments.seats_free")
                ),
            ]
        })
        .collect();
    let batch_selected = batch_options
        .first()
        .map(|option| option[0].clone())
        .unwrap_or_default();
    let kind_options = [
        EnrolmentKind::Course,
        EnrolmentKind::Package,
        EnrolmentKind::Membership,
    ]
    .into_iter()
    .map(|kind| [kind.as_str().to_owned(), kind_text(&lang, kind)])
    .collect();
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        enrolments: rows,
        kind_options,
        batch_options,
        batch_selected,
        can_manage: user.has_permission(permissions::ENROLMENTS_MANAGE),
    }))
}

/// `GET /edtech/parties/options`: the parties the picker offers as the person
/// types (`?q=`), or the labels of the values it holds (`?values=`), as
/// `number · name`.
async fn party_options(
    State(state): State<AppState>,
    user: AuthUser,
    query: renox::select::OptionQuery,
) -> Result<Json<Vec<renox::select::SelectOption>>> {
    require(&user, permissions::ENROLMENTS_MANAGE)?;
    let directory = directory(&state);
    let mut parties = Vec::new();
    if query.is_lookup() {
        for id in query.values_as::<i64>() {
            parties.extend(directory.find(&state.db, PartyId(id)).await?);
        }
    } else {
        parties = directory.search(&state.db, &query.q, 20).await?;
    }
    Ok(Json(
        parties
            .into_iter()
            .map(|party| {
                renox::select::SelectOption::new(
                    party.id.0,
                    format!("{} · {}", party.code, party.display_name),
                )
            })
            .collect(),
    ))
}

/// The form that enrols a student.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct CreateForm {
    /// `course`, `package` or `membership`.
    #[serde(default)]
    pub kind: String,
    /// The student (party).
    #[serde(default)]
    pub student: String,
    /// The guardian (party), for a student under 18.
    #[serde(default)]
    pub guardian: String,
    /// The payer (party), when not the guardian or the student.
    #[serde(default)]
    pub payer: String,
    /// The batch of a course.
    #[serde(default)]
    pub batch: String,
    /// The catalogue item of a package or membership.
    #[serde(default)]
    pub item_code: String,
    /// The name of the item.
    #[serde(default)]
    pub item_name: String,
    /// The credits of a package.
    #[serde(default)]
    pub credits: String,
    /// The price in dirhams with VAT.
    #[serde(default)]
    pub price: String,
    /// The discount in dirhams.
    #[serde(default)]
    pub discount: String,
    /// The registration fee in dirhams.
    #[serde(default)]
    pub registration_fee: String,
    /// The admin fee in dirhams.
    #[serde(default)]
    pub admin_fee: String,
    /// The instalments of a course fee.
    #[serde(default)]
    pub instalments: String,
    /// Ticked when the terms and the refund policy are accepted.
    #[serde(default)]
    pub terms: String,
}

/// `POST /edtech/enrolments`.
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<CreateForm>,
) -> Result<Response> {
    require(&user, permissions::ENROLMENTS_MANAGE)?;
    let student = party_of(&state, &form.student).await?;
    let price = parse_money(&form.price)?;
    let name = form.item_name.trim();
    let mut new = match form.kind.trim() {
        "package" => NewEnrolment::package(
            student,
            form.item_code.trim(),
            name,
            parse_number(&form.credits)?,
            price,
        ),
        "membership" => NewEnrolment::membership(student, form.item_code.trim(), name, price),
        _ => NewEnrolment::course(student, parse_number(&form.batch)?, price),
    };
    new.guardian = optional_party_of(&state, &form.guardian).await?;
    new.payer = optional_party_of(&state, &form.payer).await?;
    new.discount = parse_money(&form.discount)?;
    new.registration_fee = parse_money(&form.registration_fee)?;
    new.admin_fee = parse_money(&form.admin_fee)?;
    if !form.instalments.trim().is_empty() {
        new.instalments = parse_number(&form.instalments)?;
    }
    new.terms_accepted = ticked(&form.terms);
    let enrolment = enrolments::create(&state, &user, new).await?;
    Ok((
        Toast::success(text(&lang, "enrolments.created")),
        renox::HxRedirect(format!("/edtech/enrolments/{}", enrolment.id)),
    )
        .into_response())
}

/// One instalment of the page.
#[derive(Serialize, Debug, Clone)]
pub struct InstalmentRow {
    /// 1, 2, 3 ...
    pub number: i64,
    /// The day it is invoiced.
    pub due: String,
    /// The amount, `AED 1,260.00`.
    pub amount: String,
    /// Scheduled, invoiced or cancelled.
    pub status: String,
}

/// One freeze cause of the page.
#[derive(Serialize, Debug, Clone)]
pub struct FreezeRow {
    /// Why.
    pub cause: String,
    /// The first day.
    pub since: String,
    /// The planned last day, or a dash.
    pub until: String,
    /// The day it ended, or a dash while it holds.
    pub ended: String,
}

/// One certificate of the page.
#[derive(Serialize, Debug, Clone)]
pub struct CertificateRow {
    /// The primary key.
    pub id: i64,
    /// The number, or a draft label.
    pub number: String,
    /// The status.
    pub status: String,
}

/// What the page of one enrolment needs.
#[derive(Serialize, ViewData)]
#[view(template = "edtech/enrolment.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    id: i64,
    number: String,
    status: String,
    status_kind: &'static str,
    kind: String,
    student: String,
    guardian: String,
    payer: String,
    subject: String,
    batch_id: i64,
    price: String,
    discount: String,
    fee: String,
    registration_fee: String,
    admin_fee: String,
    instalment_count: i64,
    terms: String,
    discount_state: String,
    seat_hold_until: String,
    paid: String,
    balance: String,
    payment_link: String,
    hold_reason: String,
    on_hold: bool,
    membership: String,
    collecting: bool,
    to_collect: String,
    collect_url: String,
    preview_refund: String,
    preview_owed: String,
    withdrawn_refund: String,
    withdrawn_owed: String,
    withdrawn_reason: String,
    instalments: Vec<InstalmentRow>,
    freezes: Vec<FreezeRow>,
    certificates: Vec<CertificateRow>,
    can_activate: bool,
    can_terms: bool,
    can_discount: bool,
    can_freeze: bool,
    can_unfreeze: bool,
    can_withdraw: bool,
    can_transfer: bool,
    can_settle: bool,
    pending: bool,
    approvals: Vec<ApprovalCard>,
    settle_state: String,
    credit_notes: String,
    refunds_asked: String,
    owed_invoice: String,
    show_preview: bool,
    show_withdrawn: bool,
    has_batch: bool,
}

/// `GET /edtech/enrolments/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    require(&user, permissions::ENROLMENTS_VIEW)?;
    let e = Enrolment::find_or_404(&state.db, id).await?;
    let manage = user.has_permission(permissions::ENROLMENTS_MANAGE);
    let pending = e.status == EnrolmentStatus::PendingPayment;
    let live = matches!(
        e.status,
        EnrolmentStatus::PendingPayment | EnrolmentStatus::Active | EnrolmentStatus::Frozen
    );
    let none = text(&lang, "none");
    let guardian = match e.guardian_party_id {
        Some(party) => party_name(&state, &lang.locale, party).await,
        None => none.clone(),
    };
    let preview = if live {
        Some(enrolments::refund_for(&state, &e, wl_kernel::documents::today()).await?)
    } else {
        None
    };
    let instalments = Instalment::of_enrolment(&state.db, e.id)
        .await?
        .into_iter()
        .map(|row| InstalmentRow {
            number: row.number,
            due: row.due_date.to_string(),
            amount: money_text(row.amount),
            status: text(
                &lang,
                match row.status {
                    InstalmentStatus::Scheduled => "enrolments.instalment.scheduled",
                    InstalmentStatus::Invoiced => "enrolments.instalment.invoiced",
                    InstalmentStatus::Cancelled => "enrolments.instalment.cancelled",
                },
            ),
        })
        .collect();
    let freezes = EnrolmentFreeze::query()
        .where_(EnrolmentFreeze::ENROLMENT_ID.eq(e.id))
        .order_by(EnrolmentFreeze::ID)
        .get(&state.db)
        .await?
        .into_iter()
        .map(|row| FreezeRow {
            cause: text(&lang, &format!("enrolments.cause.{}", row.cause.as_str())),
            since: row.started_on.to_string(),
            until: row.until_on.map_or_else(|| none.clone(), |d| d.to_string()),
            ended: row.ended_on.map_or_else(|| none.clone(), |d| d.to_string()),
        })
        .collect();
    let certificates = Certificate::of_enrolment(&state.db, e.id)
        .await?
        .into_iter()
        .map(|c| CertificateRow {
            id: c.id,
            number: c.label(),
            status: super::certificates::status_text(&lang, c.status),
        })
        .collect();
    // The balance of a first payment that was only part paid.
    let (paid, balance) = if pending {
        let paid = enrolments::paid_of(&state, &e).await?;
        (money_text(paid), money_text((e.fee() + e.registration_fee - paid).max(0)))
    } else {
        (none.clone(), none.clone())
    };
    let subject = e.subject();
    let waiting = !approvals::pending_for(&state.db, &subject).await?.is_empty();
    let approval_cards = approvals::cards_for(&state.db, &subject, &user, &lang).await?;
    let settlement = &e.settlement.0;
    let withdrawn = e.status == EnrolmentStatus::Withdrawn;
    let joined = |ids: &[i64]| {
        if ids.is_empty() {
            none.clone()
        } else {
            ids.iter().map(i64::to_string).collect::<Vec<_>>().join(", ")
        }
    };
    let settle_state = if !withdrawn {
        none.clone()
    } else if settlement.done {
        text(&lang, "enrolments.settled")
    } else if let Some(problem) = &settlement.problem {
        format!("{} {problem}", text(&lang, "enrolments.not_settled"))
    } else {
        text(&lang, "enrolments.settling")
    };
    // Why it is frozen now: the open causes in words ("On hold" for a block).
    let open = EnrolmentFreeze::open_of(&state.db, e.id).await?;
    let hold_reason = if open.is_empty() {
        none.clone()
    } else {
        open.iter()
            .map(|row| text(&lang, &format!("enrolments.cause.{}", row.cause.as_str())))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let on_hold = open.iter().any(|row| row.cause == FreezeCause::Block);
    // A blocked payer or student: only the centre manager withdraws.
    let may_withdraw = !crate::blocked::is_blocked(&state, &e).await?
        || user.has_permission(permissions::ENROLMENTS_APPROVE);
    // The membership's subscription, paused with a freeze and resumed with it.
    let membership = match crate::linked::membership_of(&state, &e).await? {
        Some(found) => text(
            &lang,
            &format!("enrolments.membership_state.{}", found.status.as_str()),
        ),
        None => none.clone(),
    };
    // A payment charged back: the money to collect, with the page that sends
    // the payment link.
    let collecting = crate::chargebacks::is_outstanding(&state.db, e.id).await?;
    let to_collect = if collecting {
        money_text(crate::chargebacks::outstanding(&state, e.id).await?)
    } else {
        none.clone()
    };
    let (preview_refund, preview_owed) = preview
        .as_ref()
        .map_or((String::new(), String::new()), |r| {
            (money_text(r.refund), money_text(r.owed))
        });
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        id: e.id,
        number: e.number(),
        status: status_text(&lang, e.status),
        status_kind: status_kind(e.status),
        kind: kind_text(&lang, e.kind),
        student: party_name(&state, &lang.locale, e.student_party_id).await,
        guardian,
        payer: party_name(&state, &lang.locale, e.payer_party_id).await,
        subject: subject_text(&state, &e).await,
        batch_id: e.batch_id.unwrap_or_default(),
        price: money_text(e.price),
        discount: money_text(e.discount),
        fee: money_text(e.fee()),
        registration_fee: money_text(e.registration_fee),
        admin_fee: money_text(e.admin_fee),
        instalment_count: e.instalments,
        terms: e.terms_accepted_at.map_or_else(
            || text(&lang, "enrolments.terms_missing"),
            |at| dubai_text(at),
        ),
        discount_state: text(
            &lang,
            if e.discount_approved {
                "enrolments.discount_ok"
            } else {
                "enrolments.discount_waiting"
            },
        ),
        seat_hold_until: e
            .seat_hold_until
            .filter(|_| pending)
            .map_or_else(|| none.clone(), |at| dubai_text(at)),
        paid,
        balance,
        payment_link: e
            .payment_link_path
            .clone()
            .filter(|_| pending)
            .unwrap_or_else(|| none.clone()),
        hold_reason,
        on_hold,
        membership,
        collecting,
        to_collect,
        collect_url: crate::chargebacks::collect_path(e.id),
        preview_refund,
        preview_owed,
        withdrawn_refund: money_text(e.refund_amount),
        withdrawn_owed: money_text(e.amount_owed),
        withdrawn_reason: e.withdrawal_reason.clone().unwrap_or_else(|| none.clone()),
        instalments,
        freezes,
        certificates,
        can_activate: pending && manage,
        can_terms: pending && manage && e.terms_accepted_at.is_none(),
        can_discount: pending
            && !e.discount_approved
            && user.has_permission(permissions::ENROLMENTS_APPROVE),
        can_freeze: matches!(e.status, EnrolmentStatus::Active) && manage,
        can_unfreeze: e.status == EnrolmentStatus::Frozen && manage,
        can_withdraw: live && manage && may_withdraw,
        can_settle: withdrawn && !settlement.done && manage,
        pending: waiting,
        approvals: approval_cards,
        settle_state,
        credit_notes: joined(&settlement.credit_notes),
        refunds_asked: joined(&settlement.refunds),
        owed_invoice: settlement
            .owed_invoice
            .map_or_else(|| none.clone(), |id| id.to_string()),
        can_transfer: manage
            && e.kind == EnrolmentKind::Course
            && matches!(e.status, EnrolmentStatus::Active | EnrolmentStatus::Frozen),
        show_preview: live,
        show_withdrawn: e.status == EnrolmentStatus::Withdrawn,
        has_batch: e.batch_id.is_some(),
    }))
}

/// The toast and refresh that answer a transition.
fn done(lang: &Lang, key: &str) -> Result<Response> {
    Ok((Toast::success(text(lang, key)), renox::HxRefresh).into_response())
}

/// `POST /edtech/enrolments/{id}/terms`.
async fn terms(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    enrolments::accept_terms(&state, &user, id).await?;
    done(&lang, "enrolments.terms_done")
}

/// `POST /edtech/enrolments/{id}/discount`.
async fn discount(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    enrolments::approve_discount(&state, &user, id).await?;
    done(&lang, "enrolments.discount_done")
}

/// `POST /edtech/enrolments/{id}/activate`.
async fn activate(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    enrolments::activate(&state, Some(&user), id).await?;
    done(&lang, "enrolments.activated")
}

/// The form of `freeze`.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct FreezeForm {
    /// The last day of the freeze.
    #[serde(default)]
    pub until: String,
}

/// `POST /edtech/enrolments/{id}/freeze`.
async fn freeze(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<FreezeForm>,
) -> Result<Response> {
    let until = parse_date(&form.until)?;
    enrolments::freeze(&state, Some(&user), id, FreezeCause::Student, Some(until)).await?;
    done(&lang, "enrolments.frozen")
}

/// `POST /edtech/enrolments/{id}/unfreeze`.
async fn unfreeze(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    enrolments::unfreeze(&state, Some(&user), id, FreezeCause::Student).await?;
    done(&lang, "enrolments.unfrozen")
}

/// The form of `withdraw`.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct WithdrawForm {
    /// Why the student leaves.
    #[serde(default)]
    pub reason: String,
}

/// `POST /edtech/enrolments/{id}/withdraw`.
async fn withdraw(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<WithdrawForm>,
) -> Result<Response> {
    let request = enrolments::request_withdrawal(&state, Some(&user), id, &form.reason).await?;
    if request.approval_request.is_some() {
        return done(&lang, "enrolments.withdrawal_sent");
    }
    done(&lang, "enrolments.withdrawn")
}

/// `POST /edtech/enrolments/{id}/settle`: repeats the money steps of a
/// withdrawal that stopped.
async fn settle(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    require(&user, permissions::ENROLMENTS_MANAGE)?;
    crate::settlement::settle(&state, id).await?;
    done(&lang, "enrolments.settled_now")
}

/// The form of `transfer`.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct TransferForm {
    /// The target batch's number.
    #[serde(default)]
    pub target: String,
}

/// `POST /edtech/enrolments/{id}/transfer`.
async fn transfer(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<TransferForm>,
) -> Result<Response> {
    let target = parse_number(&form.target)?;
    let moved = crate::transfer::transfer(&state, Some(&user), id, target).await?;
    Ok((
        Toast::success(text(&lang, "transfer.done")),
        renox::HxRedirect(format!("/edtech/enrolments/{}", moved.id)),
    )
        .into_response())
}

/// One reopened invoice of the page that collects.
#[derive(Serialize, Debug, Clone)]
pub struct CollectRow {
    /// The invoice's number.
    pub invoice: String,
    /// What is open on it, `AED 2,520.00`.
    pub balance: String,
    /// The day it was reopened.
    pub reopened: String,
    /// Why the payment was reversed.
    pub reason: String,
}

/// What the page that collects a reversed payment needs.
#[derive(Serialize, ViewData)]
#[view(template = "edtech/collect.html")]
pub(crate) struct CollectPage {
    tx: Texts,
    id: i64,
    number: String,
    student: String,
    payer: String,
    amount: String,
    rows: Vec<CollectRow>,
    can_send: bool,
    withdraw_url: String,
    enrolment_url: String,
}

/// `GET /edtech/enrolments/{id}/collect`: the money to collect after a payment
/// was charged back, with *send payment link* and the way to withdraw.
async fn collect(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    require(&user, permissions::ENROLMENTS_VIEW)?;
    let e = Enrolment::find_or_404(&state.db, id).await?;
    let books = wl_kernel::invoicing::invoicer(&state);
    let none = text(&lang, "none");
    let mut rows = Vec::new();
    for row in crate::chargeback::Chargeback::outstanding_of(&state.db, e.id).await? {
        let summary = books
            .find(&state.db, wl_kernel::invoicing::InvoiceId(row.invoice_id))
            .await?;
        let (invoice, balance) = match summary {
            Some(found) => (
                found.number.unwrap_or_else(|| format!("#{}", row.invoice_id)),
                found.balance.max(0),
            ),
            None => (format!("#{}", row.invoice_id), row.amount),
        };
        rows.push(CollectRow {
            invoice,
            balance: money_text(balance),
            reopened: row.reopened_on.to_string(),
            reason: row
                .reversal_reason
                .clone()
                .unwrap_or_else(|| none.clone()),
        });
    }
    let amount = crate::chargebacks::outstanding(&state, e.id).await?;
    Ok(View::typed(CollectPage {
        tx: texts::all(&lang),
        id: e.id,
        number: e.number(),
        student: party_name(&state, &lang.locale, e.student_party_id).await,
        payer: party_name(&state, &lang.locale, e.payer_party_id).await,
        amount: money_text(amount),
        can_send: amount > 0 && user.has_permission(permissions::ENROLMENTS_MANAGE),
        rows,
        withdraw_url: crate::chargebacks::withdraw_path(e.id),
        enrolment_url: crate::chargebacks::withdraw_path(e.id),
    }))
}

/// `POST /edtech/enrolments/{id}/payment-link`: makes the payment link for the
/// money to collect and sends it to the payer.
async fn payment_link(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    crate::chargebacks::send_link(&state, &user, id).await?;
    done(&lang, "chargeback.link_sent")
}
