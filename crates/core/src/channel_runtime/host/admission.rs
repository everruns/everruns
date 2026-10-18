//! Delivery waits for authority assignment without delaying inbound acknowledgement.
use super::*;

pub(super) async fn prepare_delivery(inner: &Inner, turn: &Delivery, saved: bool) -> bool {
    if !saved {
        let pending = PendingDelivery {
            channel: turn.channel.config.name.clone(),
            session_id: turn.session_id.clone(),
            input_message_id: turn.input_message_id.clone(),
            target: turn.target.clone(),
            after_sequence: 0,
        };
        if let Err(error) = inner
            .store
            .save_pending(&pending, &inner.holder, inner.lease)
            .await
        {
            warn!(%error, "channel: could not retain admitted delivery");
            return false;
        }
    }
    let mut renewed = tokio::time::Instant::now();
    loop {
        let status = tokio::time::timeout(
            inner.lease / 3,
            inner.port.resolve_send(
                &turn.channel.config,
                &turn.session_id,
                &turn.input_message_id,
            ),
        )
        .await;
        match status {
            Ok(Ok(SendResolution::Assigned { steered: false, .. })) => return true,
            Ok(Ok(SendResolution::Assigned { steered: true, .. } | SendResolution::Rejected)) => {
                if let Err(error) = inner
                    .store
                    .clear_pending(&turn.session_id, &turn.input_message_id)
                    .await
                {
                    warn!(%error, "channel: could not clear settled admission delivery");
                }
                return false;
            }
            Ok(Ok(SendResolution::Suspended)) => return false,
            Ok(Ok(SendResolution::Pending)) => {}
            Ok(Err(error)) => {
                warn!(%error, "channel: admission lookup failed; delivery retained");
                return false;
            }
            Err(_) => {
                warn!("channel: admission lookup timed out; delivery retained");
                return false;
            }
        }
        if renewed.elapsed() >= inner.lease / 3 {
            if let Err(error) = inner
                .store
                .renew_pending(
                    &turn.session_id,
                    &turn.input_message_id,
                    &inner.holder,
                    inner.lease,
                )
                .await
            {
                warn!(%error, "channel: admission delivery ownership lost");
                return false;
            }
            renewed = tokio::time::Instant::now();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub(super) fn assigned_events(
    port: Arc<dyn ChannelSessionPort>,
    channel: ChannelConfig,
    session: String,
    input: String,
    events: ChannelEventStream,
) -> ChannelEventStream {
    futures::stream::once(async move {
        loop {
            match port.resolve_send(&channel, &session, &input).await {
                Ok(SendResolution::Pending) => tokio::time::sleep(Duration::from_millis(50)).await,
                Ok(SendResolution::Assigned {
                    turn_id: Some(turn),
                    ..
                }) => {
                    let events = events.filter(move |event| {
                        futures::future::ready(event.turn_id.as_deref().is_none_or(|id| id == turn))
                    });
                    return turn_events(events.boxed(), None);
                }
                Ok(SendResolution::Assigned {
                    turn_id: None,
                    steered,
                }) => return turn_events(events, (!steered).then_some(input)),
                Ok(SendResolution::Rejected | SendResolution::Suspended) => {
                    return futures::stream::empty().boxed();
                }
                Err(error) => {
                    warn!(%error, "channel: streaming admission lookup failed");
                    return futures::stream::empty().boxed();
                }
            }
        }
    })
    .flatten()
    .boxed()
}
