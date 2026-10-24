//! Shared by the host tests of the Manufacturing demo company: the demo client
//! booted and seeded as `manufacturing:demo` does.

#![allow(dead_code)]

use renox::testing::TestApp;
use wl_kernel::ClientConfig;

use crate::support::noon_clock;

/// The demo client of the Manufacturing vertical.
const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-manufacturing/client.toml"
);

/// The demo company, seeded.
pub async fn demo() -> TestApp {
    let config = ClientConfig::load(CLIENT).expect("the demo client of the Manufacturing vertical");
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
    host::manufacturing_demo::seed(app.kernel())
        .await
        .expect("the demo company is seeded");
    app
}
