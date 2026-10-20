//! Fields only a person or only a company has (wl-parties §3), one row per
//! party in a table of its own, so later issues add columns here without
//! touching Party.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// What only a person has.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_parties_person_details", unique(party_id))]
pub struct PersonDetails {
    /// The primary key.
    pub id: i64,
    /// The person party.
    pub party_id: i64,
    /// The date of birth.
    pub date_of_birth: Option<NaiveDate>,
    /// The nationality (ISO 3166 alpha-2 code).
    pub nationality: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// What only a company has.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_parties_company_details", unique(party_id))]
pub struct CompanyDetails {
    /// The primary key.
    pub id: i64,
    /// The company party.
    pub party_id: i64,
    /// The legal form, e.g. `llc`.
    pub legal_form: Option<String>,
    /// The licensing authority, e.g. DED or a free zone.
    pub licence_authority: Option<String>,
    /// The trade licence number.
    pub trade_licence_no: Option<String>,
    /// Whether the company is VAT registered.
    pub vat_registered: bool,
    /// The tax registration number (15 digits).
    pub trn: Option<String>,
    /// The e-invoicing recipient identifier (Peppol participant ID, scheme +
    /// identifier), stored trimmed and as entered; no format check until the
    /// ASP's format is confirmed. `wl-invoicing` needs it before it issues
    /// an invoice to a recipient in e-invoicing scope.
    pub einvoice_recipient_id: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}
