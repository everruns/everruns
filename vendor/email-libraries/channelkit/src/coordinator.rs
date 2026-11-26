use crate::*;
use futures_util::future::BoxFuture;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    future::Future,
    sync::{Arc, Mutex},
};
use tokio::sync::{Mutex as AsyncMutex, Semaphore};

type Handler<C> = Arc<dyn Fn(C) -> BoxFuture<'static, Result<()>> + Send + Sync>;
type ErrorHook = Arc<dyn Fn(Arc<Error>) -> BoxFuture<'static, ()> + Send + Sync>;
type Authorization<D> =
    Arc<dyn Fn(EventContext<D, Event>) -> BoxFuture<'static, Result<bool>> + Send + Sync>;

#[derive(Clone)]
pub struct Filter(Arc<dyn Fn(&Message) -> bool + Send + Sync>);
impl Filter {
    pub fn new(filter: impl Fn(&Message) -> bool + Send + Sync + 'static) -> Self {
        Self(Arc::new(filter))
    }
    pub fn text_contains(text: impl Into<String>) -> Self {
        let text = text.into();
        Self::new(move |m| m.text.contains(&text))
    }
    pub fn human() -> Self {
        Self::new(|m| !m.author.is_bot)
    }
    pub fn and(self, other: Self) -> Self {
        Self::new(move |m| self.matches(m) && other.matches(m))
    }
    pub fn or(self, other: Self) -> Self {
        Self::new(move |m| self.matches(m) || other.matches(m))
    }
    pub fn matches(&self, message: &Message) -> bool {
        (self.0)(message)
    }
}

impl std::ops::Not for Filter {
    type Output = Self;
    fn not(self) -> Self {
        Self::new(move |m| !self.matches(m))
    }
}

struct ConversationRuntime {
    serial: Arc<AsyncMutex<()>>,
    pending: Arc<Semaphore>,
}
struct Seen {
    ids: HashSet<(ConversationKey, String)>,
    order: VecDeque<(ConversationKey, String)>,
}
impl Seen {
    fn new() -> Self {
        Self {
            ids: HashSet::new(),
            order: VecDeque::new(),
        }
    }
    fn record(&mut self, key: (ConversationKey, String)) {
        if self.ids.insert(key.clone()) {
            self.order.push_back(key);
        }
        while self.order.len() > 10_000 {
            if let Some(old) = self.order.pop_front() {
                self.ids.remove(&old);
            }
        }
    }
}
#[derive(Clone)]
pub struct Thread {
    key: ConversationKey,
    adapter: Arc<dyn Adapter>,
    state: Arc<dyn State>,
    source: Option<Arc<Message>>,
    history: Arc<ObservedHistory>,
}
impl Thread {
    pub fn key(&self) -> &ConversationKey {
        &self.key
    }
    pub async fn post(&self, content: impl Into<Content>) -> Result<SentMessage> {
        let content = content.into();
        content.validate()?;
        self.adapter.check_scope(&self.key)?;
        let text = content.text().to_owned();
        let receipt = if let Some(source) = &self.source {
            self.adapter.reply(source, content).await?
        } else {
            self.adapter.post(&self.key, content).await?
        };
        for reference in receipt
            .messages
            .iter()
            .filter(|_| receipt.messages.len() == 1)
        {
            self.history.append(Message {
                reference: reference.clone(),
                author: User {
                    id: "self".into(),
                    display_name: None,
                    is_bot: true,
                },
                text: text.clone(),
                attachments: vec![],
                direct: self.source.as_ref().is_some_and(|s| s.direct),
                mentioned: false,
                reply_to: self.source.as_ref().map(|s| s.reference.id.clone()),
                timestamp: None,
            });
        }
        Ok(SentMessage {
            receipt,
            adapter: self.adapter.clone(),
        })
    }
    pub async fn subscribe(&self) -> Result<()> {
        self.state.subscribe(&self.key, true).await
    }
    pub async fn unsubscribe(&self) -> Result<()> {
        self.state.subscribe(&self.key, false).await
    }
    pub async fn is_subscribed(&self) -> Result<bool> {
        self.state.subscribed(&self.key).await
    }
    pub async fn state(&self) -> Result<Option<serde_json::Value>> {
        self.state.get(&self.key).await
    }
    pub async fn set_state(&self, value: serde_json::Value) -> Result<()> {
        self.state.set(&self.key, value).await
    }
    pub async fn history(&self, cursor: Option<&str>, limit: usize) -> Result<HistoryPage> {
        if self.adapter.capabilities().history == Support::Unsupported {
            if cursor.is_some() {
                return Err(Error::Unsupported("observed history cursors"));
            }
            self.observed_history(limit)
        } else {
            self.adapter.history(&self.key, cursor, limit).await
        }
    }
    pub fn observed_history(&self, limit: usize) -> Result<HistoryPage> {
        self.history.page(&self.key, limit)
    }
    pub async fn stream(
        &self,
        stream: TextStream,
        cancel: CancellationToken,
    ) -> Result<SentMessage> {
        let receipt = if let Some(source) = &self.source {
            self.adapter.stream_reply(source, stream, cancel).await?
        } else {
            self.adapter.stream(&self.key, stream, cancel).await?
        };
        Ok(SentMessage {
            receipt,
            adapter: self.adapter.clone(),
        })
    }
    pub async fn react(&self, target: &MessageRef, emoji: Option<&str>) -> Result<()> {
        if target.conversation != self.key {
            return Err(Error::Scope);
        }
        self.adapter.react(target, emoji).await
    }
}
#[derive(Clone)]
pub struct Channel {
    thread: Thread,
}
impl Channel {
    pub fn key(&self) -> &ConversationKey {
        self.thread.key()
    }
    pub fn capabilities(&self) -> Capabilities {
        self.thread.adapter.capabilities()
    }
}
#[derive(Clone)]
pub struct SentMessage {
    receipt: Receipt,
    adapter: Arc<dyn Adapter>,
}
impl SentMessage {
    pub fn receipt(&self) -> &Receipt {
        &self.receipt
    }
    fn single(&self) -> Result<&MessageRef> {
        if self.receipt.messages.len() != 1 {
            return Err(Error::Unsupported(
                "single-message operation on a split reply",
            ));
        }
        Ok(&self.receipt.messages[0])
    }
    pub async fn react(&self, emoji: &str) -> Result<()> {
        self.adapter.react(self.single()?, Some(emoji)).await
    }
    pub async fn like(&self) -> Result<()> {
        self.react("👍").await
    }
    pub async fn clear_reaction(&self) -> Result<()> {
        self.adapter.react(self.single()?, None).await
    }
    pub async fn edit(&self, content: impl Into<Content>) -> Result<()> {
        self.adapter.edit(self.single()?, content.into()).await
    }
    pub async fn delete(&self) -> Result<()> {
        self.adapter.delete(self.single()?).await
    }
}

pub struct EventContext<D, E> {
    event: E,
    thread: Thread,
    data: Arc<D>,
    cancel: CancellationToken,
}
impl<D, E: Clone> Clone for EventContext<D, E> {
    fn clone(&self) -> Self {
        Self {
            event: self.event.clone(),
            thread: self.thread.clone(),
            data: self.data.clone(),
            cancel: self.cancel.clone(),
        }
    }
}
impl<D, E> EventContext<D, E> {
    pub fn event(&self) -> &E {
        &self.event
    }
    pub fn thread(&self) -> &Thread {
        &self.thread
    }
    pub fn channel(&self) -> Channel {
        Channel {
            thread: self.thread.clone(),
        }
    }
    pub fn data(&self) -> &D {
        &self.data
    }
    pub fn cancellation(&self) -> CancellationToken {
        self.cancel.clone()
    }
}
pub type MessageContext<D = ()> = EventContext<D, Message>;
pub type ReactionContext<D = ()> = EventContext<D, ReactionEvent>;
pub type ActionContext<D = ()> = EventContext<D, ActionEvent>;
pub type CommandContext<D = ()> = EventContext<D, CommandEvent>;
impl<D> MessageContext<D> {
    pub fn message(&self) -> &Message {
        &self.event
    }
}

struct Routes<D> {
    message: Option<Handler<MessageContext<D>>>,
    direct: Option<Handler<MessageContext<D>>>,
    mention: Option<Handler<MessageContext<D>>>,
    subscribed: Option<Handler<MessageContext<D>>>,
    filters: Vec<(Filter, Handler<MessageContext<D>>)>,
    reaction: Option<Handler<ReactionContext<D>>>,
    action: Option<Handler<ActionContext<D>>>,
    command: Option<Handler<CommandContext<D>>>,
    lifecycle: Option<Handler<EventContext<D, Event>>>,
    authorize: Option<Authorization<D>>,
}
impl<D> Default for Routes<D> {
    fn default() -> Self {
        Self {
            message: None,
            direct: None,
            mention: None,
            subscribed: None,
            filters: vec![],
            reaction: None,
            action: None,
            command: None,
            lifecycle: None,
            authorize: None,
        }
    }
}

pub struct ChannelsBuilder<D = ()> {
    adapters: Vec<Arc<dyn Adapter>>,
    data: Arc<D>,
    state: Arc<dyn State>,
    routes: Routes<D>,
    error: Option<ErrorHook>,
    pending: usize,
    conversations: usize,
    duplicate_route: bool,
}
pub struct Channels<D = ()> {
    adapters: HashMap<(Provider, String), Arc<dyn Adapter>>,
    data: Arc<D>,
    state: Arc<dyn State>,
    routes: Routes<D>,
    error: Option<ErrorHook>,
    pending: usize,
    conversations: usize,
    runtime: Mutex<HashMap<ConversationKey, ConversationRuntime>>,
    seen: Mutex<Seen>,
    history: Arc<ObservedHistory>,
}
impl Channels<()> {
    pub fn builder() -> ChannelsBuilder<()> {
        ChannelsBuilder {
            adapters: vec![],
            data: Arc::new(()),
            state: Arc::new(MemoryState::default()),
            routes: Routes::default(),
            error: None,
            pending: 32,
            conversations: 1024,
            duplicate_route: false,
        }
    }
}
macro_rules! route {
    ($method:ident, $field:ident, $context:ty) => {
        pub fn $method<F, Fut>(mut self, handler: F) -> Self
        where
            F: Fn($context) -> Fut + Send + Sync + 'static,
            Fut: Future<Output = Result<()>> + Send + 'static,
        {
            self.duplicate_route |= self.routes.$field.is_some();
            self.routes.$field = Some(Arc::new(move |context| Box::pin(handler(context))));
            self
        }
    };
}
impl<D: Send + Sync + 'static> ChannelsBuilder<D> {
    pub fn adapter(mut self, adapter: impl Adapter + 'static) -> Self {
        self.adapters.push(Arc::new(adapter));
        self
    }
    pub fn shared_adapter(mut self, adapter: Arc<dyn Adapter>) -> Self {
        self.adapters.push(adapter);
        self
    }
    /// Set application data before registering handlers so their context type is known.
    pub fn data<T: Send + Sync + 'static>(self, data: T) -> ChannelsBuilder<T> {
        let registered = self.routes.message.is_some()
            || self.routes.direct.is_some()
            || self.routes.mention.is_some()
            || self.routes.subscribed.is_some()
            || !self.routes.filters.is_empty()
            || self.routes.reaction.is_some()
            || self.routes.action.is_some()
            || self.routes.command.is_some()
            || self.routes.lifecycle.is_some()
            || self.routes.authorize.is_some();
        ChannelsBuilder {
            adapters: self.adapters,
            data: Arc::new(data),
            state: self.state,
            routes: Routes::default(),
            error: self.error,
            pending: self.pending,
            conversations: self.conversations,
            duplicate_route: self.duplicate_route || registered,
        }
    }
    pub fn state(mut self, state: impl State + 'static) -> Self {
        self.state = Arc::new(state);
        self
    }
    pub fn limits(mut self, conversations: usize, pending_per_conversation: usize) -> Self {
        self.conversations = conversations;
        self.pending = pending_per_conversation;
        self
    }
    route!(on_message, message, MessageContext<D>);
    route!(on_direct_message, direct, MessageContext<D>);
    route!(on_new_mention, mention, MessageContext<D>);
    route!(on_subscribed_message, subscribed, MessageContext<D>);
    route!(on_reaction, reaction, ReactionContext<D>);
    route!(on_action, action, ActionContext<D>);
    route!(on_command, command, CommandContext<D>);
    route!(on_event, lifecycle, EventContext<D, Event>);
    pub fn on_message_matching<F, Fut>(mut self, filter: Filter, handler: F) -> Self
    where
        F: Fn(MessageContext<D>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.routes
            .filters
            .push((filter, Arc::new(move |context| Box::pin(handler(context)))));
        self
    }
    pub fn before_dispatch<F, Fut>(mut self, handler: F) -> Self
    where
        F: Fn(EventContext<D, Event>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<bool>> + Send + 'static,
    {
        self.routes.authorize = Some(Arc::new(move |context| Box::pin(handler(context))));
        self
    }
    pub fn on_error<F, Fut>(mut self, handler: F) -> Self
    where
        F: Fn(Arc<Error>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.error = Some(Arc::new(move |error| Box::pin(handler(error))));
        self
    }
    pub fn build(self) -> Result<Channels<D>> {
        if self.duplicate_route {
            return Err(Error::InvalidInput(
                "duplicate handler or data set after handlers",
            ));
        }
        if self.adapters.is_empty() || self.pending == 0 || self.conversations == 0 {
            return Err(Error::InvalidInput("coordinator configuration"));
        }
        let mut adapters = HashMap::new();
        for adapter in self.adapters {
            if adapter.installation().is_empty()
                || adapters
                    .insert(
                        (adapter.provider(), adapter.installation().to_owned()),
                        adapter,
                    )
                    .is_some()
            {
                return Err(Error::InvalidInput("duplicate or empty installation"));
            }
        }
        Ok(Channels {
            adapters,
            data: self.data,
            state: self.state,
            routes: self.routes,
            error: self.error,
            pending: self.pending,
            conversations: self.conversations,
            runtime: Mutex::new(HashMap::new()),
            seen: Mutex::new(Seen::new()),
            history: Arc::new(ObservedHistory::default()),
        })
    }
}
impl<D: Send + Sync + 'static> Channels<D> {
    pub fn receive(
        &self,
        provider: Provider,
        installation: &str,
        request: &WebhookRequest,
    ) -> Result<Ingress> {
        self.adapters
            .get(&(provider, installation.into()))
            .ok_or(Error::Scope)?
            .receive(request)
    }
    pub fn thread(&self, key: ConversationKey) -> Result<Thread> {
        key.validate()?;
        let adapter = self
            .adapters
            .get(&(key.provider, key.installation.clone()))
            .ok_or(Error::Scope)?
            .clone();
        Ok(Thread {
            key,
            adapter,
            state: self.state.clone(),
            source: None,
            history: self.history.clone(),
        })
    }
    pub async fn dispatch(&self, event: Event) -> Result<bool> {
        self.dispatch_with_cancellation(event, CancellationToken::new())
            .await
    }
    /// Returns false for an observed duplicate, filtered bot, or authorization refusal.
    pub async fn dispatch_with_cancellation(
        &self,
        event: Event,
        cancel: CancellationToken,
    ) -> Result<bool> {
        let key = event.conversation().clone();
        let mut thread = self.thread(key.clone())?;
        if let EventKind::Message(message) = &event.kind {
            thread.source = Some(Arc::new(message.clone()));
        }
        if event.id.is_empty() || event.id.len() > 1024 {
            return Err(Error::InvalidInput("event identifier"));
        }
        let (serial, pending) = {
            let mut runtime = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
            if !runtime.contains_key(&key) && runtime.len() >= self.conversations {
                runtime.retain(|_, r| Arc::strong_count(&r.serial) > 1);
                if runtime.len() >= self.conversations {
                    return Err(Error::Capacity("active conversations"));
                }
            }
            let entry = runtime
                .entry(key.clone())
                .or_insert_with(|| ConversationRuntime {
                    serial: Arc::new(AsyncMutex::new(())),
                    pending: Arc::new(Semaphore::new(self.pending)),
                });
            (entry.serial.clone(), entry.pending.clone())
        };
        let _permit = pending
            .try_acquire_owned()
            .map_err(|_| Error::Capacity("pending conversation work"))?;
        let _guard = tokio::select! { biased; _ = cancel.cancelled() => return Err(Error::Cancelled), guard = serial.lock() => guard };
        let id = (key, event.id.clone());
        if self
            .seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .ids
            .contains(&id)
        {
            return Ok(false);
        }
        let context = EventContext {
            event: event.clone(),
            thread: thread.clone(),
            data: self.data.clone(),
            cancel: cancel.clone(),
        };
        if let Some(authorize) = &self.routes.authorize
            && !authorize(context.clone()).await?
        {
            return Ok(false);
        }
        let outcome = match event.kind {
            EventKind::Message(message) => {
                if message.author.is_bot {
                    return Ok(false);
                }
                self.history.append(message.clone());
                let subscribed = thread.is_subscribed().await?;
                let handler = if message.direct && self.routes.direct.is_some() {
                    self.routes.direct.as_ref()
                } else if subscribed && self.routes.subscribed.is_some() {
                    self.routes.subscribed.as_ref()
                } else if !subscribed && message.mentioned && self.routes.mention.is_some() {
                    self.routes.mention.as_ref()
                } else {
                    self.routes
                        .filters
                        .iter()
                        .find(|(f, _)| f.matches(&message))
                        .map(|(_, h)| h)
                        .or(self.routes.message.as_ref())
                };
                match handler {
                    Some(h) => {
                        h(EventContext {
                            event: message,
                            thread,
                            data: self.data.clone(),
                            cancel,
                        })
                        .await
                    }
                    None => Ok(()),
                }
            }
            EventKind::Reaction(event) => match &self.routes.reaction {
                Some(h) => {
                    h(EventContext {
                        event,
                        thread,
                        data: self.data.clone(),
                        cancel,
                    })
                    .await
                }
                None => Ok(()),
            },
            EventKind::Action(event) => match &self.routes.action {
                Some(h) => {
                    h(EventContext {
                        event,
                        thread,
                        data: self.data.clone(),
                        cancel,
                    })
                    .await
                }
                None => Ok(()),
            },
            EventKind::Command(event) => match &self.routes.command {
                Some(h) => {
                    h(EventContext {
                        event,
                        thread,
                        data: self.data.clone(),
                        cancel,
                    })
                    .await
                }
                None => Ok(()),
            },
            _ => match &self.routes.lifecycle {
                Some(h) => h(context).await,
                None => Ok(()),
            },
        };
        match outcome {
            Ok(()) => {
                self.seen
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .record(id);
                Ok(true)
            }
            Err(error) => {
                if let Some(hook) = &self.error {
                    let error = Arc::new(error);
                    hook(error.clone()).await;
                    Err(Error::Observed(error))
                } else {
                    Err(error)
                }
            }
        }
    }
}

#[derive(Default)]
struct ObservedHistory {
    inner: Mutex<(VecDeque<(Message, usize)>, usize)>,
}
impl ObservedHistory {
    fn append(&self, message: Message) {
        let cost = message.text.len()
            + message.reference.id.len()
            + message.reference.conversation.channel.len()
            + message
                .attachments
                .iter()
                .map(|a| a.id.len() + a.name.as_ref().map_or(0, String::len))
                .sum::<usize>()
            + 512;
        if cost > MAX_OUTPUT_BYTES {
            return;
        }
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner
            .0
            .iter()
            .any(|(m, _)| m.reference == message.reference)
        {
            return;
        }
        while inner.0.len() >= 1024 || inner.1 + cost > 8 * 1024 * 1024 {
            if let Some((_, cost)) = inner.0.pop_front() {
                inner.1 -= cost;
            } else {
                break;
            }
        }
        inner.1 += cost;
        inner.0.push_back((message, cost));
    }
    fn page(&self, key: &ConversationKey, limit: usize) -> Result<HistoryPage> {
        if limit == 0 || limit > 100 {
            return Err(Error::InvalidInput("observed history limit"));
        }
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut messages = inner
            .0
            .iter()
            .rev()
            .filter(|(m, _)| &m.reference.conversation == key)
            .take(limit)
            .map(|(m, _)| m.clone())
            .collect::<Vec<_>>();
        messages.reverse();
        Ok(HistoryPage {
            messages,
            next_cursor: None,
            complete: false,
            native: false,
        })
    }
}
