//! Activity trait definition

use async_trait::async_trait;
use serde::{Serialize, de::DeserializeOwned};

use super::{ActivityContext, ActivityError};

/// An activity is a unit of work that may fail and be retried
///
/// **Experimental** (`workflows` feature): the API may change in any release.
///
/// Activities are the building blocks of workflows. They represent
/// discrete operations that:
/// - Are executed by workers outside the workflow
/// - May take a long time to complete
/// - Can fail and be retried
/// - Can send heartbeats for liveness
///
/// # Example
///
/// ```
/// use everruns_durable::{Activity, ActivityContext, ActivityError};
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Serialize, Deserialize)]
/// struct SendEmail {
///     to: String,
/// }
///
/// #[derive(Serialize, Deserialize)]
/// struct Sent {
///     message_id: String,
/// }
///
/// struct SendEmailActivity;
///
/// #[async_trait::async_trait]
/// impl Activity for SendEmailActivity {
///     const TYPE: &'static str = "send_email";
///     type Input = SendEmail;
///     type Output = Sent;
///
///     async fn execute(&self, ctx: &ActivityContext, input: SendEmail) -> Result<Sent, ActivityError> {
///         if !input.to.contains('@') {
///             // Retrying cannot fix a bad address.
///             return Err(ActivityError::non_retryable("invalid address").with_type("validation"));
///         }
///         Ok(Sent { message_id: format!("{}-{}", ctx.activity_id, ctx.attempt) })
///     }
/// }
///
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() {
/// let ctx = ActivityContext::new(uuid::Uuid::now_v7(), "email-1".into(), 1, 3);
/// let sent = SendEmailActivity
///     .execute(&ctx, SendEmail { to: "ada@example.com".into() })
///     .await
///     .unwrap();
/// assert_eq!(sent.message_id, "email-1-1");
///
/// let err = SendEmailActivity
///     .execute(&ctx, SendEmail { to: "nobody".into() })
///     .await
///     .err()
///     .unwrap();
/// assert!(!err.retryable);
/// # }
/// ```
#[async_trait]
pub trait Activity: Send + Sync + 'static {
    /// Unique type identifier for this activity
    ///
    /// This is used to look up the activity in the registry.
    const TYPE: &'static str;

    /// Input type for the activity
    type Input: Serialize + DeserializeOwned + Send;

    /// Output type for the activity
    type Output: Serialize + DeserializeOwned + Send;

    /// Execute the activity
    ///
    /// The context provides:
    /// - Attempt information
    /// - Heartbeat functionality
    /// - Cancellation token
    ///
    /// # Errors
    ///
    /// Return `ActivityError::retryable()` for transient failures that should be retried.
    /// Return `ActivityError::non_retryable()` for permanent failures.
    async fn execute(
        &self,
        ctx: &ActivityContext,
        input: Self::Input,
    ) -> Result<Self::Output, ActivityError>;
}
