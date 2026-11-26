use crate::*;
use async_trait::async_trait;
use futures_util::stream::BoxStream;
use tokio_util::sync::CancellationToken;

pub type TextStream = BoxStream<'static, Result<String>>;

/// A host may persist each accepted part before an adapter sends the next one.
/// The library owns no storage or recovery task. A callback failure stops output.
#[async_trait]
pub trait ReceiptObserver: Send + Sync {
    async fn confirmed(&self, reference: &MessageRef) -> Result<()>;
}

#[async_trait]
pub trait Adapter: Send + Sync {
    fn provider(&self) -> Provider;
    fn installation(&self) -> &str;
    fn capabilities(&self) -> Capabilities;
    fn receive(&self, request: &WebhookRequest) -> Result<Ingress>;
    async fn post(&self, target: &ConversationKey, content: Content) -> Result<Receipt>;
    async fn reply(&self, source: &Message, content: Content) -> Result<Receipt> {
        self.post(&source.reference.conversation, content).await
    }
    async fn react(&self, target: &MessageRef, emoji: Option<&str>) -> Result<()>;
    async fn edit(&self, _target: &MessageRef, _content: Content) -> Result<()> {
        Err(Error::Unsupported("edit"))
    }
    async fn delete(&self, _target: &MessageRef) -> Result<()> {
        Err(Error::Unsupported("delete"))
    }
    async fn history(
        &self,
        _target: &ConversationKey,
        _cursor: Option<&str>,
        _limit: usize,
    ) -> Result<HistoryPage> {
        Err(Error::Unsupported("native history"))
    }
    async fn typing(&self, _target: &MessageRef) -> Result<()> {
        Err(Error::Unsupported("typing"))
    }
    async fn stream(
        &self,
        target: &ConversationKey,
        stream: TextStream,
        cancel: CancellationToken,
    ) -> Result<Receipt> {
        self.post(target, buffer_stream(stream, cancel).await?)
            .await
    }
    /// Preserve provider reply context, including WhatsApp's reply window.
    async fn stream_reply(
        &self,
        source: &Message,
        stream: TextStream,
        cancel: CancellationToken,
    ) -> Result<Receipt> {
        self.check_scope(&source.reference.conversation)?;
        self.reply(source, buffer_stream(stream, cancel).await?)
            .await
    }
    fn check_scope(&self, target: &ConversationKey) -> Result<()> {
        target.validate()?;
        if target.provider != self.provider() || target.installation != self.installation() {
            Err(Error::Scope)
        } else {
            Ok(())
        }
    }
}
async fn buffer_stream(stream: TextStream, cancel: CancellationToken) -> Result<Content> {
    use futures_util::StreamExt;
    let mut stream = bounded_stream(stream, cancel);
    let mut output = String::new();
    while let Some(chunk) = stream.next().await {
        output.push_str(&chunk?);
    }
    Ok(Content::Text(output))
}

pub const MAX_STREAM_CHUNKS: usize = 4096;
pub const MAX_STREAM_DURATION: std::time::Duration = std::time::Duration::from_secs(300);

/// Wrap a source with shared byte/chunk/deadline/cancellation limits.
/// Providers still account for confirmed side effects and stop native streams on errors.
pub fn bounded_stream(source: TextStream, cancel: CancellationToken) -> TextStream {
    use futures_util::StreamExt;
    let state = (
        source,
        cancel,
        tokio::time::Instant::now() + MAX_STREAM_DURATION,
        0usize,
        0usize,
    );
    Box::pin(futures_util::stream::try_unfold(
        state,
        |(mut source, cancel, deadline, chunks, bytes)| async move {
            if tokio::time::Instant::now() >= deadline {
                return Err(Error::DeadlineExceeded);
            }
            let next = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(Error::Cancelled),
                value = tokio::time::timeout_at(deadline, source.next()) => value.map_err(|_| Error::DeadlineExceeded)?,
            };
            let Some(chunk) = next else {
                return Ok(None);
            };
            let chunk = chunk?;
            let chunks = chunks + 1;
            let bytes = bytes.saturating_add(chunk.len());
            if chunks > MAX_STREAM_CHUNKS {
                return Err(Error::Capacity("stream chunks"));
            }
            if bytes > MAX_OUTPUT_BYTES {
                return Err(Error::Capacity("stream output"));
            }
            Ok(Some((chunk, (source, cancel, deadline, chunks, bytes))))
        },
    ))
}
