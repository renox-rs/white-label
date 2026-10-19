//! The acknowledgement services (wl-documents §4.3, §5.7, `DOC.RUL.04`,
//! `HRM.ACL.10`, issue #53).
//!
//! - **Request** ([`Acknowledgements::request`]): a required reader with no
//!   `pending` or `acknowledged` acknowledgement of the SOP's current version
//!   gets a `pending` one, due in the SOP's due days (from the return date for
//!   a reader `on_leave`), and is told. [`Acknowledgements::reconcile_sop`]
//!   and [`Acknowledgements::reconcile_employee`] ask whoever is missing and
//!   cancel the pending ones of readers who are no longer required.
//! - **Open** ([`Acknowledgements::open`]): the reader's session opens the
//!   version, or an accepted translation of it; the opening and the file's
//!   checksum are kept.
//! - **Acknowledge** ([`Acknowledgements::acknowledge`]): only the reader, in
//!   their own session, for the SOP's current version, after opening a file
//!   whose checksum is unchanged. [`Acknowledgements::record_in_person`]: a
//!   staff user is the witness and the reader's drawn signature is the
//!   evidence; the reader is then told on their own channel.
//! - **Replace and cancel**: a newer version replaces the pending ones; an
//!   employee who left, died, lost the role or scope, or a withdrawn SOP
//!   cancels them.
//!
//! The employees come from the kernel's `Workforce` (wl-hr), so this module
//! does not depend on the HR crate. An employee that `Workforce` does not list
//! (absconded) keeps their pending acknowledgements: reminders pause.

use renox::auth::permissions::Scope;
use renox::chrono::{Days, NaiveDate};
use renox::db::now;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::Subject;
use wl_kernel::documents::{SopAcknowledged, SopId, SopReaderRef};
use wl_kernel::workforce::{EmployeeId, StaffProfile, workforce};

use super::machine::{ACKNOWLEDGE, AckRun, CANCEL, REPLACE};
use super::model::{AckStatus, METHOD_CLICK, METHOD_DRAWN, SopAcknowledgement};
use super::notices;
use crate::sops::readers::matches;
use crate::sops::{Sop, SopStatus, Sops};
use crate::{Document, DocumentStatus, DocumentVersion};

/// The language of an SOP version itself: the client's base language. A
/// translation carries its own.
pub const VERSION_LANGUAGE: &str = "en";

/// The reason a newer version gives.
const REASON_REPLACED: &str = "A newer version was accepted.";

/// Why an acknowledgement was not opened or recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckRefused {
    /// There is no such acknowledgement, or no such file for it.
    NotFound,
    /// Only the reader acknowledges for themselves.
    NotReader,
    /// Only a staff user who may witness records in person.
    NotWitness,
    /// It is not `pending`.
    NotPending,
    /// The version is no longer the SOP's current one: the reader is sent to
    /// the current version.
    Superseded,
    /// The reader has not opened the version or a translation of it.
    NotOpened,
    /// The opened file changed after it was opened.
    ChecksumChanged,
    /// An in-person record needs the drawn signature.
    SignatureRequired,
    /// The file is no longer stored.
    FileMissing,
}

impl AckRefused {
    /// The text key of the refusal.
    fn key(self) -> &'static str {
        match self {
            AckRefused::NotFound => "wl_documents.acks.refused.not_found",
            AckRefused::NotReader => "wl_documents.acks.refused.not_reader",
            AckRefused::NotWitness => "wl_documents.acks.refused.not_witness",
            AckRefused::NotPending => "wl_documents.acks.refused.not_pending",
            AckRefused::Superseded => "wl_documents.acks.refused.superseded",
            AckRefused::NotOpened => "wl_documents.acks.refused.not_opened",
            AckRefused::ChecksumChanged => "wl_documents.acks.refused.checksum",
            AckRefused::SignatureRequired => "wl_documents.acks.refused.signature",
            AckRefused::FileMissing => "wl_documents.acks.refused.file_missing",
        }
    }

    /// The English explanation.
    pub fn message(self) -> String {
        crate::texts::english(self.key(), &[])
    }

    /// The explanation in the page's language.
    pub fn message_in(self, lang: &Lang) -> String {
        crate::texts::t(lang, self.key())
    }

    /// The refusal as an HTTP error (404 for an unknown acknowledgement, 403
    /// for someone who may not act, else 422).
    pub fn into_error(self) -> Error {
        match self {
            AckRefused::NotFound => Error::NotFound,
            AckRefused::NotReader | AckRefused::NotWitness => Error::Forbidden,
            _ => Error::Status(StatusCode::UNPROCESSABLE_ENTITY, self.message()),
        }
    }
}

/// A refusal or the thing.
pub type Refusal<T> = std::result::Result<T, AckRefused>;

/// The reader's own session: where the confirmation came from.
#[derive(Debug, Clone, Default)]
pub struct OwnSession {
    /// The reader's IP address.
    pub ip: Option<String>,
    /// The reader's device (user agent).
    pub user_agent: Option<String>,
}

/// A file the reader opened: the version, or an accepted translation of it.
#[derive(Debug, Clone)]
pub struct OpenedFile {
    /// The document: the version or the translation.
    pub document: Document,
    /// Its current file.
    pub file: DocumentVersion,
    /// Its language.
    pub language: String,
}

/// What a reconciliation did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Reconciled {
    /// Acknowledgements requested.
    pub requested: usize,
    /// Pending acknowledgements cancelled.
    pub cancelled: usize,
}

impl Reconciled {
    fn add(&mut self, other: Reconciled) {
        self.requested += other.requested;
        self.cancelled += other.cancelled;
    }
}

/// The acknowledgement service.
pub struct Acknowledgements;

/// The scope a staff scope names.
fn scope_of(subject: &Subject) -> Scope {
    Scope::new(&subject.subject_type, subject.subject_id)
}

/// The scopes an employee works in; the global scope when they have none.
fn scopes_or_global(scopes: &[Subject]) -> Vec<Scope> {
    if scopes.is_empty() {
        vec![Scope::global()]
    } else {
        scopes.iter().map(scope_of).collect()
    }
}

/// Whether one of the `readers` lists `role` working in `scopes`.
pub fn lists(readers: &[SopReaderRef], role: &str, scopes: &[Subject]) -> bool {
    let scopes = scopes_or_global(scopes);
    readers
        .iter()
        .any(|reader| scopes.iter().any(|scope| matches(reader, role, scope)))
}

/// Whether the SOP applies to the employee's legal entity: an SOP of every
/// entity does; an SOP of one entity does not apply to an employee of
/// another. An employee with no entity on record is not excluded.
pub fn entity_fits(sop: &Sop, entity: Option<i64>) -> bool {
    match (sop.legal_entity(), entity) {
        (Some(wanted), Some(own)) => wanted == own,
        _ => true,
    }
}

/// Whether `profile` is a required reader of `sop` with `readers`: listed by
/// the workforce, of the SOP's entity, and named by a reader.
pub fn is_required(sop: &Sop, readers: &[SopReaderRef], profile: &StaffProfile) -> bool {
    profile.listed
        && entity_fits(sop, profile.legal_entity_id)
        && lists(readers, &profile.role, &profile.scopes)
}

/// `day` plus `days` calendar days.
fn plus(day: NaiveDate, days: i64) -> NaiveDate {
    day.checked_add_days(Days::new(u64::try_from(days.max(0)).unwrap_or(0)))
        .unwrap_or(day)
}

impl Acknowledgements {
    /// The acknowledgement `id`, if there is one.
    pub async fn find(db: &Db, id: i64) -> Result<Option<SopAcknowledgement>> {
        SopAcknowledgement::find(db, id).await
    }

    /// The acknowledgements of `sop`'s version `document`, oldest first.
    pub async fn of_version(
        db: &Db,
        sop: i64,
        document: i64,
    ) -> Result<Vec<SopAcknowledgement>> {
        SopAcknowledgement::query()
            .where_(SopAcknowledgement::SOP_ID.eq(sop))
            .where_(SopAcknowledgement::DOCUMENT_ID.eq(document))
            .order_by(SopAcknowledgement::ID)
            .get(db)
            .await
    }

    /// The `pending` acknowledgements of `employee`, oldest first.
    pub async fn pending_of(db: &Db, employee: i64) -> Result<Vec<SopAcknowledgement>> {
        SopAcknowledgement::query()
            .where_(SopAcknowledgement::EMPLOYEE_ID.eq(employee))
            .where_(SopAcknowledgement::STATUS.eq(AckStatus::Pending))
            .order_by(SopAcknowledgement::ID)
            .get(db)
            .await
    }

    /// The staff profile of employee `id`.
    pub async fn profile(state: &AppState, id: i64) -> Result<Option<StaffProfile>> {
        workforce(state).staff(state, EmployeeId(id)).await
    }

    /// The current accepted version document of `sop`, with its running
    /// number; `None` for a draft or withdrawn SOP.
    pub async fn current(state: &AppState, sop: &Sop) -> Result<Option<(Document, i64)>> {
        let Some(id) = sop.current_document_id else {
            return Ok(None);
        };
        let Some(document) = Document::find(&state.db, id).await? else {
            return Ok(None);
        };
        if document.status != DocumentStatus::Accepted {
            return Ok(None);
        }
        let number = Sops::version_number(&state.db, &document).await?;
        Ok(Some((document, number)))
    }

    /// The required readers of `sop` among the workforce: the profiles
    /// [`is_required`] holds for.
    pub async fn required(state: &AppState, sop: &Sop) -> Result<Vec<StaffProfile>> {
        let readers = Sops::readers(&state.db, sop.id).await?;
        let staff = workforce(state);
        let mut found = Vec::new();
        for id in staff.all_staff(&state.db).await? {
            if let Some(profile) = staff.staff(state, id).await?
                && is_required(sop, &readers, &profile)
            {
                found.push(profile);
            }
        }
        Ok(found)
    }

    /// The `pending` or `acknowledged` acknowledgement of version `document`
    /// of `sop` for `employee`, if any.
    async fn standing(
        db: &Db,
        sop: i64,
        document: i64,
        employee: i64,
    ) -> Result<Option<SopAcknowledgement>> {
        SopAcknowledgement::query()
            .where_(SopAcknowledgement::SOP_ID.eq(sop))
            .where_(SopAcknowledgement::DOCUMENT_ID.eq(document))
            .where_(SopAcknowledgement::EMPLOYEE_ID.eq(employee))
            .where_(SopAcknowledgement::STATUS.is_in([AckStatus::Pending, AckStatus::Acknowledged]))
            .first(db)
            .await
    }

    /// `request`: asks `profile` to acknowledge version `version` (number
    /// `number`) of `sop`, on `day`. `None` when the reader already has a
    /// `pending` or `acknowledged` acknowledgement of this version. The reader
    /// is told unless `tell` is false.
    pub async fn request(
        state: &AppState,
        sop: &Sop,
        version: &Document,
        number: i64,
        profile: &StaffProfile,
        day: NaiveDate,
        tell: bool,
    ) -> Result<Option<SopAcknowledgement>> {
        if Self::standing(&state.db, sop.id, version.id, profile.employee.0)
            .await?
            .is_some()
        {
            return Ok(None);
        }
        let start = match (profile.on_leave, profile.returns_on) {
            (true, Some(back)) => back.max(day),
            _ => day,
        };
        let ack = SopAcknowledgement::create(
            &state.db,
            SopAcknowledgement {
                sop_id: sop.id,
                document_id: version.id,
                version_no: number,
                employee_id: profile.employee.0,
                requested_on: day,
                due_date: plus(start, sop.ack_due_days),
                ..SopAcknowledgement::default()
            },
        )
        .await?;
        if tell {
            notices::requested(state, &ack, sop, profile).await?;
        }
        Ok(Some(ack))
    }

    /// Asks every required reader of `sop` who has no acknowledgement of its
    /// current version, and cancels the `pending` ones of readers who are
    /// listed but no longer required. A draft or withdrawn SOP does nothing.
    pub async fn reconcile_sop(
        state: &AppState,
        sop_id: i64,
        day: NaiveDate,
    ) -> Result<Reconciled> {
        let mut done = Reconciled::default();
        let Some(sop) = Sop::find(&state.db, sop_id).await? else {
            return Ok(done);
        };
        let Some((version, number)) = Self::current(state, &sop).await? else {
            return Ok(done);
        };
        let required = Self::required(state, &sop).await?;
        for profile in &required {
            if Self::request(state, &sop, &version, number, profile, day, true)
                .await?
                .is_some()
            {
                done.requested += 1;
            }
        }
        let pending: Vec<SopAcknowledgement> = Self::of_version(&state.db, sop.id, version.id)
            .await?
            .into_iter()
            .filter(|a| a.status == AckStatus::Pending)
            .collect();
        for ack in pending {
            if required.iter().any(|p| p.employee.0 == ack.employee_id) {
                continue;
            }
            // Not listed (absconded) keeps theirs: the reminders pause.
            let Some(profile) = Self::profile(state, ack.employee_id).await? else {
                continue;
            };
            if profile.listed {
                end(state, &ack, CANCEL, "No longer a required reader.").await?;
                done.cancelled += 1;
            }
        }
        Ok(done)
    }

    /// Asks the employee for every active SOP they are a required reader of,
    /// and cancels their `pending` ones of SOPs they no longer read.
    pub async fn reconcile_employee(
        state: &AppState,
        employee: i64,
        day: NaiveDate,
    ) -> Result<Reconciled> {
        let mut done = Reconciled::default();
        let Some(profile) = Self::profile(state, employee).await? else {
            return Ok(done);
        };
        if !profile.listed {
            return Ok(done);
        }
        for sop in Sops::list(&state.db, Some(SopStatus::Active)).await? {
            let Some((version, number)) = Self::current(state, &sop).await? else {
                continue;
            };
            let readers = Sops::readers(&state.db, sop.id).await?;
            if is_required(&sop, &readers, &profile) {
                if Self::request(state, &sop, &version, number, &profile, day, true)
                    .await?
                    .is_some()
                {
                    done.requested += 1;
                }
            } else if let Some(ack) =
                Self::standing(&state.db, sop.id, version.id, employee).await?
                && ack.status == AckStatus::Pending
            {
                end(state, &ack, CANCEL, "No longer a required reader.").await?;
                done.cancelled += 1;
            }
        }
        Ok(done)
    }

    /// Reconciles every active SOP.
    pub async fn reconcile_all(state: &AppState, day: NaiveDate) -> Result<Reconciled> {
        let mut done = Reconciled::default();
        for sop in Sops::list(&state.db, Some(SopStatus::Active)).await? {
            done.add(Self::reconcile_sop(state, sop.id, day).await?);
        }
        Ok(done)
    }

    /// `replace`: the pending acknowledgements of `sop` for a version other
    /// than `keep` (a newer one was accepted). Returns how many.
    pub async fn replace_pending(state: &AppState, sop: i64, keep: i64) -> Result<usize> {
        let old: Vec<SopAcknowledgement> = SopAcknowledgement::query()
            .where_(SopAcknowledgement::SOP_ID.eq(sop))
            .where_(SopAcknowledgement::STATUS.eq(AckStatus::Pending))
            .where_(SopAcknowledgement::DOCUMENT_ID.ne(keep))
            .order_by(SopAcknowledgement::ID)
            .get(&state.db)
            .await?;
        for ack in &old {
            end(state, ack, REPLACE, REASON_REPLACED).await?;
        }
        Ok(old.len())
    }

    /// `cancel`: every pending acknowledgement of `sop` (withdrawn). Returns
    /// their ids.
    pub async fn cancel_for_sop(state: &AppState, sop: i64, reason: &str) -> Result<Vec<i64>> {
        let pending: Vec<SopAcknowledgement> = SopAcknowledgement::query()
            .where_(SopAcknowledgement::SOP_ID.eq(sop))
            .where_(SopAcknowledgement::STATUS.eq(AckStatus::Pending))
            .order_by(SopAcknowledgement::ID)
            .get(&state.db)
            .await?;
        for ack in &pending {
            end(state, ack, CANCEL, reason).await?;
        }
        Ok(pending.iter().map(|a| a.id).collect())
    }

    /// `cancel`: every pending acknowledgement of `employee` (exited, died or
    /// never started). Returns how many.
    pub async fn cancel_for_employee(
        state: &AppState,
        employee: i64,
        reason: &str,
    ) -> Result<usize> {
        let pending = Self::pending_of(&state.db, employee).await?;
        for ack in &pending {
            end(state, ack, CANCEL, reason).await?;
        }
        Ok(pending.len())
    }

    /// The acknowledgement `id` for the reader `by`, still pending, of the
    /// SOP's current version.
    async fn readable(
        state: &AppState,
        id: i64,
        by: i64,
    ) -> Result<Refusal<(SopAcknowledgement, Sop)>> {
        let Some(ack) = SopAcknowledgement::find(&state.db, id).await? else {
            return Ok(Err(AckRefused::NotFound));
        };
        let profile = Self::profile(state, ack.employee_id).await?;
        if profile.and_then(|p| p.user_id) != Some(by) {
            return Ok(Err(AckRefused::NotReader));
        }
        let sop = Sop::find_or_404(&state.db, ack.sop_id).await?;
        if ack.status != AckStatus::Pending {
            return Ok(Err(if ack.status == AckStatus::Replaced {
                AckRefused::Superseded
            } else {
                AckRefused::NotPending
            }));
        }
        if sop.current_document_id != Some(ack.document_id) {
            return Ok(Err(AckRefused::Superseded));
        }
        Ok(Ok((ack, sop)))
    }

    /// The file `document` of acknowledgement `ack`: the version itself, or
    /// an accepted translation of it.
    async fn file_of(
        state: &AppState,
        ack: &SopAcknowledgement,
        document: i64,
    ) -> Result<Refusal<OpenedFile>> {
        let Some(doc) = Document::find(&state.db, document).await? else {
            return Ok(Err(AckRefused::NotFound));
        };
        let language = if doc.id == ack.document_id {
            VERSION_LANGUAGE.to_owned()
        } else {
            match Sops::translation_of(&state.db, doc.id).await? {
                Some(link)
                    if link.version_document_id == ack.document_id
                        && doc.status == DocumentStatus::Accepted =>
                {
                    link.language
                }
                _ => return Ok(Err(AckRefused::NotFound)),
            }
        };
        let Some(file) = doc.current_file(&state.db).await? else {
            return Ok(Err(AckRefused::FileMissing));
        };
        Ok(Ok(OpenedFile {
            document: doc,
            file,
            language,
        }))
    }

    /// The reader `by` opens `document` (the version, or an accepted
    /// translation of it) for acknowledgement `id`: the opening and the
    /// file's checksum are kept, and the file is returned to be shown. A
    /// version that is no longer current is refused with
    /// [`AckRefused::Superseded`].
    pub async fn open(
        state: &AppState,
        id: i64,
        by: i64,
        document: i64,
    ) -> Result<Refusal<OpenedFile>> {
        let (ack, _) = match Self::readable(state, id, by).await? {
            Ok(found) => found,
            Err(refused) => return Ok(Err(refused)),
        };
        let opened = match Self::file_of(state, &ack, document).await? {
            Ok(opened) => opened,
            Err(refused) => return Ok(Err(refused)),
        };
        SopAcknowledgement::query()
            .where_(SopAcknowledgement::ID.eq(ack.id))
            .update(
                &state.db,
                &[
                    ("opened_document_id", &Some(opened.document.id)),
                    ("opened_checksum", &Some(opened.file.checksum.clone())),
                    ("opened_at", &Some(now())),
                    ("updated_at", &now()),
                ],
            )
            .await?;
        Ok(Ok(opened))
    }

    /// `acknowledge` in the reader's own session (`HRM.ACL.10`): `by` is the
    /// reader's account. Emits `SopAcknowledged`.
    pub async fn acknowledge(
        state: &AppState,
        id: i64,
        by: i64,
        session: OwnSession,
    ) -> Result<Refusal<SopAcknowledgement>> {
        let (ack, _) = match Self::readable(state, id, by).await? {
            Ok(found) => found,
            Err(refused) => return Ok(Err(refused)),
        };
        let Some(opened_id) = ack.opened_document_id else {
            return Ok(Err(AckRefused::NotOpened));
        };
        let opened = match Self::file_of(state, &ack, opened_id).await? {
            Ok(opened) => opened,
            Err(AckRefused::FileMissing) => return Ok(Err(AckRefused::FileMissing)),
            // A translation that is no longer accepted: open it again.
            Err(_) => return Ok(Err(AckRefused::NotOpened)),
        };
        let unchanged = ack.opened_checksum.as_deref() == Some(opened.file.checksum.as_str());
        let run = AckRun {
            by: Some(by),
            current: true,
            opened: true,
            checksum_ok: unchanged,
            method: METHOD_CLICK.to_owned(),
            document_id: Some(opened.document.id),
            language: opened.language.clone(),
            checksum: opened.file.checksum.clone(),
            ip: session.ip,
            user_agent: session.user_agent,
            ..AckRun::default()
        };
        if !unchanged {
            return Ok(Err(AckRefused::ChecksumChanged));
        }
        run_acknowledge(state, &ack, run, None).await.map(Ok)
    }

    /// `acknowledge` in person (`HRM.ACL.10`): the staff user `witness` records
    /// the reader's drawn `signature`. The caller checks that `witness` may
    /// witness for this reader. Emits `SopAcknowledged` and tells the reader
    /// on their own channel what was recorded and by whom.
    pub async fn record_in_person(
        state: &AppState,
        id: i64,
        witness: i64,
        signature: &str,
    ) -> Result<Refusal<SopAcknowledgement>> {
        let Some(ack) = SopAcknowledgement::find(&state.db, id).await? else {
            return Ok(Err(AckRefused::NotFound));
        };
        if ack.status != AckStatus::Pending {
            return Ok(Err(AckRefused::NotPending));
        }
        let sop = Sop::find_or_404(&state.db, ack.sop_id).await?;
        if sop.current_document_id != Some(ack.document_id) {
            return Ok(Err(AckRefused::Superseded));
        }
        let signature = signature.trim();
        if signature.is_empty() {
            return Ok(Err(AckRefused::SignatureRequired));
        }
        let version = Document::find_or_404(&state.db, ack.document_id).await?;
        let Some(file) = version.current_file(&state.db).await? else {
            return Ok(Err(AckRefused::FileMissing));
        };
        let run = AckRun {
            by: Some(witness),
            current: true,
            opened: false,
            checksum_ok: true,
            method: METHOD_DRAWN.to_owned(),
            document_id: Some(version.id),
            language: VERSION_LANGUAGE.to_owned(),
            checksum: file.checksum,
            signature: Some(signature.to_owned()),
            witness_id: Some(witness),
            ..AckRun::default()
        };
        run_acknowledge(state, &ack, run, Some(witness)).await.map(Ok)
    }

    /// Whether the user `user` may witness an in-person record for `profile`:
    /// the reader's manager, or a holder of `sops.manage`.
    pub fn may_witness(user: &User, profile: &StaffProfile) -> bool {
        user.has_permission(crate::permissions::SOPS_MANAGE) || profile.managers.contains(&user.id)
    }
}

/// Runs `acknowledge` with `run` and, after the commit, emits the event and,
/// in person, tells the reader.
async fn run_acknowledge(
    state: &AppState,
    ack: &SopAcknowledgement,
    run: AckRun,
    witness: Option<i64>,
) -> Result<SopAcknowledgement> {
    ack.transition(ACKNOWLEDGE)
        .by_system()
        .input(&run)
        .meta(renox::serde_json::json!({ "ran_by": run.by }))
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    let done = SopAcknowledgement::find_or_404(&state.db, ack.id).await?;
    state
        .emit(SopAcknowledged {
            sop: SopId(done.sop_id),
            employee: done.reader(),
            witness,
        })
        .await?;
    if let Some(witness) = witness {
        let sop = Sop::find_or_404(&state.db, done.sop_id).await?;
        if let Some(profile) = Acknowledgements::profile(state, done.employee_id).await? {
            notices::in_person_recorded(state, &done, &sop, &profile, witness).await?;
            SopAcknowledgement::query()
                .where_(SopAcknowledgement::ID.eq(done.id))
                .update(
                    &state.db,
                    &[("in_person_notified_at", &Some(now())), ("updated_at", &now())],
                )
                .await?;
        }
    }
    Ok(SopAcknowledgement::find_or_404(&state.db, ack.id).await?)
}

/// Runs `replace` or `cancel` on `ack` with `reason`.
async fn end(state: &AppState, ack: &SopAcknowledgement, transition: &str, reason: &str) -> Result {
    ack.transition(transition)
        .by_system()
        .reason(reason)
        .input(&AckRun {
            reason: Some(reason.to_owned()),
            ..AckRun::default()
        })
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reader(role: &str, scope: Option<(&str, &str)>) -> SopReaderRef {
        SopReaderRef {
            all_staff: false,
            role: role.into(),
            scope_type: scope.map_or("", |s| s.0).into(),
            scope_id: scope.map_or("", |s| s.1).into(),
        }
    }

    fn profile(role: &str, scopes: Vec<Subject>) -> StaffProfile {
        StaffProfile {
            employee: EmployeeId(7),
            party: wl_kernel::PartyId(50),
            user_id: None,
            role: role.into(),
            scopes,
            legal_entity_id: None,
            listed: true,
            on_leave: false,
            returns_on: None,
            managers: vec![],
        }
    }

    #[test]
    fn a_reader_lists_the_role_in_the_scope_or_everywhere() {
        let site1 = Subject::new("property.site", 1);
        let readers = [reader("site_worker", Some(("property.site", "1")))];
        assert!(lists(&readers, "site_worker", &[site1.clone()]));
        assert!(!lists(&readers, "site_worker", &[Subject::new("property.site", 2)]));
        assert!(!lists(&readers, "driver", &[site1]));
        // No scope: global, which a scoped reader does not list.
        assert!(!lists(&readers, "site_worker", &[]));
        assert!(lists(&[reader("site_worker", None)], "site_worker", &[]));
        let all = SopReaderRef {
            all_staff: true,
            ..reader("", None)
        };
        assert!(lists(&[all], "anyone", &[]));
    }

    #[test]
    fn an_sop_of_one_entity_skips_the_employees_of_another() {
        let every = Sop::default();
        let one = Sop {
            legal_entity_type: "legal_entity".into(),
            legal_entity_id: 2,
            ..Sop::default()
        };
        assert!(entity_fits(&every, Some(1)));
        assert!(entity_fits(&one, Some(2)));
        assert!(!entity_fits(&one, Some(1)));
        assert!(entity_fits(&one, None));
    }

    #[test]
    fn only_a_listed_employee_is_required() {
        let sop = Sop::default();
        let readers = [reader("site_worker", None)];
        let ali = profile("site_worker", vec![Subject::new("property.site", 1)]);
        assert!(is_required(&sop, &readers, &ali));
        let gone = StaffProfile {
            listed: false,
            ..ali.clone()
        };
        assert!(!is_required(&sop, &readers, &gone));
        assert!(!is_required(&sop, &readers, &profile("driver", vec![])));
    }

    #[test]
    fn the_due_date_adds_calendar_days() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();
        assert_eq!(plus(day, 7), NaiveDate::from_ymd_opt(2026, 11, 9).unwrap());
        assert_eq!(plus(day, 0), day);
        assert_eq!(plus(day, -3), day);
    }

    #[test]
    fn refusals_have_messages_and_the_right_status() {
        assert!(!AckRefused::Superseded.message().is_empty());
        assert!(matches!(AckRefused::NotFound.into_error(), Error::NotFound));
        assert!(matches!(AckRefused::NotReader.into_error(), Error::Forbidden));
        assert!(matches!(
            AckRefused::NotOpened.into_error(),
            Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _)
        ));
    }
}
