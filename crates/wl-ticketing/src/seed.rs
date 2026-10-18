//! The idempotent seeder with the §7 defaults (#146 A4, A6): the SLA policy
//! `standard`, the hold reasons Parts, Access, Waiting on requester and
//! Approval, and the categories `general` and `task`
//! (the kernel's [`TASK_CATEGORY`], for staff tasks other modules open). With
//! `invoicing` enabled, [`install_with`] adds `billing_dispute` too (#67 A6).
//!
//! [`install`] adds the rows the database lacks and keeps every row that is
//! there, so a client's edits survive a re-run. The targets of a policy are
//! added only with the policy: a target the client removed stays removed.

use renox::prelude::*;

use crate::config::{Category, HoldReason, SlaPolicy, SlaTarget};
use crate::priority::Priority;

/// The key of the default SLA policy.
pub const STANDARD_POLICY: &str = "standard";

/// The key of the default category.
pub const GENERAL_CATEGORY: &str = "general";

/// The key of the category of staff tasks other modules open.
pub const TASK_CATEGORY: &str = wl_kernel::tickets::TASK_CATEGORY;

/// The seeded categories as (key, English name).
pub const CATEGORIES: [(&str, &str); 2] = [(GENERAL_CATEGORY, "General"), (TASK_CATEGORY, "Task")];

/// The key of the category of invoice disputes, seeded when invoicing is on.
pub const BILLING_DISPUTE_CATEGORY: &str = "billing_dispute";

/// The categories seeded when `invoicing` is enabled, as (key, English name).
pub const INVOICING_CATEGORIES: [(&str, &str); 1] = [(BILLING_DISPUTE_CATEGORY, "Billing dispute")];

/// One SLA working hour, in minutes.
const HOUR: i64 = 60;

/// One SLA day: nine working hours.
const DAY: i64 = 9 * HOUR;

/// The §7 targets of `standard` as (priority, response, resolution), in
/// working minutes: Urgent 2 h / 24 h, High 8 h / 27 h, Normal 9 h / 63 h,
/// Low 27 h / 126 h.
pub const STANDARD_TARGETS: [(Priority, i64, i64); 4] = [
    (Priority::Urgent, 2 * HOUR, 24 * HOUR),
    (Priority::High, 8 * HOUR, 3 * DAY),
    (Priority::Normal, DAY, 7 * DAY),
    (Priority::Low, 3 * DAY, 14 * DAY),
];

/// The seeded hold reasons as (key, name, pauses the SLA).
pub const HOLD_REASONS: [(&str, &str, bool); 4] = [
    ("parts", "Parts", true),
    ("access", "Access", true),
    ("waiting_on_requester", "Waiting on requester", true),
    ("approval", "Approval", false),
];

/// What [`install`] created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Installed {
    /// SLA policies created (with their targets).
    pub policies: usize,
    /// Hold reasons created.
    pub hold_reasons: usize,
    /// Categories created.
    pub categories: usize,
}

impl Installed {
    /// The number of rows of every kind created.
    pub fn total(&self) -> usize {
        self.policies + self.hold_reasons + self.categories
    }
}

/// Stores the defaults the database lacks, in one transaction, for an app
/// without invoicing.
pub async fn install(db: &Db) -> Result<Installed> {
    install_with(db, false).await
}

/// Like [`install`]; with `invoicing` the categories of invoicing are added
/// too. A category stays when invoicing is turned off later.
pub async fn install_with(db: &Db, invoicing: bool) -> Result<Installed> {
    let mut tx = db.begin().await?;
    let mut installed = Installed::default();

    let policy = match SlaPolicy::by_key(&mut tx, STANDARD_POLICY).await? {
        Some(policy) => policy,
        None => {
            let policy = SlaPolicy::create(
                &mut tx,
                SlaPolicy {
                    key: STANDARD_POLICY.to_owned(),
                    name_en: "Standard".to_owned(),
                    ..SlaPolicy::default()
                },
            )
            .await?;
            for (priority, response, resolution) in STANDARD_TARGETS {
                SlaTarget::create(
                    &mut tx,
                    SlaTarget {
                        policy_id: policy.id,
                        priority,
                        response_minutes: response,
                        resolution_minutes: resolution,
                        ..SlaTarget::default()
                    },
                )
                .await?;
            }
            installed.policies += 1;
            policy
        }
    };

    for (key, name, pauses_sla) in HOLD_REASONS {
        if HoldReason::by_key(&mut tx, key).await?.is_none() {
            HoldReason::create(
                &mut tx,
                HoldReason {
                    key: key.to_owned(),
                    name_en: name.to_owned(),
                    pauses_sla,
                    ..HoldReason::default()
                },
            )
            .await?;
            installed.hold_reasons += 1;
        }
    }

    let invoicing_categories: &[(&str, &str)] = if invoicing {
        &INVOICING_CATEGORIES
    } else {
        &[]
    };
    for &(key, name) in CATEGORIES.iter().chain(invoicing_categories) {
        if Category::by_key(&mut tx, key).await?.is_none() {
            Category::create(
                &mut tx,
                Category {
                    key: key.to_owned(),
                    name_en: name.to_owned(),
                    default_priority: Priority::Normal,
                    sla_policy_id: Some(policy.id),
                    requires_signoff: false,
                    active: true,
                    ..Category::default()
                },
            )
            .await?;
            installed.categories += 1;
        }
    }

    tx.commit().await?;
    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_standard_targets_are_the_section_7_matrix_in_working_hours() {
        let hours: Vec<_> = STANDARD_TARGETS
            .iter()
            .map(|(p, response, resolution)| (*p, response / 60, resolution / 60))
            .collect();
        assert_eq!(
            hours,
            [
                (Priority::Urgent, 2, 24),
                (Priority::High, 8, 27),
                (Priority::Normal, 9, 63),
                (Priority::Low, 27, 126),
            ]
        );
    }

    #[test]
    fn only_approval_does_not_pause_the_sla() {
        let keys: Vec<_> = HOLD_REASONS.iter().map(|(k, _, _)| *k).collect();
        assert_eq!(
            keys,
            ["parts", "access", "waiting_on_requester", "approval"]
        );
        for (key, _, pauses) in HOLD_REASONS {
            assert_eq!(pauses, key != "approval", "{key}");
        }
    }

    #[renox::test]
    async fn the_billing_dispute_category_is_seeded_only_with_invoicing_and_keeps_its_label() {
        use renox::testing::TestApp;

        let app = TestApp::new(App::new().module(crate::WlTicketing::new())).await;
        let db = app.db();
        install(db).await.unwrap();
        assert!(
            Category::by_key(db, BILLING_DISPUTE_CATEGORY)
                .await
                .unwrap()
                .is_none()
        );

        let added = install_with(db, true).await.unwrap();
        assert_eq!(added.categories, 1);
        let mut dispute = Category::by_key(db, BILLING_DISPUTE_CATEGORY)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(dispute.name_en, "Billing dispute");
        assert_eq!(dispute.default_priority, Priority::Normal);
        assert!(dispute.active && !dispute.requires_signoff && !dispute.chargeable);
        let policy = SlaPolicy::by_key(db, STANDARD_POLICY)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(dispute.sla_policy_id, Some(policy.id));

        dispute.name_en = "Invoice query".to_owned();
        dispute.save(db).await.unwrap();
        assert_eq!(install_with(db, true).await.unwrap(), Installed::default());
        let renamed = Category::by_key(db, BILLING_DISPUTE_CATEGORY)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(renamed.name_en, "Invoice query");
        assert_eq!(Category::query().count(db).await.unwrap(), 3);
    }

    #[renox::test]
    async fn seeding_twice_leaves_one_of_each_and_keeps_client_edits() {
        use renox::testing::TestApp;

        let app = TestApp::new(App::new().module(crate::WlTicketing::new())).await;
        let db = app.db();
        let first = install(db).await.unwrap();
        assert_eq!(
            first,
            Installed {
                policies: 1,
                hold_reasons: 4,
                categories: 2
            }
        );

        // The client edits a target, a hold reason and the category.
        let policy = SlaPolicy::by_key(db, STANDARD_POLICY)
            .await
            .unwrap()
            .unwrap();
        let mut urgent = policy.target(db, Priority::Urgent).await.unwrap().unwrap();
        urgent.response_minutes = 30;
        urgent.save(db).await.unwrap();
        let mut approval = HoldReason::by_key(db, "approval").await.unwrap().unwrap();
        approval.pauses_sla = true;
        approval.save(db).await.unwrap();
        let mut general = Category::by_key(db, GENERAL_CATEGORY)
            .await
            .unwrap()
            .unwrap();
        general.default_priority = Priority::High;
        general.save(db).await.unwrap();

        assert_eq!(install(db).await.unwrap(), Installed::default());
        assert_eq!(SlaPolicy::query().count(db).await.unwrap(), 1);
        assert_eq!(policy.targets(db).await.unwrap().len(), 4);
        assert_eq!(HoldReason::ordered(db).await.unwrap().len(), 4);
        assert_eq!(Category::query().count(db).await.unwrap(), 2);
        let task = Category::by_key(db, TASK_CATEGORY).await.unwrap().unwrap();
        assert_eq!(task.default_priority, Priority::Normal);
        assert!(task.active && !task.requires_signoff);
        let urgent = policy.target(db, Priority::Urgent).await.unwrap().unwrap();
        assert_eq!(urgent.response_minutes, 30);
        let approval = HoldReason::by_key(db, "approval").await.unwrap().unwrap();
        assert!(approval.pauses_sla);
        let general = Category::by_key(db, GENERAL_CATEGORY)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(general.default_priority, Priority::High);
        assert_eq!(general.sla_policy_id, Some(policy.id));
    }
}
