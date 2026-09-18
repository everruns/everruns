//! Session filesystem operations.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.

use crate::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_session_read_file(
        &self,
        request: Request<SessionReadFileRequest>,
    ) -> Result<Response<SessionReadFileResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        // Read file via WorkspaceFileService
        let file = self
            .session_file_service
            .read_file(session_id, &req.path)
            .await
            .map_err(|e| {
                tracing::error!("Failed to read file: {}", e);
                Status::internal("Failed to read file")
            })?;

        use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

        let proto_file = file.map(|f| proto::SessionFile {
            id: Some(uuid_to_proto_uuid(f.id)),
            session_id: Some(uuid_to_proto_uuid(f.session_id)),
            path: f.path.clone(),
            name: f.name.clone(),
            content: f.content,
            encoding: f.encoding,
            is_directory: f.is_directory,
            is_readonly: f.is_readonly,
            size_bytes: f.size_bytes,
            created_at: Some(datetime_to_proto_timestamp(f.created_at)),
            updated_at: Some(datetime_to_proto_timestamp(f.updated_at)),
        });

        Ok(Response::new(SessionReadFileResponse { file: proto_file }))
    }

    pub(crate) async fn handle_session_write_file(
        &self,
        request: Request<SessionWriteFileRequest>,
    ) -> Result<Response<SessionWriteFileResponse>, Status> {
        use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        // Check if file already exists
        let existing = self
            .session_file_service
            .read_file(session_id, &req.path)
            .await
            .map_err(|e| {
                tracing::error!("Failed to check file: {}", e);
                Status::internal("Failed to check file")
            })?;

        let file = if existing.is_some() {
            // Update existing file
            let update = UpdateFileInput {
                content: Some(req.content.clone()),
                encoding: None,
                is_readonly: None,
            };
            self.session_file_service
                .update_file(session_id, &req.path, update)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to update file: {}", e);
                    Status::internal("Failed to update file")
                })?
                .ok_or_else(|| Status::internal("File disappeared during update"))?
        } else {
            // Create new file
            let create = CreateFileInput {
                path: req.path.clone(),
                content: Some(req.content.clone()),
                encoding: None,
                is_readonly: None,
            };
            self.session_file_service
                .create_file(session_id, create)
                .await
                .map_err(|e| {
                    let msg = e.to_string();
                    if msg.contains("already exists") {
                        Status::already_exists(msg)
                    } else {
                        tracing::error!("Failed to create file: {}", e);
                        Status::internal("Failed to create file")
                    }
                })?
        };

        let proto_file = proto::SessionFile {
            id: Some(uuid_to_proto_uuid(file.id)),
            session_id: Some(uuid_to_proto_uuid(file.session_id)),
            path: file.path.clone(),
            name: file.name.clone(),
            content: file.content,
            encoding: file.encoding,
            is_directory: file.is_directory,
            is_readonly: file.is_readonly,
            size_bytes: file.size_bytes,
            created_at: Some(datetime_to_proto_timestamp(file.created_at)),
            updated_at: Some(datetime_to_proto_timestamp(file.updated_at)),
        };

        Ok(Response::new(SessionWriteFileResponse {
            file: Some(proto_file),
        }))
    }

    pub(crate) async fn handle_session_write_file_if_content_matches(
        &self,
        request: Request<SessionWriteFileIfContentMatchesRequest>,
    ) -> Result<Response<SessionWriteFileIfContentMatchesResponse>, Status> {
        use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        let file = self
            .session_file_service
            .update_file_if_content_matches(
                session_id,
                &req.path,
                &req.expected_content,
                &req.expected_encoding,
                &req.content,
                &req.encoding,
            )
            .await
            .map_err(|e| {
                tracing::error!("Failed to conditionally update file: {}", e);
                Status::internal("Failed to update file")
            })?;

        let proto_file = file.map(|file| proto::SessionFile {
            id: Some(uuid_to_proto_uuid(file.id)),
            session_id: Some(uuid_to_proto_uuid(file.session_id)),
            path: file.path.clone(),
            name: file.name.clone(),
            content: file.content,
            encoding: file.encoding,
            is_directory: file.is_directory,
            is_readonly: file.is_readonly,
            size_bytes: file.size_bytes,
            created_at: Some(datetime_to_proto_timestamp(file.created_at)),
            updated_at: Some(datetime_to_proto_timestamp(file.updated_at)),
        });

        Ok(Response::new(SessionWriteFileIfContentMatchesResponse {
            file: proto_file,
        }))
    }

    pub(crate) async fn handle_session_delete_file(
        &self,
        request: Request<SessionDeleteFileRequest>,
    ) -> Result<Response<SessionDeleteFileResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        // Delete via WorkspaceFileService
        let deleted = self
            .session_file_service
            .delete(session_id, &req.path, req.recursive)
            .await
            .map_err(|e| {
                tracing::error!("Failed to delete file: {}", e);
                Status::internal("Failed to delete file")
            })?;

        Ok(Response::new(SessionDeleteFileResponse { deleted }))
    }

    pub(crate) async fn handle_session_list_directory(
        &self,
        request: Request<SessionListDirectoryRequest>,
    ) -> Result<Response<SessionListDirectoryResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        // List directory via WorkspaceFileService
        let files = self
            .session_file_service
            .list_directory(session_id, &req.path)
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("not found") || msg.contains("not a directory") {
                    tracing::debug!("Directory not found: {}", req.path);
                    Status::not_found(msg)
                } else {
                    tracing::error!("Failed to list directory: {}", e);
                    Status::internal("Failed to list directory")
                }
            })?;

        use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

        let proto_files: Vec<proto::FileInfo> = files
            .iter()
            .map(|f| proto::FileInfo {
                id: Some(uuid_to_proto_uuid(f.id)),
                session_id: Some(uuid_to_proto_uuid(f.session_id)),
                path: f.path.clone(),
                name: f.name.clone(),
                is_directory: f.is_directory,
                is_readonly: f.is_readonly,
                size_bytes: f.size_bytes,
                created_at: Some(datetime_to_proto_timestamp(f.created_at)),
                updated_at: Some(datetime_to_proto_timestamp(f.updated_at)),
            })
            .collect();

        Ok(Response::new(SessionListDirectoryResponse {
            files: proto_files,
        }))
    }

    pub(crate) async fn handle_session_stat_file(
        &self,
        request: Request<SessionStatFileRequest>,
    ) -> Result<Response<SessionStatFileResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        // Get file stat via WorkspaceFileService
        let stat = self
            .session_file_service
            .stat(session_id, &req.path)
            .await
            .map_err(|e| {
                tracing::error!("Failed to stat file: {}", e);
                Status::internal("Failed to stat file")
            })?;

        use everruns_internal_protocol::datetime_to_proto_timestamp;

        let proto_stat = stat.map(|s| proto::FileStat {
            path: s.path.clone(),
            name: s.name.clone(),
            is_directory: s.is_directory,
            is_readonly: s.is_readonly,
            size_bytes: s.size_bytes,
            created_at: Some(datetime_to_proto_timestamp(s.created_at)),
            updated_at: Some(datetime_to_proto_timestamp(s.updated_at)),
        });

        Ok(Response::new(SessionStatFileResponse { stat: proto_stat }))
    }

    pub(crate) async fn handle_session_grep_files(
        &self,
        request: Request<SessionGrepFilesRequest>,
    ) -> Result<Response<SessionGrepFilesResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        let options = everruns_core::GrepOptions {
            path_pattern: req.path_pattern,
            before_context: req.before_context as usize,
            after_context: req.after_context as usize,
            offset: req.offset as usize,
            limit: req.limit as usize,
            max_bytes: req.max_bytes as usize,
        };
        let grep_result = everruns_core::session_files::SessionFileSystem::grep_files_with_options(
            &self.session_file_service,
            everruns_provider::typed_id::SessionId::from_uuid(session_id),
            &req.pattern,
            &options,
        )
        .await
        .map_err(|e| {
            // Check if it's a regex error
            if e.to_string().contains("regex") {
                return Status::invalid_argument(format!("Invalid regex pattern: {}", e));
            }
            tracing::error!("Failed to grep files: {}", e);
            Status::internal("Failed to grep files")
        })?;

        let matches = grep_result
            .matches
            .into_iter()
            .map(|item| proto::GrepMatch {
                path: item.path,
                line_number: item.line_number as u64,
                line: item.line,
            })
            .collect();
        let blocks = grep_result
            .blocks
            .into_iter()
            .map(|block| proto::GrepContextBlock {
                path: block.path,
                start_line: block.start_line as u64,
                end_line: block.end_line as u64,
                match_line_numbers: block
                    .match_line_numbers
                    .into_iter()
                    .map(|line| line as u64)
                    .collect(),
                lines: block
                    .lines
                    .into_iter()
                    .map(|line| proto::GrepContextLine {
                        line_number: line.line_number as u64,
                        line: line.line,
                        is_match: line.is_match,
                    })
                    .collect(),
            })
            .collect();

        Ok(Response::new(SessionGrepFilesResponse {
            matches,
            blocks,
            total_matches: grep_result.total_matches as u64,
            returned_matches: grep_result.returned_matches as u64,
            bytes_returned: grep_result.bytes_returned as u64,
            bytes_total: grep_result.bytes_total as u64,
            next_offset: grep_result.next_offset.map(|offset| offset as u64),
            byte_truncated: grep_result.byte_truncated,
        }))
    }

    pub(crate) async fn handle_session_create_directory(
        &self,
        request: Request<SessionCreateDirectoryRequest>,
    ) -> Result<Response<SessionCreateDirectoryResponse>, Status> {
        use everruns_internal_protocol::{datetime_to_proto_timestamp, uuid_to_proto_uuid};

        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        // Create directory via WorkspaceFileService
        let create = CreateDirectoryInput {
            path: req.path.clone(),
        };

        let file_info = self
            .session_file_service
            .create_directory(session_id, create)
            .await
            .map_err(|e| {
                // Check if it's a "file exists" error
                if e.to_string().contains("file exists") || e.to_string().contains("A file exists")
                {
                    return Status::already_exists("A file with this path already exists");
                }
                tracing::error!("Failed to create directory: {}", e);
                Status::internal("Failed to create directory")
            })?;

        let proto_file_info = proto::FileInfo {
            id: Some(uuid_to_proto_uuid(file_info.id)),
            session_id: Some(uuid_to_proto_uuid(file_info.session_id)),
            path: file_info.path.clone(),
            name: file_info.name.clone(),
            is_directory: file_info.is_directory,
            is_readonly: file_info.is_readonly,
            size_bytes: file_info.size_bytes,
            created_at: Some(datetime_to_proto_timestamp(file_info.created_at)),
            updated_at: Some(datetime_to_proto_timestamp(file_info.updated_at)),
        };

        Ok(Response::new(SessionCreateDirectoryResponse {
            directory: Some(proto_file_info),
        }))
    }
}
