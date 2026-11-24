//! Recovery of leased delivery tasks, retaining uncertain output.
use super::*;

/// New pending rows distinguish a pre-intent crash from unaudited legacy output.
pub(super) const CHECKPOINT_PROTOCOL: &str = "everruns_output_checkpoint";

impl ChannelHost {
    /// Resume every delivery a previous process left unfinished and no live
    /// process holds. Returns how many were resumed. Unavailable channels retain
    /// their pending output for later recovery or operator reconciliation.
    pub async fn recover(&self) -> Result<usize, ChannelError> {
        let mut resumed = 0;
        let claimed = self
            .inner
            .store
            .claim_pending(&self.inner.holder, self.inner.lease)
            .await?;
        for pending in claimed {
            let found = match self.inner.channels.get(&pending.channel) {
                Some(channel) => Ok(Some(channel.clone())),
                None => match &self.inner.resolver {
                    Some(resolver) => resolver.resolve(&pending.channel).await,
                    None => Ok(None),
                },
            };
            let channel = match found {
                Ok(Some(channel)) if channel.driver().is_some() => channel,
                Ok(_) => {
                    // Absence of a channel or ledger cannot prove an old writer
                    // sent nothing. Preserve even unaudited legacy rows while
                    // the channel is unpublished, deleted or temporarily missing.
                    self.inner
                        .store
                        .release_pending(
                            &pending.session_id,
                            &pending.input_message_id,
                            &self.inner.holder,
                        )
                        .await?;
                    continue;
                }
                Err(error) => {
                    warn!(channel = %pending.channel, %error, "channel: recovery deferred");
                    self.inner
                        .store
                        .release_pending(
                            &pending.session_id,
                            &pending.input_message_id,
                            &self.inner.holder,
                        )
                        .await?;
                    continue;
                }
            };
            let Some(driver) = channel.driver().cloned() else {
                continue;
            };
            if !driver.requires_checkpoint()
                && pending.target.extra.contains_key(CHECKPOINT_PROTOCOL)
            {
                // Do not even replay events when a rollback driver cannot
                // interpret this output protocol, including its pre-intent row.
                continue;
            }
            if driver.requires_checkpoint()
                && pending
                    .target
                    .extra
                    .get(CHECKPOINT_PROTOCOL)
                    .map(String::as_str)
                    != Some("1")
                && !matches!(
                    self.inner
                        .store
                        .delivery_checkpoint(&pending.session_id, &pending.input_message_id)
                        .await,
                    Ok(Some(_))
                )
            {
                // A legacy writer may have accepted output without recording an
                // intent. Retain its row; a new adapter cannot safely replay it.
                warn!(session_id=%pending.session_id,"channel: legacy delivery requires reconciliation before checkpointed recovery");
                continue;
            }
            let events = self
                .inner
                .port
                .events(
                    &channel.config,
                    &pending.session_id,
                    Some(pending.after_sequence),
                )
                .await?;
            self.spawn_delivery(
                channel,
                driver,
                pending.session_id,
                pending.input_message_id,
                pending.target,
                events,
                true,
            )
            .await?;
            resumed += 1;
        }
        Ok(resumed)
    }

    /// Run [`recover`](Self::recover) every `interval` until the task is
    /// aborted. A host whose processes come and go needs it running: a
    /// delivery whose process died is only taken over once its lease ran out,
    /// and one whose channel could not be read is given back for later.
    pub fn spawn_recovery(&self, interval: Duration) -> tokio::task::JoinHandle<()> {
        let host = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(interval);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                match host.recover().await {
                    Ok(0) => {}
                    Ok(resumed) => debug!(resumed, "channel: resumed pending deliveries"),
                    Err(error) => {
                        crate::report_repeated_failure!(warn, %error, "channel: recovery failed")
                    }
                }
            }
        })
    }
}
