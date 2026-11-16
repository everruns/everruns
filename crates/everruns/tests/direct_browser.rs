#![cfg(all(feature = "direct-browser", not(feature = "browserless")))]
use everruns::{
    Agent, Engine, Model,
    browser::{BrowserNetworkMode, DirectBrowserCapability, DirectBrowserConfig},
};

#[tokio::test]
async fn direct_browser_registers_with_framework_engine() {
    let browser = DirectBrowserCapability::new(
        DirectBrowserConfig::new(
            "ws://127.0.0.1:9222/devtools/browser/test",
            BrowserNetworkMode::BrowserOwned,
        )
        .unwrap(),
    );
    let engine = Engine::new();
    engine.register_capability(browser.clone()).unwrap();
    let agent = Agent::builder()
        .instructions("Test")
        .model(Model::simulated("done"))
        .capability("browser_use")
        .build()
        .unwrap();
    let session = engine.create(agent);
    session.start().await.unwrap();
    let inspection = session.inspect().await.unwrap();
    assert!(inspection.tools.iter().any(|tool| tool.name == "browser"));
    browser.close(session.session_id()).await.unwrap();
}
