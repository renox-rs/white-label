//! The static values of 04 that the host turns into the modules' own seeds
//! (BL-ACC-23 wires them): the client KYC document kinds (§9 *Document
//! blocks*).
//!
//! Each KYC kind blocks engagement `activate` (`engagement.activate`, block
//! mode) with the override of `DOC.ACL.02` (the MLRO's approval, 72 hours),
//! for the parties whose role its condition names: the client company's
//! trade licence, and the passport and Emirates ID of each UBO
//! (the condition field `party_role`).

/// One KYC document kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KycKind {
    /// The kind's key.
    pub key: &'static str,
    /// The English name.
    pub name_en: &'static str,
    /// The Arabic name.
    pub name_ar: &'static str,
    /// The roles of the screened parties that must hold it (`client`,
    /// `ubo`).
    pub roles: &'static [&'static str],
    /// Days before expiry the responsible role is reminded (look-ahead).
    pub look_ahead_days: i64,
}

/// The client company's trade licence.
pub const TRADE_LICENCE: &str = "trade_licence";
/// A UBO's passport.
pub const PASSPORT: &str = "passport";
/// A UBO's Emirates ID.
pub const EMIRATES_ID: &str = "emirates_id";

/// The KYC kinds of §9 *Document blocks*, renewed by the compliance officer
/// (MLRO), 30 days of look-ahead.
pub const KYC_KINDS: &[KycKind] = &[
    KycKind {
        key: TRADE_LICENCE,
        name_en: "Trade licence",
        name_ar: "الرخصة التجارية",
        roles: &["client"],
        look_ahead_days: 30,
    },
    KycKind {
        key: PASSPORT,
        name_en: "Passport",
        name_ar: "جواز السفر",
        roles: &["ubo"],
        look_ahead_days: 30,
    },
    KycKind {
        key: EMIRATES_ID,
        name_en: "Emirates ID",
        name_ar: "الهوية الإماراتية",
        roles: &["ubo"],
        look_ahead_days: 30,
    },
];
