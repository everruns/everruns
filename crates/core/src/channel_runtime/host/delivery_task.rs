//! Leased channel reply delivery after authoritative assignment.
use super::*;

pub(super) async fn run_delivery(
    inner: Arc<Inner>,
    turn: Delivery,
    mut events: ChannelEventStream,
) {
    let fenced = inner.store.supports_delivery_checkpoints();
    if turn.driver.requires_checkpoint() && !fenced {
        warn!(session_id = %turn.session_id, "channel: driver requires an output-ledger store; delivery retained");
        return;
    }
    if fenced
        && inner
            .store
            .renew_pending(
                &turn.session_id,
                &turn.input_message_id,
                &inner.holder,
                inner.lease,
            )
            .await
            .is_err()
    {
        warn!(session_id = %turn.session_id, "channel: delivery ownership lost before dispatch; no provider writes attempted");
        return;
    }
    if fenced && !turn.driver.requires_checkpoint() {
        // Check before admission can clear the row, and before a legacy adapter
        // performs any writes. Changing the driver must not erase its output ledger.
        match inner
            .store
            .delivery_checkpoint(&turn.session_id, &turn.input_message_id)
            .await
        {
            Ok(None) => {}
            Ok(Some(_)) => {
                warn!(session_id = %turn.session_id, "channel: legacy driver cannot replay checkpointed output; delivery retained");
                return;
            }
            Err(_) => {
                warn!(session_id = %turn.session_id, "channel: output checkpoint unavailable; delivery retained");
                return;
            }
        }
    }
    if !admission::prepare_delivery(&inner, &turn).await {
        return;
    }
    let Delivery {
        channel,
        driver,
        session_id,
        input_message_id,
        target,
    } = turn;
    let context = delivery_context(&inner, &channel.config, driver.as_ref(), &target);
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
    delivery.set_observer(inner.delivery_observer.clone());
    let checkpointed = driver.requires_checkpoint();
    if checkpointed {
        if let Err(error) = delivery
            .enable_checkpoints(inner.store.clone(), inner.holder.clone(), &session_id)
            .await
        {
            warn!(%session_id, %error, "channel: delivery checkpoint unavailable; no provider writes attempted");
            return;
        }
        if !save_links(&inner, &channel, &session_id, &mut delivery).await {
            return;
        }
        if delivery.needs_reconciliation().await {
            delivery.close_uncertain_streams().await;
            warn!(%session_id, "channel: uncertain output retained for reconciliation");
            return;
        }
        if delivery.is_finished() {
            let _ = inner
                .store
                .complete_delivery(&session_id, &input_message_id, &inner.holder)
                .await;
            return;
        }
    }
    let mut flush = tokio::time::interval(inner.flush_interval);
    flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Renew well before the lease runs out, so a slow store round trip never
    // lets another process take a delivery this one is running.
    let mut renew = tokio::time::interval(inner.lease / 3);
    renew.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    renew.tick().await;

    // Keep ownership alive while a provider call or its retry delay is awaiting I/O.
    // Cancelling this future after a lost lease leaves its committed intent uncertain.
    let finished = {
        let pumping = async {
            loop {
                tokio::select! {
                    event = events.next() => {
                        let Some(event) = event else {
                            delivery.flush().await;
                            return false;
                        };
                        learn_reacted_key(&inner, &channel, &session_id, &mut delivery, &event).await;
                        let step = delivery.observe(&event).await;
                        if !save_links(&inner, &channel, &session_id, &mut delivery).await && checkpointed { return false; }
                        if let DeliveryStep::Finished(_) = step { return true; }
                    }
                    _ = flush.tick(), if delivery.has_pending_work() => delivery.flush().await,
                }
            }
        };
        tokio::pin!(pumping);
        loop {
            tokio::select! {
                finished = &mut pumping => break finished,
                _ = renew.tick() => {
                    if let Err(error) = inner.store.renew_pending(&session_id, &input_message_id, &inner.holder, inner.lease).await {
                        warn!(%session_id, %error, "channel: delivery ownership lost; writes stopped");
                        return;
                    }
                }
            }
        }
    };
    if !finished {
        return;
    }

    if delivery.needs_recovery().await {
        warn!(%session_id, "channel: output retained for retry or reconciliation");
        return;
    }
    let completion = if fenced {
        inner
            .store
            .complete_delivery(&session_id, &input_message_id, &inner.holder)
            .await
    } else {
        inner
            .store
            .clear_pending(&session_id, &input_message_id)
            .await
    };
    if let Err(error) = completion {
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
) -> bool {
    let mut saved = true;
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
            saved = false;
        }
    }
    saved
}

/// The host stamps scope after the driver derives credentials. Providers cannot
/// replace a configured tenant/network boundary with their default transport.
pub(super) fn delivery_context(
    inner: &Inner,
    channel: &ChannelConfig,
    driver: &dyn ChannelDriver,
    target: &DeliveryTarget,
) -> everruns_contracts::runtime::channel::DeliveryContext {
    let mut context = driver.delivery_context(target);
    if let Some(egress) = &inner.egress {
        context.egress = Some(inner.port.attachment_egress(channel, egress.clone()));
    }
    context
}
