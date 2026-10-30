//! Image and file artifact resolution.
//!
//! Resolution answers with metadata and a signed HTTP link
//! (`api::internal_blobs`); workers fetch the bytes over HTTP, never over
//! gRPC, and upload generated images the same way.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.

use crate::api::internal_blobs::BlobKind;
use crate::worker_link::grpc_service::*;

fn validate_file_resolution_limits(file_count: usize, total_bytes: usize) -> Result<(), Status> {
    if file_count > crate::api::files::MAX_FILES_PER_RESOLUTION {
        return Err(Status::resource_exhausted(format!(
            "At most {} files can be resolved at once",
            crate::api::files::MAX_FILES_PER_RESOLUTION
        )));
    }
    if total_bytes > crate::api::files::MAX_FILE_SIZE_BYTES {
        return Err(Status::resource_exhausted(format!(
            "Combined file size exceeds {} bytes",
            crate::api::files::MAX_FILE_SIZE_BYTES
        )));
    }
    Ok(())
}

impl WorkerServiceImpl {
    pub(crate) async fn handle_resolve_image(
        &self,
        request: Request<ResolveImageRequest>,
    ) -> Result<Response<ResolveImageResponse>, Status> {
        let req = request.into_inner();
        let image_id = parse_uuid(req.image_id.as_ref())?;

        // Metadata only: the worker fetches the bytes from the signed URL.
        let info = self
            .db
            .get_image_info(req.org_id, image_id)
            .await
            .map_err(|e| {
                tracing::error!(%image_id, error = %e, "Failed to get image");
                Status::internal("Failed to get image")
            })?;
        let Some(info) = info else {
            return Ok(Response::new(ResolveImageResponse::default()));
        };

        Ok(Response::new(ResolveImageResponse {
            found: true,
            media_type: info.content_type,
            url: self.blob_url(BlobKind::Image, image_id, req.org_id)?,
        }))
    }

    pub(crate) async fn handle_resolve_images(
        &self,
        request: Request<ResolveImagesRequest>,
    ) -> Result<Response<ResolveImagesResponse>, Status> {
        let req = request.into_inner();

        let mut images = std::collections::HashMap::new();
        for proto_id in req.image_ids {
            let image_id = parse_uuid(Some(&proto_id))?;
            match self.db.get_image_info(req.org_id, image_id).await {
                Ok(Some(info)) => {
                    images.insert(
                        image_id.to_string(),
                        ResolvedImageData {
                            media_type: info.content_type,
                            url: self.blob_url(BlobKind::Image, image_id, req.org_id)?,
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

        Ok(Response::new(ResolveImagesResponse { images }))
    }

    pub(crate) async fn handle_resolve_files(
        &self,
        request: Request<ResolveFilesRequest>,
    ) -> Result<Response<ResolveFilesResponse>, Status> {
        let req = request.into_inner();
        let file_ids = req
            .file_ids
            .iter()
            .map(|proto_id| parse_uuid(Some(proto_id)))
            .collect::<Result<std::collections::HashSet<_>, _>>()?;
        validate_file_resolution_limits(file_ids.len(), 0)?;

        // The combined size bound still applies: the worker inlines these
        // bytes into one model request.
        let mut total_bytes = 0usize;
        let mut files = std::collections::HashMap::new();
        for file_id in file_ids {
            match self.db.get_file_info(req.org_id, file_id).await {
                Ok(Some(info)) => {
                    let size = usize::try_from(info.size_bytes)
                        .map_err(|_| Status::internal("Invalid stored file size"))?;
                    total_bytes = total_bytes
                        .checked_add(size)
                        .ok_or_else(|| Status::resource_exhausted("File batch is too large"))?;
                    validate_file_resolution_limits(files.len() + 1, total_bytes)?;
                    files.insert(
                        file_id.to_string(),
                        ResolvedFileData {
                            media_type: info.content_type,
                            filename: info.filename.unwrap_or_default(),
                            url: self.blob_url(BlobKind::File, file_id, req.org_id)?,
                            size_bytes: info.size_bytes,
                        },
                    );
                }
                Ok(None) => {
                    tracing::debug!(%file_id, "File not found during batch resolution");
                }
                Err(e) => {
                    tracing::error!(%file_id, error = %e, "Failed to get file metadata during batch resolution");
                    return Err(Status::internal("Failed to resolve file metadata"));
                }
            }
        }

        Ok(Response::new(ResolveFilesResponse { files }))
    }

    pub(crate) async fn handle_get_image_artifact(
        &self,
        request: Request<GetImageArtifactRequest>,
    ) -> Result<Response<GetImageArtifactResponse>, Status> {
        let req = request.into_inner();
        let image_id = parse_uuid(req.image_id.as_ref())?;
        let info = self
            .db
            .get_image_info(req.org_id, image_id)
            .await
            .map_err(|e| {
                tracing::error!(%image_id, error = %e, "Failed to get image artifact");
                Status::internal("Failed to get image artifact")
            })?;
        let Some(info) = info else {
            return Ok(Response::new(GetImageArtifactResponse::default()));
        };

        Ok(Response::new(GetImageArtifactResponse {
            url: self.blob_url(BlobKind::Image, image_id, req.org_id)?,
            info: Some(Self::image_info_row_to_proto(info)),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_resolution_limits_accept_boundaries() {
        validate_file_resolution_limits(
            crate::api::files::MAX_FILES_PER_RESOLUTION,
            crate::api::files::MAX_FILE_SIZE_BYTES,
        )
        .unwrap();
    }

    #[test]
    fn file_resolution_limits_reject_oversized_batches() {
        let too_many =
            validate_file_resolution_limits(crate::api::files::MAX_FILES_PER_RESOLUTION + 1, 0)
                .unwrap_err();
        assert_eq!(too_many.code(), tonic::Code::ResourceExhausted);

        let too_large =
            validate_file_resolution_limits(1, crate::api::files::MAX_FILE_SIZE_BYTES + 1)
                .unwrap_err();
        assert_eq!(too_large.code(), tonic::Code::ResourceExhausted);
    }
}
