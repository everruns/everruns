// Per-request command dispatch helpers.
//
// Most handlers only build a Command, run it, and serialize the output.
// `Dispatcher` collapses that boilerplate to a single chained call and gives
// every handler the same shape — making it cheap to add cross-cutting
// concerns later (audit, metrics, request logging) in a single place.
//
// `Command::run` already centralises authorization (it evaluates
// `Command::policy()` against the active `PermissionResolver`). `Dispatcher`
// adds the equivalent centralisation for the HTTP envelope.

use axum::Json;
use axum::http::StatusCode;
use serde::Serialize;

use super::common::{
    ApiResult, ErrorResponse, ListResponse, PaginatedResponse, ResourceUrlable, UrlBuilder,
    WithUrls,
};
use crate::auth::ResolvedOrg;
use crate::domains::common::{Command, Ctx, Paginated};

/// Per-request bundle of the pieces a handler needs to dispatch a Command and
/// shape the HTTP response.
pub struct Dispatcher {
    pub ctx: Ctx,
    pub url_builder: UrlBuilder,
}

impl Dispatcher {
    /// Run `cmd` and serialize its output as JSON.
    pub async fn run<C: Command>(&self, cmd: C) -> ApiResult<C::Output> {
        Ok(Json(cmd.run(&self.ctx).await?))
    }

    /// Run `cmd` and wrap the output with `self_url`/`view_url`/`ui_link`.
    pub async fn run_with_urls<C>(&self, cmd: C) -> ApiResult<WithUrls<C::Output>>
    where
        C: Command,
        C::Output: ResourceUrlable + Serialize,
    {
        let out = cmd.run(&self.ctx).await?;
        Ok(Json(self.url_builder.wrap(out)))
    }

    /// Run `cmd`, serialize output as JSON, return HTTP 201. Use for POST
    /// handlers whose output type does not need URL decoration.
    pub async fn run_created<C: Command>(
        &self,
        cmd: C,
    ) -> Result<(StatusCode, Json<C::Output>), (StatusCode, Json<ErrorResponse>)> {
        let out = cmd.run(&self.ctx).await?;
        Ok((StatusCode::CREATED, Json(out)))
    }

    /// Run `cmd`, wrap with URLs, return as HTTP 201.
    pub async fn run_created_with_urls<C>(
        &self,
        cmd: C,
    ) -> Result<(StatusCode, Json<WithUrls<C::Output>>), (StatusCode, Json<ErrorResponse>)>
    where
        C: Command,
        C::Output: ResourceUrlable + Serialize,
    {
        let out = cmd.run(&self.ctx).await?;
        Ok((StatusCode::CREATED, Json(self.url_builder.wrap(out))))
    }

    /// Run `cmd` and return HTTP 204. Use for DELETE handlers.
    pub async fn run_no_content<C: Command>(
        &self,
        cmd: C,
    ) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
        cmd.run(&self.ctx).await?;
        Ok(StatusCode::NO_CONTENT)
    }

    /// Run `cmd` whose output is `Vec<T>` and wrap as `ListResponse<T>`.
    pub async fn run_list<C, T>(&self, cmd: C) -> ApiResult<ListResponse<T>>
    where
        C: Command<Output = Vec<T>>,
        T: Serialize,
    {
        let items = cmd.run(&self.ctx).await?;
        Ok(Json(ListResponse::new(items)))
    }

    /// Run `cmd` whose output is `Vec<T>` and wrap as
    /// `ListResponse<WithUrls<T>>` with every element URL-decorated.
    pub async fn run_list_with_urls<C, T>(&self, cmd: C) -> ApiResult<ListResponse<WithUrls<T>>>
    where
        C: Command<Output = Vec<T>>,
        T: ResourceUrlable + Serialize,
    {
        let items = cmd.run(&self.ctx).await?;
        Ok(Json(ListResponse::new(items).with_urls(&self.url_builder)))
    }

    /// Run `cmd` whose output is `Paginated<T>` and re-shape to
    /// `PaginatedResponse<WithUrls<T>>` with every element URL-decorated.
    pub async fn run_paginated_with_urls<C, T>(
        &self,
        cmd: C,
    ) -> ApiResult<PaginatedResponse<WithUrls<T>>>
    where
        C: Command<Output = Paginated<T>>,
        T: ResourceUrlable + Serialize,
    {
        let result = cmd.run(&self.ctx).await?;
        Ok(Json(
            PaginatedResponse::new(result.data, result.total, result.offset, result.limit)
                .with_urls(&self.url_builder),
        ))
    }
}

/// AppStates implement this to expose `state.dispatcher(&org)` in handlers.
///
/// Use the [`impl_dispatchable!`] macro for the typical AppState shape: an
/// inherent `ctx(&self, &ResolvedOrg) -> Ctx` method plus an `auth: AuthState`
/// field.
pub trait Dispatchable {
    fn dispatcher(&self, org: &ResolvedOrg) -> Dispatcher;
}

/// Implement [`Dispatchable`] for an AppState that already has `ctx(&org)` and
/// `auth: AuthState`.
macro_rules! impl_dispatchable {
    ($state:ty) => {
        impl crate::api::dispatch::Dispatchable for $state {
            fn dispatcher(
                &self,
                org: &crate::auth::ResolvedOrg,
            ) -> crate::api::dispatch::Dispatcher {
                crate::api::dispatch::Dispatcher {
                    ctx: {
                        let mut ctx = self.ctx(org);
                        // Org-scoped requests use effective flags (system + org opt-in).
                        ctx.feature_flags = org.feature_flags.clone();
                        ctx
                    },
                    url_builder: crate::api::common::UrlBuilder::from_auth_config(
                        &self.auth.config,
                    ),
                }
            }
        }
    };
}

pub(crate) use impl_dispatchable;
