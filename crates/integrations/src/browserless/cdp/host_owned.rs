//! Host-owned CDP context lifecycle, separate from the hosted Browserless guard.

use super::CdpSession;
use serde_json::{Value, json};

impl CdpSession {
    /// Attach to a host-owned browser with browser-side networking. Never attaches the
    /// default context. The host must enforce the sandbox's network boundary.
    /// THREAT[TM-BROWSER-002]: context ownership confines target discovery and attachment.
    pub(crate) async fn connect_direct(
        ws_url: &str,
        headers: &[(String, String)],
        context: Option<&str>,
    ) -> Result<Self, String> {
        let mut session = Self::open_socket_with_headers(ws_url, None, headers)
            .await
            .map_err(|_| {
                "Direct browser connection failed (endpoint and credentials redacted)".to_string()
            })?;
        let context_id = if let Some(id) = context {
            if !session.context_exists(id).await? {
                return Err(
                    "Browser session was lost; close it before starting a fresh browser context"
                        .into(),
                );
            }
            id.to_owned()
        } else {
            let result = session
                .send_browser_command(
                    "Target.createBrowserContext",
                    json!({"disposeOnDetach": false}),
                )
                .await?;
            result
                .get("browserContextId")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or("Browser did not create an isolated context")?
                .to_owned()
        };
        session.browser_context_id = context_id.clone();
        let attached = async {
            let target = match session.find_page_in_context(&context_id).await? {
                Some(id) => id,
                None => session.create_blank_page_target(&context_id).await?,
            };
            session.attach_page(&target).await
        }
        .await;
        if let Err(error) = attached {
            if context.is_none() {
                let _ = session
                    .send_browser_command(
                        "Target.disposeBrowserContext",
                        json!({"browserContextId":context_id}),
                    )
                    .await;
            }
            session.disconnect().await;
            return Err(error);
        }
        Ok(session)
    }

    pub(crate) async fn dispose_direct(
        ws_url: &str,
        headers: &[(String, String)],
        context_id: &str,
    ) -> Result<(), String> {
        let mut session = Self::open_socket_with_headers(ws_url, None, headers)
            .await
            .map_err(|_| "Direct browser connection failed".to_string())?;
        if session.context_exists(context_id).await? {
            session
                .send_browser_command(
                    "Target.disposeBrowserContext",
                    json!({"browserContextId": context_id}),
                )
                .await?;
        }
        session.disconnect().await;
        Ok(())
    }
}
