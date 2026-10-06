//! Local headless Chromium for tests that need a real browser.
//!
//! Tests skip when no Chromium is installed (set `CHROMIUM_PATH` to run).

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::browserless::browser_egress::BrowserEgress;
use crate::browserless::cdp::CdpSession;

/// A local Chromium to drive, if this machine has one. CI runners ship Google
/// Chrome; cloud agent containers ship Playwright's Chromium.
pub(crate) fn chromium_binary() -> Option<String> {
    if let Ok(path) = std::env::var("CHROMIUM_PATH")
        && !path.is_empty()
    {
        return Some(path);
    }
    let fixed = [
        "/opt/pw-browsers/chromium-1194/chrome-linux/chrome",
        "/usr/bin/google-chrome",
        "/usr/bin/google-chrome-stable",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
    ];
    if let Some(path) = fixed.iter().find(|p| std::path::Path::new(p).exists()) {
        return Some((*path).to_string());
    }
    // Any Playwright chromium build.
    std::fs::read_dir("/opt/pw-browsers")
        .ok()?
        .flatten()
        .map(|entry| entry.path().join("chrome-linux/chrome"))
        .find(|path| path.exists())
        .map(|path| path.to_string_lossy().into_owned())
}

pub(crate) struct LocalChromium {
    pub child: Child,
    pub ws_url: String,
    _profile: tempdir::Profile,
}

mod tempdir {
    /// A throwaway profile directory, removed on drop.
    pub struct Profile(pub std::path::PathBuf);

    impl Profile {
        pub fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "everruns-computer-use-{}",
                everruns_contracts::typed_id::SessionId::new()
            ));
            std::fs::create_dir_all(&dir).expect("create profile dir");
            Self(dir)
        }
    }

    impl Drop for Profile {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

pub(crate) async fn launch_chromium() -> Option<LocalChromium> {
    launch_chromium_with_args(&[]).await
}

/// Launch with extra Chromium flags, e.g. `--host-resolver-rules` to make the
/// browser's own DNS answer a hostname with an internal address.
pub(crate) async fn launch_chromium_with_args(extra_args: &[&str]) -> Option<LocalChromium> {
    let binary = chromium_binary()?;
    let profile = tempdir::Profile::new();
    let mut child = Command::new(&binary)
        .args([
            "--headless=new",
            "--no-sandbox",
            "--disable-gpu",
            "--no-first-run",
            "--no-default-browser-check",
            "--remote-debugging-port=0",
            "about:blank",
        ])
        .args(extra_args)
        .arg(format!("--user-data-dir={}", profile.0.display()))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let stderr = child.stderr.take()?;
    let mut lines = BufReader::new(stderr).lines();
    let ws_url = tokio::time::timeout(Duration::from_secs(20), async {
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(url) = line.strip_prefix("DevTools listening on ") {
                return Some(url.trim().to_string());
            }
        }
        None
    })
    .await
    .ok()??;
    // Keep draining stderr so Chromium never blocks on a full pipe.
    tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
    Some(LocalChromium {
        child,
        ws_url,
        _profile: profile,
    })
}

/// Connect a guarded session to a freshly started Chromium. Test builds use a
/// 1s CDP connect timeout; a loaded CI runner can take longer, so retry.
pub(crate) async fn connect_guarded(
    browser: &LocalChromium,
    egress: Arc<BrowserEgress>,
) -> CdpSession {
    let mut last_error = String::new();
    for _ in 0..20 {
        match CdpSession::connect(&browser.ws_url, egress.clone(), None).await {
            Ok(session) => return session,
            Err(error) => {
                last_error = error;
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
    }
    panic!("connect to local Chromium over CDP: {last_error}");
}
