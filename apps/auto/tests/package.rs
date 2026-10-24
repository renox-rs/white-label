//! The package checks of 08 Auto: the models match the migrations and the
//! views compile.

mod support;

use support::boot;

#[renox::test]
async fn the_models_match_the_schema_and_the_views_compile() {
    let t = boot().await;
    t.app.assert_models_match_schema().await;
    t.app.assert_views_compile();
}
