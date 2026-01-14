// MCP Server service for business logic

use crate::storage::{
    EncryptionService, McpServerRow, StorageBackend,
    models::{CreateMcpServerRow, UpdateMcpServer},
};
use anyhow::{Result, anyhow};
use everruns_core::{McpServer, McpServerStatus, McpServerTransportType};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use crate::api::mcp_servers::{CreateMcpServerRequest, UpdateMcpServerRequest};

pub struct McpServerService {
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
}

impl McpServerService {
    pub fn new(db: Arc<StorageBackend>, encryption: Option<Arc<EncryptionService>>) -> Self {
        Self { db, encryption }
    }

    pub async fn create(&self, req: CreateMcpServerRequest) -> Result<McpServer> {
        // Encrypt API key if provided
        let api_key_encrypted = if let Some(api_key) = &req.api_key {
            let encryption = self
                .encryption
                .as_ref()
                .ok_or_else(|| anyhow!("Encryption not configured. Cannot store API key."))?;
            Some(encryption.encrypt_string(api_key)?)
        } else {
            None
        };

        let input = CreateMcpServerRow {
            name: req.name,
            description: req.description,
            url: req.url,
            transport_type: req.transport_type.to_string(),
            api_key_encrypted,
            headers: req
                .headers
                .map(|h| serde_json::to_value(h).unwrap_or_default()),
            settings: None,
        };

        let row = self.db.create_mcp_server(input).await?;
        Ok(Self::row_to_mcp_server(&row))
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<McpServer>> {
        let row = self.db.get_mcp_server(id).await?;
        Ok(row.as_ref().map(Self::row_to_mcp_server))
    }

    pub async fn list(&self) -> Result<Vec<McpServer>> {
        let rows = self.db.list_mcp_servers().await?;
        Ok(rows.iter().map(Self::row_to_mcp_server).collect())
    }

    pub async fn update(&self, id: Uuid, req: UpdateMcpServerRequest) -> Result<Option<McpServer>> {
        // Encrypt API key if provided
        let api_key_encrypted = if let Some(api_key) = &req.api_key {
            let encryption = self
                .encryption
                .as_ref()
                .ok_or_else(|| anyhow!("Encryption not configured. Cannot store API key."))?;
            Some(encryption.encrypt_string(api_key)?)
        } else {
            None
        };

        let input = UpdateMcpServer {
            name: req.name,
            description: req.description,
            url: req.url,
            transport_type: req.transport_type.map(|t| t.to_string()),
            status: req.status.map(|s| s.to_string()),
            api_key_encrypted,
            headers: req
                .headers
                .map(|h| serde_json::to_value(h).unwrap_or_default()),
            settings: None,
        };

        let row = self.db.update_mcp_server(id, input).await?;
        Ok(row.as_ref().map(Self::row_to_mcp_server))
    }

    pub async fn delete(&self, id: Uuid) -> Result<bool> {
        self.db.delete_mcp_server(id).await
    }

    fn row_to_mcp_server(row: &McpServerRow) -> McpServer {
        // Parse headers from JSON
        let headers: HashMap<String, String> =
            serde_json::from_value(row.headers.clone()).unwrap_or_default();

        McpServer {
            id: row.id,
            name: row.name.clone(),
            description: row.description.clone(),
            url: row.url.clone(),
            transport_type: McpServerTransportType::from(row.transport_type.as_str()),
            status: McpServerStatus::from(row.status.as_str()),
            api_key_set: row.api_key_set,
            headers,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}
