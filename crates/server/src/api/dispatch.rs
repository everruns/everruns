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

use super::common::{ApiResult, ErrorResponse, ResourceUrlable, UrlBuilder, WithUrls};
use crate::auth::ResolvedOrg;
use crate::domains::common::{Command, Ctx};

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
                    ctx: self.ctx(org),
                    url_builder: crate::api::common::UrlBuilder::from_auth_config(
                        &self.auth.config,
                    ),
                }
            }
        }
    };
}

pub(crate) use impl_dispatchable;
