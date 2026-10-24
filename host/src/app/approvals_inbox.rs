//! The approvals inbox (`GET /approvals`, BL-MPW-29, `docs/UX.md` §2.1): the
//! decisions waiting for the signed-in person, each in words, and the
//! requests the person made themselves.
//!
//! - **Waiting for you** is [`approvals::awaiting_items`]' source: the pending
//!   requests whose current step the person may decide. A row names the action
//!   in words ([`approvals::awaiting::action_label`]; an action nobody gave
//!   words shows *Approval needed*), the record by its name and not its id,
//!   who asked, on which day and why, links to the record, and offers
//!   *Approve*, *Reject* and *Return for changes*: the sheets post to the
//!   kernel's decide routes (`wl_kernel.approvals.approve|reject|return`), which
//!   keep the rules: the requester never decides (they are not in this list),
//!   *Reject* and *Return* need a note.
//! - **My requests** are the person's own pending requests with who they wait
//!   for (*Waiting for the MD*, or *Waiting for another user* when the step is
//!   a permission), and the ones returned to them with the approver's note.
//! - The reason is read from the request's payload (`reason`, `note`, `text`,
//!   `comment`), the only place the kernel keeps it; a request without one
//!   shows no reason line.
//!
//! - **Pay accounts** waiting for a second user (`pending_approval`, wl-hr
//!   `HRM.ACL.05`, BL-MPW-34) are a state machine transition, not a kernel
//!   approval request, so they are added here: a user holding
//!   `pay_accounts.approve` who is not the requester sees them in *Waiting for
//!   you* with *Approve* and *Reject* (they post to the pay account's own
//!   transition routes); the requester sees them in *My requests* as *Waiting
//!   for another approver*. Only the last 4 digits are ever shown. The
//!   dashboard counts them ([`pay_account_items`]).
//!
//! Decision: the subject's name comes from the registered subject label, else
//! from [`subject_text`] for the Manpower and HR records, else nothing: the
//! page never shows `hr.employee #2`. A request whose subject moved on shows
//! no *Dismiss* (not built): the kernel ends such a request itself.

use renox::nav::NavLink;
use renox::prelude::*;
use renox::serde_json::Value;
use serde::Serialize;
use wl_kernel::approvals::{
    self, ApprovalCard, ApprovalRequest, ApprovalStatus, awaiting::action_label,
};
use wl_kernel::{BusinessCalendar, Subject};

/// The inbox module.
pub struct ApprovalsInbox;

impl Module for ApprovalsInbox {
    fn name(&self) -> &'static str {
        "approvals_inbox"
    }

    fn register(&self, app: &mut Registry) {
        approvals::register_subject_link("hr.employee", |id| format!("/hr/employees/{id}"));
        app.nav(
            "app",
            NavLink::route("approvals.inbox", "nav.approvals")
                .icon("circle-check")
                .order(1)
                .when(|user| !wl_kernel::portal::is_portal_only(user)),
        );
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .get("/approvals", index)
            .name("approvals.inbox")
            .require_auth()
            .require_verified()
    }
}

/// One approval, as the page shows it.
#[derive(Serialize, Debug, Clone)]
struct Row {
    id: i64,
    /// What is asked, in words.
    action: String,
    /// The record, by its name; empty when it has none.
    subject: String,
    /// The record's page, when its module has one.
    url: String,
    requester: String,
    /// `Requested by Amal Youssef on 01 Nov 2026`.
    requested: String,
    reason: String,
    status: String,
    status_kind: &'static str,
    /// Why the person cannot decide: who it waits for, or the returned note.
    state: String,
    can_decide: bool,
    can_withdraw: bool,
    labels: std::collections::BTreeMap<&'static str, String>,
    /// More lines of the row (a pay account: the number's end, the holder, why
    /// the name differs).
    detail: Vec<String>,
    /// A pay account waiting for a second user, decided on its own routes.
    pay_account: bool,
    /// Where *Approve* posts (pay accounts only).
    approve_url: String,
    /// Where *Reject* posts (pay accounts only).
    reject_url: String,
}

/// The first non-empty text of the payload under a name a reason goes by.
fn reason_of(payload: &Value) -> String {
    ["reason", "note", "text", "comment", "waiver_reason"]
        .iter()
        .filter_map(|key| payload.get(key).and_then(Value::as_str))
        .map(str::trim)
        .find(|text| !text.is_empty())
        .unwrap_or_default()
        .to_owned()
}

/// A role key as the words of whoever holds it.
fn role_words(role: &str) -> String {
    match role {
        "md" => "the MD".to_owned(),
        "hr" => "HR".to_owned(),
        "admin" | "company_admin" => "the admin".to_owned(),
        "account_manager" => "the account manager".to_owned(),
        other => format!("the {}", other.replace('_', " ")),
    }
}

/// Who a request waits for, in words.
fn waiting_for(lang: &Lang, request: &ApprovalRequest) -> String {
    match request.step().and_then(|step| step.roles.first()) {
        Some(role) => lang.t("approvals.waiting_for", &[("who", &role_words(role))]),
        None => lang.t("approvals.waiting_other", &[]),
    }
}

/// The name of the record a request is about, or empty.
async fn subject_text(state: &AppState, lang: &Lang, subject: &Subject) -> String {
    if let Ok(Some(label)) = wl_kernel::documents::subject_label(&state.db, subject).await {
        return label;
    }
    if let Some(name) = manpower::approvals_text::subject_name(state, subject, &lang.locale).await {
        return name;
    }
    if let Some(name) = hr_subject(state, lang, subject).await {
        return name;
    }
    invoicing_subject(state, lang, subject)
        .await
        .unwrap_or_default()
}

/// The name of an invoice or a rate card version.
async fn invoicing_subject(state: &AppState, lang: &Lang, subject: &Subject) -> Option<String> {
    match subject.subject_type.as_str() {
        "invoicing.rate_card_version" => {
            let version =
                wl_invoicing::rate_card::RateCardVersion::find(&state.db, subject.subject_id)
                    .await
                    .ok()??;
            let card = wl_invoicing::rate_card::RateCard::find(&state.db, version.rate_card_id)
                .await
                .ok()??;
            let client = wl_kernel::parties::directory(state)
                .display_name(&state.db, wl_kernel::PartyId(card.party_id), &lang.locale)
                .await
                .ok()??;
            Some(lang.t(
                "approvals.rate_card_of",
                &[
                    ("client", &client),
                    ("date", &version.effective_from.format("%d %b %Y")),
                ],
            ))
        }
        "invoicing.invoice" => {
            wl_invoicing::Invoice::find(&state.db, subject.subject_id)
                .await
                .ok()??
                .number
        }
        _ => None,
    }
}

/// The employee's name, for the HR records.
async fn hr_subject(state: &AppState, lang: &Lang, subject: &Subject) -> Option<String> {
    let employee_id = match subject.subject_type.as_str() {
        "hr.employee" => subject.subject_id,
        "hr.pay_account" => {
            wl_hr::PayAccount::find(&state.db, subject.subject_id)
                .await
                .ok()??
                .employee_id
        }
        "hr.settlement" => {
            wl_hr::FinalSettlement::find(&state.db, subject.subject_id)
                .await
                .ok()??
                .employee_id
        }
        _ => return None,
    };
    let employee = wl_hr::Employee::find(&state.db, employee_id).await.ok()??;
    wl_kernel::parties::directory(state)
        .display_name(
            &state.db,
            wl_kernel::PartyId(employee.party_id),
            &lang.locale,
        )
        .await
        .ok()?
}

async fn row(
    state: &AppState,
    lang: &Lang,
    user: &AuthUser,
    request: &ApprovalRequest,
) -> Result<Row> {
    let card: ApprovalCard = approvals::card(&state.db, request, user, lang).await?;
    let day = request.created_at.map_or_else(String::new, |at| {
        at.with_timezone(&BusinessCalendar::offset())
            .format("%d %b %Y")
            .to_string()
    });
    let subject = request.subject();
    // Built before the awaits below: a `&dyn Display` list is not `Send`.
    let requested = lang.t(
        "approvals.requested",
        &[("name", &card.requester), ("date", &day)],
    );
    let reason = match reason_of(&request.payload.0) {
        reason if !reason.is_empty() => reason,
        _ => manpower::approvals_text::request_reason(state, request, &lang.locale)
            .await
            .unwrap_or_default(),
    };
    Ok(Row {
        id: request.id,
        action: action_label(lang, &request.action),
        subject: subject_text(state, lang, &subject).await,
        url: approvals::subject_url(&request.subject_type, request.subject_id).unwrap_or_default(),
        requester: card.requester.clone(),
        requested,
        reason,
        status: card.status_label.clone(),
        status_kind: card.status_kind,
        state: String::new(),
        can_decide: card.can_decide,
        can_withdraw: card.can_withdraw,
        labels: card.labels,
        detail: Vec::new(),
        pay_account: false,
        approve_url: String::new(),
        reject_url: String::new(),
    })
}

/// The pay accounts waiting for approval (`pending_approval`), oldest first.
async fn pending_pay_accounts(state: &AppState) -> Result<Vec<wl_hr::PayAccount>> {
    wl_hr::PayAccount::query()
        .where_(wl_hr::PayAccount::STATUS.eq(wl_hr::PayAccountStatus::PendingApproval))
        .order_by(wl_hr::PayAccount::ID)
        .get(&state.db)
        .await
}

/// A pending pay account as a row; `can_decide` when `user` may approve it.
async fn pay_account_row(
    state: &AppState,
    lang: &Lang,
    account: &wl_hr::PayAccount,
    can_decide: bool,
) -> Result<Row> {
    let name = hr_subject(state, lang, &account.subject())
        .await
        .unwrap_or_default();
    let requester = match account.requested_by {
        Some(id) => User::find(&state.db, id)
            .await?
            .map(|user| user.name)
            .unwrap_or_default(),
        None => String::new(),
    };
    let day = account.created_at.map_or_else(String::new, |at| {
        at.with_timezone(&BusinessCalendar::offset())
            .format("%d %b %Y")
            .to_string()
    });
    let requested = lang.t(
        "approvals.requested",
        &[("name", &requester), ("date", &day)],
    );
    let mut detail = vec![lang.t("approvals.pay_account_ending", &[("last4", &account.last4)])];
    if !account.holder_name.trim().is_empty() {
        detail.push(lang.t(
            "approvals.pay_account_holder",
            &[("name", &account.holder_name)],
        ));
    }
    let explanation = account.mismatch_explanation.clone().unwrap_or_default();
    if !explanation.trim().is_empty() {
        detail.push(lang.t("approvals.pay_account_mismatch", &[("text", &explanation)]));
    }
    let mut labels = std::collections::BTreeMap::new();
    for (key, text) in [
        ("approve", "approvals.pay_approve"),
        ("reject", "approvals.pay_reject"),
        ("approve_title", "approvals.pay_approve_title"),
        ("reject_title", "approvals.pay_reject_title"),
        ("comment", "approvals.pay_reason"),
        ("comment_reject", "approvals.pay_reason_hint"),
        ("cancel", "approvals.pay_cancel"),
    ] {
        labels.insert(key, lang.t(text, &[]));
    }
    labels.insert(
        "approve_message",
        lang.t("approvals.pay_approve_message", &[("name", &name)]),
    );
    Ok(Row {
        id: account.id,
        action: lang.t("approvals.pay_account", &[]),
        subject: name,
        url: format!("/hr/pay-accounts/{}", account.id),
        requester,
        requested,
        reason: String::new(),
        status: String::new(),
        status_kind: "warning",
        state: if can_decide {
            String::new()
        } else {
            lang.t("approvals.waiting_approver", &[])
        },
        can_decide,
        can_withdraw: false,
        labels,
        detail,
        pay_account: true,
        approve_url: format!("/hr/pay-accounts/{}/transitions/approve", account.id),
        reject_url: format!("/hr/pay-accounts/{}/transitions/reject", account.id),
    })
}

/// A line of the dashboard's *awaiting you*: the title and the page.
pub struct PayAccountItem {
    /// `Approve a new pay account: Ravi Kumar`.
    pub title: String,
    /// The inbox, where it is decided.
    pub url: String,
}

/// The pay accounts `user` may decide now: they hold `pay_accounts.approve`
/// and did not ask for the account.
pub async fn pay_account_items(
    state: &AppState,
    lang: &Lang,
    user: &AuthUser,
) -> Result<Vec<PayAccountItem>> {
    if !user.has_permission(wl_hr::permissions::PAY_ACCOUNTS_APPROVE) {
        return Ok(Vec::new());
    }
    let mut items = Vec::new();
    for account in pending_pay_accounts(state).await? {
        if account.requested_by == Some(user.id) {
            continue;
        }
        let name = hr_subject(state, lang, &account.subject())
            .await
            .unwrap_or_default();
        items.push(PayAccountItem {
            title: lang.t("approvals.pay_account_item", &[("name", &name)]),
            url: "/approvals".to_owned(),
        });
    }
    Ok(items)
}

/// What the inbox shows.
async fn build(state: &AppState, lang: &Lang, user: &AuthUser) -> Result<(Vec<Row>, Vec<Row>)> {
    let mut waiting = Vec::new();
    for request in approvals::awaiting::awaiting(&state.db, user).await? {
        waiting.push(row(state, lang, user, &request).await?);
    }
    let may_approve = user.has_permission(wl_hr::permissions::PAY_ACCOUNTS_APPROVE);
    let mut mine = Vec::new();
    for account in pending_pay_accounts(state).await? {
        if account.requested_by == Some(user.id) {
            mine.push(pay_account_row(state, lang, &account, false).await?);
        } else if may_approve {
            waiting.push(pay_account_row(state, lang, &account, true).await?);
        }
    }
    // Every pending request of the person: the page pages them (no cap).
    for request in ApprovalRequest::query()
        .where_(ApprovalRequest::REQUESTER_ID.eq(user.id))
        .where_(ApprovalRequest::STATUS.eq(ApprovalStatus::Pending))
        .order_by_desc(ApprovalRequest::ID)
        .get(&state.db)
        .await?
    {
        let mut line = row(state, lang, user, &request).await?;
        line.state = waiting_for(lang, &request);
        mine.push(line);
    }
    for item in approvals::needs_action_items(&state.db, user, lang).await? {
        let Some(request) = ApprovalRequest::find(&state.db, item.request_id).await? else {
            continue;
        };
        let mut line = row(state, lang, user, &request).await?;
        line.state = match &item.comment {
            Some(note) => lang.t("approvals.returned_with", &[("note", note)]),
            None => lang.t("approvals.returned", &[]),
        };
        mine.push(line);
    }
    Ok((waiting, mine))
}

/// Rows per page of each tab.
pub const PER_PAGE: usize = 25;

/// The inbox's query: a search over the action, the record, who asked, the
/// reason and the detail lines (`?q=`), the open tab (`?tab=mine`) and the
/// page of each tab (`?page=`, `?mine_page=`).
#[derive(serde::Deserialize, Debug, Default)]
pub struct InboxQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    tab: String,
    #[serde(default)]
    page: Option<usize>,
    #[serde(default)]
    mine_page: Option<usize>,
}

/// Whether `row` holds `needle` (lower case) in its words.
fn matches(row: &Row, needle: &str) -> bool {
    needle.is_empty()
        || [
            row.action.as_str(),
            row.subject.as_str(),
            row.requester.as_str(),
            row.reason.as_str(),
            row.state.as_str(),
            row.status.as_str(),
        ]
        .into_iter()
        .chain(row.detail.iter().map(String::as_str))
        .any(|text| text.to_lowercase().contains(needle))
}

/// `text` for a query string.
fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// One tab's page of rows and its links.
#[derive(Serialize, Debug, Clone, Default)]
struct Pager {
    /// Rows the search matches, on every page.
    total: usize,
    /// More than one page.
    paged: bool,
    /// `1–25 of 40`.
    range: String,
    /// The previous page's address, if any.
    prev: Option<String>,
    /// The next page's address, if any.
    next: Option<String>,
}

/// Page `page` (from 1) of `rows`, with links that keep the search, the
/// other tab's page and the open tab (`tab`), the page itself under `param`;
/// `range_text` words `1–25 of 40` from the first, last and total.
fn page_of(
    range_text: &dyn Fn(usize, usize, usize) -> String,
    rows: Vec<Row>,
    page: usize,
    param: &str,
    tab: &str,
    keep: &[(&str, String)],
) -> (Vec<Row>, Pager) {
    let total = rows.len();
    let pages = total.div_ceil(PER_PAGE).max(1);
    let page = page.clamp(1, pages);
    let start = (page - 1) * PER_PAGE;
    let shown: Vec<Row> = rows.into_iter().skip(start).take(PER_PAGE).collect();
    let link = |to: usize| {
        let mut parts: Vec<String> = keep
            .iter()
            .filter(|(_, value)| !value.is_empty())
            .map(|(name, value)| format!("{name}={}", encode(value)))
            .collect();
        parts.push(format!("{param}={to}"));
        parts.push(format!("tab={tab}"));
        format!("/approvals?{}", parts.join("&"))
    };
    let range = if total == 0 {
        String::new()
    } else {
        range_text(start + 1, start + shown.len(), total)
    };
    let pager = Pager {
        total,
        paged: pages > 1,
        range,
        prev: (page > 1).then(|| link(page - 1)),
        next: (page < pages).then(|| link(page + 1)),
    };
    (shown, pager)
}

/// `GET /approvals`: the page for a browser, the rows as JSON otherwise. The
/// search (`?q=`) narrows both tabs; the page shows each tab 25 rows at a
/// time.
///
/// Decision: the rows are built in code from two sources (the kernel's
/// approval requests the person may decide, worked out per step, and the pay
/// accounts waiting for a second user), each in words and with its own
/// decide buttons, so they stay cards (docs/UX.md 2.1: on a phone the buttons
/// have the width of the screen) rather than a `renox::grid` over one table;
/// the search and the pages are done here.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: renox::axum::http::HeaderMap,
    renox::axum::extract::Query(query): renox::axum::extract::Query<InboxQuery>,
) -> Result<Response> {
    let (waiting, mine) = build(&state, &lang, &user).await?;
    let q = query.q.trim().to_owned();
    let needle = q.to_lowercase();
    let waiting: Vec<Row> = waiting.into_iter().filter(|row| matches(row, &needle)).collect();
    let mine: Vec<Row> = mine.into_iter().filter(|row| matches(row, &needle)).collect();
    if !wl_kernel::negotiate::wants_html(&headers) {
        return Ok(
            Json(renox::serde_json::json!({ "waiting": waiting, "mine": mine })).into_response(),
        );
    }
    let range_text = |from: usize, to: usize, total: usize| {
        lang.t(
            "approvals.range",
            &[("from", &from), ("to", &to), ("total", &total)],
        )
    };
    let waiting_page = query.page.unwrap_or(1);
    let mine_page = query.mine_page.unwrap_or(1);
    let (waiting, waiting_pager) = page_of(
        &range_text,
        waiting,
        waiting_page,
        "page",
        "waiting",
        &[("q", q.clone()), ("mine_page", mine_page.to_string())],
    );
    let (mine, mine_pager) = page_of(
        &range_text,
        mine,
        mine_page,
        "mine_page",
        "mine",
        &[("q", q.clone()), ("page", waiting_page.to_string())],
    );
    // `open_tab`, not `tab`: a view value named like a kit tag is shadowed.
    let open_tab = if query.tab == "mine" { "mine" } else { "waiting" };
    Ok(view(
        "approvals/index.html",
        context! { waiting, mine, waiting_pager, mine_pager, q, open_tab },
    )
    .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, subject: &str) -> Row {
        Row {
            id,
            action: "Release a client contract from a credit hold".to_owned(),
            subject: subject.to_owned(),
            url: String::new(),
            requester: "Amal Youssef".to_owned(),
            requested: String::new(),
            reason: String::new(),
            status: String::new(),
            status_kind: "warning",
            state: String::new(),
            can_decide: true,
            can_withdraw: false,
            labels: std::collections::BTreeMap::new(),
            detail: Vec::new(),
            pay_account: false,
            approve_url: String::new(),
            reject_url: String::new(),
        }
    }

    fn words(from: usize, to: usize, total: usize) -> String {
        format!("{from}-{to} of {total}")
    }

    /// #530: a long inbox pages 25 cards at a time and keeps the search and
    /// the other tab's page in its links.
    #[test]
    fn a_long_inbox_pages_and_keeps_the_search() {
        let rows: Vec<Row> = (1..=60).map(|id| row(id, &format!("CC-{id:03}"))).collect();
        let keep = [("q", "al quoz".to_owned()), ("mine_page", "2".to_owned())];
        let (first, pager) = page_of(&words, rows.clone(), 1, "page", "waiting", &keep);
        assert_eq!(first.len(), PER_PAGE);
        assert_eq!(first[0].id, 1);
        assert_eq!(pager.total, 60);
        assert!(pager.paged);
        assert_eq!(pager.range, "1-25 of 60");
        assert_eq!(pager.prev, None);
        assert_eq!(
            pager.next.as_deref(),
            Some("/approvals?q=al%20quoz&mine_page=2&page=2&tab=waiting")
        );
        let (last, pager) = page_of(&words, rows.clone(), 3, "page", "waiting", &keep);
        assert_eq!(last.iter().map(|r| r.id).collect::<Vec<_>>(), (51..=60).collect::<Vec<_>>());
        assert_eq!(pager.range, "51-60 of 60");
        assert_eq!(pager.next, None);
        // A page past the end shows the last one.
        let (past, _) = page_of(&words, rows, 9, "page", "waiting", &keep);
        assert_eq!(past[0].id, 51);
        let (none, pager) = page_of(&words, Vec::new(), 1, "page", "waiting", &[]);
        assert!(none.is_empty() && !pager.paged && pager.range.is_empty());
    }

    /// The search reads the action, the record and who asked, in any case.
    #[test]
    fn the_search_reads_the_words_of_a_row() {
        let release = row(1, "CC-DEMO-003, Al Quoz Logistics Park LLC");
        assert!(matches(&release, "al quoz"));
        assert!(matches(&release, "amal"));
        assert!(matches(&release, "credit hold"));
        assert!(matches(&release, ""));
        assert!(!matches(&release, "zzqqxx"));
    }
}
