// Generic HTTP adapter for domain commands declared with `#[command(http = ..)]`.
//
// Decision: a command that declares `http = <mode>` is served by one generic
// axum handler instead of a hand-written one. The handler merges path
// parameters, query string and JSON body into one params object, coerces
// textual scalars against the command's param schema (the same coercion MCP
// uses), deserializes the command and runs it through `Dispatcher`, so policy,
// feature flags, metrics and error mapping stay on the shared chokepoints.
//
// Decision: the path is written once, in `CommandMeta::path`. The route, the
// catalog entry and the OpenAPI operation all read it. The OpenAPI operation
// is generated next to the command and added to the document at runtime via
// inventory (`add_command_paths`), so a command cannot ship an HTTP route that
// the spec does not describe.
//
// Handlers that do real transport work (multipart, streaming, custom headers)
// stay hand-written; they simply omit `http = ..`.

use std::collections::BTreeMap;
use std::future::Future;

use axum::{
    Router,
    body::Bytes,
    extract::{FromRef, Query, RawPathParams, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{MethodFilter, on},
};
use serde::Serialize;

use super::common::{ErrorResponse, ResourceUrlable};
use super::dispatch::{Dispatchable, Dispatcher};
use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, CommandError, Paginated};

/// A command served over REST by the generic handler. Implemented by
/// `#[command(http = <mode>)]`; `Respond` is the response shape.
pub trait HttpCommand: Command {
    type Respond: Respond<Self>;
}

/// Shapes a command's output into an HTTP response.
pub trait Respond<C: Command> {
    fn respond(dispatcher: Dispatcher, cmd: C) -> impl Future<Output = Response> + Send;
}

/// Response shapes, named as in `#[command(http = <mode>)]`.
pub mod respond {
    use super::*;

    pub struct Plain;
    pub struct WithUrls;
    pub struct Created;
    pub struct CreatedWithUrls;
    pub struct NoContent;
    pub struct List;
    pub struct ListWithUrls;
    /// A bare JSON array of URL-decorated items, for routes that predate
    /// `ListResponse` and must keep their shape.
    pub struct VecWithUrls;
    pub struct PaginatedWithUrls;

    fn into<T: IntoResponse>(
        result: Result<T, (StatusCode, axum::Json<ErrorResponse>)>,
    ) -> Response {
        match result {
            Ok(ok) => ok.into_response(),
            Err(err) => err.into_response(),
        }
    }

    impl<C: Command> Respond<C> for Plain {
        async fn respond(d: Dispatcher, cmd: C) -> Response {
            into(d.run(cmd).await)
        }
    }

    impl<C> Respond<C> for WithUrls
    where
        C: Command,
        C::Output: ResourceUrlable + Serialize,
    {
        async fn respond(d: Dispatcher, cmd: C) -> Response {
            into(d.run_with_urls(cmd).await)
        }
    }

    impl<C: Command> Respond<C> for Created {
        async fn respond(d: Dispatcher, cmd: C) -> Response {
            into(d.run_created(cmd).await)
        }
    }

    impl<C> Respond<C> for CreatedWithUrls
    where
        C: Command,
        C::Output: ResourceUrlable + Serialize,
    {
        async fn respond(d: Dispatcher, cmd: C) -> Response {
            into(d.run_created_with_urls(cmd).await)
        }
    }

    impl<C: Command> Respond<C> for NoContent {
        async fn respond(d: Dispatcher, cmd: C) -> Response {
            into(d.run_no_content(cmd).await)
        }
    }

    impl<C, T> Respond<C> for List
    where
        C: Command<Output = Vec<T>>,
        T: Serialize + Send,
    {
        async fn respond(d: Dispatcher, cmd: C) -> Response {
            into(d.run_list(cmd).await)
        }
    }

    impl<C, T> Respond<C> for ListWithUrls
    where
        C: Command<Output = Vec<T>>,
        T: ResourceUrlable + Serialize + Send,
    {
        async fn respond(d: Dispatcher, cmd: C) -> Response {
            into(d.run_list_with_urls(cmd).await)
        }
    }

    impl<C, T> Respond<C> for VecWithUrls
    where
        C: Command<Output = Vec<T>>,
        T: ResourceUrlable + Serialize + Send,
    {
        async fn respond(d: Dispatcher, cmd: C) -> Response {
            into(d.run_vec_with_urls(cmd).await)
        }
    }

    impl<C, T> Respond<C> for PaginatedWithUrls
    where
        C: Command<Output = Paginated<T>>,
        T: ResourceUrlable + Serialize + Send,
    {
        async fn respond(d: Dispatcher, cmd: C) -> Response {
            into(d.run_paginated_with_urls(cmd).await)
        }
    }
}

/// Build a command from the request's path parameters, query string and body.
///
/// Body fields come first, then the query, then path parameters, so the URL
/// identity of a resource always wins over a body field of the same name.
/// Path and query values arrive as text and are coerced against the command's
/// param schema (`"true"` for a boolean, `"20"` for an integer).
pub(crate) fn params_from_request<C: Command>(
    path: impl IntoIterator<Item = (String, String)>,
    query: BTreeMap<String, String>,
    body: &[u8],
) -> Result<C, CommandError> {
    let mut text = serde_json::Map::new();
    for (key, value) in query {
        text.insert(key, serde_json::Value::String(value));
    }
    for (key, value) in path {
        text.insert(key, serde_json::Value::String(value));
    }
    let mut text = serde_json::Value::Object(text);
    super::mcp_endpoint::catalog::coerce_json_text_params(
        &<C as Command>::param_schema(),
        &mut text,
    )
    .map_err(CommandError::bad_request)?;
    let serde_json::Value::Object(text) = text else {
        unreachable!("built as an object above");
    };

    let body = if body.iter().all(u8::is_ascii_whitespace) {
        serde_json::Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_slice::<serde_json::Value>(body)
            .map_err(|e| CommandError::bad_request(format!("Invalid JSON body: {e}")))?
    };
    let params = match body {
        serde_json::Value::Object(mut fields) => {
            fields.extend(text);
            serde_json::Value::Object(fields)
        }
        other if text.is_empty() => other,
        _ => {
            return Err(CommandError::bad_request(
                "Request body must be a JSON object",
            ));
        }
    };

    serde_json::from_value(params)
        .map_err(|e| CommandError::unprocessable(format!("Invalid request: {e}")))
}

async fn handle<S, C>(
    State(state): State<S>,
    org: ResolvedOrg,
    path: RawPathParams,
    Query(query): Query<BTreeMap<String, String>>,
    body: Bytes,
) -> Response
where
    S: Dispatchable + Clone + Send + Sync + 'static,
    AuthState: FromRef<S>,
    C: HttpCommand,
{
    let path = path
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()));
    let cmd = match params_from_request::<C>(path, query, &body) {
        Ok(cmd) => cmd,
        Err(err) => return <(StatusCode, axum::Json<ErrorResponse>)>::from(err).into_response(),
    };
    C::Respond::respond(state.dispatcher(&org), cmd).await
}

/// Register commands on a router at their declared method and path.
pub trait CommandRouterExt<S> {
    fn command<C: HttpCommand>(self) -> Self;
}

impl<S> CommandRouterExt<S> for Router<S>
where
    S: Dispatchable + Clone + Send + Sync + 'static,
    AuthState: FromRef<S>,
{
    fn command<C: HttpCommand>(self) -> Self {
        let meta = C::meta();
        let filter = match meta.method {
            "GET" => MethodFilter::GET,
            "POST" => MethodFilter::POST,
            "PUT" => MethodFilter::PUT,
            "PATCH" => MethodFilter::PATCH,
            "DELETE" => MethodFilter::DELETE,
            other => panic!("command {} has unsupported method {other}", meta.name),
        };
        self.route(meta.path, on(filter, handle::<S, C>))
    }
}

/// OpenAPI operation of one `#[command(http = ..)]` command.
pub struct CommandOpenApi {
    add: fn(&mut utoipa::openapi::OpenApi),
}

inventory::collect!(CommandOpenApi);

impl CommandOpenApi {
    pub const fn of<P>() -> Self
    where
        P: utoipa::Path + utoipa::__dev::SchemaReferences + for<'t> utoipa::__dev::Tags<'t>,
    {
        Self {
            add: add_operation::<P>,
        }
    }
}

fn add_operation<P>(openapi: &mut utoipa::openapi::OpenApi)
where
    P: utoipa::Path + utoipa::__dev::SchemaReferences + for<'t> utoipa::__dev::Tags<'t>,
{
    // `#[openapi(paths(..))]` applies a path's tags when it registers it;
    // runtime registration has to do the same.
    let mut operation = P::operation();
    let tags: Vec<String> = <P as utoipa::__dev::Tags>::tags()
        .into_iter()
        .map(str::to_string)
        .collect();
    if !tags.is_empty() {
        operation.tags = Some(tags);
    }
    openapi
        .paths
        .add_path_operation(P::path(), P::methods(), operation);
    let mut schemas = Vec::new();
    P::schemas(&mut schemas);
    let components = openapi
        .components
        .get_or_insert_with(utoipa::openapi::Components::default);
    for (name, schema) in schemas {
        components.schemas.entry(name).or_insert(schema);
    }
}

/// Add every generated command operation to `openapi`.
pub fn add_command_paths(openapi: &mut utoipa::openapi::OpenApi) {
    for entry in inventory::iter::<CommandOpenApi> {
        (entry.add)(openapi);
    }
}

#[cfg(test)]
#[path = "command_http_tests.rs"]
mod tests;
