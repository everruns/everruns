//! Image and file artifact resolution.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.

use crate::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_resolve_image(
        &self,
        request: Request<ResolveImageRequest>,
    ) -> Result<Response<ResolveImageResponse>, Status> {
        let req = request.into_inner();
        let image_id = parse_uuid(req.image_id.as_ref())?;

        // Prefer presigned URL to avoid sending image data over gRPC
        if let Some(url) = self.presigned_image_url(image_id, req.org_id) {
            // Metadata-only lookup — avoids loading the full image blob
            let media_type = match self.db.get_image_info(req.org_id, image_id).await {
                Ok(Some(info)) => info.content_type,
                Ok(None) => {
                    return Ok(Response::new(ResolveImageResponse {
                        found: false,
                        base64: String::new(),
                        media_type: String::new(),
                        url: String::new(),
                    }));
                }
                Err(e) => {
                    tracing::error!(%image_id, error = %e, "Failed to get image");
                    return Err(Status::internal("Failed to get image"));
                }
            };

            return Ok(Response::new(ResolveImageResponse {
                found: true,
                base64: String::new(),
                media_type,
                url,
            }));
        }

        // Fallback: send base64 over gRPC (when API_BASE_URL is not configured)
        let image_row = match self.db.get_image(req.org_id, image_id).await {
            Ok(Some(row)) => row,
            Ok(None) => {
                return Ok(Response::new(ResolveImageResponse {
                    found: false,
                    base64: String::new(),
                    media_type: String::new(),
                    url: String::new(),
                }));
            }
            Err(e) => {
                tracing::error!(%image_id, error = %e, "Failed to get image");
                return Err(Status::internal("Failed to get image"));
            }
        };

        let base64_data = base64::engine::general_purpose::STANDARD.encode(&image_row.data);

        Ok(Response::new(ResolveImageResponse {
            found: true,
            base64: base64_data,
            media_type: image_row.content_type,
            url: String::new(),
        }))
    }

    pub(crate) async fn handle_resolve_images(
        &self,
        request: Request<ResolveImagesRequest>,
    ) -> Result<Response<ResolveImagesResponse>, Status> {
        let req = request.into_inner();

        let mut images = std::collections::HashMap::new();
        let use_presigned = self.api_base_url.is_some() && self.presign_secret.is_some();

        for proto_id in req.image_ids {
            let image_id = parse_uuid(Some(&proto_id))?;

            if use_presigned {
                // Metadata-only lookup — avoids loading the full image blob
                match self.db.get_image_info(req.org_id, image_id).await {
                    Ok(Some(info)) => {
                        let url = self
                            .presigned_image_url(image_id, req.org_id)
                            .unwrap_or_default();
                        images.insert(
                            image_id.to_string(),
                            ResolvedImageData {
                                base64: String::new(),
                                media_type: info.content_type,
                                url,
                            },
                        );
                    }
                    Ok(None) => {
                        tracing::debug!(%image_id, "Image not found during batch resolution");
                    }
                    Err(e) => {
                        tracing::warn!(%image_id, error = %e, "Failed to get image during batch resolution");
                    }
                }
            } else {
                // Fallback: base64 over gRPC
                match self.db.get_image(req.org_id, image_id).await {
                    Ok(Some(row)) => {
                        let base64_data =
                            base64::engine::general_purpose::STANDARD.encode(&row.data);
                        images.insert(
                            image_id.to_string(),
                            ResolvedImageData {
                                base64: base64_data,
                                media_type: row.content_type,
                                url: String::new(),
                            },
                        );
                    }
                    Ok(None) => {
                        tracing::debug!(%image_id, "Image not found during batch resolution");
                    }
                    Err(e) => {
                        tracing::warn!(%image_id, error = %e, "Failed to get image during batch resolution");
                    }
                }
            }
        }

        Ok(Response::new(ResolveImagesResponse { images }))
    }

    pub(crate) async fn handle_resolve_files(
        &self,
        request: Request<ResolveFilesRequest>,
    ) -> Result<Response<ResolveFilesResponse>, Status> {
        let req = request.into_inner();

        let mut files = std::collections::HashMap::new();

        // Files are always returned inline as base64 (no presigned-URL
        // variant): prompt-attached files are size-capped at upload.
        for proto_id in req.file_ids {
            let file_id = parse_uuid(Some(&proto_id))?;
            match self.db.get_file(req.org_id, file_id).await {
                Ok(Some(row)) => {
                    let base64_data = base64::engine::general_purpose::STANDARD.encode(&row.data);
                    files.insert(
                        file_id.to_string(),
                        ResolvedFileData {
                            base64: base64_data,
                            media_type: row.content_type,
                            filename: row.filename.unwrap_or_default(),
                        },
                    );
                }
                Ok(None) => {
                    tracing::debug!(%file_id, "File not found during batch resolution");
                }
                Err(e) => {
                    tracing::warn!(%file_id, error = %e, "Failed to get file during batch resolution");
                }
            }
        }

        Ok(Response::new(ResolveFilesResponse { files }))
    }

    pub(crate) async fn handle_create_image_artifact(
        &self,
        request: Request<CreateImageArtifactRequest>,
    ) -> Result<Response<CreateImageArtifactResponse>, Status> {
        let req = request.into_inner();
        if req.data.len() > crate::api::images::MAX_IMAGE_SIZE {
            return Err(Status::invalid_argument(format!(
                "Image exceeds maximum size of {}MB",
                crate::api::images::MAX_IMAGE_SIZE / (1024 * 1024)
            )));
        }
        let metadata = req
            .metadata
            .as_ref()
            .map(everruns_internal_protocol::proto_struct_to_json)
            .unwrap_or_else(|| serde_json::json!({}));
        let (thumbnail_data, thumbnail_content_type) =
            crate::api::images::generate_thumbnail(&req.data, &req.content_type)
                .map(|(data, content_type)| (Some(data), Some(content_type)))
                .unwrap_or((None, None));

        let row = self
            .db
            .create_image(
                req.org_id,
                crate::storage::models::CreateImageRow {
                    org_id: req.org_id,
                    filename: req.filename,
                    content_type: req.content_type,
                    size_bytes: req.data.len() as i64,
                    data: req.data,
                    thumbnail_data,
                    thumbnail_content_type,
                    metadata,
                },
            )
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Failed to create image artifact");
                Status::internal("Failed to create image artifact")
            })?;

        Ok(Response::new(CreateImageArtifactResponse {
            image: Some(Self::image_info_row_to_proto(
                crate::storage::models::ImageInfoRow {
                    id: row.id,
                    org_id: row.org_id,
                    filename: row.filename,
                    content_type: row.content_type,
                    size_bytes: row.size_bytes,
                    metadata: row.metadata,
                    created_at: row.created_at,
                },
            )),
        }))
    }

    pub(crate) async fn handle_get_image_artifact(
        &self,
        request: Request<GetImageArtifactRequest>,
    ) -> Result<Response<GetImageArtifactResponse>, Status> {
        let req = request.into_inner();
        let image_id = parse_uuid(req.image_id.as_ref())?;
        let row = self.db.get_image(req.org_id, image_id).await.map_err(|e| {
            tracing::error!(%image_id, error = %e, "Failed to get image artifact");
            Status::internal("Failed to get image artifact")
        })?;

        Ok(Response::new(GetImageArtifactResponse {
            image: row.map(Self::image_row_to_proto),
        }))
    }

    pub(crate) async fn handle_get_image_artifact_info(
        &self,
        request: Request<GetImageArtifactInfoRequest>,
    ) -> Result<Response<GetImageArtifactInfoResponse>, Status> {
        let req = request.into_inner();
        let image_id = parse_uuid(req.image_id.as_ref())?;
        let row = self
            .db
            .get_image_info(req.org_id, image_id)
            .await
            .map_err(|e| {
                tracing::error!(%image_id, error = %e, "Failed to get image artifact info");
                Status::internal("Failed to get image artifact info")
            })?;

        Ok(Response::new(GetImageArtifactInfoResponse {
            image: row.map(Self::image_info_row_to_proto),
        }))
    }
}
