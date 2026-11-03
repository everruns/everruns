//! Task notification stream.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.
//!
//! Decision: a subscription ends when the server stops its gRPC listener on
//! shutdown (`crate::shutdown`). The stream never ends on its own, so without
//! this the listener waits out its stop timeout and the process exits with the
//! stream cut. Ending it lets the listener stop at once; the worker polls and
//! resubscribes to another replica.

use super::support::*;
use crate::worker_link::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_subscribe_task_notifications(
        &self,
        request: Request<SubscribeTaskNotificationsRequest>,
    ) -> Result<Response<TaskNotificationStream>, Status> {
        let authority = super::super::authority::resolve(request.extensions())?.clone();
        let db = self.db.clone();
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
        let (tx, rx) =
            tokio::sync::mpsc::channel::<std::result::Result<TaskNotification, Status>>(128);
        let activity_types_set: std::collections::HashSet<String> =
            activity_types.into_iter().collect();

        // Spawn a task to forward notifications to the stream
        let broadcaster_for_cleanup = broadcaster.clone();
        let worker_id_for_cleanup = worker_id.clone();
        let stop = crate::shutdown::server_shutdown().stop_grpc().clone();
        let stop_forwarding = stop.clone();
        tokio::spawn(async move {
            let mut receiver = subscription.receiver;
            let mut heartbeat_interval = tokio::time::interval(std::time::Duration::from_secs(30));

            loop {
                tokio::select! {
                    _ = stop_forwarding.cancelled() => break,

                    // Handle incoming task notifications
                    notification = receiver.recv() => {
                        match notification {
                            Ok(payload) => {
                                // Filter by activity type, and by worker for a
                                // targeted wake-up (sticky sessions).
                                if activity_types_set.contains(&payload.activity_type)
                                    && payload.is_for(&worker_id)
                                {
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
        // Queued notifications and heartbeats must not extend a legacy stream
        // across protection cutover. A failure terminates the stream.
        let stream = futures::stream::try_unfold(rx, move |mut receiver| {
            let db = db.clone();
            let authority = authority.clone();
            async move {
                let Some(message) = receiver.recv().await else {
                    return Ok(None);
                };
                super::super::authority::admit_ingress(&db, &authority).await?;
                Ok(Some((message?, receiver)))
            }
        });
        Ok(Response::new(Box::pin(end_on_stop(stream, stop))))
    }
}

/// End `stream` once `stop` fires, so a draining server's gRPC listener is not
/// held open by a subscription that would otherwise run forever.
fn end_on_stop<S: futures::Stream + Send + 'static>(
    stream: S,
    stop: tokio_util::sync::CancellationToken,
) -> impl futures::Stream<Item = S::Item> + Send + 'static {
    use futures::StreamExt;
    stream.take_until(stop.cancelled_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[tokio::test]
    async fn a_subscription_ends_when_the_grpc_listener_stops() {
        let stop = tokio_util::sync::CancellationToken::new();
        let (tx, rx) = tokio::sync::mpsc::channel::<u32>(4);
        let mut stream = Box::pin(end_on_stop(
            tokio_stream::wrappers::ReceiverStream::new(rx),
            stop.clone(),
        ));

        tx.send(1).await.unwrap();
        assert_eq!(stream.next().await, Some(1));

        stop.cancel();
        // The sender is still alive, so only the stop can end the stream.
        let ended = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next()).await;
        assert_eq!(ended.expect("the stream ends on stop"), None);
        drop(tx);
    }

    #[tokio::test]
    async fn a_subscription_keeps_delivering_until_the_stop() {
        let stop = tokio_util::sync::CancellationToken::new();
        let mut stream = Box::pin(end_on_stop(futures::stream::iter([1, 2, 3]), stop));
        assert_eq!(stream.by_ref().collect::<Vec<_>>().await, vec![1, 2, 3]);
    }
}
