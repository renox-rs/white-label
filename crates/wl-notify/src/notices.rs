//! Statutory notices (wl-notify §5.4): a notice with legal effect that
//! gave up, the proof staff record that it was delivered another way, and
//! the answer [`status`] gives requesters (#59 A2, A3, A5, A6).
//!
//! A requester starts a deadline only from a real delivery: the provider's
//! `delivered_at`, or the delivery date a proof states. Never from the time
//! a proof was typed in (`recorded_at`).

use renox::chrono::NaiveDate;
use renox::db::Json;
use renox::prelude::*;
use renox::serde_json::{self, Value};
use serde::{Deserialize, Serialize};
use wl_kernel::notify::{
    NoticeStatus, NotifyRecipient, ProofMethod, StatutoryNoticeProofRecorded, UndeliveredNotice,
};
use wl_kernel::{Subject, documents};

use crate::message::{Message, MessageStatus};

/// One statutory notice that gave up, with its proof once recorded.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_notify_statutory_notices",
    unique(chain_id),
    index(subject_type, subject_id)
)]
pub struct StatutoryNotice {
    /// The primary key.
    pub id: i64,
    /// The first message of the chain that gave up.
    pub chain_id: i64,
    /// The notification type's key.
    pub type_key: String,
    /// The type of the record it is about.
    pub subject_type: String,
    /// The id of the record it is about.
    pub subject_id: i64,
    /// The request's short label for the subject.
    pub subject_label: Option<String>,
    /// Who the notice was for (the kernel's recipient, as JSON).
    pub recipient: Json<Value>,
    /// The message that gave up.
    pub message_id: i64,
    /// When the notice gave up.
    pub gave_up_at: DateTime,
    /// The staff task, once opened.
    pub ticket_id: Option<i64>,
    /// How the notice was delivered, once a proof is recorded.
    pub proof_method: Option<ProofMethod>,
    /// The delivery date the proof states.
    pub delivered_on: Option<NaiveDate>,
    /// The proof's note.
    pub proof_note: Option<String>,
    /// The user who recorded the proof.
    pub recorded_by: Option<i64>,
    /// When the proof was recorded (never a deadline start).
    pub recorded_at: Option<DateTime>,
    /// Where the attachment is on the storage disk.
    pub attachment_key: Option<String>,
    /// The attachment's file name.
    pub attachment_name: Option<String>,
    /// The attachment's MIME type.
    pub attachment_mime: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl StatutoryNotice {
    /// The record for the chain that gave up with `message`, not stored yet.
    pub fn for_message(message: &Message, gave_up_at: DateTime) -> Self {
        StatutoryNotice {
            chain_id: message.chain(),
            type_key: message.type_key.clone(),
            subject_type: message.subject_type.clone(),
            subject_id: message.subject_id,
            subject_label: message.subject_label.clone(),
            recipient: Json(serde_json::to_value(message.person()).unwrap_or(Value::Null)),
            message_id: message.id,
            gave_up_at,
            ..Default::default()
        }
    }

    /// The record about the chain `chain`, if any.
    pub async fn by_chain(db: &Db, chain: i64) -> Result<Option<StatutoryNotice>> {
        StatutoryNotice::query()
            .where_(StatutoryNotice::CHAIN_ID.eq(chain))
            .first(db)
            .await
    }

    /// The record about `subject`, oldest first.
    pub async fn of_subject(db: &Db, subject: &Subject) -> Result<Vec<StatutoryNotice>> {
        StatutoryNotice::query()
            .where_(StatutoryNotice::SUBJECT_TYPE.eq(subject.subject_type.as_str()))
            .where_(StatutoryNotice::SUBJECT_ID.eq(subject.subject_id))
            .order_by(StatutoryNotice::ID)
            .get(db)
            .await
    }

    /// Whether a proof was recorded.
    pub fn has_proof(&self) -> bool {
        self.delivered_on.is_some()
    }

    /// Who the notice was for, if the stored recipient reads back.
    pub fn recipient(&self) -> Option<NotifyRecipient> {
        serde_json::from_value(self.recipient.0.clone()).ok()
    }

    /// The record's subject.
    pub fn subject(&self) -> Subject {
        Subject {
            subject_type: self.subject_type.clone(),
            subject_id: self.subject_id,
        }
    }

    /// The page where staff record the proof.
    pub fn proof_path(&self) -> String {
        format!("/statutory-notices/{}/proof", self.id)
    }
}

/// The attachment of a proof, as uploaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofFile {
    /// The file's name.
    pub name: String,
    /// The file's MIME type.
    pub mime: String,
    /// The file's size in bytes.
    pub size: usize,
}

/// The largest attachment (20 MB, as the unblock evidence of wl-parties).
pub const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;

/// What staff typed into the proof form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofInput {
    /// How the notice was delivered.
    pub method: ProofMethod,
    /// The delivery date; none when left empty.
    pub delivered_on: Option<NaiveDate>,
    /// The note.
    pub note: String,
    /// The attachment, if one was uploaded.
    pub attachment: Option<ProofFile>,
}

/// Why a proof is refused (422, with the field it belongs to).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofRefusal {
    /// The type does not offer the method.
    MethodNotOffered,
    /// No delivery date.
    DateMissing,
    /// The delivery date is after today.
    DateInFuture,
    /// The delivery date is before the day the notice gave up.
    DateBeforeGiveUp,
    /// The method needs an attachment.
    AttachmentMissing,
    /// A `call` needs a note.
    NoteMissing,
    /// The attachment is not a PDF, image or text file.
    AttachmentType,
    /// The attachment is larger than [`MAX_ATTACHMENT_BYTES`].
    AttachmentSize,
}

impl ProofRefusal {
    /// The form field the refusal belongs to.
    pub fn field(self) -> &'static str {
        match self {
            ProofRefusal::MethodNotOffered => "method",
            ProofRefusal::DateMissing
            | ProofRefusal::DateInFuture
            | ProofRefusal::DateBeforeGiveUp => "delivered_on",
            ProofRefusal::AttachmentMissing
            | ProofRefusal::AttachmentType
            | ProofRefusal::AttachmentSize => "attachment",
            ProofRefusal::NoteMissing => "note",
        }
    }

    /// The text key of the message.
    pub fn text_key(self) -> &'static str {
        match self {
            ProofRefusal::MethodNotOffered => "wl_notify.proof.method_not_offered",
            ProofRefusal::DateMissing => "wl_notify.proof.date_missing",
            ProofRefusal::DateInFuture => "wl_notify.proof.date_in_future",
            ProofRefusal::DateBeforeGiveUp => "wl_notify.proof.date_before_give_up",
            ProofRefusal::AttachmentMissing => "wl_notify.proof.attachment_missing",
            ProofRefusal::NoteMissing => "wl_notify.proof.note_missing",
            ProofRefusal::AttachmentType => "wl_notify.proof.attachment_type",
            ProofRefusal::AttachmentSize => "wl_notify.proof.attachment_size",
        }
    }
}

/// Whether `mime` is a PDF, an image or a plain-text file.
fn allowed_mime(mime: &str) -> bool {
    let mime = mime.trim().to_ascii_lowercase();
    mime == "application/pdf" || mime == "text/plain" || mime.starts_with("image/")
}

/// Checks a proof (A6). `offered` are the type's methods, `gave_up_on` the
/// Asia/Dubai date the notice gave up (the same day is allowed) and `today`
/// the Asia/Dubai date now. Everything but `call` needs an attachment; a
/// `call` needs a note instead.
pub fn validate_proof(
    offered: &[ProofMethod],
    input: &ProofInput,
    gave_up_on: NaiveDate,
    today: NaiveDate,
) -> Result<(), ProofRefusal> {
    if !offered.contains(&input.method) {
        return Err(ProofRefusal::MethodNotOffered);
    }
    let Some(delivered_on) = input.delivered_on else {
        return Err(ProofRefusal::DateMissing);
    };
    if delivered_on > today {
        return Err(ProofRefusal::DateInFuture);
    }
    if delivered_on < gave_up_on {
        return Err(ProofRefusal::DateBeforeGiveUp);
    }
    if let Some(file) = &input.attachment {
        if !allowed_mime(&file.mime) {
            return Err(ProofRefusal::AttachmentType);
        }
        if file.size > MAX_ATTACHMENT_BYTES {
            return Err(ProofRefusal::AttachmentSize);
        }
    } else if input.method != ProofMethod::Call {
        return Err(ProofRefusal::AttachmentMissing);
    }
    if input.method == ProofMethod::Call && input.note.trim().is_empty() {
        return Err(ProofRefusal::NoteMissing);
    }
    Ok(())
}

/// Whether the notice of `type_key` about `subject` reached its recipient
/// (A2): follows the newest message chain of that subject and type.
pub async fn status(db: &Db, subject: &Subject, type_key: &str) -> Result<NoticeStatus> {
    let newest = Message::query()
        .where_(Message::SUBJECT_TYPE.eq(subject.subject_type.as_str()))
        .where_(Message::SUBJECT_ID.eq(subject.subject_id))
        .where_(Message::TYPE_KEY.eq(type_key))
        .order_by_desc(Message::CHAIN_ID)
        .first(db)
        .await?;
    let Some(newest) = newest else {
        return Ok(NoticeStatus::NotDelivered);
    };
    let chain = newest.chain();
    let last = Message::query()
        .where_(Message::CHAIN_ID.eq(chain))
        .order_by_desc(Message::ID)
        .first(db)
        .await?
        .unwrap_or(newest);
    if matches!(last.status, MessageStatus::Delivered | MessageStatus::Read) {
        let at = last
            .delivered_at
            .or(last.updated_at)
            .unwrap_or_else(renox::db::now);
        return Ok(NoticeStatus::Delivered { at });
    }
    // The record belongs to the chain that gave up; an alternative
    // recipient's chain carries the first chain's id.
    let mut notice = StatutoryNotice::by_chain(db, chain).await?;
    if notice.is_none()
        && let Some(root) = last.alternative_of
    {
        notice = StatutoryNotice::by_chain(db, root).await?;
    }
    Ok(match notice.and_then(|n| n.delivered_on) {
        Some(delivered_on) => NoticeStatus::ProofRecorded { delivered_on },
        None => NoticeStatus::NotDelivered,
    })
}

/// The statutory notices about `subject` that gave up and are still not
/// delivered: no proof, and the newest chain of their type not delivered.
pub async fn undelivered(db: &Db, subject: &Subject) -> Result<Vec<UndeliveredNotice>> {
    let mut out = Vec::new();
    for notice in StatutoryNotice::of_subject(db, subject).await? {
        if notice.has_proof() {
            continue;
        }
        if status(db, subject, &notice.type_key).await? != NoticeStatus::NotDelivered {
            continue;
        }
        out.push(UndeliveredNotice {
            notice: notice.id,
            type_key: notice.type_key.clone(),
            label: notice
                .subject_label
                .clone()
                .unwrap_or_else(|| notice.type_key.clone()),
            gave_up_at: notice.gave_up_at,
            proof_path: notice.proof_path(),
        });
    }
    Ok(out)
}

/// The proof method called `name` in a form (`hand_delivery`), if any.
pub fn parse_method(name: &str) -> Option<ProofMethod> {
    ProofMethod::ALL
        .iter()
        .copied()
        .find(|method| method.as_str() == name.trim())
}

/// Why a proof is not recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofError {
    /// The rules refuse the proof (422).
    Refused(ProofRefusal),
    /// The notice already has a proof, or was delivered (409).
    AlreadyRecorded,
}

/// `user` records the proof that the notice `notice` was delivered another
/// way (A6, A9). In one transaction it checks the rules, keeps the
/// attachment privately under `notify/proofs/{notice}/` and stores the
/// proof. After the commit it closes the staff task and emits
/// `StatutoryNoticeProofRecorded`. A second proof, or a notice that was
/// delivered after all, is [`ProofError::AlreadyRecorded`] and writes nothing.
///
/// # Errors
///
/// `NotFound` for an unknown notice; storage and database errors.
pub async fn record_proof(
    state: &AppState,
    user: i64,
    notice: i64,
    input: ProofInput,
    file: Option<&Upload>,
) -> Result<std::result::Result<StatutoryNotice, ProofError>> {
    let found = StatutoryNotice::find(&state.db, notice)
        .await?
        .ok_or(Error::NotFound)?;
    let delivered = matches!(
        status(&state.db, &found.subject(), &found.type_key).await?,
        NoticeStatus::Delivered { .. }
    );
    if found.has_proof() || delivered {
        return Ok(Err(ProofError::AlreadyRecorded));
    }
    let offered = crate::notifier::type_def(&state.db, &found.type_key)
        .await?
        .proof_methods;
    if let Err(refusal) = validate_proof(
        &offered,
        &input,
        local_date(found.gave_up_at),
        documents::today(),
    ) {
        return Ok(Err(ProofError::Refused(refusal)));
    }
    let mut tx = state.db.begin_immediate().await?;
    let mut record = StatutoryNotice::query()
        .where_(StatutoryNotice::ID.eq(notice))
        .lock_for_update()
        .first(&mut tx)
        .await?
        .ok_or(Error::NotFound)?;
    if record.has_proof() {
        return Ok(Err(ProofError::AlreadyRecorded));
    }
    if let (Some(file), Some(attachment)) = (file, &input.attachment) {
        let key = file
            .store(&state.storage, &format!("notify/proofs/{notice}"))
            .await?;
        record.attachment_key = Some(key);
        record.attachment_name = Some(attachment.name.clone());
        record.attachment_mime = Some(attachment.mime.clone());
    }
    let delivered_on = input.delivered_on.ok_or(Error::NotFound)?;
    record.proof_method = Some(input.method);
    record.delivered_on = Some(delivered_on);
    let note = input.note.trim();
    record.proof_note = (!note.is_empty()).then(|| note.to_owned());
    record.recorded_by = Some(user);
    record.recorded_at = Some(renox::db::now());
    record.save(&mut tx).await?;
    tx.commit().await?;
    if let Some(ticket) = record.ticket_id {
        let note = crate::texts::english("wl_notify.proof.task_note");
        wl_kernel::tickets::task_creator(state)
            .complete(state, wl_kernel::tickets::TicketId(ticket), &note)
            .await?;
    }
    state
        .emit(StatutoryNoticeProofRecorded {
            notice: record.id,
            subject: record.subject(),
            type_key: record.type_key.clone(),
            method: input.method,
            delivered_on,
        })
        .await?;
    Ok(Ok(record))
}

/// The Asia/Dubai date `at` falls on.
pub fn local_date(at: DateTime) -> NaiveDate {
    documents::dubai_date(at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, d).unwrap()
    }

    fn input(method: ProofMethod) -> ProofInput {
        ProofInput {
            method,
            delivered_on: Some(day(3)),
            note: String::new(),
            attachment: Some(ProofFile {
                name: "signed.pdf".into(),
                mime: "application/pdf".into(),
                size: 10,
            }),
        }
    }

    fn check(input: &ProofInput) -> Result<(), ProofRefusal> {
        validate_proof(ProofMethod::ALL, input, day(1), day(5))
    }

    #[test]
    fn a_proof_with_an_attachment_passes() {
        assert_eq!(check(&input(ProofMethod::HandDelivery)), Ok(()));
        assert_eq!(check(&input(ProofMethod::Courier)), Ok(()));
    }

    #[test]
    fn a_courier_proof_needs_an_attachment() {
        let mut bare = input(ProofMethod::Courier);
        bare.attachment = None;
        assert_eq!(check(&bare), Err(ProofRefusal::AttachmentMissing));
    }

    #[test]
    fn a_call_needs_a_note_and_no_attachment() {
        let mut call = input(ProofMethod::Call);
        call.attachment = None;
        assert_eq!(check(&call), Err(ProofRefusal::NoteMissing));
        call.note = "  Spoke to Sara M. at 10:15 ".into();
        assert_eq!(check(&call), Ok(()));
    }

    #[test]
    fn a_method_the_type_does_not_offer_is_refused() {
        let offered = [ProofMethod::Courier];
        let refused = validate_proof(&offered, &input(ProofMethod::Call), day(1), day(5));
        assert_eq!(refused, Err(ProofRefusal::MethodNotOffered));
        assert_eq!(ProofRefusal::MethodNotOffered.field(), "method");
    }

    #[test]
    fn the_delivery_date_is_between_the_give_up_day_and_today() {
        let mut p = input(ProofMethod::HandDelivery);
        p.delivered_on = None;
        assert_eq!(check(&p), Err(ProofRefusal::DateMissing));
        p.delivered_on = Some(day(6));
        assert_eq!(check(&p), Err(ProofRefusal::DateInFuture));
        p.delivered_on = Some(day(5));
        assert_eq!(check(&p), Ok(()));
        p.delivered_on = Some(day(1));
        assert_eq!(check(&p), Ok(()), "the same day is allowed");
        p.delivered_on = Some(NaiveDate::from_ymd_opt(2026, 8, 31).unwrap());
        assert_eq!(check(&p), Err(ProofRefusal::DateBeforeGiveUp));
        assert_eq!(ProofRefusal::DateBeforeGiveUp.field(), "delivered_on");
    }

    #[test]
    fn only_pdf_image_and_text_files_are_accepted() {
        for mime in ["application/pdf", "image/png", "IMAGE/JPEG", "text/plain"] {
            let mut p = input(ProofMethod::Courier);
            p.attachment.as_mut().unwrap().mime = mime.into();
            assert_eq!(check(&p), Ok(()), "{mime}");
        }
        for mime in ["application/zip", "text/html", ""] {
            let mut p = input(ProofMethod::Courier);
            p.attachment.as_mut().unwrap().mime = mime.into();
            assert_eq!(check(&p), Err(ProofRefusal::AttachmentType), "{mime}");
        }
    }

    #[test]
    fn every_refusal_has_a_text() {
        use ProofRefusal::*;
        for refusal in [
            MethodNotOffered,
            DateMissing,
            DateInFuture,
            DateBeforeGiveUp,
            AttachmentMissing,
            NoteMissing,
            AttachmentType,
            AttachmentSize,
        ] {
            let key = refusal.text_key();
            assert_ne!(crate::texts::english(key), key);
        }
    }

    #[test]
    fn a_record_is_made_for_the_chain_that_gave_up() {
        let message = Message {
            id: 9,
            chain_id: Some(4),
            type_key: "lease.changed_terms_notice".into(),
            subject_type: "property.lease".into(),
            subject_id: 7,
            ..Default::default()
        };
        let notice = StatutoryNotice::for_message(&message, renox::db::now());
        assert_eq!(notice.chain_id, 4);
        assert_eq!(notice.message_id, 9);
        assert_eq!(notice.subject(), Subject::new("property.lease", 7));
        assert!(!notice.has_proof());
        assert!(notice.recipient().is_some());
    }
}
