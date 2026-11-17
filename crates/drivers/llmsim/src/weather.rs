//! Provider conditions shared by every session: outages, back-pressure and
//! rate-limit budgets, selected by markers in the model name.
//!
//! Decisions:
//! - `-storm-{kind}-{every}-{for}` puts every call into `kind` (`429`, `500`,
//!   `stall` or `slow`) for `for` seconds out of every `every`, on the wall
//!   clock. A real provider incident hits all sessions at once, and the wall
//!   clock lets every worker process agree on the window with no coordination.
//!   A 429 carries `retry-after` until the window ends, so a load test shows
//!   whether retries pile up and stampede when it does.
//! - `-busy-{knee}` slows calls once more than `knee` are in flight in this
//!   process: first token and token gaps grow with in-flight/knee, the way a
//!   provider slows under our own concurrency.
//! - `-tpm-{n}` gives this process a token-per-minute budget of `n` prompt
//!   tokens shared by all sessions, refilled continuously. A call that does not
//!   fit fails as rate limited with the wait until it would. Budgets are per
//!   worker process, so divide a fleet-wide limit by the worker count.
//! - Scenarios keep their own token pacing; conditions add to the first token
//!   only.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use everruns_contracts::error::{AgentLoopError, LlmErrorKind};
use llmsim::latency::LatencyProfile;
use tokio::time::Instant;

/// What a storm does to a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StormKind {
    RateLimited,
    ServerError,
    Stall,
    Slow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Storm {
    kind: StormKind,
    every_secs: u64,
    for_secs: u64,
}

/// Conditions named in a model id.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Conditions {
    storm: Option<Storm>,
    busy_knee: Option<usize>,
    tpm: Option<u64>,
}

/// The digits right after `marker` in `model`.
fn number_after(model: &str, marker: &str) -> Option<u64> {
    let start = model.find(marker)? + marker.len();
    let digits: String = model[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok().filter(|n| *n > 0)
}

fn storm(model: &str) -> Option<Storm> {
    let start = model.find("-storm-")? + "-storm-".len();
    let mut parts = model[start..].splitn(4, '-');
    let kind = match parts.next()? {
        "429" => StormKind::RateLimited,
        "500" => StormKind::ServerError,
        "stall" => StormKind::Stall,
        "slow" => StormKind::Slow,
        _ => return None,
    };
    let every_secs: u64 = parts.next()?.parse().ok()?;
    let for_secs: u64 = parts
        .next()?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()?;
    (every_secs > 0 && for_secs > 0 && for_secs <= every_secs).then_some(Storm {
        kind,
        every_secs,
        for_secs,
    })
}

impl Conditions {
    pub(crate) fn from_model(model: &str) -> Self {
        Self {
            storm: storm(model),
            busy_knee: number_after(model, "-busy-").map(|n| n as usize),
            tpm: number_after(model, "-tpm-"),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// What the conditions do to one call.
#[derive(Debug, Default)]
pub(crate) struct Effect {
    /// Fail the call with this error.
    pub error: Option<AgentLoopError>,
    /// Open the stream and never send anything.
    pub stall: bool,
    /// Added before the first token.
    pub extra_ttft: Duration,
    /// Multiplies the gap between tokens.
    pub tbt_scale: f64,
    /// Held for the life of the call, counting it as in flight.
    pub in_flight: Option<InFlight>,
}

static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

/// Counts a call as in flight until dropped.
#[derive(Debug)]
pub(crate) struct InFlight;

impl InFlight {
    fn enter() -> (Self, usize) {
        (Self, IN_FLIGHT.fetch_add(1, Ordering::SeqCst) + 1)
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    }
}

struct Bucket {
    tokens: f64,
    at: Instant,
}

static BUCKETS: LazyLock<Mutex<HashMap<u64, Arc<Mutex<Bucket>>>>> = LazyLock::new(Mutex::default);

/// Take `cost` tokens from the `tpm` budget, or the wait until it fits.
fn take(tpm: u64, cost: u64) -> Result<(), Duration> {
    let bucket = {
        let Ok(mut buckets) = BUCKETS.lock() else {
            return Ok(());
        };
        buckets
            .entry(tpm)
            .or_insert_with(|| {
                Arc::new(Mutex::new(Bucket {
                    tokens: tpm as f64,
                    at: Instant::now(),
                }))
            })
            .clone()
    };
    let Ok(mut bucket) = bucket.lock() else {
        return Ok(());
    };
    let per_sec = tpm as f64 / 60.0;
    let now = Instant::now();
    bucket.tokens =
        (bucket.tokens + now.duration_since(bucket.at).as_secs_f64() * per_sec).min(tpm as f64);
    bucket.at = now;
    // A call larger than the whole budget is let through when the bucket is
    // full, as providers do, rather than failing forever.
    let cost = (cost as f64).min(tpm as f64);
    if bucket.tokens >= cost {
        bucket.tokens -= cost;
        Ok(())
    } else {
        Err(Duration::from_secs_f64((cost - bucket.tokens) / per_sec))
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Conditions {
    /// Apply the conditions to a call of `prompt_tokens` on `profile`, at
    /// wall-clock second `now`.
    fn effect_at(&self, now: u64, prompt_tokens: u64, profile: &LatencyProfile) -> Effect {
        let mut effect = Effect {
            tbt_scale: 1.0,
            ..Effect::default()
        };
        if let Some(storm) = self.storm {
            let into = now % storm.every_secs;
            if into < storm.for_secs {
                let left = storm.for_secs - into;
                match storm.kind {
                    StormKind::RateLimited => {
                        effect.error = Some(
                            AgentLoopError::llm_kind(
                                LlmErrorKind::RateLimited,
                                "llmsim storm: rate limit exceeded",
                            )
                            .with_retry_after_secs(left),
                        );
                        return effect;
                    }
                    StormKind::ServerError => {
                        effect.error = Some(AgentLoopError::llm_kind(
                            LlmErrorKind::Unavailable,
                            "llmsim storm: server error",
                        ));
                        return effect;
                    }
                    StormKind::Stall => {
                        effect.stall = true;
                        return effect;
                    }
                    StormKind::Slow => {
                        effect.extra_ttft += Duration::from_secs(10);
                        effect.tbt_scale *= 5.0;
                    }
                }
            }
        }
        if let Some(tpm) = self.tpm
            && let Err(wait) = take(tpm, prompt_tokens)
        {
            effect.error = Some(
                AgentLoopError::llm_kind(
                    LlmErrorKind::RateLimited,
                    "llmsim: tokens per minute exceeded",
                )
                .with_retry_after_secs(wait.as_secs().max(1)),
            );
            return effect;
        }
        if let Some(knee) = self.busy_knee {
            let (guard, in_flight) = InFlight::enter();
            effect.in_flight = Some(guard);
            let load = in_flight as f64 / knee as f64;
            if load > 1.0 {
                let base = Duration::from_millis(profile.ttft_mean_ms.max(1000));
                effect.extra_ttft += base.mul_f64(load - 1.0);
                effect.tbt_scale *= load;
            }
        }
        effect
    }

    /// Apply the conditions to a call now.
    pub(crate) fn effect(&self, prompt_tokens: u64, profile: &LatencyProfile) -> Effect {
        self.effect_at(now_secs(), prompt_tokens, profile)
    }
}

/// `profile` with the effect's timing applied.
pub(crate) fn paced(mut profile: LatencyProfile, effect: &Effect) -> LatencyProfile {
    profile.ttft_mean_ms = profile
        .ttft_mean_ms
        .saturating_add(effect.extra_ttft.as_millis() as u64);
    profile.tbt_mean_ms = ((profile.tbt_mean_ms as f64) * effect.tbt_scale).round() as u64;
    profile
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_parse_and_plain_names_have_none() {
        assert!(Conditions::from_model("llmsim-default").is_empty());
        let all = Conditions::from_model("llmsim-default-storm-429-600-120-busy-50-tpm-90000");
        assert_eq!(
            all.storm,
            Some(Storm {
                kind: StormKind::RateLimited,
                every_secs: 600,
                for_secs: 120
            })
        );
        assert_eq!(all.busy_knee, Some(50));
        assert_eq!(all.tpm, Some(90_000));
        // A window longer than its period, or an unknown kind, is ignored.
        assert_eq!(storm("x-storm-429-60-120"), None);
        assert_eq!(storm("x-storm-teapot-60-10"), None);
    }

    #[test]
    fn storms_hit_inside_their_window_only() {
        let conditions = Conditions::from_model("m-storm-429-600-120");
        let profile = LatencyProfile::instant();
        let inside = conditions.effect_at(6_000 + 30, 10, &profile);
        let err = inside.error.expect("rate limited inside the window");
        assert_eq!(err.retry_after_secs(), Some(90));
        assert!(
            conditions
                .effect_at(6_000 + 200, 10, &profile)
                .error
                .is_none()
        );

        let stall = Conditions::from_model("m-storm-stall-600-120").effect_at(6_000, 10, &profile);
        assert!(stall.stall);
        let slow = Conditions::from_model("m-storm-slow-600-120").effect_at(6_000, 10, &profile);
        assert_eq!(slow.extra_ttft, Duration::from_secs(10));
        assert_eq!(slow.tbt_scale, 5.0);
        let down = Conditions::from_model("m-storm-500-600-120").effect_at(6_000, 10, &profile);
        assert!(down.error.is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn the_token_budget_is_shared_and_refills() {
        // A budget no other test uses, since buckets are process-wide.
        let tpm = 6_017;
        assert!(take(tpm, 6_000).is_ok());
        let wait = take(tpm, 3_000).expect_err("budget spent");
        assert!(
            wait > Duration::from_secs(29) && wait < Duration::from_secs(31),
            "{wait:?}"
        );
        tokio::time::advance(Duration::from_secs(30)).await;
        assert!(take(tpm, 3_000).is_ok());
    }

    #[test]
    fn calls_slow_down_past_the_knee() {
        // Hold three calls in flight with a knee of one.
        let conditions = Conditions::from_model("m-busy-1");
        let profile = LatencyProfile::instant();
        let first = conditions.effect_at(0, 1, &profile);
        let second = conditions.effect_at(0, 1, &profile);
        let third = conditions.effect_at(0, 1, &profile);
        // Other tests may hold calls too, so only the growth is certain.
        assert!(third.extra_ttft > first.extra_ttft || first.extra_ttft > Duration::ZERO);
        assert!(third.tbt_scale >= 3.0);
        assert!(second.in_flight.is_some());
        let paced = paced(LatencyProfile::new(1000, 0, 10, 0), &third);
        assert!(paced.tbt_mean_ms >= 30);
    }
}
