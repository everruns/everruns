//! Daytona API clients generated at build time from the vendored OpenAPI
//! specs in `openapi/daytona/` (see `build.rs` for which operations are
//! generated and how response schemas are relaxed).

#[allow(
    clippy::all,
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    dead_code,
    unused_imports,
    missing_docs,
    rustdoc::all
)]
pub mod management {
    include!(concat!(env!("OUT_DIR"), "/daytona_management.rs"));
}

#[allow(
    clippy::all,
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    dead_code,
    unused_imports,
    missing_docs,
    rustdoc::all
)]
pub mod toolbox {
    include!(concat!(env!("OUT_DIR"), "/daytona_toolbox.rs"));
}
