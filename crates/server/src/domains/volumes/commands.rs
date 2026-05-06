use super::types::{
    CreateVolumeRequest, CreateVolumeRow, ListVolumesQuery, UpdateVolume, UpdateVolumeRequest,
    VolumeResponse, volume_response,
};
use super::{VOLUME_MANAGE, VOLUME_VIEW};
use crate::domains::common::*;
use everruns_core::Policy;
use everruns_core::typed_id::VolumeId;
use everruns_durable::UpdateField;
use serde::Deserialize;
use utoipa::ToSchema;

fn validate_name(name: &str) -> Result<String, CommandError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(CommandError::bad_request("Volume name cannot be empty"));
    }
    if trimmed.chars().count() > 255 {
        return Err(CommandError::bad_request(
            "Volume name must be at most 255 characters",
        ));
    }
    Ok(trimmed.to_string())
}

fn parse_volume_id(volume_id: &str) -> Result<VolumeId, CommandError> {
    volume_id
        .parse()
        .map_err(|_| CommandError::not_found("Volume"))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListVolumes {
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub include_archived: Option<bool>,
}

impl From<ListVolumesQuery> for ListVolumes {
    fn from(query: ListVolumesQuery) -> Self {
        Self {
            search: query.search,
            include_archived: query.include_archived,
        }
    }
}

impl Command for ListVolumes {
    type Output = Vec<VolumeResponse>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_volumes",
            category: "volumes",
            description: "List workspace volumes in the current organization.",
            method: "GET",
            path: "/v1/volumes",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&VOLUME_VIEW)
    }

    fn output_schema() -> serde_json::Value {
        array_output_schema(output_schema_for::<VolumeResponse>())
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<VolumeResponse>, CommandError> {
        let rows = ctx
            .db
            .list_volumes(
                ctx.org_id(),
                self.search.as_deref(),
                self.include_archived.unwrap_or(false),
            )
            .await
            .map_err(classify_anyhow)?;
        rows.into_iter()
            .map(|row| volume_response(row).map_err(classify_anyhow))
            .collect()
    }
}

inventory::submit! { CommandDescriptor::of::<ListVolumes>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateVolume {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

impl From<CreateVolumeRequest> for CreateVolume {
    fn from(request: CreateVolumeRequest) -> Self {
        Self {
            name: request.name,
            description: request.description,
        }
    }
}

impl Command for CreateVolume {
    type Output = VolumeResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_volume",
            category: "volumes",
            description: "Create a workspace volume in the current organization.",
            method: "POST",
            path: "/v1/volumes",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&VOLUME_MANAGE)
    }

    fn output_schema() -> serde_json::Value {
        output_schema_for::<VolumeResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<VolumeResponse, CommandError> {
        let name = validate_name(&self.name)?;
        let input = CreateVolumeRow {
            public_id: VolumeId::new().to_string(),
            name,
            description: self.description,
            owner_principal_id: None,
            resolved_owner_user_id: ctx.caller.user_id,
        };
        let row = ctx
            .db
            .create_volume(ctx.org_id(), input)
            .await
            .map_err(classify_anyhow)?;
        volume_response(row).map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<CreateVolume>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetVolume {
    pub volume_id: String,
}

impl Command for GetVolume {
    type Output = VolumeResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_volume",
            category: "volumes",
            description: "Get a workspace volume by ID.",
            method: "GET",
            path: "/v1/volumes/{volume_id}",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("volume_id")
    }

    fn policy() -> Option<&'static Policy> {
        Some(&VOLUME_VIEW)
    }

    fn output_schema() -> serde_json::Value {
        output_schema_for::<VolumeResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<VolumeResponse, CommandError> {
        let id = parse_volume_id(&self.volume_id)?;
        let row = ctx
            .db
            .get_volume(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Volume"))?;
        volume_response(row).map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<GetVolume>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateVolumeCmd {
    pub volume_id: String,
    #[serde(flatten)]
    pub request: UpdateVolumeRequest,
}

impl Command for UpdateVolumeCmd {
    type Output = VolumeResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_volume",
            category: "volumes",
            description: "Update a workspace volume.",
            method: "PATCH",
            path: "/v1/volumes/{volume_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&VOLUME_MANAGE)
    }

    fn output_schema() -> serde_json::Value {
        output_schema_for::<VolumeResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<VolumeResponse, CommandError> {
        let id = parse_volume_id(&self.volume_id)?;
        let existing = ctx
            .db
            .get_volume(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Volume"))?;
        let name = self
            .request
            .name
            .as_deref()
            .map(validate_name)
            .transpose()?;
        let row = ctx
            .db
            .update_volume(
                ctx.org_id(),
                existing.id,
                UpdateVolume {
                    name,
                    description: match self.request.description {
                        UpdateField::Set(description) => Some(Some(description)),
                        UpdateField::Clear => Some(None),
                        UpdateField::Unchanged => None,
                    },
                    status: None,
                },
            )
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Volume"))?;
        volume_response(row).map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateVolumeCmd>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteVolume {
    pub volume_id: String,
}

impl Command for DeleteVolume {
    type Output = ();

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_volume",
            category: "volumes",
            description: "Archive a workspace volume.",
            method: "DELETE",
            path: "/v1/volumes/{volume_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&VOLUME_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<(), CommandError> {
        let id = parse_volume_id(&self.volume_id)?;
        let existing = ctx
            .db
            .get_volume(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Volume"))?;
        let archived = ctx
            .db
            .archive_volume(ctx.org_id(), existing.id)
            .await
            .map_err(classify_anyhow)?;
        if archived {
            Ok(())
        } else {
            Err(CommandError::not_found("Volume"))
        }
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteVolume>() }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::common::Ctx;
    use crate::storage::StorageBackend;
    use everruns_core::{Caller, DEFAULT_ORG_ID, OrgRole};
    use std::sync::Arc;

    fn ctx_for_org(org_id: i64) -> Ctx {
        Ctx::minimal_for_test(
            Caller {
                org_id,
                org_public_id: everruns_core::organization::org_public_id_from_internal(org_id),
                user_id: None,
                role: OrgRole::Owner,
                is_platform_user: false,
                is_internal: false,
            },
            Arc::new(StorageBackend::in_memory()),
            None,
        )
    }

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
    async fn volume_lifecycle_round_trip() {
        let ctx = ctx_for_org(DEFAULT_ORG_ID);

        let created = CreateVolume {
            name: "Team Memory".into(),
            description: Some("Shared working files".into()),
        }
        .run(&ctx)
        .await
        .expect("create volume");

        assert!(created.id.to_string().starts_with("vol_"));
        assert_eq!(created.name, "Team Memory");
        assert_eq!(created.status, "active");

        let listed = ListVolumes {
            search: Some("memory".into()),
            include_archived: None,
        }
        .run(&ctx)
        .await
        .expect("list volumes");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);

        let updated = UpdateVolumeCmd {
            volume_id: created.id.to_string(),
            request: UpdateVolumeRequest {
                name: Some("Team Files".into()),
                description: UpdateField::Set("Shared files".into()),
            },
        }
        .run(&ctx)
        .await
        .expect("update volume");
        assert_eq!(updated.name, "Team Files");
        assert_eq!(updated.description.as_deref(), Some("Shared files"));

        let cleared = UpdateVolumeCmd {
            volume_id: created.id.to_string(),
            request: UpdateVolumeRequest {
                name: None,
                description: UpdateField::Clear,
            },
        }
        .run(&ctx)
        .await
        .expect("clear volume description");
        assert_eq!(cleared.name, "Team Files");
        assert_eq!(cleared.description, None);

        DeleteVolume {
            volume_id: created.id.to_string(),
        }
        .run(&ctx)
        .await
        .expect("archive volume");

        let active = ListVolumes {
            search: None,
            include_archived: None,
        }
        .run(&ctx)
        .await
        .expect("list active volumes");
        assert!(active.is_empty());

        let archived = ListVolumes {
            search: None,
            include_archived: Some(true),
        }
        .run(&ctx)
        .await
        .expect("list archived volumes");
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].status, "archived");
    }

    #[tokio::test]
    async fn volume_ids_do_not_cross_orgs() {
        let db = Arc::new(StorageBackend::in_memory());
        let org_one = ctx_with_db(1, db.clone());
        let org_two = ctx_with_db(2, db);

        let created = CreateVolume {
            name: "Private".into(),
            description: None,
        }
        .run(&org_one)
        .await
        .expect("create volume");

        let err = GetVolume {
            volume_id: created.id.to_string(),
        }
        .run(&org_two)
        .await
        .expect_err("other org should not read volume");
        assert!(matches!(err, CommandError::NotFound(_)));
    }

    #[tokio::test]
    async fn duplicate_active_volume_names_conflict() {
        let ctx = ctx_for_org(DEFAULT_ORG_ID);
        CreateVolume {
            name: "Research".into(),
            description: None,
        }
        .run(&ctx)
        .await
        .expect("create volume");

        let err = CreateVolume {
            name: "research".into(),
            description: None,
        }
        .run(&ctx)
        .await
        .expect_err("duplicate name should fail");
        assert!(matches!(err, CommandError::Conflict(_)));
    }
}
