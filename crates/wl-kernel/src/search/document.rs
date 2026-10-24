//! What a module says about one record ([`SearchDocument`]) and how it is
//! turned into the stored rows: identifiers normalised (and hashed when
//! sensitive), sensitive values left out of the text, match keys added.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use renox::auth::permissions::Scope;
use serde::Serialize;

use super::{SearchDocumentRow, SearchIdentifierRow};
use crate::subject::Subject;

/// The kinds of identifier the modules use. A kind is plain text, so a
/// module may use one of its own; these are the shared ones.
pub mod kinds {
    /// A document number from a sequence: invoice, receipt, lease, ticket…
    pub const NUMBER: &str = "number";
    /// A UAE tax registration number.
    pub const TRN: &str = "trn";
    /// An Emirates ID number (personal: always hashed).
    pub const EMIRATES_ID: &str = "emirates_id";
    /// A passport number (personal: always hashed).
    pub const PASSPORT: &str = "passport";
    /// A labour card or work permit number (personal: always hashed).
    pub const LABOUR_CARD: &str = "labour_card";
    /// A visa file number (personal: always hashed).
    pub const VISA: &str = "visa";
    /// A phone number, kept in E.164 (personal: always hashed).
    pub const PHONE: &str = "phone";
    /// An email address (personal: always hashed).
    pub const EMAIL: &str = "email";
    /// A unit, flat or plot number.
    pub const UNIT: &str = "unit";
    /// A vehicle plate number.
    pub const PLATE: &str = "plate";
    /// A vehicle identification number.
    pub const VIN: &str = "vin";
    /// A cheque number.
    pub const CHEQUE: &str = "cheque";
    /// A stock keeping unit or an item code.
    pub const SKU: &str = "sku";
    /// A barcode.
    pub const BARCODE: &str = "barcode";
    /// A lot or batch number.
    pub const LOT: &str = "lot";
    /// A serial number.
    pub const SERIAL: &str = "serial";
    /// A tracking, waybill or container number.
    pub const TRACKING: &str = "tracking";
    /// An Ejari registration number.
    pub const EJARI: &str = "ejari";

    /// The kinds that are personal data: stored only as a hash, whatever
    /// the module asked.
    pub const PERSONAL: &[&str] = &[EMIRATES_ID, PASSPORT, LABOUR_CARD, VISA, PHONE, EMAIL];
}

/// A record's data class (SECURITY.md §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    /// Invoices, tickets, catalogue, bookings.
    #[default]
    Normal,
    /// Identity numbers, phone, address, photos of people.
    Personal,
    /// Medical data.
    Health,
    /// Salaries, bank details, payroll.
    Financial,
}

impl Sensitivity {
    /// The stored name.
    pub fn as_str(self) -> &'static str {
        match self {
            Sensitivity::Normal => "normal",
            Sensitivity::Personal => "personal",
            Sensitivity::Health => "health",
            Sensitivity::Financial => "financial",
        }
    }

    /// Whether values of this class stay out of the full-text columns.
    pub fn is_sensitive(self) -> bool {
        self != Sensitivity::Normal
    }
}

/// A value a record is found by exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identifier {
    /// What it is, one of [`kinds`] or a module's own.
    pub kind: String,
    /// The value as the record holds it.
    pub value: String,
    /// Its data class; personal kinds are always at least `Personal`.
    pub sensitivity: Sensitivity,
}

/// How a module describes one record for the search index.
#[derive(Debug, Clone, Default)]
pub struct SearchDocument {
    title: String,
    secondary: String,
    keywords: Vec<String>,
    identifiers: Vec<Identifier>,
    status: Option<String>,
    voided: bool,
    scope: Scope,
    legal_entity_id: Option<i64>,
    sensitivity: Sensitivity,
    parties: Vec<i64>,
    requires: Option<String>,
}

impl SearchDocument {
    /// A document whose result reads `title`.
    pub fn new(title: impl Into<String>) -> Self {
        SearchDocument {
            title: title.into(),
            ..Default::default()
        }
    }

    /// The line under the title, e.g. the customer's name.
    pub fn secondary(mut self, text: impl Into<String>) -> Self {
        self.secondary = text.into();
        self
    }

    /// More words the record is found by: another spelling of the name, the
    /// building, the item names.
    pub fn keyword(mut self, word: impl Into<String>) -> Self {
        let word = word.into();
        if !word.trim().is_empty() {
            self.keywords.push(word);
        }
        self
    }

    /// A value the record is found by exactly. A personal kind
    /// ([`kinds::PERSONAL`]) is stored only as a hash.
    pub fn identifier(self, kind: &str, value: impl Into<String>) -> Self {
        let sensitivity = if kinds::PERSONAL.contains(&kind) {
            Sensitivity::Personal
        } else {
            Sensitivity::Normal
        };
        self.identifier_of_class(kind, value, sensitivity)
    }

    /// A value of class `sensitivity` the record is found by exactly; a
    /// sensitive one is stored only as a hash and kept out of the text.
    pub fn identifier_of_class(
        mut self,
        kind: &str,
        value: impl Into<String>,
        sensitivity: Sensitivity,
    ) -> Self {
        let value = value.into();
        if !value.trim().is_empty() {
            let sensitivity = if kinds::PERSONAL.contains(&kind) && !sensitivity.is_sensitive() {
                Sensitivity::Personal
            } else {
                sensitivity
            };
            self.identifiers.push(Identifier {
                kind: kind.to_owned(),
                value,
                sensitivity,
            });
        }
        self
    }

    /// The record's status, shown as a badge.
    pub fn status(mut self, status: impl Into<String>) -> Self {
        self.status = Some(status.into());
        self
    }

    /// Marks the record voided; it stays findable by its number.
    pub fn voided(mut self, voided: bool) -> Self {
        self.voided = voided;
        self
    }

    /// The record's scope (global by default).
    pub fn scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }

    /// The legal entity the record belongs to.
    pub fn legal_entity(mut self, legal_entity_id: impl Into<Option<i64>>) -> Self {
        self.legal_entity_id = legal_entity_id.into();
        self
    }

    /// The record's data class (`Normal` by default).
    pub fn sensitivity(mut self, sensitivity: Sensitivity) -> Self {
        self.sensitivity = sensitivity;
        self
    }

    /// A party the record belongs to (the invoice's customer, the ticket's
    /// requester): the portal search (BL-AGT-05) shows a portal-visible
    /// record only to the portal users acting for one of its parties. A
    /// record without a party is never found in the portal. Call it again
    /// for a record of several parties.
    pub fn party(mut self, party_id: i64) -> Self {
        if !self.parties.contains(&party_id) {
            self.parties.push(party_id);
        }
        self
    }

    /// A permission this record needs on top of its type's (a confidential
    /// ticket: `tickets.confidential.view`), held in the record's scope. The
    /// type declares it with
    /// [`SearchableBuilder::record_permission`](super::SearchableBuilder::record_permission);
    /// a permission it did not declare is held by nobody, so the record is
    /// then found by nobody (never by too many).
    pub fn requires(mut self, permission: impl Into<String>) -> Self {
        let permission = permission.into();
        if !permission.trim().is_empty() {
            self.requires = Some(permission);
        }
        self
    }

    /// The identifiers declared so far.
    pub fn identifiers(&self) -> &[Identifier] {
        &self.identifiers
    }

    /// The parties declared so far ([`party`](Self::party)).
    pub fn parties(&self) -> &[i64] {
        &self.parties
    }

    /// The record's own permission, if it needs one
    /// ([`requires`](Self::requires)).
    pub fn required_permission(&self) -> Option<&str> {
        self.requires.as_deref()
    }

    /// The rows stored for `subject`.
    pub(super) fn into_rows(
        self,
        subject: &Subject,
        permission: &str,
        portal_visible: bool,
    ) -> (SearchDocumentRow, Vec<SearchIdentifierRow>) {
        let mut hidden: Vec<String> = Vec::new();
        let mut identifiers: Vec<SearchIdentifierRow> = Vec::new();
        let mut seen = BTreeSet::new();
        for identifier in &self.identifiers {
            let normalized = normalize_identifier(&identifier.kind, &identifier.value);
            if normalized.is_empty() {
                continue;
            }
            let sensitive = identifier.sensitivity.is_sensitive();
            if sensitive {
                hidden.push(identifier.value.trim().to_owned());
                hidden.push(normalized.clone());
            }
            let value = if sensitive {
                hash(&identifier.kind, &normalized)
            } else {
                normalized
            };
            if seen.insert((identifier.kind.clone(), value.clone())) {
                identifiers.push(SearchIdentifierRow {
                    subject_type: subject.subject_type.clone(),
                    subject_id: subject.subject_id,
                    kind: identifier.kind.clone(),
                    value,
                    hashed: sensitive,
                    ..Default::default()
                });
            }
        }
        let title = scrub(&self.title, &hidden);
        let secondary = scrub(&self.secondary, &hidden);
        let keywords = scrub(&self.keywords.join(" "), &hidden);
        let forms = forms_of(&[&title, &secondary, &keywords]);
        let document = SearchDocumentRow {
            subject_type: subject.subject_type.clone(),
            subject_id: subject.subject_id,
            title,
            secondary,
            keywords,
            forms,
            status: self.status,
            voided: self.voided,
            // The record's own permission when it needs one; the type's
            // permission is checked by its subject type in any case.
            permission: self.requires.unwrap_or_else(|| permission.to_owned()),
            portal_visible,
            scope_type: self.scope.kind().to_owned(),
            scope_id: self.scope.id().to_owned(),
            legal_entity_id: self.legal_entity_id,
            sensitivity: self.sensitivity.as_str().to_owned(),
            ..Default::default()
        };
        (document, identifiers)
    }
}

/// `value` as stored and compared for `kind`: Arabic-Indic digits read as
/// ASCII; a phone in E.164 (`050 123 4567` → `+971501234567`); an email in
/// lower case; anything else in upper case without spaces, dashes, slashes
/// and dots (`inv pgp 2026 0004` → `INVPGP20260004`).
pub fn normalize_identifier(kind: &str, value: &str) -> String {
    let value = ascii_digits(value.trim());
    match kind {
        kinds::PHONE => normalize_phone(&value),
        kinds::EMAIL => value.to_lowercase(),
        _ => value
            .chars()
            .filter(|c| !c.is_whitespace() && !matches!(c, '-' | '/' | '.' | '_'))
            .flat_map(char::to_uppercase)
            .collect(),
    }
}

/// Arabic-Indic and Persian digits as ASCII digits; nothing else changes.
fn ascii_digits(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '\u{0660}'..='\u{0669}' => char::from(b'0' + (c as u32 - 0x0660) as u8),
            '\u{06F0}'..='\u{06F9}' => char::from(b'0' + (c as u32 - 0x06F0) as u8),
            other => other,
        })
        .collect()
}

/// A UAE-first E.164 form: `+` and the digits; `00` becomes `+`, a leading
/// `0` the UAE code `+971`, a bare `971…` gets its `+`.
fn normalize_phone(value: &str) -> String {
    let digits: String = value.chars().filter(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return String::new();
    }
    if value.starts_with('+') {
        format!("+{digits}")
    } else if let Some(rest) = digits.strip_prefix("00") {
        format!("+{rest}")
    } else if let Some(rest) = digits.strip_prefix('0') {
        format!("+971{rest}")
    } else if digits.starts_with("971") && digits.len() >= 11 {
        format!("+{digits}")
    } else {
        digits
    }
}

/// The HMAC of a sensitive value under `APP_KEY`, prefixed by its kind so
/// one value of two kinds gives two hashes.
pub(super) fn hash(kind: &str, normalized: &str) -> String {
    renox::webhook::hmac_sha256_hex(hash_key(), format!("{kind}:{normalized}"))
}

/// `APP_KEY` as the app loads it, read once per process. A local app
/// without one uses a fixed key; production refuses to start without one.
fn hash_key() -> &'static [u8] {
    static KEY: OnceLock<Vec<u8>> = OnceLock::new();
    KEY.get_or_init(
        || match renox::Config::load().ok().and_then(|config| config.key) {
            Some(key) => key.into_bytes(),
            None => {
                eprintln!("search: no APP_KEY; sensitive identifiers are hashed with a local key");
                b"wl-search-local-key".to_vec()
            }
        },
    )
}

/// `text` without the sensitive values: each one removed where it appears
/// as typed, then every word whose identifier form equals one of them.
fn scrub(text: &str, hidden: &[String]) -> String {
    if hidden.is_empty() {
        return text.split_whitespace().collect::<Vec<_>>().join(" ");
    }
    let mut text = text.to_owned();
    for value in hidden.iter().filter(|v| !v.is_empty()) {
        text = text.replace(value.as_str(), " ");
    }
    let hidden: BTreeSet<String> = hidden
        .iter()
        .map(|value| normalize_identifier("", value))
        .filter(|value| !value.is_empty())
        .collect();
    text.split_whitespace()
        .filter(|word| !hidden.contains(&normalize_identifier("", word)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The distinct Arabic–Latin match keys of the words of `texts`.
fn forms_of(texts: &[&str]) -> String {
    let mut seen = BTreeSet::new();
    let mut forms = Vec::new();
    for word in texts.iter().flat_map(|text| text.split_whitespace()) {
        let key = renox::text::match_key(word);
        if !key.is_empty() && seen.insert(key.clone()) {
            forms.push(key);
        }
    }
    forms.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_compared_without_spaces_dashes_and_case() {
        assert_eq!(
            normalize_identifier(kinds::NUMBER, "inv pgp 2026 0004"),
            "INVPGP20260004"
        );
        assert_eq!(
            normalize_identifier(kinds::NUMBER, "INV-PGP-2026-0004"),
            "INVPGP20260004"
        );
        assert_eq!(
            normalize_identifier(kinds::PLATE, "DXB A 12345"),
            "DXBA12345"
        );
        assert_eq!(
            normalize_identifier(kinds::PLATE, "dxb-a-12345"),
            "DXBA12345"
        );
        assert_eq!(
            normalize_identifier(kinds::EMIRATES_ID, "٧٨٤-١٩٩٠-١٢٣٤٥٦٧-١"),
            "784199012345671"
        );
    }

    #[test]
    fn phones_are_kept_in_e164() {
        for typed in [
            "050 123 4567",
            "+971 50 123 4567",
            "00971501234567",
            "971501234567",
        ] {
            assert_eq!(normalize_identifier(kinds::PHONE, typed), "+971501234567");
        }
    }

    #[test]
    fn a_personal_kind_is_hashed_and_kept_out_of_the_text() {
        let subject = Subject::new("parties.party", 7);
        let (document, identifiers) = SearchDocument::new("Mohammed Al Ali")
            .keyword("784-1990-1234567-1")
            .identifier(kinds::EMIRATES_ID, "784-1990-1234567-1")
            .into_rows(&subject, "parties.view", false);
        assert_eq!(document.keywords, "");
        assert_eq!(identifiers.len(), 1);
        assert!(identifiers[0].hashed);
        assert!(!identifiers[0].value.contains("784"));
        assert_eq!(
            identifiers[0].value,
            hash(kinds::EMIRATES_ID, "784199012345671")
        );
    }

    #[test]
    fn a_word_equal_to_a_sensitive_value_is_left_out_however_it_is_typed() {
        let hidden = vec![
            "784-1990-1234567-1".to_owned(),
            "784199012345671".to_owned(),
        ];
        assert_eq!(scrub("EID 784199012345671 of Ali", &hidden), "EID of Ali");
        assert_eq!(scrub("EID 784-1990-1234567-1", &hidden), "EID");
    }

    #[test]
    fn match_keys_join_arabic_and_latin_spellings() {
        let forms = forms_of(&["محمد العلي", "Mohammed Al Ali"]);
        let keys: BTreeSet<&str> = forms.split(' ').collect();
        assert!(keys.contains(renox::text::match_key("محمد").as_str()));
        assert_eq!(
            renox::text::match_key("محمد"),
            renox::text::match_key("Mohammed")
        );
    }
}
