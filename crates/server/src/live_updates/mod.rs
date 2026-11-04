//! Push delivery to clients and workers: session event fan-out (SSE), and the
//! task and notification broadcasters over PostgreSQL LISTEN/NOTIFY or
//! NATS.

// Event delivery abstraction (InMemory / NATS JetStream)
pub mod event_delivery;

// Shared NATS client connection and the NATS-backed task broadcaster
pub mod nats;
pub mod nats_task_notifications;

// Clears cached feature flags and agent/harness rows when their tables change
pub mod cache_invalidation;

// Notification broadcaster for push-based user inbox delivery
pub mod notification_notifications;

// PostgreSQL LISTEN/NOTIFY listener connection guardrails
pub mod pg_listener_config;

// Task notification broadcaster for push-based task notifications
pub mod task_notifications;
