//! A client admin changing a kind's `blocks` (wl-documents §5.1, A7, A8):
//! the acceptance check AC7 of #45, through the service and the kind page.

mod support;

use renox::prelude::*;
use wl_documents::{
    DocumentKind, KindBlock, RefusalReason, add_block, blocks_for, remove_block, set_block_mode,
};
use wl_kernel::documents::{BlockMode, OverrideMode};

use support::{boot, civil_defence, driving_licence, work_permit};

const ACTIVATE: &str = "deployment.activate";

/// The `(guarded, mode, locked)` of a kind's entries.
async fn entries(db: &Db, kind: &DocumentKind) -> Vec<(String, BlockMode, bool)> {
    kind.blocks(db)
        .await
        .unwrap()
        .into_iter()
        .map(|b: KindBlock| (b.guarded, b.mode, b.locked))
        .collect()
}

/// AC7 through the service: a locked entry can be neither removed nor
/// switched to `flag`; a new entry can be added; the seed keeps it.
#[renox::test]
async fn a_client_cannot_loosen_a_legal_block_but_can_add_entries() {
    let t = boot().await;
    t.install(&[work_permit()]).await;
    let db = t.db();
    let kind = t.kind("work_permit").await;
    assert!(kind.legal_block);
    assert_eq!(kind.override_mode, OverrideMode::Never);
    let before = entries(db, &kind).await;
    assert_eq!(before, [(ACTIVATE.to_owned(), BlockMode::Block, true)]);

    let refused = remove_block(db, "work_permit", ACTIVATE)
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(refused.reason, RefusalReason::Locked);
    assert_eq!(
        refused.to_string(),
        "Work permit is a legal block: deployment.activate cannot be removed or changed"
    );
    let refused = set_block_mode(db, "work_permit", ACTIVATE, BlockMode::Flag)
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(refused.reason, RefusalReason::Locked);
    assert_eq!(entries(db, &kind).await, before);
    assert_eq!(
        blocks_for(db, ACTIVATE).await.unwrap()[0].mode,
        BlockMode::Block
    );

    // Adding an entry works once a module guards the transition.
    let added = add_block(db, "work_permit", "run.dispatch", BlockMode::Block, None)
        .await
        .unwrap()
        .unwrap();
    assert!(added.locked, "an entry of a legal-block kind is locked");
    let refused = add_block(db, "work_permit", "nobody.guards", BlockMode::Block, None)
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(refused.reason, RefusalReason::NoProvider);

    // Re-running the seed keeps what the client added.
    assert_eq!(
        wl_documents::kinds::install_seeds(db, [&work_permit()])
            .await
            .unwrap(),
        0
    );
    let kept = entries(db, &kind).await;
    assert_eq!(
        kept,
        [
            (ACTIVATE.to_owned(), BlockMode::Block, true),
            ("run.dispatch".to_owned(), BlockMode::Block, true),
        ]
    );
}

/// A kind that is not a legal block can be edited freely, and the seed
/// does not undo it.
#[renox::test]
async fn the_entries_of_an_ordinary_kind_can_be_changed() {
    let t = boot().await;
    t.install(&[driving_licence()]).await;
    let db = t.db();

    add_block(db, "driving_licence", ACTIVATE, BlockMode::Flag, None)
        .await
        .unwrap()
        .unwrap();
    let switched = set_block_mode(db, "driving_licence", ACTIVATE, BlockMode::Block)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(switched.mode, BlockMode::Block);
    remove_block(db, "driving_licence", "run.dispatch")
        .await
        .unwrap()
        .unwrap();
    t.install(&[driving_licence()]).await;
    let kind = t.kind("driving_licence").await;
    assert_eq!(
        entries(db, &kind).await,
        [(ACTIVATE.to_owned(), BlockMode::Block, false)]
    );
}

/// AC7 through the kind page: the refusal is a 422 with the message and
/// the entry is unchanged; adding an entry works.
#[renox::test]
async fn the_kind_page_refuses_a_locked_entry_and_adds_a_new_one() {
    let t = boot().await;
    t.install(&[work_permit()]).await;
    let admin = t.kind_admin().await;
    t.app.acting_as(&admin);
    let message = "Work permit is a legal block: deployment.activate cannot be removed or changed";

    t.app
        .get("/document-kinds/work_permit")
        .await
        .assert_ok()
        .assert_see(ACTIVATE)
        .assert_see(message);

    t.app
        .htmx()
        .delete(&format!("/document-kinds/work_permit/blocks/{ACTIVATE}"))
        .await
        .assert_status(422)
        .assert_see(message);
    t.app
        .htmx()
        .post(
            &format!("/document-kinds/work_permit/blocks/{ACTIVATE}/mode"),
            &[("mode", "flag")],
        )
        .await
        .assert_status(422)
        .assert_see(message);
    let kind = t.kind("work_permit").await;
    assert_eq!(
        entries(t.db(), &kind).await,
        [(ACTIVATE.to_owned(), BlockMode::Block, true)]
    );

    t.app
        .htmx()
        .post(
            "/document-kinds/work_permit/blocks",
            &[("guarded", "run.dispatch"), ("mode", "block")],
        )
        .await
        .assert_ok();
    assert_eq!(entries(t.db(), &kind).await.len(), 2);
    // A transition no module guards is refused with its message.
    t.app
        .htmx()
        .post(
            "/document-kinds/work_permit/blocks",
            &[("guarded", "nobody.guards"), ("mode", "block")],
        )
        .await
        .assert_status(422);
    assert_eq!(entries(t.db(), &kind).await.len(), 2);

    // Without document_kinds.manage the page is closed.
    let reviewer = t.reviewer().await;
    t.app.acting_as(&reviewer);
    t.app
        .get("/document-kinds/work_permit")
        .await
        .assert_forbidden();
}

/// The override mode through the kind page: an ordinary kind shows the
/// form and switches; a legal block shows no form and refuses with the
/// reason, also when the client posts it anyway.
#[renox::test]
async fn the_kind_page_sets_the_override_mode_except_for_a_legal_block() {
    let t = boot().await;
    t.install(&[driving_licence(), work_permit()]).await;
    let admin = t.kind_admin().await;
    t.app.acting_as(&admin);
    let locked = "Work permit is a legal block: its override mode is locked to never";

    assert_eq!(
        t.kind("driving_licence").await.override_mode,
        OverrideMode::WithApproval
    );
    t.app
        .get("/document-kinds/driving_licence")
        .await
        .assert_ok()
        .assert_see("/document-kinds/driving_licence/override-mode");
    t.app
        .htmx()
        .post(
            "/document-kinds/driving_licence/override-mode",
            &[("mode", "never")],
        )
        .await
        .assert_ok();
    assert_eq!(
        t.kind("driving_licence").await.override_mode,
        OverrideMode::Never
    );

    t.app
        .get("/document-kinds/work_permit")
        .await
        .assert_ok()
        .assert_see(locked)
        .assert_dont_see("/document-kinds/work_permit/override-mode");
    t.app
        .htmx()
        .post(
            "/document-kinds/work_permit/override-mode",
            &[("mode", "with_approval")],
        )
        .await
        .assert_status(422)
        .assert_see(locked);
    assert_eq!(
        t.kind("work_permit").await.override_mode,
        OverrideMode::Never
    );

    let reviewer = t.reviewer().await;
    t.app.acting_as(&reviewer);
    t.app
        .htmx()
        .post(
            "/document-kinds/driving_licence/override-mode",
            &[("mode", "with_approval")],
        )
        .await
        .assert_forbidden();
}

/// A condition on an entry is stored and read back with its values, and a
/// seeded legal-block entry is put back when a client tampered with it.
#[renox::test]
async fn a_seeded_condition_survives_and_a_tampered_entry_is_locked_again() {
    let t = boot().await;
    t.install(&[civil_defence()]).await;
    let db = t.db();
    let entries_now = blocks_for(db, "ticket.resolve").await.unwrap();
    assert_eq!(entries_now.len(), 1);
    let condition = entries_now[0].condition.clone().expect("a condition");
    assert_eq!(condition.field, "category");
    assert_eq!(condition.values, ["fire_alarm", "sprinkler"]);

    KindBlock::query()
        .where_(KindBlock::GUARDED.eq("ticket.resolve"))
        .update(db, &[("mode", &"flag"), ("locked", &false)])
        .await
        .unwrap();
    t.install(&[civil_defence()]).await;
    let entry = &blocks_for(db, "ticket.resolve").await.unwrap()[0];
    assert_eq!(entry.mode, BlockMode::Block);
    assert!(entry.locked);
    assert_eq!(entry.condition.as_ref(), Some(&condition));
}
