// In-memory storage: Images, Skills, Skill Files

use super::super::models::*;
use super::InMemoryDatabase;
use super::matches_search_tokens;
use anyhow::Result;
use anyhow::anyhow;
use everruns_core::{ImageId, McpServerId, SkillId};
use uuid::Uuid;

impl InMemoryDatabase {
    // ============================================
    // Images
    // ============================================

    pub async fn create_image(&self, org_id: i64, input: CreateImageRow) -> Result<ImageRow> {
        let now = Self::now();
        let id = ImageId::new();
        let row = ImageRow {
            id,
            org_id,
            filename: input.filename,
            content_type: input.content_type,
            size_bytes: input.size_bytes,
            data: input.data,
            thumbnail_data: input.thumbnail_data,
            thumbnail_content_type: input.thumbnail_content_type,
            metadata: input.metadata,
            created_at: now,
        };
        self.images.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_image(&self, org_id: i64, id: Uuid) -> Result<Option<ImageRow>> {
        let id = ImageId::from_uuid(id);
        Ok(self
            .images
            .read()
            .get(&id)
            .filter(|img| img.org_id == org_id)
            .cloned())
    }

    pub async fn get_image_info(&self, org_id: i64, id: Uuid) -> Result<Option<ImageInfoRow>> {
        let id = ImageId::from_uuid(id);
        Ok(self
            .images
            .read()
            .get(&id)
            .filter(|img| img.org_id == org_id)
            .map(|img| ImageInfoRow {
                id: img.id,
                org_id: img.org_id,
                filename: img.filename.clone(),
                content_type: img.content_type.clone(),
                size_bytes: img.size_bytes,
                metadata: img.metadata.clone(),
                created_at: img.created_at,
            }))
    }

    pub async fn delete_image(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let id = ImageId::from_uuid(id);
        let mut images = self.images.write();
        if let Some(img) = images.get(&id) {
            if img.org_id != org_id {
                return Ok(false);
            }
        } else {
            return Ok(false);
        }
        Ok(images.remove(&id).is_some())
    }

    pub async fn list_images(
        &self,
        org_id: i64,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ImageInfoRow>> {
        let images = self.images.read();
        let mut result: Vec<_> = images
            .values()
            .filter(|img| img.org_id == org_id)
            .map(|img| ImageInfoRow {
                id: img.id,
                org_id: img.org_id,
                filename: img.filename.clone(),
                content_type: img.content_type.clone(),
                size_bytes: img.size_bytes,
                metadata: img.metadata.clone(),
                created_at: img.created_at,
            })
            .collect();
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        let result = result
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .collect();
        Ok(result)
    }

    pub async fn update_mcp_server_tools(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateMcpServerTools,
    ) -> Result<Option<McpServerRow>> {
        let id = McpServerId::from_uuid(id);
        let mut servers = self.mcp_servers.write();
        if let Some(server) = servers.get_mut(&id) {
            if server.org_id != org_id {
                return Ok(None);
            }
            server.cached_tools = input.cached_tools;
            server.tools_cached_at = Some(Self::now());
            server.updated_at = Self::now();
            return Ok(Some(server.clone()));
        }
        Ok(None)
    }

    // ============================================
    // Skills
    // ============================================

    pub async fn create_skill(&self, org_id: i64, input: CreateSkillRow) -> Result<SkillRow> {
        if self
            .skills
            .read()
            .values()
            .any(|s| s.name == input.name && s.org_id == org_id)
        {
            return Err(anyhow!("Skill with name '{}' already exists", input.name));
        }

        let now = Self::now();
        let id = SkillId::new();

        let row = SkillRow {
            id,
            public_id: input.public_id,
            org_id,
            name: input.name,
            description: input.description,
            license: input.license,
            compatibility: input.compatibility,
            metadata: input.metadata,
            allowed_tools: input.allowed_tools,
            instructions: input.instructions,
            source_type: input.source_type,
            archive_data: input.archive_data,
            status: "active".to_string(),
            version: input.version,
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
        };

        self.skills.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_skill(&self, org_id: i64, id: Uuid) -> Result<Option<SkillRow>> {
        let id = SkillId::from_uuid(id);
        Ok(self
            .skills
            .read()
            .get(&id)
            .filter(|s| s.org_id == org_id)
            .cloned())
    }

    pub async fn get_skill_by_name(&self, org_id: i64, name: &str) -> Result<Option<SkillRow>> {
        Ok(self
            .skills
            .read()
            .values()
            .find(|s| s.org_id == org_id && s.name == name)
            .cloned())
    }

    pub async fn list_skills(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<SkillRow>> {
        let mut skills: Vec<_> = self
            .skills
            .read()
            .values()
            .filter(|s| s.org_id == org_id)
            .filter(|s| {
                if include_archived {
                    s.status != "deleted"
                } else {
                    s.status != "archived" && s.status != "deleted"
                }
            })
            .filter(|s| matches_search_tokens(search, &[&s.name, &s.description]))
            .cloned()
            .collect();
        skills.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(skills)
    }

    pub async fn update_skill(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateSkill,
    ) -> Result<Option<SkillRow>> {
        let id = SkillId::from_uuid(id);
        let mut skills = self.skills.write();
        if let Some(skill) = skills.get_mut(&id) {
            if skill.org_id != org_id {
                return Ok(None);
            }
            if let Some(name) = input.name {
                skill.name = name;
            }
            if let Some(description) = input.description {
                skill.description = description;
            }
            if let Some(license) = input.license {
                skill.license = Some(license);
            }
            if let Some(compatibility) = input.compatibility {
                skill.compatibility = Some(compatibility);
            }
            if let Some(metadata) = input.metadata {
                skill.metadata = metadata;
            }
            if let Some(allowed_tools) = input.allowed_tools {
                skill.allowed_tools = Some(allowed_tools);
            }
            if let Some(instructions) = input.instructions {
                skill.instructions = instructions;
            }
            if let Some(status) = input.status {
                skill.status = status;
            }
            if let Some(version) = input.version {
                skill.version = version;
            }
            if let Some(archive_data) = input.archive_data {
                skill.archive_data = Some(archive_data);
            }
            if let Some(source_type) = input.source_type {
                skill.source_type = source_type;
            }
            skill.updated_at = Self::now();
            return Ok(Some(skill.clone()));
        }
        Ok(None)
    }

    pub async fn delete_skill(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let skill_id = SkillId::from_uuid(id);
        let mut skills = self.skills.write();
        if let Some(skill) = skills.get_mut(&skill_id) {
            if skill.org_id != org_id || !matches!(skill.status.as_str(), "active" | "disabled") {
                return Ok(false);
            }
            skill.status = "archived".to_string();
            skill.archived_at = Some(Self::now());
            skill.updated_at = Self::now();
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn destroy_skill(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let skill_id = SkillId::from_uuid(id);
        let mut skills = self.skills.write();
        if let Some(skill) = skills.get_mut(&skill_id) {
            if skill.org_id != org_id || skill.status != "archived" {
                return Ok(false);
            }
            skill.status = "deleted".to_string();
            skill.deleted_at = Some(Self::now());
            skill.updated_at = Self::now();
            return Ok(true);
        }
        Ok(false)
    }

    // ============================================
    // Skill Files
    // ============================================

    pub async fn create_skill_file(&self, input: CreateSkillFileRow) -> Result<SkillFileRow> {
        let now = Self::now();
        let row = SkillFileRow {
            id: Uuid::now_v7(),
            skill_id: input.skill_id,
            path: input.path,
            content: input.content,
            content_binary: input.content_binary,
            is_binary: input.is_binary,
            size_bytes: input.size_bytes,
            created_at: now,
        };

        self.skill_files.write().push(row.clone());
        Ok(row)
    }

    pub async fn list_skill_files(&self, skill_id: Uuid) -> Result<Vec<SkillFileRow>> {
        let mut files: Vec<_> = self
            .skill_files
            .read()
            .iter()
            .filter(|f| f.skill_id == skill_id)
            .cloned()
            .collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }

    pub async fn delete_skill_files(&self, skill_id: Uuid) -> Result<u64> {
        let mut files = self.skill_files.write();
        let before = files.len();
        files.retain(|f| f.skill_id != skill_id);
        Ok((before - files.len()) as u64)
    }
}
