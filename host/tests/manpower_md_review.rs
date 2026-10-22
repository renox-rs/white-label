//! BL-MPW-15 (#429), 09 §4.1, wl-parties §5.6: with the block effect of a
//! deployment set to `review`, the decider is the role `md` and the host of a
//! manpower agency defines that role, so the MD user gets the review notice.
//! (Before, `md` did not exist and `users_with_role_in` found nobody.)
//!
//! This file has one test: the block-effect registry is process-wide.

mod manpower_support;
mod support;

use host::roles::COMPLIANCE;
use manpower_support::boot_seeded;
use renox::prelude::*;
use support::{company, person};
use wl_kernel::block_effects::{
    AgreementFuture, AgreementKind, AgreementRef, AgreementSource, BlockEffect,
    BlockEffectOverride, Decision,
};
use wl_kernel::{PartyId, Subject};

/// The deployments of the blocked client: one, with no scope.
struct Deployments;

impl AgreementSource for Deployments {
    fn open_for<'a>(
        &'a self,
        _state: &'a AppState,
        _party: PartyId,
    ) -> AgreementFuture<'a, Vec<AgreementRef>> {
        Box::pin(async {
            Ok(vec![AgreementRef {
                subject: Subject::new("manpower.deployment", 1),
                label: "Deployment 1".into(),
                scope: None,
            }])
        })
    }

    fn decide<'a>(
        &'a self,
        _state: &'a AppState,
        _user: i64,
        _subject: &'a Subject,
        _decision: Decision,
    ) -> AgreementFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

/// With the effect of `deployment` set to Review and the decider `md` (the role
/// the agency's seed names), blocking the client tells the user who holds `md`,
/// and not a user without the role.
#[renox::test]
async fn the_md_user_gets_the_block_effect_review_notice_when_the_effect_is_review() {
    let app = boot_seeded().await;
    AgreementKind::new("deployment")
        .owner("manpower")
        .source(Deployments)
        .register();
    BlockEffectOverride::kind("deployment")
        .effect(BlockEffect::Review)
        .decider(manpower::permissions::MD_ROLE)
        .register();
    let mona = person(&app, "Mona", "md").await;
    let clerk = person(&app, "Clark", "member").await;
    let officer = person(&app, "Comply", COMPLIANCE).await;
    let client = company(&app, "Gulf Tower LLC").await;

    app.acting_as(&officer);
    app.htmx()
        .post(
            &format!("/parties/{}/block", client.0),
            &[
                ("reason_code", "fraud_suspected"),
                ("note", "Forged cheques"),
            ],
        )
        .await
        .assert_ok();

    let bells = mona.unread_notifications(app.db()).await.unwrap();
    assert_eq!(bells.len(), 1, "the MD is told of the review");
    assert!(
        clerk
            .unread_notifications(app.db())
            .await
            .unwrap()
            .is_empty()
    );
}
