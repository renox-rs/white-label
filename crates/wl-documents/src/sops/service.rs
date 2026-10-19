//! The SOP service: creating and updating SOPs with a code that is unique
//! per legal entity, and the derived status ([`Sops`]).

use renox::db::now;
use renox::prelude::*;

use super::model::{Sop, SopStatus};
use crate::Document;
use crate::permissions;
use crate::texts;

/// A 422 with `message`.
pub(super) fn refused(message: String) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)
}

/// Review interval in months an SOP gets unless told otherwise.
pub const DEFAULT_REVIEW_INTERVAL_MONTHS: i64 = 12;

/// Days to acknowledge a new version unless told otherwise.
pub const DEFAULT_ACK_DUE_DAYS: i64 = 7;

/// What a form or a caller gives to create or change an SOP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SopInput {
    /// The code, e.g. `SOP-HSE-01`; unique per legal entity.
    pub code: String,
    /// The English title.
    pub title_en: String,
    /// The Arabic title; may be empty.
    pub title_ar: String,
    /// The category, e.g. `hse`; may be empty.
    pub category: String,
    /// The legal entity's type; empty with `legal_entity_id` 0 when the SOP
    /// applies to every entity.
    pub legal_entity_type: String,
    /// The legal entity's id; 0 for none.
    pub legal_entity_id: i64,
    /// The role whose holders accept, reject and review it.
    pub owner_role: String,
    /// Months between reviews.
    pub review_interval_months: i64,
    /// Days a reader has to acknowledge a new version.
    pub ack_due_days: i64,
}

impl SopInput {
    /// An input for `code` and the English `title`, owned by `owner_role`,
    /// with the default intervals and no legal entity.
    pub fn new(code: &str, title_en: &str, owner_role: &str) -> Self {
        SopInput {
            code: code.to_owned(),
            title_en: title_en.to_owned(),
            title_ar: String::new(),
            category: String::new(),
            legal_entity_type: String::new(),
            legal_entity_id: 0,
            owner_role: owner_role.to_owned(),
            review_interval_months: DEFAULT_REVIEW_INTERVAL_MONTHS,
            ack_due_days: DEFAULT_ACK_DUE_DAYS,
        }
    }

    /// Sets the Arabic title.
    pub fn title_ar(mut self, title: &str) -> Self {
        self.title_ar = title.to_owned();
        self
    }

    /// Sets the category.
    pub fn category(mut self, category: &str) -> Self {
        self.category = category.to_owned();
        self
    }

    /// Limits the SOP to one legal entity.
    pub fn legal_entity(mut self, entity_type: &str, id: i64) -> Self {
        self.legal_entity_type = entity_type.to_owned();
        self.legal_entity_id = id;
        self
    }

    /// Sets the review interval in months.
    pub fn review_every(mut self, months: i64) -> Self {
        self.review_interval_months = months;
        self
    }

    /// Sets the days to acknowledge.
    pub fn ack_due_days(mut self, days: i64) -> Self {
        self.ack_due_days = days;
        self
    }

    /// The input trimmed and checked; the text key of the first problem.
    fn checked(mut self) -> std::result::Result<Self, &'static str> {
        for field in [
            &mut self.code,
            &mut self.title_en,
            &mut self.title_ar,
            &mut self.category,
            &mut self.legal_entity_type,
            &mut self.owner_role,
        ] {
            *field = field.trim().to_owned();
        }
        if self.code.is_empty() {
            return Err("wl_documents.sops.refused.blank_code");
        }
        if self.title_en.is_empty() {
            return Err("wl_documents.sops.refused.blank_title");
        }
        if self.owner_role.is_empty() {
            return Err("wl_documents.sops.refused.blank_owner_role");
        }
        if self.review_interval_months < 1 || self.ack_due_days < 1 {
            return Err("wl_documents.sops.refused.bad_interval");
        }
        // An entity is both a type and an id, or neither (stored as '' / 0).
        if self.legal_entity_type.is_empty() != (self.legal_entity_id == 0)
            || self.legal_entity_id < 0
        {
            return Err("wl_documents.sops.refused.bad_entity");
        }
        Ok(self)
    }
}

/// The SOP service.
pub struct Sops;

impl Sops {
    /// Creates an SOP, `draft` until a version is accepted.
    ///
    /// # Errors
    ///
    /// `Forbidden` without `sops.manage`; 422 for a blank code, title or
    /// owner role, an interval under 1, a half-given legal entity, or a code
    /// that already exists for that legal entity; the database's errors.
    pub async fn create(state: &AppState, by: &AuthUser, input: SopInput) -> Result<Sop> {
        require_manage(by)?;
        let lang = state.current_lang();
        let input = input
            .checked()
            .map_err(|key| refused(texts::t(&lang, key)))?;
        ensure_unique(state, &input, None).await?;
        Sop::create(
            &state.db,
            Sop {
                code: input.code,
                title_en: input.title_en,
                title_ar: input.title_ar,
                category: input.category,
                legal_entity_type: input.legal_entity_type,
                legal_entity_id: input.legal_entity_id,
                owner_role: input.owner_role,
                review_interval_months: input.review_interval_months,
                ack_due_days: input.ack_due_days,
                ..Default::default()
            },
        )
        .await
    }

    /// Changes the SOP `sop` to `input`. Its versions, readers and status
    /// stay as they are.
    ///
    /// # Errors
    ///
    /// As [`Sops::create`]; `NotFound` for an unknown SOP.
    pub async fn update(state: &AppState, by: &AuthUser, sop: i64, input: SopInput) -> Result<Sop> {
        require_manage(by)?;
        let lang = state.current_lang();
        let input = input
            .checked()
            .map_err(|key| refused(texts::t(&lang, key)))?;
        let mut row = Sop::find_or_404(&state.db, sop).await?;
        ensure_unique(state, &input, Some(row.id)).await?;
        row.code = input.code;
        row.title_en = input.title_en;
        row.title_ar = input.title_ar;
        row.category = input.category;
        row.legal_entity_type = input.legal_entity_type;
        row.legal_entity_id = input.legal_entity_id;
        row.owner_role = input.owner_role;
        row.review_interval_months = input.review_interval_months;
        row.ack_due_days = input.ack_due_days;
        row.updated_at = Some(now());
        row.save(&state.db).await?;
        Ok(row)
    }

    /// The status of `sop`, derived from its current version.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn status(db: &Db, sop: &Sop) -> Result<SopStatus> {
        let current = match sop.current_document_id {
            Some(id) => Document::find(db, id).await?.map(|d| d.status),
            None => None,
        };
        Ok(SopStatus::of(current))
    }

    /// The SOPs, by code then legal entity; only those with `status` when
    /// given.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn list(db: &Db, status: Option<SopStatus>) -> Result<Vec<Sop>> {
        let mut found = Vec::new();
        for sop in Sop::query()
            .order_by(Sop::CODE)
            .order_by(Sop::LEGAL_ENTITY_ID)
            .get(db)
            .await?
        {
            if status.is_none() || Some(Self::status(db, &sop).await?) == status {
                found.push(sop);
            }
        }
        Ok(found)
    }
}

/// `by` must hold `sops.manage`.
pub(super) fn require_manage(by: &AuthUser) -> Result<()> {
    if by.has_permission(permissions::SOPS_MANAGE) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// Refuses a code already used by another SOP of the same legal entity.
async fn ensure_unique(state: &AppState, input: &SopInput, own_id: Option<i64>) -> Result<()> {
    let same = Sop::query()
        .where_(Sop::CODE.eq(input.code.as_str()))
        .where_(Sop::LEGAL_ENTITY_TYPE.eq(input.legal_entity_type.as_str()))
        .where_(Sop::LEGAL_ENTITY_ID.eq(input.legal_entity_id))
        .get(&state.db)
        .await?;
    if same.iter().any(|sop| Some(sop.id) != own_id) {
        return Err(refused(texts::t_with(
            &state.current_lang(),
            "wl_documents.sops.refused.code_taken",
            &[("code", &input.code)],
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_input_is_trimmed_and_checked() {
        let ok = SopInput::new("  SOP-HSE-01 ", " Working at height ", "hse_officer")
            .checked()
            .unwrap();
        assert_eq!(ok.code, "SOP-HSE-01");
        assert_eq!(ok.title_en, "Working at height");
        assert_eq!(ok.review_interval_months, 12);
        assert_eq!(ok.ack_due_days, 7);
    }

    #[test]
    fn a_blank_field_or_a_bad_number_is_refused() {
        let base = || SopInput::new("SOP-1", "Title", "owner");
        assert!(SopInput::new(" ", "Title", "owner").checked().is_err());
        assert!(SopInput::new("SOP-1", "", "owner").checked().is_err());
        assert!(SopInput::new("SOP-1", "Title", " ").checked().is_err());
        assert!(base().review_every(0).checked().is_err());
        assert!(base().ack_due_days(0).checked().is_err());
        assert!(base().checked().is_ok());
    }

    #[test]
    fn a_legal_entity_is_a_type_and_an_id_or_neither() {
        let base = || SopInput::new("SOP-1", "Title", "owner");
        assert!(base().legal_entity("legal_entity", 2).checked().is_ok());
        assert!(base().legal_entity("legal_entity", 0).checked().is_err());
        assert!(base().legal_entity("", 2).checked().is_err());
        assert!(base().legal_entity("legal_entity", -1).checked().is_err());
    }
}
