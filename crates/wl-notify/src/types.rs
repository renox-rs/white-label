//! Notification types (wl-notify §3 *Notification type*): the kinds of
//! message modules send, as the client has them.
//!
//! Modules declare their types with `wl_kernel::notify::register_type` in
//! `register`. The seeder and `notify:install` store the registered types
//! the database lacks ([`install`]); a type already there (maybe changed by
//! the client: statutory flag, give-up reaction, staff role told) is kept as
//! it is, like the kernel's legal entities and approval policies.

use renox::db::Json;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::notify::{
    self, Category, GiveUpReaction, NotificationTypeDef, NotifyChannel, ProofMethod, TemplateDef,
    Urgency,
};

/// A notification type as stored for this client.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_notify_types", unique(key))]
pub struct NotificationType {
    /// The primary key.
    pub id: i64,
    /// The key, e.g. `invoice.reminder`.
    pub key: String,
    /// The module that sends it, e.g. `invoicing`.
    pub module: String,
    /// The row of the wl-notify §5.4 default table, e.g. `invoices`.
    pub group_key: String,
    /// What it is for.
    pub category: Category,
    /// How urgent it is.
    pub urgency: Urgency,
    /// Whether it is a notice with legal or contractual effect.
    pub statutory: bool,
    /// What happens on give-up, as configured (run order:
    /// `wl_kernel::notify::reaction_order`).
    pub give_up: Json<Vec<GiveUpReaction>>,
    /// The staff role told on give-up, e.g. `collector`.
    pub staff_role: Option<String>,
    /// The channels to try, in order.
    pub channels: Json<Vec<NotifyChannel>>,
    /// The default texts per channel and language.
    pub templates: Json<Vec<TemplateDef>>,
    /// The proof methods a statutory notice of this type accepts.
    pub proof_methods: Json<Vec<ProofMethod>>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl NotificationType {
    /// The row [`install`] stores for a registered type.
    pub fn from_def(def: &NotificationTypeDef) -> Self {
        NotificationType {
            key: def.key.clone(),
            module: def.module.clone(),
            group_key: def.group.clone(),
            category: def.category,
            urgency: def.urgency,
            statutory: def.statutory,
            give_up: Json(def.give_up.clone()),
            staff_role: def.staff_role.clone(),
            channels: Json(def.channels.clone()),
            templates: Json(def.templates.clone()),
            proof_methods: Json(def.proof_methods.clone()),
            ..Default::default()
        }
    }

    /// The type as the client has it, in the kernel's shape.
    pub fn to_def(&self) -> NotificationTypeDef {
        NotificationTypeDef {
            key: self.key.clone(),
            module: self.module.clone(),
            group: self.group_key.clone(),
            category: self.category,
            urgency: self.urgency,
            statutory: self.statutory,
            give_up: self.give_up.0.clone(),
            staff_role: self.staff_role.clone(),
            channels: self.channels.0.clone(),
            templates: self.templates.0.clone(),
            proof_methods: self.proof_methods.0.clone(),
        }
    }

    /// The give-up reactions in the order they run: `mark_contact_invalid`
    /// first, `none` left out.
    pub fn reactions(&self) -> Vec<GiveUpReaction> {
        notify::reaction_order(&self.give_up.0)
    }

    /// Whether the client never changed the row: `updated_at` is unset or
    /// still the creation time (creating a row stamps both). The page that
    /// edits a type saves the row, which moves `updated_at` on.
    pub fn is_unedited(&self) -> bool {
        match (self.updated_at, self.created_at) {
            (None, _) => true,
            (Some(updated), Some(created)) => updated <= created,
            (Some(_), None) => false,
        }
    }

    /// The stored type `key`, if any.
    pub async fn by_key(db: &Db, key: &str) -> Result<Option<NotificationType>> {
        NotificationType::query()
            .where_(NotificationType::KEY.eq(key))
            .first(db)
            .await
    }
}

/// Stores the registered notification types the database lacks (by key).
/// Types already there are left as they are, so the client's changes are
/// kept on later runs; the one exception is a row with an empty `give_up`
/// that was never edited ([`NotificationType::is_unedited`]), stored before
/// the §5.4 defaults existed: it gets the registered type's reactions (the
/// default when the module gave none) and, when it has no role, its staff
/// role. That write leaves `updated_at` alone, so a second run changes
/// nothing. Returns how many were created.
pub async fn install(db: &Db) -> Result<usize> {
    let mut tx = db.begin().await?;
    let mut created = 0;
    for def in notify::notification_types() {
        let known = NotificationType::query()
            .where_(NotificationType::KEY.eq(def.key.as_str()))
            .first(&mut tx)
            .await?;
        if let Some(mut row) = known {
            if row.give_up.0.is_empty() && row.is_unedited() {
                let default = notify::default_give_up(&def);
                let (reactions, role) = if def.give_up.is_empty() {
                    (default.reactions, default.staff_role)
                } else {
                    (def.give_up.clone(), def.staff_role.clone())
                };
                row.give_up = Json(reactions);
                if row.staff_role.is_none() {
                    row.staff_role = role;
                }
                renox::db::sql(format!(
                    "UPDATE {} SET give_up = ?, staff_role = ? WHERE id = ?",
                    NotificationType::TABLE
                ))
                .bind(row.give_up.clone())
                .bind(row.staff_role.clone())
                .bind(row.id)
                .execute(&mut tx)
                .await?;
            }
            continue;
        }
        NotificationType::create(&mut tx, NotificationType::from_def(&def)).await?;
        created += 1;
    }
    tx.commit().await?;
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;
    use GiveUpReaction::{AlternativeRecipient, MarkContactInvalid, StaffTask};

    fn reminder() -> NotificationTypeDef {
        NotificationTypeDef::new("invoice.reminder", "invoicing")
            .group("invoices")
            .category(Category::Reminder)
            .channels(&[NotifyChannel::Email, NotifyChannel::Sms])
            .give_up(&[AlternativeRecipient, MarkContactInvalid, StaffTask])
            .staff_role("collector")
            .template(
                NotifyChannel::Email,
                "en",
                Some("Invoice {{ number }} is overdue"),
                "Please pay invoice {{ number }}.",
            )
    }

    #[test]
    fn a_registered_type_is_stored_as_declared() {
        let def = reminder();
        let row = NotificationType::from_def(&def);
        assert_eq!(row.key, "invoice.reminder");
        assert_eq!(row.group_key, "invoices");
        assert_eq!(row.category, Category::Reminder);
        assert!(!row.statutory);
        assert_eq!(row.staff_role.as_deref(), Some("collector"));
        assert_eq!(row.channels.0, [NotifyChannel::Email, NotifyChannel::Sms]);
        assert_eq!(row.to_def(), def);
    }

    #[test]
    fn the_proof_methods_are_stored_and_read_back() {
        let def = reminder().proof_methods(&[ProofMethod::Courier, ProofMethod::Call]);
        let row = NotificationType::from_def(&def);
        assert_eq!(
            row.proof_methods.0,
            [ProofMethod::Courier, ProofMethod::Call]
        );
        assert_eq!(row.to_def(), def);
        assert_eq!(
            NotificationType::from_def(&reminder()).proof_methods.0,
            ProofMethod::ALL.to_vec()
        );
    }

    #[test]
    fn a_row_is_unedited_until_it_has_an_update_time() {
        let mut row = NotificationType::from_def(&reminder());
        assert!(row.is_unedited());
        let created = renox::db::now();
        row.created_at = Some(created);
        row.updated_at = Some(created);
        assert!(row.is_unedited());
        row.updated_at = Some(renox::db::now());
        assert!(!row.is_unedited());
    }

    #[test]
    fn the_stored_reactions_run_mark_contact_invalid_first() {
        let row = NotificationType::from_def(&reminder());
        assert_eq!(
            row.reactions(),
            [MarkContactInvalid, AlternativeRecipient, StaffTask]
        );
        let mut changed = row.clone();
        changed.give_up = Json(vec![GiveUpReaction::None]);
        assert!(changed.reactions().is_empty());
    }
}
