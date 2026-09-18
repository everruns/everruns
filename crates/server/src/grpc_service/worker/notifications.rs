//! Task notification stream.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.

use super::support::*;
use crate::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_subscribe_task_notifications(
        &self,
        request: Request<SubscribeTaskNotificationsRequest>,
    ) -> Result<Response<TaskNotificationStream>, Status> {
        let req = request.into_inner();
        let worker_id = req.worker_id.clone();
        let activity_types = req.activity_types.clone();

        // Get the broadcaster
        let broadcaster = self
            .task_broadcaster
            .as_ref()
            .ok_or_else(|| Status::unavailable("Task notifications not enabled"))?
            .clone();

        tracing::info!(
            worker_id = %worker_id,
            activity_types = ?activity_types,
            "Worker subscribing to task notifications"
        );

        // Subscribe to notifications
        let subscription = broadcaster
            .subscribe(worker_id.clone(), activity_types.clone())
            .await;

        // Create a channel for the stream
        let (tx, rx) = tokio::sync::mpsc::channel(128);
        let activity_types_set: std::collections::HashSet<String> =
            activity_types.into_iter().collect();

        // Spawn a task to forward notifications to the stream
        let broadcaster_for_cleanup = broadcaster.clone();
        let worker_id_for_cleanup = worker_id.clone();
        tokio::spawn(async move {
            let mut receiver = subscription.receiver;
            let mut heartbeat_interval = tokio::time::interval(std::time::Duration::from_secs(30));

            loop {
                tokio::select! {
                    // Handle incoming task notifications
                    notification = receiver.recv() => {
                        match notification {
                            Ok(payload) => {
                                // Filter by activity type
                                if activity_types_set.contains(&payload.activity_type) {
                                    let proto_notification = TaskNotification {
                                        notification_type: TaskNotificationType::TaskAvailable.into(),
                                        activity_type: payload.activity_type,
                                        pending_count: payload.pending_count,
                                        timestamp: Some(everruns_internal_protocol::datetime_to_proto_timestamp(
                                            chrono::Utc::now(),
                                        )),
                                    };

                                    if tx.send(Ok(proto_notification)).await.is_err() {
                                        // Client disconnected
                                        tracing::debug!(
                                            worker_id = %worker_id,
                                            "Client disconnected from task notification stream"
                                        );
                                        break;
                                    }
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                                // Missed some notifications, but that's OK - worker will poll
                                tracing::warn!(
                                    worker_id = %worker_id,
                                    missed_count = count,
                                    "Task notification stream lagged"
                                );
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                // Broadcaster shut down
                                tracing::info!(
                                    worker_id = %worker_id,
                                    "Task notification broadcaster shut down"
                                );
                                break;
                            }
                        }
                    }

                    // Send periodic heartbeats
                    _ = heartbeat_interval.tick() => {
                        let heartbeat = TaskNotification {
                            notification_type: TaskNotificationType::Heartbeat.into(),
                            activity_type: String::new(),
                            pending_count: 0,
                            timestamp: Some(everruns_internal_protocol::datetime_to_proto_timestamp(
                                chrono::Utc::now(),
                            )),
                        };

                        if tx.send(Ok(heartbeat)).await.is_err() {
                            // Client disconnected
                            tracing::debug!(
                                worker_id = %worker_id,
                                "Client disconnected during heartbeat"
                            );
                            break;
                        }
                    }
                }
            }

            // Clean up subscription
            broadcaster_for_cleanup
                .unsubscribe(&worker_id_for_cleanup)
                .await;
        });

        // Return the receiver as a stream
        let stream = ReceiverStream::new(rx);
        Ok(Response::new(Box::pin(stream)))
    }
}
