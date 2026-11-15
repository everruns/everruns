//! The display of an E2B Sandbox created with a desktop (EVE-1133).
//!
//! A Sandbox Template with `target.options.desktop` makes the session sandbox
//! provider create the primary Sandbox from E2B's `desktop` template, and the
//! `sandbox_desktop` capability drives it through [`E2BDesktopSession`], the
//! shared X display session ([`crate::x11_desktop`]) over envd.
//!
//! Decision: the desktop is an option of the primary Sandbox, not a
//! flagged capability with a Sandbox of its own, so it shares the machine,
//! files, lifetime and network rule of the shell and file tools.
//!
//! Security: egress is the Sandbox's E2B network policy: the template rule and
//! the session network access list, enforced by E2B for every process on the
//! desktop, browser included (`network.rs`, TM-E2B-005).

use async_trait::async_trait;
use everruns_contracts::runtime::computer_use::DisplaySize;

pub use crate::x11_desktop::*;

use crate::e2b::client::E2BClient;
use crate::e2b::state::SandboxState;

/// E2B template with Xvfb, xdotool and a desktop session.
pub const E2B_DESKTOP_TEMPLATE: &str = "desktop";

/// A desktop sandbox reached through envd.
pub struct E2BDesktopHost {
    client: E2BClient,
    state: SandboxState,
}

/// The same, borrowed: for one display start during create.
struct E2BHostRef<'a> {
    client: &'a E2BClient,
    state: &'a SandboxState,
}

#[async_trait]
impl DesktopHost for E2BHostRef<'_> {
    async fn exec_argv(
        &self,
        program: &str,
        args: &[String],
        env: &[(&str, &str)],
        timeout_ms: u64,
    ) -> Result<DesktopExecOutput, String> {
        let result = self
            .client
            .exec_argv(self.state, program, args, env, None, Some(timeout_ms))
            .await?;
        Ok(DesktopExecOutput {
            exit_code: result.exit_code,
            stdout: result.stdout,
            stderr: result.stderr,
        })
    }

    async fn read_file(&self, path: &str) -> Result<Vec<u8>, String> {
        self.client.read_file_bytes(self.state, path).await
    }
}

#[async_trait]
impl DesktopHost for E2BDesktopHost {
    async fn exec_argv(
        &self,
        program: &str,
        args: &[String],
        env: &[(&str, &str)],
        timeout_ms: u64,
    ) -> Result<DesktopExecOutput, String> {
        self.borrowed()
            .exec_argv(program, args, env, timeout_ms)
            .await
    }

    async fn read_file(&self, path: &str) -> Result<Vec<u8>, String> {
        self.borrowed().read_file(path).await
    }
}

impl E2BDesktopHost {
    fn borrowed(&self) -> E2BHostRef<'_> {
        E2BHostRef {
            client: &self.client,
            state: &self.state,
        }
    }
}

/// Start the display of a desktop sandbox (a no-op when it already runs at
/// `display`).
pub async fn start_e2b_display(
    client: &E2BClient,
    state: &SandboxState,
    display: DisplaySize,
) -> Result<(), String> {
    start_display(&E2BHostRef { client, state }, display).await
}

/// One call's handle on a desktop sandbox's display.
pub type E2BDesktopSession = X11DesktopSession<E2BDesktopHost>;

impl X11DesktopSession<E2BDesktopHost> {
    /// Bring up the display at `display` in a sandbox built from the desktop
    /// template (a no-op when it already runs at that size). Ownership of
    /// `state` is the caller's job.
    pub async fn start(
        client: E2BClient,
        state: SandboxState,
        display: DisplaySize,
    ) -> Result<Self, String> {
        Self::open(E2BDesktopHost { client, state }, display).await
    }
}
