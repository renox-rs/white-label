//! The kinds for registrations and certifications every client gets
//! (wl-documents §3 *Seeded kinds for registrations and certifications*,
//! `DOC.EXP.01`, issue #54): trademark registration, Halal certificate, ISO
//! certificate, product registration / conformity and other certification.
//!
//! They have names in English and Arabic, an expiry date and reminders
//! (`DOC.RUL.01`), sensitivity `normal`, no portal upload, the client's own
//! retention rule for company documents, and **no `blocks`**: a vertical may
//! add entries later, the seed never does. They need no review, so a staff
//! user records them accepted at once and a wrong number or expiry date is
//! corrected in place by `documents.manage` (reminders rescheduled, audited).
//!
//! The fields of the spec map onto a document's columns: the number is the
//! registration or certificate number; the authority is the registry,
//! certification body or issuer; the issue date is the filing date; the
//! title and the file's note carry the mark, the classes, the standard and
//! the products or sites covered. *Other certification* has an expiry date
//! that is optional: its kind does not require one, and reminders run when
//! a document has one.
//!
//! **Enabled per vertical.** Each kind is stored on when the client's
//! vertical (`[client] app`) is one of the kind's, else *available and off*
//! (a holder of `document_kinds.manage` turns it on from the kind page,
//! [`crate::kinds::set_enabled`]). The platform table is [`REGISTRATIONS`];
//! a vertical's settings issue changes it, and the responsible role per
//! kind, with [`register_vertical`] ([`VerticalSettings`]).
//!
//! **Responsible role.** *Owner* is the deployment's business owner role
//! (`[roles] owner`, else the company administrators). *Quality role, else
//! owner* is [`VerticalSettings::quality_role`], else the owner; the
//! platform knows one: `qc_inspector` for `manufacturing`. A vertical that
//! seeds its own role for a kind (06: the storekeeper for the Halal
//! certificate) names it in [`VerticalSettings::roles`]; one role per kind
//! is stored, so a second role the vertical wants told goes in the kind's
//! `also_notify`, which is for the expiry notice only.
//!
//! **Subjects.** The legal entity (`legal_entity`) always; the item
//! (`inventory.item`) only in a vertical with an item catalogue
//! ([`ITEM_VERTICALS`]); the supplier party (`parties.party`) for the Halal
//! certificate and *Other certification*.
//!
//! A kind a vertical registered itself with
//! [`register_kind`](crate::register_kind) under the same key wins:
//! [`install`] stores these after the registered kinds and skips a key the
//! database already has, as every install keeps what the client changed.

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};

use renox::prelude::*;
use wl_kernel::ClientConfig;
use wl_kernel::parties::PARTY_SUBJECT_TYPE;

use crate::kinds::{self, KindSeed};

/// The key of *Trademark registration*.
pub const TRADEMARK: &str = "trademark_registration";
/// The key of *Halal certificate*.
pub const HALAL: &str = "halal_certificate";
/// The key of *ISO certificate*.
pub const ISO: &str = "iso_certificate";
/// The key of *Product registration / conformity*.
pub const PRODUCT: &str = "product_registration";
/// The key of *Other certification*.
pub const OTHER: &str = "other_certification";

/// The subject type of a legal entity.
pub const LEGAL_ENTITY_SUBJECT: &str = "legal_entity";
/// The subject type of an item of the catalogue (`STK.ITM.01`).
pub const ITEM_SUBJECT: &str = "inventory.item";

/// The verticals (`[client] app`) with an item catalogue: construction,
/// clinics, F&B, manufacturing and auto (`STK.ITM.01`). Others offer the
/// legal entity subject only.
pub const ITEM_VERTICALS: &[&str] = &["construction", "clinics", "fnb", "manufacturing", "auto"];

/// The quality role the platform knows: manufacturing's QC inspector.
pub const MANUFACTURING_QUALITY_ROLE: &str = "qc_inspector";

/// Who is responsible for a kind unless a vertical says otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Responsible {
    /// The business owner role of the deployment.
    Owner,
    /// The vertical's quality role, else the owner.
    QualityOrOwner,
}

/// One row of the platform table.
#[derive(Debug, Clone, Copy)]
pub struct Registration {
    /// The kind's key.
    pub key: &'static str,
    /// The English name.
    pub name_en: &'static str,
    /// The Arabic name.
    pub name_ar: &'static str,
    /// Whether the supplier party is a subject.
    pub supplier: bool,
    /// Whether the item is a subject (where the vertical has items).
    pub item: bool,
    /// The days before expiry to remind.
    pub reminders: &'static [i64],
    /// Whether the expiry date is required.
    pub requires_expiry: bool,
    /// Who is responsible.
    pub responsible: Responsible,
    /// The verticals the kind is on in; `None` is every vertical.
    pub enabled_in: Option<&'static [&'static str]>,
}

/// The platform table (§3).
pub const REGISTRATIONS: [Registration; 5] = [
    Registration {
        key: TRADEMARK,
        name_en: "Trademark registration",
        name_ar: "تسجيل علامة تجارية",
        supplier: false,
        item: false,
        reminders: &[180, 90, 30],
        requires_expiry: true,
        responsible: Responsible::Owner,
        enabled_in: None,
    },
    Registration {
        key: HALAL,
        name_en: "Halal certificate",
        name_ar: "شهادة حلال",
        supplier: true,
        item: true,
        reminders: &[90, 30, 7],
        requires_expiry: true,
        responsible: Responsible::QualityOrOwner,
        enabled_in: Some(&["fnb", "manufacturing"]),
    },
    Registration {
        key: ISO,
        name_en: "ISO certificate",
        name_ar: "شهادة آيزو",
        supplier: false,
        item: false,
        reminders: &[90, 30, 7],
        requires_expiry: true,
        responsible: Responsible::QualityOrOwner,
        enabled_in: Some(&["construction", "manufacturing"]),
    },
    Registration {
        key: PRODUCT,
        name_en: "Product registration / conformity",
        name_ar: "تسجيل المنتج / شهادة المطابقة",
        supplier: false,
        item: true,
        reminders: &[90, 30, 7],
        requires_expiry: true,
        responsible: Responsible::QualityOrOwner,
        enabled_in: Some(&[]),
    },
    Registration {
        key: OTHER,
        name_en: "Other certification",
        name_ar: "شهادة أخرى",
        supplier: true,
        item: true,
        reminders: &[90, 30, 7],
        requires_expiry: false,
        responsible: Responsible::Owner,
        enabled_in: None,
    },
];

/// What a vertical's settings say about these kinds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerticalSettings {
    /// The vertical's role for quality or compliance; the owner where none.
    pub quality_role: Option<String>,
    /// The keys of the kinds that are on in the vertical, replacing the
    /// platform table; `None` keeps the table.
    pub enabled: Option<Vec<String>>,
    /// The responsible role per kind key, replacing *owner* and *quality
    /// role, else owner*.
    pub roles: BTreeMap<String, String>,
}

impl VerticalSettings {
    /// Settings that change nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// The vertical's quality role.
    pub fn quality_role(mut self, role: &str) -> Self {
        self.quality_role = Some(role.to_owned());
        self
    }

    /// The kinds that are on, by key.
    pub fn enabled(mut self, keys: &[&str]) -> Self {
        self.enabled = Some(keys.iter().map(|k| (*k).to_owned()).collect());
        self
    }

    /// The responsible role of the kind `key`.
    pub fn role(mut self, key: &str, role: &str) -> Self {
        self.roles.insert(key.to_owned(), role.to_owned());
        self
    }
}

static VERTICALS: RwLock<BTreeMap<String, Arc<VerticalSettings>>> = RwLock::new(BTreeMap::new());

/// Registers the settings of the vertical `app` (`[client] app`); a vertical
/// calls it in `register`. Registering the same app again replaces it.
pub fn register_vertical(app: &str, settings: VerticalSettings) {
    VERTICALS
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(app.to_owned(), Arc::new(settings));
}

/// The settings registered for `app`, with the platform's own quality role
/// where the vertical did not set one.
pub fn settings_of(app: &str) -> VerticalSettings {
    let registered = VERTICALS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(app)
        .map(|settings| settings.as_ref().clone())
        .unwrap_or_default();
    if registered.quality_role.is_none() && app == "manufacturing" {
        return registered.quality_role(MANUFACTURING_QUALITY_ROLE);
    }
    registered
}

/// The five kinds for the vertical `app` and the business owner role
/// `owner`, with `settings` applied. No seed has `blocks`.
pub fn seeds(app: &str, owner: &str, settings: &VerticalSettings) -> Vec<KindSeed> {
    let has_items = ITEM_VERTICALS.contains(&app);
    REGISTRATIONS
        .iter()
        .map(|row| {
            let mut seed = KindSeed::new(row.key, row.name_en)
                .name_ar(row.name_ar)
                .subject_type(LEGAL_ENTITY_SUBJECT)
                .numbered()
                .reminders(row.reminders)
                .without_review();
            if row.item && has_items {
                seed = seed.subject_type(ITEM_SUBJECT);
            }
            if row.supplier {
                seed = seed.subject_type(PARTY_SUBJECT_TYPE);
            }
            if row.requires_expiry {
                seed = seed.expires();
            }
            let role = settings.roles.get(row.key).map_or_else(
                || match row.responsible {
                    Responsible::Owner => owner,
                    Responsible::QualityOrOwner => settings.quality_role.as_deref().unwrap_or(owner),
                },
                String::as_str,
            );
            seed = seed.responsible_role(role);
            let on = match &settings.enabled {
                Some(keys) => keys.iter().any(|k| k == row.key),
                None => row.enabled_in.is_none_or(|apps| apps.contains(&app)),
            };
            if on { seed } else { seed.off() }
        })
        .collect()
}

/// Stores the five kinds for the client of `config` with `settings`; a kind
/// the database already has is kept as the client left it. Returns how many
/// were created.
pub async fn install_for(
    db: &Db,
    config: &ClientConfig,
    settings: &VerticalSettings,
) -> Result<usize> {
    let seeds = seeds(&config.client.app, config.owner_role(), settings);
    kinds::install_seeds(db, seeds.iter()).await
}

/// [`install_for`] the app's [`ClientConfig`] with the settings registered
/// for its vertical. Without a config nothing is stored: the owner role and
/// the vertical are not known.
pub async fn install(state: &AppState) -> Result<usize> {
    let Some(config) = state.provided::<Arc<ClientConfig>>() else {
        return Ok(0);
    };
    let settings = settings_of(&config.client.app);
    install_for(&state.db, &config, &settings).await
}

/// Stores the kinds the modules and verticals registered
/// ([`kinds::install`]) and then these ([`install`]); returns how many kinds
/// were created in all.
pub async fn install_all(state: &AppState) -> Result<usize> {
    let registered = kinds::install(&state.db).await?;
    Ok(registered + install(state).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed<'a>(seeds: &'a [KindSeed], key: &str) -> &'a KindSeed {
        seeds.iter().find(|s| s.key == key).expect("a seed")
    }

    fn allows(seed: &KindSeed, subject_type: &str) -> bool {
        seed.subject_types.iter().any(|t| t == subject_type)
    }

    #[test]
    fn no_seed_has_blocks_a_legal_block_or_a_portal_upload() {
        for app in ["fnb", "manufacturing", "property", "unknown"] {
            for seed in seeds(app, "admin", &VerticalSettings::new()) {
                assert!(seed.blocks.is_empty(), "{}", seed.key);
                assert!(!seed.legal_block && !seed.compliance_critical);
                assert!(!seed.portal_upload);
                assert_eq!(seed.sensitivity, crate::Sensitivity::Normal);
                assert!(seed.name_ar.len() > 2);
            }
        }
    }

    #[test]
    fn a_new_fnb_client_has_three_kinds_on_and_iso_off() {
        let seeds = seeds("fnb", "owner", &VerticalSettings::new());
        let on: Vec<_> = seeds.iter().filter(|s| s.enabled).map(|s| s.key.as_str()).collect();
        assert_eq!(on, [TRADEMARK, HALAL, OTHER]);
        assert!(!seed(&seeds, ISO).enabled);
        assert!(!seed(&seeds, PRODUCT).enabled);
    }

    #[test]
    fn reminders_and_roles_follow_the_table() {
        let seeds = seeds("manufacturing", "md", &VerticalSettings::new().quality_role("qc_inspector"));
        assert_eq!(seed(&seeds, TRADEMARK).reminder_offsets, [180, 90, 30]);
        assert_eq!(seed(&seeds, TRADEMARK).responsible_role.as_deref(), Some("md"));
        assert_eq!(seed(&seeds, ISO).reminder_offsets, [90, 30, 7]);
        assert_eq!(seed(&seeds, ISO).responsible_role.as_deref(), Some("qc_inspector"));
        assert_eq!(seed(&seeds, OTHER).responsible_role.as_deref(), Some("md"));
        assert!(seed(&seeds, ISO).enabled && seed(&seeds, HALAL).enabled);
    }

    #[test]
    fn quality_role_else_owner() {
        let seeds = seeds("fnb", "owner", &VerticalSettings::new());
        assert_eq!(seed(&seeds, HALAL).responsible_role.as_deref(), Some("owner"));
        let seeds = super::seeds("fnb", "owner", &VerticalSettings::new().role(HALAL, "storekeeper"));
        assert_eq!(seed(&seeds, HALAL).responsible_role.as_deref(), Some("storekeeper"));
    }

    #[test]
    fn items_are_offered_only_where_there_is_a_catalogue() {
        let fnb = seeds("fnb", "owner", &VerticalSettings::new());
        assert!(allows(seed(&fnb, HALAL), ITEM_SUBJECT));
        assert!(allows(seed(&fnb, HALAL), PARTY_SUBJECT_TYPE));
        assert!(!allows(seed(&fnb, TRADEMARK), ITEM_SUBJECT));
        let property = seeds("property", "owner", &VerticalSettings::new());
        assert_eq!(seed(&property, HALAL).subject_types, [LEGAL_ENTITY_SUBJECT, PARTY_SUBJECT_TYPE]);
        assert_eq!(seed(&property, PRODUCT).subject_types, [LEGAL_ENTITY_SUBJECT]);
    }

    #[test]
    fn a_vertical_can_turn_other_kinds_on() {
        let seeds = seeds("manufacturing", "owner", &VerticalSettings::new().enabled(&[PRODUCT, OTHER]));
        assert!(seed(&seeds, PRODUCT).enabled && !seed(&seeds, HALAL).enabled);
    }

    #[test]
    fn other_certification_does_not_require_an_expiry_date() {
        let seeds = seeds("property", "owner", &VerticalSettings::new());
        assert!(!seed(&seeds, OTHER).has_expiry);
        assert!(seed(&seeds, TRADEMARK).has_expiry);
    }
}
