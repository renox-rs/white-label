//! The 01 Property tenant portal (01 §3.1, §3.3; wl-portal §4, §7;
//! BL-POR-02, BL-POR-03):
//!
//! - the audience `tenant` and its portal roles, seeded ([`TENANT`]):
//!   `primary_contact` (lease, finance, documents, requests, users) and
//!   `finance_contact` (sees and pays invoices only); the company admin
//!   changes their permissions in the portal settings;
//! - `POR.ACL.01`: a terminated lease (`LeaseTerminated`, which also follows
//!   a `force_terminate`, and `LeaseForceTerminated` itself) runs `end` on
//!   the tenant's grants, but only when the tenant has no lease left in
//!   `active`, `renewal_due`, `expired` (holding over) or `notice_given`;
//!   wl-portal's daily timer closes the grant 90 days later.
//!
//! - the tenant sections of 01 (BL-PRP-30, `POR.SLF.01`, `PRP.ACL.01`):
//!   *Lease* ([`LeaseSection`]), *Cheques* ([`ChequesSection`]) and
//!   *Documents* ([`DocumentsSection`]), registered with wl-portal at start-up
//!   for the audience `tenant`. Each lists only the records of the party the
//!   user acts for ([`property::portal`]); wl-portal refuses a section the
//!   user's grants do not allow, even by URL, so the finance contact (who has
//!   neither `portal.leases.view` nor `portal.documents.view`) gets no *Lease*
//!   or *Cheques* entry. A document of a lease of the party downloads at
//!   `GET /portal/{party}/lease-documents/{document}` ([`download`]); a
//!   `read_only` grant keeps both after the lease ended.
//!
//! - the owner portal of 01 (BL-PRP-29, `POR.SLF.02`, `PRP.ACL.02`): the
//!   audience `owner` ([`OWNER`], role `owner`), its *Statements* section
//!   ([`StatementsSection`]: the published and settled statements of the
//!   owner's agreements) and the dispute of a statement line from the portal
//!   (`POST /portal/{party}/owner-statements/{statement}/dispute`,
//!   [`dispute`]), which sets the dispute flag and opens the ticket as staff
//!   do (BL-PRP-21). When the management agreement ends (`AgreementEnded`)
//!   the owner's grants turn `read_only`: the statements still read for 90
//!   days and a dispute is refused; wl-portal's daily timer closes the grant
//!   on day 91 (an agreement ended on 2028-06-30 reads until 2028-09-28).
//!
//! - *Requests* (BL-TKT-14, [`RequestsSection`]): the tenant's maintenance
//!   requests (raised by its portal users or on its units) with their stored
//!   status; *New request* (`GET/POST /portal/{party}/requests[/new]`) raises
//!   one with photos, also while the party is blocked, never with a
//!   `read_only` grant; a request's page shows only the comments marked
//!   visible and, once resolved, the charge with Confirm and Reopen.
//!
//! The other tenant sections (invoices, deposit) come with their modules'
//! issues (BL-INV-31, BL-PAY-27); [`SECTIONS`] is the order and permission
//! each one registers with.

use property::lease::LEASE_SUBJECT_TYPE;
use property::portal::{PortalRow, TENANT_DOCUMENT_KINDS};
use property::{AgreementEnded, Lease, LeaseForceTerminated, LeaseStatus, LeaseTerminated};
use renox::Download;
use renox::prelude::*;
use wl_documents::{Document, DocumentKind, DocumentStatus};
use wl_kernel::PartyId;
use wl_kernel::documents::today;
use wl_portal::sections::{PortalMenu, PortalSection, SectionFuture, SectionRow};
use wl_portal::{AudienceDef, PortalAccess, PortalAction, PortalGuard, Refused, RoleDef};

/// The audience key of the tenants.
pub const TENANT_AUDIENCE: &str = "tenant";

/// The tenant audience of 01 (§3.1), closing 90 days after `end`
/// (`POR.ACL.01`).
pub const TENANT: AudienceDef = AudienceDef {
    key: TENANT_AUDIENCE,
    label: "Tenants",
    close_after_days: 90,
    roles: &[
        RoleDef {
            key: "primary_contact",
            label: "Primary contact",
            permissions: &[
                "portal.leases.view",
                "portal.invoices.view",
                "portal.invoices.pay",
                "portal.documents.view",
                "portal.requests.create",
                "portal.users.manage",
            ],
        },
        RoleDef {
            key: "finance_contact",
            label: "Finance contact",
            permissions: &["portal.invoices.view", "portal.invoices.pay"],
        },
    ],
};

/// The audience key of the owners.
pub const OWNER_AUDIENCE: &str = "owner";

/// The owner audience of 01 (`POR.SLF.02`), read-only for 90 days after the
/// management agreement ends.
pub const OWNER: AudienceDef = AudienceDef {
    key: OWNER_AUDIENCE,
    label: "Owners",
    close_after_days: 90,
    roles: &[RoleDef {
        key: "owner",
        label: "Owner",
        permissions: &[
            "portal.statements.view",
            "portal.statements.dispute",
            "portal.documents.view",
        ],
    }],
};

/// The tenant sections: key, menu order and the portal permission each
/// needs, for the modules that register them.
pub const SECTIONS: &[(&str, i32, &str)] = &[
    ("lease", 10, "portal.leases.view"),
    ("invoices", 20, "portal.invoices.view"),
    ("cheques", 30, "portal.leases.view"),
    ("deposit", 40, "portal.leases.view"),
    ("documents", 50, "portal.documents.view"),
    ("requests", 60, "portal.requests.create"),
];

/// Seeds the tenant audience where the database lacks it; a role the
/// company admin changed keeps its permissions.
///
/// # Errors
///
/// The database's errors.
pub async fn install(state: &AppState) -> Result<usize> {
    let tenants = wl_portal::audiences::seed(&state.db, &TENANT).await?;
    let owners = wl_portal::audiences::seed(&state.db, &OWNER).await?;
    Ok(tenants + owners)
}

/// The management agreement of `owner` ended on `on`: its owner grants turn
/// `read_only` (BL-PRP-29). Returns how many did.
///
/// # Errors
///
/// The database's errors.
pub async fn agreement_ended(
    state: &AppState,
    owner: PartyId,
    on: renox::chrono::NaiveDate,
) -> Result<usize> {
    let running = property::agreement::ManagementAgreement::query()
        .where_(property::agreement::ManagementAgreement::OWNER_PARTY_ID.eq(owner.0))
        .where_(
            property::agreement::ManagementAgreement::STATUS
                .is_in(property::agreement::AgreementStatus::REPORTING),
        )
        .first(&state.db)
        .await?;
    if running.is_some() {
        return Ok(0);
    }
    let ended = PortalAccess::contract_ended(
        state,
        owner,
        OWNER_AUDIENCE,
        on,
        "management agreement ended",
    )
    .await?;
    Ok(ended.len())
}

/// A lease of `tenant` ended: `end` on the tenant's grants unless another
/// lease of theirs still runs. Returns how many grants turned read-only.
///
/// # Errors
///
/// The database's errors.
pub async fn lease_ended(state: &AppState, tenant: PartyId, reason: &str) -> Result<usize> {
    let running = Lease::query()
        .where_(Lease::TENANT_PARTY_ID.eq(tenant.0))
        .where_(Lease::STATUS.is_in(LeaseStatus::RUNNING))
        .first(&state.db)
        .await?;
    if running.is_some() {
        return Ok(0);
    }
    let ended =
        PortalAccess::contract_ended(state, tenant, TENANT_AUDIENCE, today(), reason).await?;
    Ok(ended.len())
}

/// The order and permission of the tenant section `key`, from [`SECTIONS`].
fn placed(key: &str) -> (i32, &'static str) {
    SECTIONS.iter().find(|(k, _, _)| *k == key).map_or(
        (wl_portal::sections::DEFAULT_ORDER, ""),
        |(_, order, permission)| (*order, *permission),
    )
}

/// The portal rows of `rows`.
fn section_rows(rows: Vec<PortalRow>) -> Vec<SectionRow> {
    rows.iter()
        .map(|row| SectionRow::new(&row.label, &row.status))
        .collect()
}

/// *Lease*: the tenant's lease versions with their term, rent, payment plan,
/// status and Ejari number.
pub struct LeaseSection;

impl PortalSection for LeaseSection {
    fn title(&self) -> String {
        "Lease".to_owned()
    }

    fn rows<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
    ) -> SectionFuture<'a, Vec<SectionRow>> {
        Box::pin(async move {
            Ok(section_rows(
                property::portal::tenant_leases(&state.db, party).await?,
            ))
        })
    }

    fn audience(&self) -> Option<&str> {
        Some(TENANT_AUDIENCE)
    }

    fn permission(&self) -> Option<&str> {
        Some(placed("lease").1)
    }

    fn order(&self) -> i32 {
        placed("lease").0
    }
}

/// *Cheques*: the cheques of the tenant's leases with their stored status.
pub struct ChequesSection;

impl PortalSection for ChequesSection {
    fn title(&self) -> String {
        "Cheques".to_owned()
    }

    fn rows<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
    ) -> SectionFuture<'a, Vec<SectionRow>> {
        Box::pin(async move {
            Ok(section_rows(
                property::portal::tenant_cheques(state, party).await?,
            ))
        })
    }

    fn audience(&self) -> Option<&str> {
        Some(TENANT_AUDIENCE)
    }

    fn permission(&self) -> Option<&str> {
        Some(placed("cheques").1)
    }

    fn order(&self) -> i32 {
        placed("cheques").0
    }
}

/// The documents of the tenant `party`'s leases it may see, with their kind
/// and lease, oldest first: in force or waiting for review (not superseded,
/// archived or rejected).
///
/// # Errors
///
/// The database's errors.
pub async fn tenant_documents(
    db: &Db,
    party: PartyId,
) -> Result<Vec<(Document, DocumentKind, Lease)>> {
    let mut kinds = Vec::new();
    for key in TENANT_DOCUMENT_KINDS {
        if let Some(kind) = DocumentKind::by_key(db, key).await? {
            kinds.push(kind);
        }
    }
    let mut found = Vec::new();
    for lease in property::portal::leases_of(db, party).await? {
        let documents = Document::query()
            .where_(Document::SUBJECT_TYPE.eq(LEASE_SUBJECT_TYPE))
            .where_(Document::SUBJECT_ID.eq(lease.id))
            .order_by(Document::ID)
            .get(db)
            .await?;
        for document in documents {
            if matches!(
                document.status,
                DocumentStatus::Superseded | DocumentStatus::Archived | DocumentStatus::Rejected
            ) {
                continue;
            }
            let Some(kind) = kinds.iter().find(|k| k.id == document.kind_id) else {
                continue;
            };
            found.push((document, kind.clone(), lease.clone()));
        }
    }
    Ok(found)
}

/// *Documents*: the documents of the tenant's leases it may see (the signed
/// and generated contract, the Ejari certificate, the acceptance of changed
/// terms), each downloadable.
pub struct DocumentsSection;

impl PortalSection for DocumentsSection {
    fn title(&self) -> String {
        "Documents".to_owned()
    }

    fn rows<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
    ) -> SectionFuture<'a, Vec<SectionRow>> {
        Box::pin(async move {
            Ok(tenant_documents(&state.db, party)
                .await?
                .iter()
                .map(|(document, kind, lease)| {
                    let label = format!("{} · {}", kind.name_en, lease.number_with_version());
                    let status = match document.status {
                        DocumentStatus::Accepted => "Accepted",
                        _ => "In review",
                    };
                    SectionRow::new(&label, status)
                })
                .collect())
        })
    }

    fn audience(&self) -> Option<&str> {
        Some(TENANT_AUDIENCE)
    }

    fn permission(&self) -> Option<&str> {
        Some(placed("documents").1)
    }

    fn order(&self) -> i32 {
        placed("documents").0
    }
}

/// `GET /portal/{party}/lease-documents/{document}`: the file of a document of
/// a lease of the party, for a user whose grants for the party (`active` or
/// `read_only`) allow *Documents*; 403 without them, 404 for a document that
/// is not one of the party's leases or not of a kind the tenant may see.
async fn download(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((party, id)): Path<(i64, i64)>,
) -> Result<Download> {
    let party = PartyId(party);
    let menu = PortalMenu::of(&state, user.id, party, today(), &lang)
        .await?
        .ok_or(Error::Forbidden)?;
    if !menu.allows("documents") {
        return Err(Error::Forbidden);
    }
    let (document, _, _) = tenant_documents(&state.db, party)
        .await?
        .into_iter()
        .find(|(document, _, _)| document.id == id)
        .ok_or(Error::NotFound)?;
    let version = document
        .current_file(&state.db)
        .await?
        .ok_or(Error::NotFound)?;
    let bytes = version.read(&state).await?.ok_or(Error::NotFound)?;
    Ok(Download::bytes(
        version.file_name,
        version.mime_type,
        bytes.to_vec(),
    ))
}

/// *Statements*: the owner's published and settled statements.
pub struct StatementsSection;

impl PortalSection for StatementsSection {
    fn title(&self) -> String {
        "Statements".to_owned()
    }

    fn rows<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
    ) -> SectionFuture<'a, Vec<SectionRow>> {
        Box::pin(async move {
            Ok(section_rows(
                property::portal::owner_statements(&state.db, party).await?,
            ))
        })
    }

    fn audience(&self) -> Option<&str> {
        Some(OWNER_AUDIENCE)
    }

    fn permission(&self) -> Option<&str> {
        Some("portal.statements.view")
    }

    fn order(&self) -> i32 {
        10
    }
}

/// The owner's dispute of a statement line.
#[derive(serde::Deserialize, serde::Serialize, Validate, Debug)]
pub struct DisputeForm {
    /// The line disputed; empty for the whole statement.
    pub line: Option<i64>,
    /// Why, in the owner's words.
    pub comment: Option<String>,
}

/// `POST /portal/{party}/owner-statements/{statement}/dispute`: the owner
/// disputes a line of one of its published statements. 403 without an
/// `active` owner grant for the party (a `read_only` one after the agreement
/// ended is refused with the read-only notice) or without the dispute
/// permission; 404 for a statement of another owner; 422 as staff's dispute.
async fn dispute(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((party, statement)): Path<(i64, i64)>,
    Valid(form): Valid<DisputeForm>,
) -> Result<Response> {
    let party = PartyId(party);
    let menu = PortalMenu::of(&state, user.id, party, today(), &lang)
        .await?
        .ok_or(Error::Forbidden)?;
    if !menu.audiences.contains(OWNER_AUDIENCE)
        || !menu.permissions.contains("portal.statements.dispute")
    {
        return Err(Error::Forbidden);
    }
    match PortalGuard::check(&state, user.id, party, PortalAction::Dispute).await? {
        Ok(()) => {}
        Err(Refused::GrantReadOnly(since)) => {
            return Err(Error::Status(
                StatusCode::FORBIDDEN,
                wl_portal::read_only_notice(&lang, since),
            ));
        }
        Err(_) => return Err(Error::Forbidden),
    }
    let comment = form.comment.unwrap_or_default();
    property::statement_dispute::open_by_owner(
        &state, user.id, party, statement, form.line, &comment,
    )
    .await?;
    Ok((
        Toast::success("Your dispute was sent. We will come back to you."),
        renox::HxRefresh,
    )
        .into_response())
}

/// The units the tenant `party` holds: those of its leases.
///
/// # Errors
///
/// The database's errors.
pub async fn tenant_units(db: &Db, party: PartyId) -> Result<Vec<wl_kernel::Subject>> {
    let mut units: Vec<wl_kernel::Subject> = Vec::new();
    for lease in property::portal::leases_of(db, party).await? {
        let unit = wl_kernel::Subject::new(property::unit::UNIT_SUBJECT_TYPE, lease.unit_id);
        if !units.contains(&unit) {
            units.push(unit);
        }
    }
    Ok(units)
}

/// The portal users acting for `party`.
async fn party_users(db: &Db, party: PartyId) -> Result<Vec<i64>> {
    Ok(wl_portal::AccessGrant::query()
        .where_(wl_portal::AccessGrant::PARTY_ID.eq(party.0))
        .get(db)
        .await?
        .into_iter()
        .filter_map(|grant| grant.user_id)
        .collect())
}

/// The requests of the tenant `party` (`WRK.ACL.01`): raised by its portal
/// users or on its units, newest first.
///
/// # Errors
///
/// The database's errors.
pub async fn tenant_requests(db: &Db, party: PartyId) -> Result<Vec<wl_ticketing::Ticket>> {
    let users = party_users(db, party).await?;
    let units = tenant_units(db, party).await?;
    wl_ticketing::portal_requests::requests_for(db, party, &users, &units).await
}

/// *Requests* (BL-TKT-14, `POR.SLF.01`): the tenant's maintenance requests
/// with the status ticketing stored.
pub struct RequestsSection;

impl PortalSection for RequestsSection {
    fn title(&self) -> String {
        "Requests".to_owned()
    }

    fn rows<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
    ) -> SectionFuture<'a, Vec<SectionRow>> {
        Box::pin(async move {
            Ok(tenant_requests(&state.db, party)
                .await?
                .iter()
                .map(|ticket| {
                    SectionRow::new(
                        &format!("{} · {}", ticket.number, ticket.title),
                        wl_ticketing::portal_requests::status_text(ticket.status),
                    )
                })
                .collect())
        })
    }

    fn audience(&self) -> Option<&str> {
        Some(TENANT_AUDIENCE)
    }

    fn permission(&self) -> Option<&str> {
        Some(placed("requests").1)
    }

    fn order(&self) -> i32 {
        placed("requests").0
    }
}

/// The menu of `user` for `party` when it shows *Requests*; 403 otherwise.
async fn requests_menu(
    state: &AppState,
    user: &AuthUser,
    party: PartyId,
    lang: &Lang,
) -> Result<PortalMenu> {
    let menu = PortalMenu::of(state, user.id, party, today(), lang)
        .await?
        .ok_or(Error::Forbidden)?;
    if !menu.allows("requests") {
        return Err(Error::Forbidden);
    }
    Ok(menu)
}

/// Whether `user` may raise or reopen a request for `party`: an active grant
/// (a blocked party may, it is not new business); a `read_only` grant is
/// refused with its notice.
async fn may_request(state: &AppState, user: &AuthUser, party: PartyId, lang: &Lang) -> Result<()> {
    match PortalGuard::check(state, user.id, party, PortalAction::MaintenanceRequest).await? {
        Ok(()) => Ok(()),
        Err(Refused::GrantReadOnly(since)) => Err(Error::Status(
            StatusCode::FORBIDDEN,
            wl_portal::read_only_notice(lang, since),
        )),
        Err(_) => Err(Error::Forbidden),
    }
}

/// What the New request page needs.
#[derive(serde::Serialize, ViewData)]
#[view(template = "requests/new.html")]
pub struct RequestNewPage {
    action: String,
    home_url: String,
    /// The party's units, `(id, name)`.
    units: Vec<(String, String)>,
    /// The categories open to the portal, `(key, name)`.
    categories: Vec<(String, String)>,
}

/// `GET /portal/{party}/requests/new`.
async fn request_new(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(party): Path<i64>,
) -> Result<View> {
    let party = PartyId(party);
    requests_menu(&state, &user, party, &lang).await?;
    may_request(&state, &user, party, &lang).await?;
    let mut units = Vec::new();
    for unit in tenant_units(&state.db, party).await? {
        let name = wl_kernel::documents::subject_text(&state.db, &unit).await?;
        units.push((unit.subject_id.to_string(), name));
    }
    let categories = wl_ticketing::portal_requests::portal_categories(&state.db)
        .await?
        .into_iter()
        .map(|c| (c.key.clone(), c.name(&lang.locale).to_owned()))
        .collect();
    Ok(View::typed(RequestNewPage {
        action: format!("/portal/{}/requests", party.0),
        home_url: format!("/portal/{}/requests", party.0),
        units,
        categories,
    }))
}

/// The New request form.
#[derive(serde::Deserialize, serde::Serialize, Validate, Debug)]
pub struct RequestForm {
    /// The unit's id.
    pub unit: i64,
    /// The category's key.
    #[validate(required, max = 64)]
    pub category: String,
    /// The description.
    #[validate(max = 5000)]
    pub description: Option<String>,
    /// The photos.
    #[serde(default, skip_serializing)]
    pub photos: Vec<Upload>,
}

/// `POST /portal/{party}/requests`: raises the request (channel *portal*,
/// requester the user); 403 without the section or with a `read_only`
/// grant, 422 for a unit the party does not hold or a bad category or photo.
async fn request_store(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(party): Path<i64>,
    Valid(form): Valid<RequestForm>,
) -> Result<Response> {
    let party = PartyId(party);
    requests_menu(&state, &user, party, &lang).await?;
    may_request(&state, &user, party, &lang).await?;
    let unit = wl_kernel::Subject::new(property::unit::UNIT_SUBJECT_TYPE, form.unit);
    if !tenant_units(&state.db, party).await?.contains(&unit) {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Choose one of your units.".to_owned(),
        ));
    }
    let ticket = wl_ticketing::portal_requests::raise(
        &state,
        user.id,
        party,
        wl_ticketing::portal_requests::PortalRequest {
            subject: unit,
            category: form.category.clone(),
            description: form.description.clone().unwrap_or_default(),
            photos: form.photos,
        },
    )
    .await?;
    Ok((
        Toast::success("Your request was sent."),
        Redirect::to(&format!("/portal/{}/requests/{}", party.0, ticket.id.0)),
    )
        .into_response())
}

/// One visible comment.
#[derive(serde::Serialize, Debug, Clone)]
pub struct RequestComment {
    id: i64,
    at: String,
    text: String,
}

/// What a request's page needs.
#[derive(serde::Serialize, ViewData)]
#[view(template = "requests/show.html")]
pub struct RequestPage {
    number: String,
    title: String,
    description: String,
    status: String,
    comments: Vec<RequestComment>,
    /// The amount with VAT of a chargeable job.
    charge: Option<String>,
    can_confirm: bool,
    can_reopen: bool,
    confirm_text: String,
    confirm_url: String,
    reopen_url: String,
    home_url: String,
}

/// The request `id` of the tenant `party`; 404 when it is not one of its.
async fn party_request(db: &Db, party: PartyId, id: i64) -> Result<wl_ticketing::Ticket> {
    tenant_requests(db, party)
        .await?
        .into_iter()
        .find(|t| t.id == id)
        .ok_or(Error::NotFound)
}

/// `GET /portal/{party}/requests/{ticket}`: the request, its visible
/// comments and, once resolved, Confirm (with the charge) and Reopen.
async fn request_show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((party, id)): Path<(i64, i64)>,
) -> Result<View> {
    let party = PartyId(party);
    let menu = requests_menu(&state, &user, party, &lang).await?;
    let ticket = party_request(&state.db, party, id).await?;
    let calendar = wl_kernel::calendar::calendar(&state);
    let comments = wl_ticketing::portal_requests::visible_comments(&state.db, &ticket)
        .await?
        .into_iter()
        .map(|c| RequestComment {
            id: c.id,
            at: c
                .created_at
                .map(|at| calendar.local(at).format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_default(),
            text: c.text,
        })
        .collect();
    let charge = wl_ticketing::portal_requests::charge_text(&state.db, &ticket).await?;
    let resolved = ticket.status == wl_ticketing::TicketStatus::Resolved;
    let requester = ticket.requester_user_id == Some(user.id);
    let confirm_text = match &charge {
        Some(amount) => format!("The work is done. You will be charged {amount} (incl. VAT)."),
        None => "The work is done.".to_owned(),
    };
    Ok(View::typed(RequestPage {
        number: ticket.number.clone(),
        title: ticket.title.clone(),
        description: ticket.description.clone(),
        status: wl_ticketing::portal_requests::status_text(ticket.status).to_owned(),
        comments,
        charge,
        can_confirm: resolved && requester,
        can_reopen: resolved && requester && menu.read_only_since.is_none(),
        confirm_text,
        confirm_url: format!("/portal/{}/requests/{}/confirm", party.0, ticket.id),
        reopen_url: format!("/portal/{}/requests/{}/reopen", party.0, ticket.id),
        home_url: format!("/portal/{}/requests", party.0),
    }))
}

/// `POST /portal/{party}/requests/{ticket}/confirm`: the requester confirms
/// the resolved work; the ticket closes.
async fn request_confirm(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((party, id)): Path<(i64, i64)>,
) -> Result<Response> {
    let party = PartyId(party);
    requests_menu(&state, &user, party, &lang).await?;
    let ticket = party_request(&state.db, party, id).await?;
    wl_ticketing::work::confirm(&state, &user, &ticket)
        .await
        .map_err(renox::state_machine::TransitionError::into_error)?;
    Ok((Toast::success("Thank you: the request is closed."), renox::HxRefresh).into_response())
}

/// `POST /portal/{party}/requests/{ticket}/reopen`: the requester reopens
/// the resolved work; refused for a `read_only` grant.
async fn request_reopen(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((party, id)): Path<(i64, i64)>,
) -> Result<Response> {
    let party = PartyId(party);
    requests_menu(&state, &user, party, &lang).await?;
    may_request(&state, &user, party, &lang).await?;
    let ticket = party_request(&state.db, party, id).await?;
    wl_ticketing::work::reopen(&state, &user, &ticket, renox::db::now())
        .await
        .map_err(renox::state_machine::TransitionError::into_error)?;
    Ok((Toast::success("The request is open again."), renox::HxRefresh).into_response())
}

/// The document kind of a request's photos (BL-TKT-14): images up to 20 MB
/// on a ticket, uploaded from the portal.
fn photo_kind() -> wl_documents::KindSeed {
    wl_documents::KindSeed::new(wl_ticketing::portal_requests::PHOTO_KIND, "Request photo")
        .name_ar("صورة الطلب")
        .subject_type(wl_kernel::tickets::TICKET_SUBJECT_TYPE)
        .file_types(&["jpg", "jpeg", "png", "webp", "heic"])
        .max_size(20 * 1024 * 1024)
        .portal_upload()
        .without_review()
}

/// Registers the tenant sections (BL-PRP-30, BL-TKT-14) and the owner's
/// (BL-PRP-29) with wl-portal.
pub fn register_sections() {
    wl_portal::register_section("requests", RequestsSection);
    wl_portal::register_section("lease", LeaseSection);
    wl_portal::register_section("cheques", ChequesSection);
    wl_portal::register_section("documents", DocumentsSection);
    wl_portal::register_section("owner_statements", StatementsSection);
}

/// The module: the seed, its command, the sections and the contract-end
/// listeners. The host mounts it for a Property client with the portal on.
pub struct PropertyPortal;

impl Module for PropertyPortal {
    fn name(&self) -> &'static str {
        "property_portal"
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .get("/portal/{party}/lease-documents/{document}", download)
            .name("property_portal.document")
            .post(
                "/portal/{party}/owner-statements/{statement}/dispute",
                dispute,
            )
            .name("property_portal.statement_dispute")
            .get("/portal/{party}/requests/new", request_new)
            .name("property_portal.request_new")
            .post("/portal/{party}/requests", request_store)
            .name("property_portal.request_store")
            .get("/portal/{party}/requests/{ticket}", request_show)
            .name("property_portal.request")
            .post("/portal/{party}/requests/{ticket}/confirm", request_confirm)
            .name("property_portal.request_confirm")
            .post("/portal/{party}/requests/{ticket}/reopen", request_reopen)
            .name("property_portal.request_reopen")
            .require_auth()
    }

    fn register(&self, app: &mut Registry) {
        register_sections();
        app.view_data::<RequestNewPage>()
            .view_data::<RequestPage>();
        // The photos of a portal request, stored on the ticket (BL-TKT-14).
        wl_documents::register_kind(photo_kind());
        app.command(
            "property:portal-install",
            "Store the tenant portal audience and its roles, where the database lacks them",
            |_args, state: AppState| async move {
                let created = install(&state).await?;
                println!("{created} tenant portal rows created.");
                Ok(())
            },
        );
        app.seeder(|state: AppState| async move { install(&state).await.map(|_| ()) });
        app.listen(|event: LeaseTerminated, state: AppState| async move {
            lease_ended(&state, event.tenant, "lease terminated")
                .await
                .map(|_| ())
        });
        app.listen(|event: AgreementEnded, state: AppState| async move {
            agreement_ended(&state, event.owner, event.ended_on)
                .await
                .map(|_| ())
        });
        app.listen(|event: LeaseForceTerminated, state: AppState| async move {
            lease_ended(&state, event.tenant, "lease force-terminated")
                .await
                .map(|_| ())
        });
    }
}
