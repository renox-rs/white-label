//! Shared by the host tests of the Auto demo company: the demo client
//! booted and seeded as `auto:demo` does.

#![allow(dead_code)]

use renox::testing::TestApp;
use wl_kernel::ClientConfig;

use crate::support::noon_clock;

/// The demo client of the Auto vertical.
const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-auto/client.toml"
);

/// The demo company, seeded.
pub async fn demo() -> TestApp {
    let config = ClientConfig::load(CLIENT).expect("the demo client of the Auto vertical");
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
    host::auto_demo::seed(app.kernel())
        .await
        .expect("the demo company is seeded");
    app
}
