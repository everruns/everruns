//! Container Sandbox — self-hosted container execution via Docker Engine REST API.
//!
//! Decision: Core crate (not integration) because container execution is infrastructure,
//! not an external service. Same pattern as `crates/session-sqldb/`.
//!
//! Decision: Docker Engine REST API directly, no `docker` CLI binary dependency.
//! Workers can be containerized themselves — talking to the Docker Engine API
//! over HTTP/TCP removes the need for Docker-in-Docker.

pub mod client;
