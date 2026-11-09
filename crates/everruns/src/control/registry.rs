//! The sessions a host exposes to attached processes, and the turns they run.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::{CancelError, InputMessage, RunError, SentMessage, Session, TurnHandle};

use super::wire::ControlSessionInfo;

/// The sessions a host exposes through a [`ControlServer`](super::ControlServer).
///
/// Only registered sessions are reachable from an attached process; a session
/// the host never registers cannot be listed, driven or cancelled. The
/// registry also remembers every unfinished turn of each session so a second
/// process can cancel them: [`Session::send`] is the only way to obtain a
/// [`TurnHandle`], so every send the host itself makes on a registered session
/// should go through [`send`](Self::send) (or be reported with
/// [`track`](Self::track)), otherwise an attached `cancel` cannot see it.
///
/// Cloning is cheap and shares the same registry.
#[derive(Clone, Default)]
pub struct ControlRegistry {
    inner: Arc<Mutex<BTreeMap<String, Entry>>>,
    next: Arc<AtomicU64>,
}

struct Entry {
    session: Session,
    label: String,
    /// Every tracked turn that has not finished, in tracking order. Each
    /// waiter removes only its own entry, so concurrent sends that track in
    /// any order never lose a running turn.
    active: Vec<Tracked>,
}

struct Tracked {
    key: u64,
    turn: TurnHandle,
}

impl Entry {
    /// Distinct identities of the unfinished tracked turns. Messages steered
    /// into one running turn share its identity.
    fn identities(&self) -> Vec<String> {
        let mut identities: Vec<String> = Vec::new();
        for tracked in &self.active {
            let identity = tracked.turn.identity();
            if !identities.contains(&identity) {
                identities.push(identity);
            }
        }
        identities
    }

    fn is_tracked(&self, key: u64) -> bool {
        self.active.iter().any(|tracked| tracked.key == key)
    }
}

/// Why a [`ControlRegistry`] operation did not run.
#[derive(Debug)]
#[non_exhaustive]
pub enum ControlError {
    /// No registered session has that id or label.
    NotFound(String),
    /// More than one registered session uses that label; pass the id.
    Ambiguous(String),
    /// The session has no tracked turn that is still running.
    NoActiveTurn,
    /// The session refused or failed the message.
    Run(RunError),
    /// The session closed before the request could be delivered.
    SessionClosed,
}

impl std::fmt::Display for ControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(key) => write!(f, "no session `{key}`"),
            Self::Ambiguous(key) => write!(f, "label `{key}` names more than one session"),
            Self::NoActiveTurn => f.write_str("no active turn"),
            Self::Run(error) => error.fmt(f),
            Self::SessionClosed => f.write_str("session is closed"),
        }
    }
}

impl std::error::Error for ControlError {}

impl ControlRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Expose `session` under `label`, returning its id.
    ///
    /// Registering the same session again replaces its label and keeps its
    /// tracked turns.
    pub fn register(&self, label: impl Into<String>, session: &Session) -> String {
        let id = session.id();
        let label = label.into();
        let mut sessions = self.lock();
        match sessions.get_mut(&id) {
            Some(entry) => entry.label = label,
            None => {
                sessions.insert(
                    id.clone(),
                    Entry {
                        session: session.clone(),
                        label,
                        active: Vec::new(),
                    },
                );
            }
        }
        id
    }

    /// Stop exposing a session. New requests no longer find it; attach
    /// streams already open end with `session_closed` once the host drops its
    /// last handle to the session.
    pub fn unregister(&self, key: &str) -> Option<Session> {
        let mut sessions = self.lock();
        let id = resolve(&sessions, key).ok()?;
        sessions.remove(&id).map(|entry| entry.session)
    }

    /// The registered sessions, ordered by id.
    pub fn list(&self) -> Vec<ControlSessionInfo> {
        self.lock()
            .iter()
            .map(|(id, entry)| ControlSessionInfo {
                id: id.clone(),
                label: entry.label.clone(),
                active_turns: entry.identities(),
            })
            .collect()
    }

    /// Look a session up by id, or by label when the label is unique.
    pub fn session(&self, key: &str) -> Result<Session, ControlError> {
        let sessions = self.lock();
        let id = resolve(&sessions, key)?;
        Ok(sessions[&id].session.clone())
    }

    /// Send `input` to a registered session and track the turn that accepts
    /// it, so [`cancel`](Self::cancel) can reach it.
    ///
    /// Routing is [`Session::send`]'s: an idle session starts a turn, a busy
    /// one is steered.
    pub async fn send(
        &self,
        key: &str,
        input: impl Into<InputMessage>,
    ) -> Result<SentMessage, ControlError> {
        let session = self.session(key)?;
        let sent = session.send(input).await.map_err(ControlError::Run)?;
        self.track(&session.id(), &sent);
        Ok(sent)
    }

    /// Record a turn the host started on a registered session itself.
    ///
    /// The turn stays tracked until it finishes. Unknown session ids are
    /// ignored.
    pub fn track(&self, session_id: &str, sent: &SentMessage) {
        let turn = sent.turn();
        let key = self.next.fetch_add(1, Ordering::Relaxed);
        {
            let mut sessions = self.lock();
            let Some(entry) = sessions.get_mut(session_id) else {
                return;
            };
            entry.active.push(Tracked {
                key,
                turn: turn.clone(),
            });
        }
        let registry = self.clone();
        let session_id = session_id.to_owned();
        tokio::spawn(async move {
            let _ = turn.wait().await;
            if let Some(entry) = registry.lock().get_mut(&session_id) {
                entry.active.retain(|tracked| tracked.key != key);
            }
        });
    }

    /// Cooperatively cancel every unfinished tracked turn of the session,
    /// returning the [identities](TurnHandle::identity) that were cancelled,
    /// as [`list`](Self::list) reports them.
    ///
    /// That covers the running turn and any message admitted but not yet
    /// started, so a cancel never leaves queued work to run afterwards.
    /// Semantics per turn are [`TurnHandle::cancel`]'s: the turn stops at its
    /// next await point, its tool work is torn down, and the session emits
    /// `turn.cancelled`. Messages that steered into a turn end with it. The
    /// session itself stays usable. When nothing tracked is still running,
    /// including a turn that finished while the cancel was in flight, this
    /// reports [`ControlError::NoActiveTurn`].
    pub async fn cancel(&self, key: &str) -> Result<Vec<String>, ControlError> {
        let (id, targets) = {
            let sessions = self.lock();
            let id = resolve(&sessions, key)?;
            let targets: Vec<(u64, TurnHandle)> = sessions[&id]
                .active
                .iter()
                .map(|tracked| (tracked.key, tracked.turn.clone()))
                .collect();
            (id, targets)
        };
        let tracked = |key: u64| {
            self.lock()
                .get(&id)
                .is_some_and(|entry| entry.is_tracked(key))
        };
        let mut cancelled: Vec<String> = Vec::new();
        let mut settled: Vec<u64> = Vec::new();
        let mut closed = false;
        for (key, turn) in targets {
            let identity = turn.identity();
            // Skip a duplicate (steered into a turn already cancelled) or a
            // turn whose waiter saw it finish since the snapshot: a hosted
            // cancel of a finished admission is accepted silently.
            if cancelled.contains(&identity) {
                settled.push(key);
                continue;
            }
            if !tracked(key) {
                continue;
            }
            match turn.cancel().await {
                Ok(()) => {
                    cancelled.push(identity);
                    settled.push(key);
                }
                Err(CancelError::TurnFinished) => {}
                // A turn that completed while this cancel was in flight is
                // gone from the registry: nothing to cancel, not a closed
                // session.
                Err(_) => closed |= tracked(key),
            }
        }
        // Stop tracking what was cancelled now rather than when its waiter
        // returns: cancelling the running turn also drops messages still
        // queued behind it, and on a hosting profile their handles may never
        // report an outcome.
        if let Some(entry) = self.lock().get_mut(&id) {
            entry
                .active
                .retain(|tracked| !settled.contains(&tracked.key));
        }
        match (cancelled.is_empty(), closed) {
            (false, _) => Ok(cancelled),
            (true, true) => Err(ControlError::SessionClosed),
            (true, false) => Err(ControlError::NoActiveTurn),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Entry>> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn resolve(sessions: &BTreeMap<String, Entry>, key: &str) -> Result<String, ControlError> {
    if sessions.contains_key(key) {
        return Ok(key.to_owned());
    }
    let mut matches = sessions.iter().filter(|(_, entry)| entry.label == key);
    match (matches.next(), matches.next()) {
        (Some((id, _)), None) => Ok(id.clone()),
        (Some(_), Some(_)) => Err(ControlError::Ambiguous(key.to_owned())),
        (None, _) => Err(ControlError::NotFound(key.to_owned())),
    }
}
