//! The sessions a host exposes to attached processes, and the turns they run.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::{CancelError, InputMessage, RunError, SentMessage, Session, TurnHandle};

use super::wire::ControlSessionInfo;

/// The sessions a host exposes through a [`ControlServer`](super::ControlServer).
///
/// Only registered sessions are reachable from an attached process; a session
/// the host never registers cannot be listed, driven or cancelled. The
/// registry also remembers the active turn of each session so a second process
/// can cancel it: [`Session::send`] is the only way to obtain a
/// [`TurnHandle`], so every send the host itself makes on a registered session
/// should go through [`send`](Self::send) (or be reported with
/// [`track`](Self::track)), otherwise an attached `cancel` cannot see it.
///
/// Cloning is cheap and shares the same registry.
#[derive(Clone, Default)]
pub struct ControlRegistry {
    inner: Arc<Mutex<BTreeMap<String, Entry>>>,
}

struct Entry {
    session: Session,
    label: String,
    /// The most recently tracked turn that has not yet finished.
    active: Option<TurnHandle>,
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
    /// tracked turn.
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
                        active: None,
                    },
                );
            }
        }
        id
    }

    /// Stop exposing a session. Attached streams already open keep running
    /// until they disconnect; new requests no longer find it.
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
                active_turn: entry.active.as_ref().map(TurnHandle::identity),
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
    /// The turn stays the session's active turn until it finishes or a later
    /// tracked send replaces it. Unknown session ids are ignored.
    pub fn track(&self, session_id: &str, sent: &SentMessage) {
        let turn = sent.turn();
        let identity = turn.identity();
        {
            let mut sessions = self.lock();
            let Some(entry) = sessions.get_mut(session_id) else {
                return;
            };
            entry.active = Some(turn.clone());
        }
        // Clear the slot once this turn ends, unless a newer turn took it.
        let registry = self.clone();
        let session_id = session_id.to_owned();
        tokio::spawn(async move {
            let _ = turn.wait().await;
            let mut sessions = registry.lock();
            if let Some(entry) = sessions.get_mut(&session_id)
                && entry
                    .active
                    .as_ref()
                    .is_some_and(|active| active.identity() == identity)
            {
                entry.active = None;
            }
        });
    }

    /// Cooperatively cancel the session's active turn, returning its
    /// [identity](TurnHandle::identity), as [`list`](Self::list) reports it.
    ///
    /// Semantics are [`TurnHandle::cancel`]'s: the running turn stops at its
    /// next await point, its tool work is torn down, and the session emits
    /// `turn.cancelled`. Messages that steered into that turn end with it. The
    /// session itself stays usable. A turn that already finished reports
    /// [`ControlError::NoActiveTurn`].
    pub async fn cancel(&self, key: &str) -> Result<String, ControlError> {
        let turn = {
            let sessions = self.lock();
            let id = resolve(&sessions, key)?;
            sessions[&id]
                .active
                .clone()
                .ok_or(ControlError::NoActiveTurn)?
        };
        match turn.cancel().await {
            Ok(()) => Ok(turn.identity()),
            Err(CancelError::TurnFinished) => Err(ControlError::NoActiveTurn),
            Err(_) => Err(ControlError::SessionClosed),
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
