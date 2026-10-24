//! Shared by the host tests of the Accounting demo firm: the demo client
//! booted and seeded as `accounting:demo` does.

#![allow(dead_code)]

use renox::testing::TestApp;
use wl_kernel::ClientConfig;

use crate::support::noon_clock;

/// The demo client of the Accounting vertical.
const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-accounting/client.toml"
);

/// The demo firm, seeded.
pub async fn demo() -> TestApp {
    let config = ClientConfig::load(CLIENT).expect("the demo client of the Accounting vertical");
    let app = TestApp::new(
        host::app_for(config)
            .expect("the app")
            .provide(noon_clock()),
    )
    .await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    host::accounting_demo::seed(app.kernel())
        .await
        .expect("the demo firm is seeded");
    app
}
