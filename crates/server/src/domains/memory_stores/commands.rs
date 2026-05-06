use super::types::{
    CreateMemoryStoreRequest, ListMemoriesQuery, ListMemoriesResponse, MemoryResponse,
    MemoryStoreResponse, memory_response, store_response,
};
use super::{MEMORY_STORE_MANAGE, MEMORY_STORE_VIEW};
use crate::domains::common::*;
use crate::storage::models::{CreateMemoryStoreRow, ListMemoriesFilter};
use everruns_core::Policy;
use everruns_core::memory_store::MemoryLimits;
use everruns_core::typed_id::{MemoryId, MemoryStoreId};
use serde::Deserialize;
use utoipa::ToSchema;

fn validate_store_name(name: &str) -> Result<String, CommandError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(CommandError::bad_request(
            "Memory store name cannot be empty",
        ));
    }
    if trimmed.chars().count() > 255 {
        return Err(CommandError::bad_request(
            "Memory store name must be at most 255 characters",
        ));
    }
    Ok(trimmed.to_string())
}

fn parse_store_id(store_id: &str) -> Result<MemoryStoreId, CommandError> {
    store_id
        .parse()
        .map_err(|_| CommandError::not_found("Memory store"))
}

fn parse_memory_id(memory_id: &str) -> Result<MemoryId, CommandError> {
    memory_id
        .parse()
        .map_err(|_| CommandError::not_found("Memory"))
}

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ListMemoryStores;

impl Command for ListMemoryStores {
    type Output = Vec<MemoryStoreResponse>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_memory_stores",
            category: "memory_stores",
            description: "List org-scoped memory stores.",
            method: "GET",
            path: "/v1/memory-stores",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&MEMORY_STORE_VIEW)
    }

    fn output_schema() -> serde_json::Value {
        array_output_schema(output_schema_for::<MemoryStoreResponse>())
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<MemoryStoreResponse>, CommandError> {
        let rows = ctx
            .db
            .list_memory_stores_with_counts(ctx.org_id())
            .await
            .map_err(classify_anyhow)?;

        rows.into_iter()
            .map(|row| {
                let count = row.active_count;
                let store_row = crate::storage::models::MemoryStoreDbRow {
                    id: row.id,
                    org_id: row.org_id,
                    public_id: row.public_id,
                    name: row.name,
                    is_default: row.is_default,
                    created_at: row.created_at,
                    updated_at: row.updated_at,
                };
                store_response(store_row, count).map_err(classify_anyhow)
            })
            .collect()
    }
}

inventory::submit! { CommandDescriptor::of::<ListMemoryStores>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateMemoryStore {
    pub name: String,
    #[serde(default)]
    pub is_default: bool,
}

impl From<CreateMemoryStoreRequest> for CreateMemoryStore {
    fn from(req: CreateMemoryStoreRequest) -> Self {
        Self {
            name: req.name,
            is_default: req.is_default,
        }
    }
}

impl Command for CreateMemoryStore {
    type Output = MemoryStoreResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_memory_store",
            category: "memory_stores",
            description: "Create an org-scoped memory store.",
            method: "POST",
            path: "/v1/memory-stores",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&MEMORY_STORE_MANAGE)
    }

    fn output_schema() -> serde_json::Value {
        output_schema_for::<MemoryStoreResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<MemoryStoreResponse, CommandError> {
        let name = validate_store_name(&self.name)?;

        let existing_count = ctx
            .db
            .count_memory_stores(ctx.org_id())
            .await
            .map_err(classify_anyhow)?;
        if existing_count as usize >= MemoryLimits::MAX_STORES_PER_ORG {
            return Err(CommandError::bad_request(format!(
                "memory store limit reached ({} stores per org)",
                MemoryLimits::MAX_STORES_PER_ORG
            )));
        }

        let public_id = MemoryStoreId::new().to_string();
        let row = ctx
            .db
            .create_memory_store(
                ctx.org_id(),
                CreateMemoryStoreRow {
                    public_id,
                    name,
                    is_default: self.is_default,
                },
            )
            .await
            .map_err(classify_anyhow)?;
        store_response(row, 0).map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<CreateMemoryStore>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetMemoryStore {
    pub store_id: String,
}

impl Command for GetMemoryStore {
    type Output = MemoryStoreResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_memory_store",
            category: "memory_stores",
            description: "Get a memory store by ID.",
            method: "GET",
            path: "/v1/memory-stores/{store_id}",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("store_id")
    }

    fn policy() -> Option<&'static Policy> {
        Some(&MEMORY_STORE_VIEW)
    }

    fn output_schema() -> serde_json::Value {
        output_schema_for::<MemoryStoreResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<MemoryStoreResponse, CommandError> {
        let id = parse_store_id(&self.store_id)?;
        let row = ctx
            .db
            .get_memory_store_by_public_id(ctx.org_id(), &id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Memory store"))?;
        let count = ctx
            .db
            .count_active_memories(row.id)
            .await
            .map_err(classify_anyhow)?;
        store_response(row, count).map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<GetMemoryStore>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListMemoriesCmd {
    pub store_id: String,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub tag: Option<Vec<String>>,
    #[serde(default)]
    pub include_inactive: Option<bool>,
    #[serde(default)]
    pub limit: Option<usize>,
}

impl ListMemoriesCmd {
    pub fn from_query(store_id: String, query: ListMemoriesQuery) -> Self {
        Self {
            store_id,
            query: query.query,
            kind: query.kind,
            tag: query.tag,
            include_inactive: query.include_inactive,
            limit: query.limit,
        }
    }
}

impl Command for ListMemoriesCmd {
    type Output = ListMemoriesResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_memories",
            category: "memory_stores",
            description: "List/search memories in a store.",
            method: "GET",
            path: "/v1/memory-stores/{store_id}/memories",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&MEMORY_STORE_VIEW)
    }

    fn output_schema() -> serde_json::Value {
        output_schema_for::<ListMemoriesResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<ListMemoriesResponse, CommandError> {
        let store_id = parse_store_id(&self.store_id)?;
        let store = ctx
            .db
            .get_memory_store_by_public_id(ctx.org_id(), &store_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Memory store"))?;
        let filter = ListMemoriesFilter {
            query: self.query,
            kind: self.kind,
            tags: self.tag,
            include_inactive: self.include_inactive.unwrap_or(false),
            limit: self.limit.unwrap_or(50),
        };
        let (rows, total) = ctx
            .db
            .list_memories(ctx.org_id(), Some(store.id), filter)
            .await
            .map_err(classify_anyhow)?;
        let data: Vec<MemoryResponse> = rows
            .into_iter()
            .map(|r| memory_response(r).map_err(classify_anyhow))
            .collect::<Result<_, _>>()?;
        Ok(ListMemoriesResponse { data, total })
    }
}

inventory::submit! { CommandDescriptor::of::<ListMemoriesCmd>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct ForgetMemoryCmd {
    pub store_id: String,
    pub memory_id: String,
}

impl Command for ForgetMemoryCmd {
    type Output = ();

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "forget_memory",
            category: "memory_stores",
            description: "Soft-delete (deactivate) a memory.",
            method: "DELETE",
            path: "/v1/memory-stores/{store_id}/memories/{memory_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&MEMORY_STORE_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<(), CommandError> {
        let store_id = parse_store_id(&self.store_id)?;
        let memory_id = parse_memory_id(&self.memory_id)?;
        let store = ctx
            .db
            .get_memory_store_by_public_id(ctx.org_id(), &store_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Memory store"))?;
        let removed = ctx
            .db
            .deactivate_memory(ctx.org_id(), store.id, &memory_id.to_string())
            .await
            .map_err(classify_anyhow)?;
        if removed {
            Ok(())
        } else {
            Err(CommandError::not_found("Memory"))
        }
    }
}

inventory::submit! { CommandDescriptor::of::<ForgetMemoryCmd>() }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageBackend;
    use everruns_core::{Caller, DEFAULT_ORG_ID, OrgRole};
    use std::sync::Arc;

    fn ctx_with_db(org_id: i64, db: Arc<StorageBackend>) -> Ctx {
        Ctx::minimal_for_test(
            Caller {
                org_id,
                org_public_id: everruns_core::organization::org_public_id_from_internal(org_id),
                user_id: None,
                role: OrgRole::Owner,
                is_platform_user: false,
                is_internal: false,
            },
            db,
            None,
        )
    }

    #[tokio::test]
    async fn create_and_list_stores_round_trip() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_db(DEFAULT_ORG_ID, db);

        let created = CreateMemoryStore {
            name: "Team Knowledge".into(),
            is_default: true,
        }
        .run(&ctx)
        .await
        .expect("create store");

        assert!(created.id.to_string().starts_with("mst_"));
        assert!(created.is_default);
        assert_eq!(created.active_memory_count, 0);

        let listed = ListMemoryStores.run(&ctx).await.expect("list stores");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);
    }

    #[tokio::test]
    async fn search_and_forget_via_backend_then_api() {
        use crate::storage::DbMemoryStore;
        use everruns_core::memory_store::{MemoryKind, MemoryQuery, MemoryStoreBackend};

        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_db(DEFAULT_ORG_ID, db.clone());

        let store = CreateMemoryStore {
            name: "Default".into(),
            is_default: true,
        }
        .run(&ctx)
        .await
        .expect("create default store");

        // Use the bridge backend to write memories the way the agent tools do.
        let backend = DbMemoryStore::new(db.clone(), DEFAULT_ORG_ID);
        let store_id = store.id;

        for (content, kind, importance, tags) in [
            (
                "Rust never use unwrap in prod",
                MemoryKind::Correction,
                9u8,
                vec!["rust".to_string(), "errors".to_string()],
            ),
            (
                "User prefers concise replies",
                MemoryKind::Preference,
                6u8,
                vec!["user".to_string()],
            ),
            (
                "OpenAI rate limits reset hourly",
                MemoryKind::Fact,
                4u8,
                vec!["openai".to_string()],
            ),
        ] {
            backend
                .create_memory(store_id, content.into(), Vec::new(), kind, importance, tags)
                .await
                .expect("create memory");
        }

        // Search by keyword.
        let listed = ListMemoriesCmd::from_query(
            store_id.to_string(),
            ListMemoriesQuery {
                query: Some("rust".into()),
                ..Default::default()
            },
        )
        .run(&ctx)
        .await
        .expect("list memories");
        assert_eq!(listed.data.len(), 1);
        assert_eq!(listed.total, 1);
        assert!(listed.data[0].content.contains("Rust"));

        // Filter by kind.
        let listed = ListMemoriesCmd::from_query(
            store_id.to_string(),
            ListMemoriesQuery {
                kind: Some("preference".into()),
                ..Default::default()
            },
        )
        .run(&ctx)
        .await
        .expect("list by kind");
        assert_eq!(listed.data.len(), 1);
        assert_eq!(listed.data[0].kind, "preference");

        // Filter by tag.
        let listed = ListMemoriesCmd::from_query(
            store_id.to_string(),
            ListMemoriesQuery {
                tag: Some(vec!["openai".into()]),
                ..Default::default()
            },
        )
        .run(&ctx)
        .await
        .expect("list by tag");
        assert_eq!(listed.data.len(), 1);
        assert!(listed.data[0].tags.contains(&"openai".to_string()));

        // Recall via the backend trait sees only active memories.
        let (recalled, total) = backend
            .recall(MemoryQuery {
                store_id: Some(store_id),
                query: None,
                tags: None,
                kind: None,
                limit: 50,
            })
            .await
            .expect("recall");
        assert_eq!(total, 3);
        assert_eq!(recalled.len(), 3);
        assert_eq!(recalled[0].importance, 9);

        // Forget via the API; the recalled set drops to 2.
        let target = listed.data[0].id;
        ForgetMemoryCmd {
            store_id: store_id.to_string(),
            memory_id: target.to_string(),
        }
        .run(&ctx)
        .await
        .expect("forget");

        let (after_forget, total) = backend
            .recall(MemoryQuery {
                store_id: Some(store_id),
                query: None,
                tags: None,
                kind: None,
                limit: 50,
            })
            .await
            .expect("recall after forget");
        assert_eq!(total, 2);
        assert!(after_forget.iter().all(|m| m.id != target));
    }

    #[tokio::test]
    async fn memories_isolated_across_orgs() {
        use crate::storage::DbMemoryStore;
        use everruns_core::memory_store::{MemoryKind, MemoryQuery, MemoryStoreBackend};

        let db = Arc::new(StorageBackend::in_memory());
        let org_a = ctx_with_db(1, db.clone());
        let org_b = ctx_with_db(2, db.clone());

        let store_a = CreateMemoryStore {
            name: "A".into(),
            is_default: true,
        }
        .run(&org_a)
        .await
        .expect("create store A");

        let backend_a = DbMemoryStore::new(db.clone(), 1);
        let backend_b = DbMemoryStore::new(db.clone(), 2);

        backend_a
            .create_memory(
                store_a.id,
                "secret-a".into(),
                Vec::new(),
                MemoryKind::Fact,
                5,
                vec![],
            )
            .await
            .expect("create memory");

        // Org B cannot list memories in org A's store.
        let err = ListMemoriesCmd::from_query(store_a.id.to_string(), ListMemoriesQuery::default())
            .run(&org_b)
            .await
            .expect_err("org B should not see store A");
        assert!(matches!(err, CommandError::NotFound(_)));

        // Org B's backend recall returns nothing for org A's store_id.
        let (recalled, total) = backend_b
            .recall(MemoryQuery {
                store_id: Some(store_a.id),
                query: None,
                tags: None,
                kind: None,
                limit: 50,
            })
            .await
            .expect("recall");
        assert_eq!(total, 0);
        assert!(recalled.is_empty());
    }

    #[tokio::test]
    async fn stores_isolated_across_orgs() {
        let db = Arc::new(StorageBackend::in_memory());
        let org_a = ctx_with_db(1, db.clone());
        let org_b = ctx_with_db(2, db);

        let created = CreateMemoryStore {
            name: "Private".into(),
            is_default: false,
        }
        .run(&org_a)
        .await
        .expect("create store");

        // Other org cannot read it.
        let err = GetMemoryStore {
            store_id: created.id.to_string(),
        }
        .run(&org_b)
        .await
        .expect_err("other org should not see the store");
        assert!(matches!(err, CommandError::NotFound(_)));

        // ListMemoryStores in other org should be empty.
        let listed = ListMemoryStores.run(&org_b).await.expect("list stores");
        assert!(listed.is_empty());
    }
}
