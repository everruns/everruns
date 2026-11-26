use channelkit::*;
use std::ops::Not;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone)]
struct Fake {
    installation: String,
    provider: Provider,
    posts: Arc<Mutex<Vec<(ConversationKey, Content)>>>,
}
impl Fake {
    fn new(installation: &str) -> Self {
        Self {
            installation: installation.into(),
            provider: Provider::Slack,
            posts: Arc::new(Mutex::new(vec![])),
        }
    }
}
#[async_trait]
impl Adapter for Fake {
    fn provider(&self) -> Provider {
        self.provider
    }
    fn installation(&self) -> &str {
        &self.installation
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            reactions: Support::Native,
            edit: Support::Unsupported,
            delete: Support::Unsupported,
            history: Support::Unsupported,
            streaming: Support::Emulated,
            typing: Support::Unsupported,
            media: Support::Unsupported,
        }
    }
    fn receive(&self, _: &WebhookRequest) -> Result<Ingress> {
        Ok(Ingress {
            response: WebhookResponse::text("ok"),
            events: vec![],
        })
    }
    async fn post(&self, key: &ConversationKey, content: Content) -> Result<Receipt> {
        self.check_scope(key)?;
        let mut posts = self.posts.lock().unwrap();
        let id = (posts.len() + 1).to_string();
        posts.push((key.clone(), content));
        Ok(Receipt {
            messages: vec![MessageRef {
                conversation: key.clone(),
                id,
            }],
            status: DeliveryStatus::Accepted,
        })
    }
    async fn react(&self, target: &MessageRef, _: Option<&str>) -> Result<()> {
        self.check_scope(&target.conversation)
    }
}
fn key(installation: &str) -> ConversationKey {
    ConversationKey::new(Provider::Slack, installation, "C1", Some("1.000".into())).unwrap()
}
fn message(installation: &str, id: &str, direct: bool, mentioned: bool) -> Event {
    Event {
        id: id.into(),
        kind: EventKind::Message(Message {
            reference: MessageRef {
                conversation: key(installation),
                id: id.into(),
            },
            author: User {
                id: "U1".into(),
                display_name: Some("Alice".into()),
                is_bot: false,
            },
            text: "hello".into(),
            attachments: vec![],
            direct,
            mentioned,
            reply_to: None,
            timestamp: None,
        }),
    }
}

#[tokio::test]
async fn minimal_echo_and_observed_history_without_infrastructure() {
    let fake = Fake::new("one");
    let posts = fake.posts.clone();
    let channels = Channels::builder()
        .adapter(fake)
        .on_message(|ctx| async move {
            ctx.thread()
                .post(format!("echo: {}", ctx.message().text))
                .await?;
            Ok(())
        })
        .build()
        .unwrap();
    assert!(
        channels
            .dispatch(message("one", "m1", false, false))
            .await
            .unwrap()
    );
    assert!(
        !channels
            .dispatch(message("one", "m1", false, false))
            .await
            .unwrap()
    );
    assert_eq!(posts.lock().unwrap().len(), 1);
    let history = channels
        .thread(key("one"))
        .unwrap()
        .history(None, 10)
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 2);
    assert_eq!(history.messages[1].text, "echo: hello");
    assert!(!history.complete && !history.native);
    assert!(matches!(
        channels
            .thread(key("one"))
            .unwrap()
            .history(Some("bad"), 10)
            .await,
        Err(Error::Unsupported(_))
    ));
}
#[tokio::test]
async fn mention_subscribe_followup_and_dm_precedence() {
    let fake = Fake::new("one");
    let posts = fake.posts.clone();
    let channels = Channels::builder()
        .adapter(fake)
        .on_new_mention(|ctx| async move {
            ctx.thread().subscribe().await?;
            ctx.thread().post("mention").await?;
            Ok(())
        })
        .on_subscribed_message(|ctx| async move {
            ctx.thread().post("subscribed").await?;
            Ok(())
        })
        .on_direct_message(|ctx| async move {
            ctx.thread().post("direct").await?;
            Ok(())
        })
        .on_message(|ctx| async move {
            ctx.thread().post("fallback").await?;
            Ok(())
        })
        .build()
        .unwrap();
    channels
        .dispatch(message("one", "m1", false, true))
        .await
        .unwrap();
    channels
        .dispatch(message("one", "m2", false, true))
        .await
        .unwrap();
    channels
        .dispatch(message("one", "m3", true, true))
        .await
        .unwrap();
    channels
        .thread(key("one"))
        .unwrap()
        .unsubscribe()
        .await
        .unwrap();
    channels
        .dispatch(message("one", "m4", false, false))
        .await
        .unwrap();
    let values = posts
        .lock()
        .unwrap()
        .iter()
        .map(|(_, c)| c.text().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(values, ["mention", "subscribed", "direct", "fallback"]);
}
#[tokio::test]
async fn typed_data_macros_and_filtered_first_match() {
    let fake = Fake::new("one");
    let posts = fake.posts.clone();
    let channels=channels!{adapters:[fake],data:"shared".to_owned(),on_message:|ctx:MessageContext<String>|async move{ctx.thread().post(ctx.data().clone()).await?;Ok(())},}.unwrap();
    channels
        .dispatch(message("one", "m1", false, false))
        .await
        .unwrap();
    assert_eq!(posts.lock().unwrap()[0].1.text(), "shared");
    let fake = Fake::new("one");
    let posts = fake.posts.clone();
    let channels = Channels::builder()
        .adapter(fake)
        .on_message_matching(
            Filter::human()
                .and(Filter::text_contains("hello"))
                .or(Filter::text_contains("never")),
            |ctx| async move {
                ctx.thread().post("first").await?;
                Ok(())
            },
        )
        .on_message_matching(Filter::text_contains("bad").not(), |ctx| async move {
            ctx.thread().post("second").await?;
            Ok(())
        })
        .on_message(|ctx| async move {
            ctx.thread().post("last").await?;
            Ok(())
        })
        .build()
        .unwrap();
    channels
        .dispatch(message("one", "m1", false, false))
        .await
        .unwrap();
    assert_eq!(posts.lock().unwrap()[0].1.text(), "first");
}
#[tokio::test]
async fn isolation_authorization_and_reaction_separation() {
    let count = Arc::new(AtomicUsize::new(0));
    let reaction_count = count.clone();
    let messages = Arc::new(AtomicUsize::new(0));
    let message_count = messages.clone();
    let channels = Channels::builder()
        .adapter(Fake::new("one"))
        .adapter(Fake::new("two"))
        .on_message(move |_| {
            message_count.fetch_add(1, Ordering::SeqCst);
            async { Ok(()) }
        })
        .on_reaction(move |_| {
            reaction_count.fetch_add(1, Ordering::SeqCst);
            async { Ok(()) }
        })
        .build()
        .unwrap();
    channels
        .dispatch(message("one", "same", false, false))
        .await
        .unwrap();
    channels
        .dispatch(message("two", "same", false, false))
        .await
        .unwrap();
    channels
        .dispatch(Event {
            id: "r1".into(),
            kind: EventKind::Reaction(ReactionEvent {
                target: MessageRef {
                    conversation: key("one"),
                    id: "same".into(),
                },
                actor: User {
                    id: "U1".into(),
                    display_name: None,
                    is_bot: false,
                },
                change: ReactionChange::Added("thumbsup".into()),
            }),
        })
        .await
        .unwrap();
    assert_eq!(messages.load(Ordering::SeqCst), 2);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert!(matches!(
        channels
            .dispatch(message("unknown", "a", false, false))
            .await,
        Err(Error::Scope)
    ));
    let channels = Channels::builder()
        .adapter(Fake::new("one"))
        .before_dispatch(|_| async { Ok(false) })
        .on_message(|_| async { panic!("unauthorized handler") })
        .build()
        .unwrap();
    assert!(
        !channels
            .dispatch(message("one", "a", false, false))
            .await
            .unwrap()
    );
}
#[tokio::test]
async fn bot_events_do_not_loop_and_failed_handlers_can_be_retried() {
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    let channels = Channels::builder()
        .adapter(Fake::new("one"))
        .on_message(move |_| {
            let seen = seen.clone();
            async move {
                if seen.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(Error::InvalidInput("failed"))
                } else {
                    Ok(())
                }
            }
        })
        .build()
        .unwrap();
    let mut event = message("one", "bot", false, false);
    if let EventKind::Message(m) = &mut event.kind {
        m.author.is_bot = true;
    }
    assert!(!channels.dispatch(event).await.unwrap());
    assert!(
        channels
            .dispatch(message("one", "m1", false, false))
            .await
            .is_err()
    );
    assert!(
        channels
            .dispatch(message("one", "m1", false, false))
            .await
            .unwrap()
    );
}
#[tokio::test]
async fn error_hook_preserves_original_error() {
    let seen = Arc::new(AtomicUsize::new(0));
    let count = seen.clone();
    let channels = Channels::builder()
        .adapter(Fake::new("one"))
        .on_message(|_| async { Err(Error::ReplyWindowClosed) })
        .on_error(move |error| {
            assert!(matches!(*error, Error::ReplyWindowClosed));
            count.fetch_add(1, Ordering::SeqCst);
            async {}
        })
        .build()
        .unwrap();
    match channels.dispatch(message("one", "m1", false, false)).await {
        Err(Error::Observed(error)) => assert!(matches!(*error, Error::ReplyWindowClosed)),
        _ => panic!("source lost"),
    };
    assert_eq!(seen.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn backpressure_cancellation_and_serialized_followups() {
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let begin = started.clone();
    let end = release.clone();
    let channels = Arc::new(
        Channels::builder()
            .adapter(Fake::new("one"))
            .limits(1, 1)
            .on_message(move |_| {
                let begin = begin.clone();
                let end = end.clone();
                async move {
                    begin.notify_one();
                    end.notified().await;
                    Ok(())
                }
            })
            .build()
            .unwrap(),
    );
    let first = channels.clone();
    let task =
        tokio::spawn(async move { first.dispatch(message("one", "m1", false, false)).await });
    started.notified().await;
    assert!(matches!(
        channels.dispatch(message("one", "m2", false, false)).await,
        Err(Error::Capacity(_))
    ));
    let mut different = message("one", "m3", false, false);
    if let EventKind::Message(m) = &mut different.kind {
        m.reference.conversation.channel = "C2".into();
    }
    assert!(matches!(
        channels.dispatch(different).await,
        Err(Error::Capacity(_))
    ));
    release.notify_one();
    task.await.unwrap().unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        channels
            .dispatch_with_cancellation(message("one", "m4", false, false), cancel)
            .await,
        Err(Error::Cancelled)
    ));
}
#[tokio::test]
async fn memory_state_bounds_and_isolation() {
    let state = MemoryState::new(1);
    let one = key("one");
    let two = key("two");
    state.subscribe(&one, true).await.unwrap();
    assert!(state.subscribed(&one).await.unwrap());
    assert!(!state.subscribed(&two).await.unwrap());
    assert!(matches!(
        state.subscribe(&two, true).await,
        Err(Error::Capacity(_))
    ));
    state.subscribe(&one, false).await.unwrap();
    state.subscribe(&two, true).await.unwrap();
    state
        .set(&two, serde_json::json!({"step":1}))
        .await
        .unwrap();
    assert_eq!(state.get(&two).await.unwrap().unwrap()["step"], 1);
    assert!(matches!(
        state.set(&two, serde_json::json!("x".repeat(70_000))).await,
        Err(Error::Capacity(_))
    ));
}
#[tokio::test]
async fn buffered_streams_bound_and_cancel() {
    let fake = Fake::new("one");
    let target = key("one");
    let stream = Box::pin(futures_util::stream::iter([
        Ok("hello ".into()),
        Ok("world".into()),
    ]));
    assert_eq!(
        fake.stream(&target, stream, CancellationToken::new())
            .await
            .unwrap()
            .messages
            .len(),
        1
    );
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        fake.stream(&target, Box::pin(futures_util::stream::pending()), cancel)
            .await,
        Err(Error::Cancelled)
    ));
    let stream = Box::pin(futures_util::stream::iter([Ok(
        "x".repeat(MAX_OUTPUT_BYTES + 1)
    )]));
    assert!(matches!(
        fake.stream(&target, stream, CancellationToken::new()).await,
        Err(Error::Capacity(_))
    ));
    let stream = Box::pin(futures_util::stream::iter([Err(Error::InvalidResponse)]));
    assert!(matches!(
        fake.stream(&target, stream, CancellationToken::new()).await,
        Err(Error::InvalidResponse)
    ));
}
#[test]
fn validation_unicode_macros_and_registry() {
    assert!("slack".parse::<Provider>().is_ok());
    assert!("whatsapp".parse::<Provider>().is_ok());
    assert!("telegram".parse::<Provider>().is_err());
    assert!("unknown".parse::<Provider>().is_err());
    assert!(ConversationKey::new(Provider::Slack, "", "C1", None).is_err());
    assert!(ConversationKey::new(Provider::Slack, "one", "C1", Some("".into())).is_err());
    assert_eq!(split_text("🙂αβ🙂", 2).unwrap(), ["🙂α", "β🙂"]);
    assert!(split_text("text", 0).is_err());
    assert!(split_text(&"x".repeat(MAX_OUTPUT_BYTES + 1), 10).is_err());
    let card =
        card! {title:"Title",body:"Body",actions:[Button::new("a","A"),Button::new("b","B")],}
            .unwrap();
    assert_eq!(card.actions.len(), 2);
    assert!(Card::new("", "body", vec![]).is_err());
    assert!(
        Card::new(
            "title",
            "body",
            vec![Button::new("a", "A"), Button::new("a", "B")]
        )
        .is_err()
    );
    assert!(Card::new("title", "body", vec![Button::new("", "A")]).is_err());
    assert!(Content::Text("".into()).validate().is_err());
    assert!(Content::Markdown("".into()).validate().is_err());
    assert!(Channels::builder().build().is_err());
    assert!(
        Channels::builder()
            .adapter(Fake::new("one"))
            .adapter(Fake::new("one"))
            .build()
            .is_err()
    );
    assert!(
        Channels::builder()
            .adapter(Fake::new("one"))
            .on_message(|_| async { Ok(()) })
            .on_message(|_| async { Ok(()) })
            .build()
            .is_err()
    );
    assert!(
        Channels::builder()
            .adapter(Fake::new("one"))
            .on_message(|_| async { Ok(()) })
            .data("late")
            .build()
            .is_err()
    );
    assert_eq!(
        DeliveryStatus::Read.reconcile(DeliveryStatus::Sent),
        DeliveryStatus::Read
    );
    assert_eq!(
        DeliveryStatus::Delivered.reconcile(DeliveryStatus::Read),
        DeliveryStatus::Read
    );
}
#[tokio::test]
async fn non_message_routes_and_operation_scope() {
    let count = Arc::new(AtomicUsize::new(0));
    let a = count.clone();
    let c = count.clone();
    let l = count.clone();
    let channels = Channels::builder()
        .adapter(Fake::new("one"))
        .on_action(move |_| {
            a.fetch_add(1, Ordering::SeqCst);
            async { Ok(()) }
        })
        .on_command(move |_| {
            c.fetch_add(1, Ordering::SeqCst);
            async { Ok(()) }
        })
        .on_event(move |_| {
            l.fetch_add(1, Ordering::SeqCst);
            async { Ok(()) }
        })
        .build()
        .unwrap();
    let actor = User {
        id: "U1".into(),
        display_name: None,
        is_bot: false,
    };
    for event in [
        EventKind::Action(ActionEvent {
            conversation: key("one"),
            actor: actor.clone(),
            id: "a".into(),
            value: "yes".into(),
            message_id: None,
        }),
        EventKind::Command(CommandEvent {
            conversation: key("one"),
            actor,
            command: "/help".into(),
            text: "".into(),
        }),
        EventKind::Control {
            conversation: key("one"),
            actor: None,
            action: Control::Stop,
        },
    ]
    .into_iter()
    .enumerate()
    {
        channels
            .dispatch(Event {
                id: event.0.to_string(),
                kind: event.1,
            })
            .await
            .unwrap();
    }
    assert_eq!(count.load(Ordering::SeqCst), 3);
    let thread = channels.thread(key("one")).unwrap();
    let sent = thread.post("message").await.unwrap();
    sent.like().await.unwrap();
    assert!(sent.edit("x").await.is_err());
    assert!(sent.delete().await.is_err());
    assert!(matches!(
        thread
            .react(
                &MessageRef {
                    conversation: key("two"),
                    id: "1".into()
                },
                Some("👍")
            )
            .await,
        Err(Error::Scope)
    ));
    thread.set_state(serde_json::json!({"a":1})).await.unwrap();
    assert_eq!(thread.state().await.unwrap().unwrap()["a"], 1);
}

#[tokio::test]
async fn thread_handles_stream_state_and_configuration_edges() {
    let fake = Fake::new("one");
    let channels = Channels::builder()
        .shared_adapter(Arc::new(fake))
        .build()
        .unwrap();
    let thread = channels.thread(key("one")).unwrap();
    let receipt = thread
        .stream(
            Box::pin(futures_util::stream::iter([Ok("hello".into())])),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(receipt.receipt().status, DeliveryStatus::Accepted);
    receipt.clear_reaction().await.unwrap();
    assert!(!thread.is_subscribed().await.unwrap());
    assert!(thread.state().await.unwrap().is_none());
    assert_eq!(thread.key(), &key("one"));
    assert!(thread.observed_history(0).is_err());
    assert!(thread.observed_history(101).is_err());
    let channels = Channels::builder()
        .adapter(Fake::new("one"))
        .on_message(|ctx| async move {
            assert_eq!(ctx.channel().key(), ctx.thread().key());
            assert_eq!(ctx.channel().capabilities().reactions, Support::Native);
            assert!(!ctx.cancellation().is_cancelled());
            Ok(())
        })
        .build()
        .unwrap();
    channels
        .dispatch(message("one", "a", false, false))
        .await
        .unwrap();
    assert_eq!(Provider::Slack.as_str(), "slack");
    assert_eq!(Provider::Whatsapp.as_str(), "whatsapp");
    assert!(
        Channels::builder()
            .adapter(Fake::new("one"))
            .limits(0, 1)
            .build()
            .is_err()
    );
    let mut bad = message("one", "a", false, false);
    bad.id.clear();
    assert!(channels.dispatch(bad).await.is_err());
    assert!(
        channels
            .receive(Provider::Slack, "foreign", &WebhookRequest::post(vec![]))
            .is_err()
    );
    assert!(
        channels
            .receive(Provider::Slack, "one", &WebhookRequest::post(vec![]))
            .unwrap()
            .events
            .is_empty()
    );
    let key = ConversationKey {
        provider: Provider::Slack,
        installation: "one".into(),
        channel: "C1".into(),
        thread: Some("x".repeat(257)),
    };
    assert!(key.validate().is_err());
    assert_eq!(
        DeliveryStatus::Failed.reconcile(DeliveryStatus::Read),
        DeliveryStatus::Failed
    );
    assert_eq!(
        DeliveryStatus::Read.reconcile(DeliveryStatus::Failed),
        DeliveryStatus::Failed
    );
}
#[tokio::test]
async fn host_state_hook_failure_and_unhandled_lifecycle_are_explicit() {
    struct FailingState;
    #[async_trait]
    impl State for FailingState {
        async fn subscribed(&self, _: &ConversationKey) -> Result<bool> {
            Err(Error::Capacity("host state unavailable"))
        }
        async fn subscribe(&self, _: &ConversationKey, _: bool) -> Result<()> {
            Err(Error::Scope)
        }
        async fn get(&self, _: &ConversationKey) -> Result<Option<serde_json::Value>> {
            Err(Error::Scope)
        }
        async fn set(&self, _: &ConversationKey, _: serde_json::Value) -> Result<()> {
            Err(Error::Scope)
        }
    }
    let channels = Channels::builder()
        .adapter(Fake::new("one"))
        .state(FailingState)
        .build()
        .unwrap();
    assert!(matches!(
        channels.dispatch(message("one", "a", false, false)).await,
        Err(Error::Capacity(_))
    ));
    let channels = Channels::builder()
        .adapter(Fake::new("one"))
        .build()
        .unwrap();
    let actor = User {
        id: "U1".into(),
        display_name: None,
        is_bot: false,
    };
    for (kind, id) in [
        (
            EventKind::Reaction(ReactionEvent {
                target: MessageRef {
                    conversation: key("one"),
                    id: "a".into(),
                },
                actor: actor.clone(),
                change: ReactionChange::Set(None),
            }),
            "r",
        ),
        (
            EventKind::Action(ActionEvent {
                conversation: key("one"),
                actor: actor.clone(),
                id: "a".into(),
                value: "v".into(),
                message_id: None,
            }),
            "a",
        ),
        (
            EventKind::Command(CommandEvent {
                conversation: key("one"),
                actor,
                command: "/help".into(),
                text: "".into(),
            }),
            "c",
        ),
        (
            EventKind::Deleted(MessageRef {
                conversation: key("one"),
                id: "a".into(),
            }),
            "d",
        ),
    ] {
        assert!(
            channels
                .dispatch(Event {
                    id: id.into(),
                    kind
                })
                .await
                .unwrap()
        );
    }
    assert!(
        channels
            .dispatch(message("one", "unhandled", false, false))
            .await
            .unwrap()
    );
    let state = MemoryState::new(0);
    assert!(state.set(&key("one"), serde_json::json!(1)).await.is_err());
    state.subscribe(&key("one"), false).await.unwrap();
    let state = MemoryState::new(1);
    state.subscribe(&key("one"), true).await.unwrap();
    state.set(&key("one"), serde_json::json!(1)).await.unwrap();
    state.subscribe(&key("one"), false).await.unwrap();
    assert!(!state.subscribed(&key("one")).await.unwrap());
}

#[tokio::test(start_paused = true)]
async fn streams_bound_empty_sources_and_stalled_sources_without_sending() {
    use futures_util::StreamExt;
    let fake = Fake::new("one");
    let empty = Box::pin(futures_util::stream::repeat_with(|| Ok(String::new())));
    assert!(matches!(
        fake.stream(&key("one"), empty, CancellationToken::new())
            .await,
        Err(Error::Capacity("stream chunks"))
    ));
    assert!(matches!(
        fake.stream(
            &key("one"),
            Box::pin(futures_util::stream::pending()),
            CancellationToken::new()
        )
        .await,
        Err(Error::DeadlineExceeded)
    ));
    assert!(fake.posts.lock().unwrap().is_empty());
    let mut source = bounded_stream(
        Box::pin(futures_util::stream::iter([Ok("hello".into())])),
        CancellationToken::new(),
    );
    assert_eq!(source.next().await.unwrap().unwrap(), "hello");
    assert!(source.next().await.is_none());
}

#[test]
fn feedback_requires_host_correlated_message_and_preserves_source_semantics() {
    let reply = MessageRef {
        conversation: key("one"),
        id: "reply".into(),
    };
    let mut target = reply.clone();
    target.conversation.thread = None;
    let actor = User {
        id: "U1".into(),
        display_name: None,
        is_bot: false,
    };
    for (emoji, value) in [
        ("👍", FeedbackValue::Positive),
        ("thumbsup", FeedbackValue::Positive),
        ("👎", FeedbackValue::Negative),
        ("-1", FeedbackValue::Negative),
    ] {
        let event = ReactionEvent {
            target: target.clone(),
            actor: actor.clone(),
            change: ReactionChange::Removed(emoji.into()),
        };
        let feedback = event.feedback_for(&reply).unwrap();
        assert_eq!(feedback.value, Some(value));
        assert_eq!(feedback.actor(), &actor);
        assert!(matches!(
            feedback.source,
            FeedbackSource::Reaction(ReactionEvent {
                change: ReactionChange::Removed(_),
                ..
            })
        ));
    }
    let mut event = ReactionEvent {
        target: target.clone(),
        actor: actor.clone(),
        change: ReactionChange::Set(None),
    };
    assert_eq!(event.feedback_for(&reply).unwrap().value, None);
    event.change = ReactionChange::Added("eyes".into());
    assert!(event.feedback_for(&reply).is_none());
    event.change = ReactionChange::Set(Some("+1".into()));
    assert!(event.feedback_for(&reply).is_some());
    event.target.conversation.installation = "foreign".into();
    assert!(event.feedback_for(&reply).is_none());
    let mut action = ActionEvent {
        conversation: target.conversation,
        actor: actor.clone(),
        id: "feedback".into(),
        value: "good".into(),
        message_id: Some("reply".into()),
    };
    let feedback = action.feedback_for(&reply, "good", "bad").unwrap();
    assert_eq!(feedback.actor(), &actor);
    assert!(matches!(feedback.source, FeedbackSource::Action(_)));
    action.value = "bad".into();
    assert_eq!(
        action.feedback_for(&reply, "good", "bad").unwrap().value,
        Some(FeedbackValue::Negative)
    );
    action.value = "unknown".into();
    assert!(action.feedback_for(&reply, "good", "bad").is_none());
    action.id = "good".into();
    assert!(action.feedback_for(&reply, "good", "bad").is_some());
    action.message_id = Some("other".into());
    assert!(action.feedback_for(&reply, "good", "bad").is_none());
    action.message_id = None;
    assert!(action.feedback_for(&reply, "good", "bad").is_none());
    assert!(action.feedback_for(&reply, "same", "same").is_none());
    assert!(action.feedback_for(&reply, "", "bad").is_none());
}

#[test]
fn long_markdown_preserves_fenced_blocks_and_unicode_without_oversized_parts() {
    assert_eq!(split_markdown("hello", 512).unwrap(), ["hello"]);
    assert!(split_markdown("x", 511).is_err());
    assert!(split_markdown(&"x".repeat(MAX_OUTPUT_BYTES + 1), 512).is_err());
    for marker in ["```", "~~~~", "    ```"] {
        let source = format!(
            "Intro\n{marker}rust\n{}\n{marker}\nAfter",
            "🙂α\n".repeat(1000)
        );
        let parts = split_markdown(&source, 512).unwrap();
        assert!(parts.len() > 1);
        assert!(parts.iter().all(|part| part.chars().count() <= 512));
        assert_eq!(
            parts
                .iter()
                .map(|part| part.matches('🙂').count())
                .sum::<usize>(),
            1000
        );
        if marker != "    ```" {
            assert!(parts.iter().all(|part| {
                part.lines()
                    .filter(|line| line.trim_start().starts_with(marker))
                    .count()
                    % 2
                    == 0
            }));
        }
    }
    let source = format!("```rust\n{}\n```", "🙂".repeat(2000));
    let parts = split_markdown(&source, 512).unwrap();
    assert!(
        parts
            .iter()
            .all(|part| part.starts_with("```rust\n") && part.ends_with("```"))
    );
    assert_eq!(
        parts
            .iter()
            .map(|part| part.matches('🙂').count())
            .sum::<usize>(),
        2000
    );
    let plain = "α".repeat(2000);
    assert_eq!(split_markdown(&plain, 512).unwrap().concat(), plain);
    let long_header = format!("```{}\n{}", "a".repeat(130), "x".repeat(2000));
    assert!(split_markdown(&long_header, 512).is_err());
    let open = format!("```\n{}", "x\n".repeat(1000));
    assert!(
        split_markdown(&open, 512)
            .unwrap()
            .iter()
            .all(|part| part.ends_with("```"))
    );
    let source = format!(
        "{}\n```rust\ncode\n```\n{}",
        "x".repeat(494),
        "y".repeat(600)
    );
    assert!(
        split_markdown(&source, 512)
            .unwrap()
            .iter()
            .all(|part| part.chars().count() <= 512)
    );
}
