// Rooms end to end against a real database: people and guests talking,
// threads, reactions, pins, reads, the change feed, and agents being woken and
// answering through `send_message`.

use super::*;
use crate::domains::common::*;
use crate::domains::messages::MessageService;
use crate::domains::rooms::types::*;
use crate::domains::sessions::SessionService;
use crate::live_updates::event_delivery::EventDelivery;
use crate::storage::{CreateAgentRow, CreateHarnessRow, CreateUserRow, StorageBackend};
use async_trait::async_trait;
use everruns_contracts::typed_id::{AgentId, SessionId};
use everruns_core::host::{TurnBackend, TurnRequest, TurnTicket};
use everruns_core::{Caller, DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, OrgRole};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct RecordingRunner {
    sessions: Mutex<Vec<SessionId>>,
}

#[async_trait]
impl TurnBackend for RecordingRunner {
    async fn start_turn(
        &self,
        request: TurnRequest,
    ) -> everruns_contracts::error::Result<TurnTicket> {
        self.sessions.lock().unwrap().push(request.session_id);
        Ok(TurnTicket::new(
            request.session_id,
            request.turn_id,
            std::future::pending(),
        ))
    }

    async fn cancel(&self, _session_id: SessionId) -> everruns_contracts::error::Result<bool> {
        Ok(false)
    }

    async fn is_running(&self, _session_id: SessionId) -> bool {
        false
    }

    async fn active_count(&self) -> usize {
        0
    }
}

async fn seed_agent(db: &Arc<StorageBackend>, name: &str) -> String {
    let harness = db
        .create_harness(
            DEFAULT_ORG_ID,
            CreateHarnessRow {
                localizations: Default::default(),
                name: format!("{name}-harness"),
                display_name: None,
                icon: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
                system_prompt: Some(String::new()),
                parent_harness_id: None,
                default_model_id: None,
                tags: vec![],
                initial_files: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                embedder_metadata: serde_json::json!({}),
                is_built_in: false,
            },
        )
        .await
        .expect("create harness");
    let public_id = AgentId::new().to_string();
    db.create_agent(
        DEFAULT_ORG_ID,
        CreateAgentRow {
            completion_check: Default::default(),
            localizations: Default::default(),
            public_id: public_id.clone(),
            name: name.to_string(),
            display_name: None,
            description: None,
            intro_markdown: None,
            short_description: None,
            starters: serde_json::json!([]),
            system_prompt: String::new(),
            default_model_id: None,
            harness_id: harness.id,
            tags: vec![],
            initial_files: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            max_output_tokens: None,
            communication: Default::default(),
            environments: None,
            is_built_in: false,
        },
    )
    .await
    .expect("create agent");
    public_id
}

async fn seed_person(db: &Arc<StorageBackend>, name: &str, role: &str) -> uuid::Uuid {
    let user = db
        .create_user(CreateUserRow {
            email: format!("{}@example.test", name.to_lowercase().replace(' ', ".")),
            name: name.into(),
            avatar_url: None,
            roles: vec!["user".into()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .expect("create user");
    db.ensure_membership(user.id, DEFAULT_ORG_ID, role)
        .await
        .expect("join org");
    user.id
}

fn caller(user_id: Option<uuid::Uuid>, role: OrgRole) -> Caller {
    Caller {
        org_id: DEFAULT_ORG_ID,
        org_public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
        user_id,
        role,
        is_platform_user: false,
        is_internal: false,
        read_only: false,
    }
}

fn ctx(db: &Arc<StorageBackend>, caller: Caller) -> Ctx {
    Ctx::minimal_for_test(caller, db.clone(), None)
}

/// A context whose posts wake agents, and the runner that records turns.
fn waking_ctx(db: &Arc<StorageBackend>, caller: Caller) -> (Ctx, Arc<RecordingRunner>) {
    let runner = Arc::new(RecordingRunner::default());
    let ctx = Ctx::minimal_for_test(caller, db.clone(), None)
        .with_session_service(Arc::new(SessionService::new(db.clone())))
        .with_message_service(Arc::new(MessageService::new(
            db.clone(),
            runner.clone(),
            EventDelivery::in_memory(),
        )));
    (ctx, runner)
}

fn guest(name: &str) -> RoomMemberInput {
    RoomMemberInput {
        kind: RoomMemberKind::Guest,
        user_id: None,
        agent_id: None,
        handle: None,
        display_name: Some(name.into()),
        role: None,
        wake: None,
        session_scope: None,
        instructions: None,
    }
}

fn agent_member(agent_id: &str, wake: WakeMode) -> RoomMemberInput {
    RoomMemberInput {
        kind: RoomMemberKind::Agent,
        user_id: None,
        agent_id: Some(agent_id.into()),
        handle: None,
        display_name: None,
        role: None,
        wake: Some(wake),
        session_scope: None,
        instructions: None,
    }
}

fn create(name: &str, members: Vec<RoomMemberInput>) -> CreateRoom {
    CreateRoom {
        name: name.into(),
        topic: None,
        external_ref: None,
        metadata: None,
        link_template: None,
        loop_limits: None,
        members,
    }
}

async fn members_by_handle(ctx: &Ctx, room_id: &str) -> std::collections::HashMap<String, String> {
    ListRoomMembers {
        room_id: room_id.into(),
        include_left: None,
    }
    .run(ctx)
    .await
    .unwrap()
    .data
    .into_iter()
    .map(|member| (member.handle, member.id))
    .collect()
}

async fn post(
    ctx: &Ctx,
    room_id: &str,
    text: &str,
    reply_to: Option<&str>,
    as_member: Option<&str>,
) -> PostRoomMessageResponse {
    PostRoomMessage {
        room_id: room_id.into(),
        text: text.into(),
        reply_to: reply_to.map(Into::into),
        as_member: as_member.map(Into::into),
    }
    .run(ctx)
    .await
    .expect("post")
}

use super::rules::WakeMode;

#[tokio::test]
async fn a_builder_runs_a_room_for_its_guests() {
    let db = Arc::new(StorageBackend::test_database());
    // An org API key: no user, admin role, acting for guests.
    let app = ctx(&db, caller(None, OrgRole::Admin));
    let room = CreateRoom {
        external_ref: Some("ticket-42".into()),
        link_template: Some("https://help.example.test/t/{room_id}#{message_id}".into()),
        ..create("Ticket 42", vec![guest("Ana Lopez"), guest("Bo")])
    }
    .run(&app)
    .await
    .expect("create room");
    let handles = members_by_handle(&app, &room.id).await;
    assert_eq!(handles.len(), 2, "an API key does not join its own room");
    let ana = &handles["ana-lopez"];
    let bo = &handles["bo"];

    // Without as_member a userless credential cannot post.
    let error = PostRoomMessage {
        room_id: room.id.clone(),
        text: "hello".into(),
        reply_to: None,
        as_member: None,
    }
    .run(&app)
    .await
    .unwrap_err();
    assert!(error.to_string().contains("as_member"), "{error}");

    let top = post(
        &app,
        &room.id,
        "@bo can you look? mail me at a@b.test",
        None,
        Some(ana),
    )
    .await;
    assert!(top.woke.is_empty());
    let top = top.message;
    assert_eq!(top.author.as_ref().unwrap().handle, "ana-lopez");
    assert_eq!(top.mentions.len(), 1);
    assert_eq!(top.mentions[0].handle, "bo");
    assert_eq!(
        top.url.as_deref(),
        Some(format!("https://help.example.test/t/{}#{}", room.id, top.id).as_str())
    );

    // A reply to a reply lands in the same thread.
    let reply = post(&app, &room.id, "On it", Some(&top.id), Some(bo))
        .await
        .message;
    let nested = post(&app, &room.id, "Thanks", Some(&reply.id), Some(ana))
        .await
        .message;
    assert_eq!(reply.thread_id.as_deref(), Some(top.id.as_str()));
    assert_eq!(nested.thread_id.as_deref(), Some(top.id.as_str()));
    let root = GetRoomMessage {
        room_id: room.id.clone(),
        message_id: top.id.clone(),
    }
    .run(&app)
    .await
    .unwrap();
    assert_eq!(root.reply_count, 2);

    let replies = ListRoomReplies {
        room_id: room.id.clone(),
        message_id: top.id.clone(),
        before: None,
        after: None,
        limit: None,
    }
    .run(&app)
    .await
    .unwrap();
    assert_eq!(
        replies
            .data
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>(),
        vec!["On it", "Thanks"]
    );
    let top_level = ListRoomMessages {
        room_id: room.id.clone(),
        before: None,
        after: None,
        limit: None,
    }
    .run(&app)
    .await
    .unwrap();
    assert_eq!(top_level.data.len(), 1, "replies stay out of the top level");

    // Reactions group by emoji; removing one takes only that member's.
    for member in [ana, bo] {
        ReactToRoomMessage {
            room_id: room.id.clone(),
            message_id: top.id.clone(),
            emoji: "👍".into(),
            add: true,
            as_member: Some(member.clone()),
        }
        .run(&app)
        .await
        .unwrap();
    }
    let reacted = ReactToRoomMessage {
        room_id: room.id.clone(),
        message_id: top.id.clone(),
        emoji: "👍".into(),
        add: false,
        as_member: Some(bo.clone()),
    }
    .run(&app)
    .await
    .unwrap();
    assert_eq!(reacted.reactions.len(), 1);
    assert_eq!(reacted.reactions[0].count, 1);

    // Pins and search.
    PinRoomMessage {
        room_id: room.id.clone(),
        message_id: top.id.clone(),
        pinned: true,
        as_member: Some(bo.clone()),
    }
    .run(&app)
    .await
    .unwrap();
    let pins = ListRoomPins {
        room_id: room.id.clone(),
    }
    .run(&app)
    .await
    .unwrap();
    assert_eq!(pins.data.len(), 1);
    let found = SearchRoomMessages {
        room_id: room.id.clone(),
        q: "ON IT".into(),
        limit: None,
    }
    .run(&app)
    .await
    .unwrap();
    assert_eq!(found.data.len(), 1);
    assert_eq!(found.data[0].id, reply.id);

    // The room is found by its external_ref.
    let by_ref = ListRooms {
        external_ref: Some("ticket-42".into()),
        ..Default::default()
    }
    .run(&app)
    .await
    .unwrap();
    assert_eq!(by_ref.data.len(), 1);
    let duplicate = CreateRoom {
        external_ref: Some("ticket-42".into()),
        ..create("Again", vec![])
    }
    .run(&app)
    .await
    .unwrap_err();
    assert!(
        duplicate.to_string().contains("external_ref"),
        "{duplicate}"
    );

    // The feed lists everything in order and pages on.
    let feed = ListRoomEvents {
        room_id: room.id.clone(),
        after: None,
        limit: None,
    }
    .run(&app)
    .await
    .unwrap();
    let types: Vec<&str> = feed.data.iter().map(|e| e.event_type.as_str()).collect();
    assert_eq!(types[0], "room.created");
    assert_eq!(types.iter().filter(|t| **t == "message.posted").count(), 3);
    assert!(types.contains(&"reaction.removed") && types.contains(&"message.pinned"));
    assert!(feed.data.windows(2).all(|w| w[0].seq < w[1].seq));
    let rest = ListRoomEvents {
        room_id: room.id.clone(),
        after: Some(feed.next_after),
        limit: None,
    }
    .run(&app)
    .await
    .unwrap();
    assert!(rest.data.is_empty());
    assert_eq!(rest.next_after, feed.next_after);

    // Deleting leaves a marker; archiving stops posts.
    let deleted = DeleteRoomMessage {
        room_id: room.id.clone(),
        message_id: reply.id.clone(),
        as_member: Some(bo.clone()),
    }
    .run(&app)
    .await
    .unwrap();
    assert!(deleted.deleted && deleted.text.is_empty());
    ArchiveRoom {
        room_id: room.id.clone(),
        archived: true,
    }
    .run(&app)
    .await
    .unwrap();
    let closed = PostRoomMessage {
        room_id: room.id.clone(),
        text: "late".into(),
        reply_to: None,
        as_member: Some(ana.clone()),
    }
    .run(&app)
    .await
    .unwrap_err();
    assert!(closed.to_string().contains("archived"), "{closed}");
}

#[tokio::test]
async fn people_join_by_posting_and_track_what_they_read() {
    let db = Arc::new(StorageBackend::test_database());
    let ana_id = seed_person(&db, "Ana Lopez", "member").await;
    let bo_id = seed_person(&db, "Bo Chen", "member").await;
    let ana = ctx(&db, caller(Some(ana_id), OrgRole::Member));
    let bo = ctx(&db, caller(Some(bo_id), OrgRole::Member));

    let room = create("Launch", vec![]).run(&ana).await.unwrap();
    let handles = members_by_handle(&ana, &room.id).await;
    assert_eq!(handles.keys().collect::<Vec<_>>(), vec!["ana-lopez"]);

    // Bo is not a member yet, so has nothing to read; posting joins him.
    let not_member = ReadRoomUnread {
        room_id: room.id.clone(),
        limit: None,
        peek: false,
        as_member: None,
    }
    .run(&bo)
    .await
    .unwrap_err();
    assert!(
        not_member.to_string().contains("not a member"),
        "{not_member}"
    );
    let hello = post(&bo, &room.id, "hi all", None, None).await.message;
    assert_eq!(hello.author.as_ref().unwrap().handle, "bo-chen");

    // A member who does not own the room cannot change it or act for others.
    let error = UpdateRoom {
        room_id: room.id.clone(),
        name: Some("Mine".into()),
        topic: None,
        metadata: None,
        link_template: None,
        loop_limits: None,
    }
    .run(&bo)
    .await
    .unwrap_err();
    assert!(error.to_string().contains("owners"), "{error}");
    let handles = members_by_handle(&ana, &room.id).await;
    let error = PostRoomMessage {
        room_id: room.id.clone(),
        text: "spoof".into(),
        reply_to: None,
        as_member: Some(handles["ana-lopez"].clone()),
    }
    .run(&bo)
    .await
    .unwrap_err();
    assert!(error.to_string().contains("org admin"), "{error}");

    // Ana sees Bo's message as unread, once.
    let unread = ReadRoomUnread {
        room_id: room.id.clone(),
        limit: None,
        peek: false,
        as_member: None,
    }
    .run(&ana)
    .await
    .unwrap();
    assert_eq!(unread.data.len(), 1);
    assert_eq!(unread.data[0].id, hello.id);
    let again = ReadRoomUnread {
        room_id: room.id.clone(),
        limit: None,
        peek: false,
        as_member: None,
    }
    .run(&ana)
    .await
    .unwrap();
    assert!(again.data.is_empty());

    // Replies count only in threads you are in; then the root comes along.
    let ana_top = post(&ana, &room.id, "plan?", None, None).await.message;
    post(&bo, &room.id, "unrelated reply", Some(&hello.id), None).await;
    post(&bo, &room.id, "here is the plan", Some(&ana_top.id), None).await;
    let unread = ReadRoomUnread {
        room_id: room.id.clone(),
        limit: None,
        peek: true,
        as_member: None,
    }
    .run(&ana)
    .await
    .unwrap();
    assert_eq!(
        unread
            .data
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>(),
        vec!["here is the plan"]
    );
    assert_eq!(unread.threads.len(), 1);
    assert_eq!(unread.threads[0].id, ana_top.id);

    // Who read Bo's first message: Ana did; nobody else is left.
    let readers = ListRoomMessageReaders {
        room_id: room.id.clone(),
        message_id: hello.id.clone(),
    }
    .run(&ana)
    .await
    .unwrap();
    assert_eq!(
        readers
            .read_by
            .iter()
            .map(|r| r.handle.as_str())
            .collect::<Vec<_>>(),
        vec!["ana-lopez"]
    );
    assert!(readers.not_read_by.is_empty());

    // Bo can leave on his own; his messages stay attributed.
    let bo_member = members_by_handle(&ana, &room.id).await["bo-chen"].clone();
    let left = RemoveRoomMember {
        room_id: room.id.clone(),
        member_id: bo_member,
    }
    .run(&bo)
    .await
    .unwrap();
    assert!(left.left);
    let message = GetRoomMessage {
        room_id: room.id.clone(),
        message_id: hello.id.clone(),
    }
    .run(&ana)
    .await
    .unwrap();
    assert_eq!(message.author.unwrap().handle, "bo-chen");
}

#[tokio::test]
async fn mentioning_an_agent_wakes_it_in_a_thread_session_and_its_send_message_is_posted() {
    let db = Arc::new(StorageBackend::test_database());
    let triage = seed_agent(&db, "Triage Bot").await;
    let notes = seed_agent(&db, "Note Taker").await;
    let (app, runner) = waking_ctx(&db, caller(None, OrgRole::Admin));
    let room = create(
        "Support",
        vec![
            guest("Ana"),
            agent_member(&triage, WakeMode::Mentions),
            agent_member(&notes, WakeMode::Never),
        ],
    )
    .run(&app)
    .await
    .unwrap();
    let handles = members_by_handle(&app, &room.id).await;
    let ana = &handles["ana"];

    // Not addressed: nobody wakes.
    let quiet = post(&app, &room.id, "morning", None, Some(ana)).await;
    assert!(quiet.woke.is_empty());

    // Mentioned: the triage agent wakes; the muted one never does.
    let asked = post(
        &app,
        &room.id,
        "@triage-bot @note-taker the export is broken",
        None,
        Some(ana),
    )
    .await;
    assert_eq!(asked.woke.len(), 1);
    assert_eq!(asked.woke[0].handle, "triage-bot");
    assert_eq!(asked.woke[0].outcome, "woken");
    let session_id: SessionId = asked.woke[0]
        .session_id
        .as_deref()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(runner.sessions.lock().unwrap().as_slice(), &[session_id]);

    // The session is a room session in explicit mode, told who it is.
    let session = db
        .get_session(DEFAULT_ORG_ID, session_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session.source, "room");
    assert!(session.tags.contains(&format!("room:{}", room.id)));
    assert_eq!(
        session
            .hints
            .as_ref()
            .and_then(|hints| hints.get("communication"))
            .and_then(|value| value.as_str()),
        Some("explicit")
    );
    assert!(
        session
            .system_prompt
            .as_deref()
            .unwrap_or_default()
            .contains("You are @triage-bot")
    );

    // A reply in the thread goes to the same session, without a mention.
    let follow = post(
        &app,
        &room.id,
        "it fails on Safari",
        Some(&asked.message.id),
        Some(ana),
    )
    .await;
    // Being mentioned at the root puts the agent in the thread.
    assert_eq!(follow.woke.len(), 1);
    assert_eq!(
        follow.woke[0].session_id.as_deref(),
        Some(session_id.to_string().as_str())
    );

    // The agent answers with send_message: posted as it, in the thread, one
    // hop further, and only once however often the event arrives.
    let posted = listener::post_from_session(
        &db,
        None,
        session_id,
        "Reproduced. @ana I will file it.",
        "message_0001",
    )
    .await
    .unwrap()
    .expect("posted");
    assert_eq!(
        posted.thread_id.map(|id| id.to_string()),
        Some(asked.message.id.clone())
    );
    assert_eq!(posted.hops, 1);
    assert_eq!(posted.session_id, Some(session_id));
    assert_eq!(posted.mentions.len(), 1);
    let again = listener::post_from_session(&db, None, session_id, "Reproduced.", "message_0001")
        .await
        .unwrap();
    assert!(again.is_none(), "a redelivered event is not posted twice");

    // A session that is not a room's posts nothing.
    let none = listener::post_from_session(&db, None, SessionId::new(), "hi", "message_9")
        .await
        .unwrap();
    assert!(none.is_none());

    // The feed recorded the wakes.
    let feed = ListRoomEvents {
        room_id: room.id.clone(),
        after: None,
        limit: None,
    }
    .run(&app)
    .await
    .unwrap();
    assert_eq!(
        feed.data
            .iter()
            .filter(|event| event.event_type == "agent.woken")
            .count(),
        2
    );
}

#[tokio::test]
async fn two_agents_answering_each_other_stop_at_the_hop_limit_and_the_room_is_told() {
    let db = Arc::new(StorageBackend::test_database());
    let alpha = seed_agent(&db, "Alpha").await;
    let beta = seed_agent(&db, "Beta").await;
    let (app, _runner) = waking_ctx(&db, caller(None, OrgRole::Admin));
    let room = CreateRoom {
        loop_limits: Some(super::rules::LoopLimits {
            max_hops: 2,
            ..Default::default()
        }),
        ..create(
            "Debate",
            vec![
                guest("Ana"),
                agent_member(&alpha, WakeMode::Everything),
                agent_member(&beta, WakeMode::Everything),
            ],
        )
    }
    .run(&app)
    .await
    .unwrap();
    assert_eq!(room.loop_limits.max_hops, 2);
    let handles = members_by_handle(&app, &room.id).await;
    let start = post(&app, &room.id, "discuss", None, Some(&handles["ana"])).await;
    assert_eq!(start.woke.len(), 2);
    let alpha_session: SessionId = start
        .woke
        .iter()
        .find(|w| w.handle == "alpha")
        .and_then(|w| w.session_id.as_deref())
        .unwrap()
        .parse()
        .unwrap();
    let beta_session: SessionId = start
        .woke
        .iter()
        .find(|w| w.handle == "beta")
        .and_then(|w| w.session_id.as_deref())
        .unwrap()
        .parse()
        .unwrap();

    let services_db = db.clone();
    let sessions = SessionService::new(services_db.clone());
    let runner = Arc::new(RecordingRunner::default());
    let messages = MessageService::new(services_db, runner.clone(), EventDelivery::in_memory());

    // Alpha answers (hop 1): Beta wakes. Beta answers (hop 2): the limit.
    let first = listener::post_from_session(
        &db,
        Some((&sessions, &messages)),
        alpha_session,
        "I think yes",
        "message_a1",
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(first.hops, 1);
    assert_eq!(runner.sessions.lock().unwrap().as_slice(), &[beta_session]);
    let second = listener::post_from_session(
        &db,
        Some((&sessions, &messages)),
        beta_session,
        "I think no",
        "message_b1",
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(second.hops, 2);
    assert_eq!(
        runner.sessions.lock().unwrap().len(),
        1,
        "nobody is woken past the hop limit"
    );

    let thread = ListRoomReplies {
        room_id: room.id.clone(),
        message_id: start.message.id.clone(),
        before: None,
        after: None,
        limit: None,
    }
    .run(&app)
    .await
    .unwrap();
    let notice = thread
        .data
        .iter()
        .find(|m| m.author.is_none())
        .expect("the room is told why the agent went quiet");
    assert!(
        notice.text.contains("@alpha was not woken"),
        "{}",
        notice.text
    );
}
