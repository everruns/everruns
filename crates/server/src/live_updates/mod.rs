//! Push delivery to clients and workers: session event fan-out (SSE), and the
//! task, event and notification broadcasters over PostgreSQL LISTEN/NOTIFY or
//! NATS.

// Event delivery abstraction (InMemory / NATS JetStream)
pub mod event_delivery;

// Event notification broadcaster for push-based SSE delivery (legacy PG NOTIFY)
pub mod event_notifications;

// Shared NATS client connection and the NATS-backed task broadcaster
pub mod nats;
pub mod nats_task_notifications;

// Notification broadcaster for push-based user inbox delivery
pub mod notification_notifications;

// PostgreSQL LISTEN/NOTIFY listener connection guardrails
pub mod pg_listener_config;

// Task notification broadcaster for push-based task notifications
pub mod task_notifications;
