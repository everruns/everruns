//! Leased channel reply delivery after authoritative assignment.
use super::*;

pub(super) async fn run_delivery(
    inner: Arc<Inner>,
    turn: Delivery,
    mut events: ChannelEventStream,
    pending_saved: bool,
) {
    if !admission::prepare_delivery(&inner, &turn, pending_saved).await {
        return;
    }
    let Delivery {
        channel,
        driver,
        session_id,
        input_message_id,
        target,
    } = turn;
    let context = driver.delivery_context(&target);
    let options = channel.config.delivery.clone();
    let (adapter, options) = match &channel.planner {
        Some(planner) => planner.plan(&session_id, &target, options).await,
        None => (driver.clone() as Arc<dyn ChannelDeliveryAdapter>, options),
    };
    let typed_session = session_id
        .parse::<SessionId>()
        .unwrap_or_else(|_| SessionId::new());
    let mut delivery = TurnDelivery::new(
        adapter,
        context,
        typed_session,
        input_message_id.clone(),
        options,
    );
    let mut flush = tokio::time::interval(inner.flush_interval);
    flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Renew well before the lease runs out, so a slow store round trip never
    // lets another process take a delivery this one is running.
    let mut renew = tokio::time::interval(inner.lease / 3);
    renew.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    renew.tick().await;

    loop {
        tokio::select! {
            event = events.next() => {
                let Some(event) = event else {
                    // The stream ended without a terminal event: the host is
                    // shutting down. Leave the pending delivery for recovery.
                    delivery.flush().await;
                    return;
                };
                if let DeliveryStep::Finished(_) = delivery.observe(&event).await {
                    break;
                }
            }
            _ = flush.tick(), if delivery.has_pending_work() => delivery.flush().await,
            _ = renew.tick() => {
                if let Err(error) = inner
                    .store
                    .renew_pending(&session_id, &input_message_id, &inner.holder, inner.lease)
                    .await
                {
                    warn!(%session_id, %error, "channel: could not renew a delivery lease");
                }
            }
        }
    }

    if let Err(error) = inner
        .store
        .clear_pending(&session_id, &input_message_id)
        .await
    {
        warn!(%session_id, %error, "channel: could not clear a pending delivery");
    }
}
