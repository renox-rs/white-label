//! The package checks of 04 Accounting: the models match the migrations,
//! the views compile, and the pages answer.

mod support;

use support::boot;

#[renox::test]
async fn the_models_match_the_schema_and_the_views_compile() {
    let t = boot().await;
    t.app.assert_models_match_schema().await;
    t.app.assert_views_compile();
}

#[renox::test]
async fn the_pages_answer_for_the_roles_that_may_see_them() {
    let t = boot().await;
    let mlro = t.mlro().await;
    let client = t.client().await;
    let file = t.cdd_prepared(&client).await;
    let engagement = t.engagement(&client).await;
    let manager = t.manager().await;
    t.get(&manager, "/accounting/engagements")
        .await
        .assert_ok()
        .assert_see(&engagement.number());
    t.get(
        &manager,
        &format!("/accounting/engagements/{}", engagement.id),
    )
    .await
    .assert_ok()
    .assert_see("Gulf Trading LLC")
    .assert_see("Onboarding");
    t.get(&mlro, &format!("/accounting/cdd/{}", file.id))
        .await
        .assert_ok()
        .assert_see("Omar Haddad");
    // Someone without the permissions sees nothing.
    let nobody = t.user_with_grants("Nora", &[]).await;
    t.get(&nobody, "/accounting/engagements")
        .await
        .assert_forbidden();
    t.get(&nobody, &format!("/accounting/cdd/{}", file.id))
        .await
        .assert_forbidden();
}
