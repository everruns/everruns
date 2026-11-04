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
                learn_reacted_key(&inner, &channel, &session_id, &mut delivery, &event).await;
                let step = delivery.observe(&event).await;
                save_links(&inner, &channel, &session_id, &mut delivery).await;
                if let DeliveryStep::Finished(_) = step {
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

/// Before the agent's reaction on a message this delivery did not post, look
/// up the message's platform key, so the reaction can be mirrored there.
async fn learn_reacted_key(
    inner: &Inner,
    channel: &ResolvedChannel,
    session_id: &str,
    delivery: &mut TurnDelivery,
    event: &DeliveryEvent,
) {
    use everruns_contracts::runtime::events::{MESSAGE_REACTION_ADDED, MESSAGE_REACTION_REMOVED};
    if !matches!(
        event.event_type.as_str(),
        MESSAGE_REACTION_ADDED | MESSAGE_REACTION_REMOVED
    ) {
        return;
    }
    // Only the agent's reactions are mirrored.
    if event.data.pointer("/actor/kind").and_then(Value::as_str) != Some("agent") {
        return;
    }
    let Some(message_id) = event.data.get("message_id").and_then(Value::as_str) else {
        return;
    };
    if delivery.knows_message(message_id) {
        return;
    }
    let message = super::LinkedMessage {
        session_id: session_id.to_string(),
        message_id: message_id.to_string(),
    };
    match inner.store.message_key(channel.name(), &message).await {
        Ok(Some(key)) => delivery.know_message_key(message_id, key),
        Ok(None) => {}
        Err(error) => warn!(%session_id, %error, "channel: could not look up a message key"),
    }
}

/// Persist the message keys the delivery posted under.
async fn save_links(
    inner: &Inner,
    channel: &ResolvedChannel,
    session_id: &str,
    delivery: &mut TurnDelivery,
) {
    for (key, message_id) in delivery.take_links() {
        let message = super::LinkedMessage {
            session_id: session_id.to_string(),
            message_id,
        };
        if let Err(error) = inner
            .store
            .link_message(channel.name(), &key, &message)
            .await
        {
            warn!(%session_id, %error, "channel: could not link a posted message");
        }
    }
}
