//! The pages a person opens from a link in a message (a payment link, an
//! unsubscribe link) render through the host's `layouts/public.html`: an
//! unknown or odd token answers the same plain page and never a 500.

use renox::testing::TestApp;
use wl_kernel::ClientConfig;

const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-properties/client.toml"
);

#[renox::test]
async fn public_link_pages_render_in_the_host_layout() {
    let config = ClientConfig::load(CLIENT).expect("the demo client");
    let app = TestApp::new(host::app_for(config).expect("the app")).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();

    let long = "a".repeat(200);
    for path in [
        "/pay/not-a-token".to_owned(),
        "/pay/x".to_owned(),
        format!("/pay/{long}"),
        "/unsubscribe/not-a-token".to_owned(),
    ] {
        let page = app.get(&path).await;
        page.assert_ok();
        assert!(
            page.text().contains("<main"),
            "{path} is drawn in the public layout"
        );
    }
}
