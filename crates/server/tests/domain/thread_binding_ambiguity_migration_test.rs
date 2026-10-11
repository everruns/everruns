//! 212 bound an App-era AG-UI or Public Chat thread whose session had no
//! channel to every channel of that type in the App; 222 removes the bindings
//! that ownership cannot be derived for, keeping the unambiguous ones (EVE-1244).

use crate::test_harness;
use sqlx::PgPool;

const APP_TWO_AG_UI: &str = "00000000-0000-0000-0000-0000000000a1";
const APP_ONE_EACH: &str = "00000000-0000-0000-0000-0000000000a2";
const APP_TWO_CHAT: &str = "00000000-0000-0000-0000-0000000000a3";

const AG_UI_A: &str = "00000000-0000-0000-0000-0000000000c1";
const AG_UI_B: &str = "00000000-0000-0000-0000-0000000000c2";
const AG_UI_SOLE: &str = "00000000-0000-0000-0000-0000000000c3";
const CHAT_SOLE: &str = "00000000-0000-0000-0000-0000000000c4";
const CHAT_A: &str = "00000000-0000-0000-0000-0000000000c5";
const CHAT_B: &str = "00000000-0000-0000-0000-0000000000c6";

const AMBIGUOUS_AG_UI: &str = "00000000-0000-0000-0000-0000000000e1";
const ATTRIBUTED_AG_UI: &str = "00000000-0000-0000-0000-0000000000e2";
const SOLE_AG_UI: &str = "00000000-0000-0000-0000-0000000000e3";
const SOLE_CHAT: &str = "00000000-0000-0000-0000-0000000000e4";
const AMBIGUOUS_CHAT: &str = "00000000-0000-0000-0000-0000000000e5";

async fn bindings(tx: &mut sqlx::PgConnection, session: &str) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT channel_id::text, binding_key FROM channel_bindings
         WHERE session_id = $1::uuid ORDER BY channel_id, binding_key",
    )
    .bind(session)
    .fetch_all(tx)
    .await
    .unwrap()
}

#[tokio::test]
async fn ambiguous_app_era_threads_are_not_bound_to_every_channel_of_their_app() {
    let pool = PgPool::connect(&test_harness::get_database_url())
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    let schema = format!("thread_binding_{}", uuid::Uuid::new_v4().simple());
    let setup = format!(
        "CREATE SCHEMA {schema}; SET LOCAL search_path TO {schema};
         CREATE TABLE agent_channels (
             id UUID PRIMARY KEY, public_id TEXT, app_id UUID, channel_type TEXT
         );
         CREATE TABLE sessions (
             id UUID PRIMARY KEY, channel_id UUID REFERENCES agent_channels(id),
             app_id UUID, tags TEXT[], created_at TIMESTAMPTZ DEFAULT NOW()
         );
         CREATE TABLE channel_bindings (
             channel_id UUID NOT NULL REFERENCES agent_channels(id) ON DELETE CASCADE,
             binding_key TEXT NOT NULL,
             session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
             PRIMARY KEY (channel_id, binding_key)
         );
         INSERT INTO agent_channels VALUES
             ('{AG_UI_A}', 'appchan_a', '{APP_TWO_AG_UI}', 'ag_ui'),
             ('{AG_UI_B}', 'appchan_b', '{APP_TWO_AG_UI}', 'ag_ui'),
             ('{AG_UI_SOLE}', 'appchan_sole', '{APP_ONE_EACH}', 'ag_ui'),
             ('{CHAT_SOLE}', 'appchan_chat', '{APP_ONE_EACH}', 'public_chat'),
             ('{CHAT_A}', 'appchan_chat_a', '{APP_TWO_CHAT}', 'public_chat'),
             ('{CHAT_B}', 'appchan_chat_b', '{APP_TWO_CHAT}', 'public_chat');
         INSERT INTO sessions (id, channel_id, app_id, tags) VALUES
             -- Two AG-UI doors, no channel: the App alias names neither.
             ('{AMBIGUOUS_AG_UI}', NULL, '{APP_TWO_AG_UI}',
              ARRAY['ag_ui:app:app_two', 'ag_ui:thread:legacy']),
             -- Explicitly attributed to A: stays A's alone.
             ('{ATTRIBUTED_AG_UI}', '{AG_UI_A}', '{APP_TWO_AG_UI}',
              ARRAY['ag_ui:app:app_two', 'ag_ui:thread:attributed']),
             -- One AG-UI door (beside a Public Chat one): unambiguous.
             ('{SOLE_AG_UI}', NULL, '{APP_ONE_EACH}',
              ARRAY['ag_ui:app:app_one', 'ag_ui:thread:sole']),
             ('{SOLE_CHAT}', NULL, '{APP_ONE_EACH}',
              ARRAY['public_chat:app:app_one', 'public_chat:thread:t',
                    'public_chat:visitor:anonymous:v']),
             ('{AMBIGUOUS_CHAT}', NULL, '{APP_TWO_CHAT}',
              ARRAY['public_chat:app:app_chat', 'public_chat:thread:t',
                    'public_chat:visitor:anonymous:v']);"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(setup.as_str()))
        .execute(&mut *tx)
        .await
        .unwrap();

    sqlx::raw_sql(include_str!(
        "../../migrations/212_thread_channel_bindings.sql"
    ))
    .execute(&mut *tx)
    .await
    .unwrap();
    // The defect: 212 alone fans the ambiguous session out to both channels.
    assert_eq!(bindings(&mut tx, AMBIGUOUS_AG_UI).await.len(), 2);
    assert_eq!(bindings(&mut tx, AMBIGUOUS_CHAT).await.len(), 2);

    sqlx::raw_sql(include_str!(
        "../../migrations/222_unbind_ambiguous_app_era_threads.sql"
    ))
    .execute(&mut *tx)
    .await
    .unwrap();

    assert!(bindings(&mut tx, AMBIGUOUS_AG_UI).await.is_empty());
    assert!(bindings(&mut tx, AMBIGUOUS_CHAT).await.is_empty());
    assert_eq!(
        bindings(&mut tx, ATTRIBUTED_AG_UI).await,
        vec![(AG_UI_A.to_string(), "thread:attributed".to_string())]
    );
    assert_eq!(
        bindings(&mut tx, SOLE_AG_UI).await,
        vec![(AG_UI_SOLE.to_string(), "thread:sole".to_string())]
    );
    assert_eq!(
        bindings(&mut tx, SOLE_CHAT).await,
        vec![(
            CHAT_SOLE.to_string(),
            "thread:t|public_chat:visitor:anonymous:v".to_string()
        )]
    );

    // Deploy-safe: rerunning the repair changes nothing.
    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM channel_bindings")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!(
        "../../migrations/222_unbind_ambiguous_app_era_threads.sql"
    ))
    .execute(&mut *tx)
    .await
    .unwrap();
    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM channel_bindings")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(before, after);

    tx.rollback().await.unwrap();
}
