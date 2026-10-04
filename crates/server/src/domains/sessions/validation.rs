use crate::domains::common::CommandError;

pub(super) fn validation_error(
    error: (
        axum::http::StatusCode,
        axum::Json<crate::api::common::ErrorResponse>,
    ),
) -> CommandError {
    let body = error.1.0;
    let message = body.detail.unwrap_or_else(|| {
        if body.title.is_empty() {
            "Request failed".to_string()
        } else {
            body.title
        }
    });
    match error.0 {
        axum::http::StatusCode::NOT_FOUND => CommandError::not_found_msg(message),
        _ => CommandError::bad_request(message),
    }
}

pub(super) fn limit_validation_error(_: crate::api::validation::ValidationError) -> CommandError {
    CommandError::bad_request(crate::api::validation::VALIDATION_ERROR_MESSAGE)
}
