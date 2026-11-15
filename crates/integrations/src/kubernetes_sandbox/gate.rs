//! A deployment's say over each isolated sandbox run: who may start a pod,
//! for how long, and what it ran.
//!
//! Decisions:
//! - A *run* is one pod's life, from creation to deletion. Every pod start
//!   (create and resume) asks the installed [`PlatformSandboxGate`] first; a
//!   refusal is the error the agent sees, in plain words.
//! - The grant carries a hard lifetime, enforced by Kubernetes itself as the
//!   pod's `activeDeadlineSeconds`, so what a run can cost is bounded even if
//!   no Everruns process ever comes back to settle it.
//! - Settling happens where pods are deleted (pause, delete, cleanup), with the
//!   time the pod actually existed, capped at the lifetime. A settle that
//!   fails is logged, never fatal: the gate owner must treat an unsettled
//!   grant as having used its whole lifetime.
//! - No gate installed means no limits beyond the deployment settings: OSS
//!   deployments run sandboxes on their own cluster for free.
//!
//! # Example
//!
//! ```
//! use std::{sync::Arc, time::Duration};
//! use async_trait::async_trait;
//! use everruns_integrations::kubernetes_sandbox::gate::{
//!     PlatformSandboxGate, SandboxGrant, SandboxRun, install_gate,
//! };
//!
//! struct OneHour;
//!
//! #[async_trait]
//! impl PlatformSandboxGate for OneHour {
//!     async fn admit(&self, run: &SandboxRun) -> Result<SandboxGrant, String> {
//!         if run.org_id.is_none() {
//!             return Err("sandboxes need an organization".into());
//!         }
//!         Ok(SandboxGrant { id: run.run_id.clone(), lifetime: Duration::from_secs(3600) })
//!     }
//!     async fn settle(&self, _grant: &str, _run: &SandboxRun, _ran: Duration) -> Result<(), String> {
//!         Ok(())
//!     }
//! }
//!
//! install_gate(Arc::new(OneHour));
//! ```

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;

use super::spec::ANNOTATION_SIZE;

/// One pod's run, as the gate sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxRun {
    /// Unique per pod start; a settle names the grant, not this.
    pub run_id: String,
    /// The sandbox (its pod and volume name).
    pub sandbox: String,
    /// Public organization id, when the session has one.
    pub org_id: Option<String>,
    pub session_id: Option<String>,
    /// `small`, `medium` or `large`.
    pub size: String,
}

/// Permission for one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxGrant {
    /// What [`PlatformSandboxGate::settle`] receives back; at most 63
    /// characters of `[A-Za-z0-9._-]`, since it is kept on the pod.
    pub id: String,
    /// The pod is stopped by Kubernetes after this long, at the latest.
    pub lifetime: Duration,
}

/// Decides whether a sandbox pod may start, and records what it ran.
#[async_trait]
pub trait PlatformSandboxGate: Send + Sync {
    /// `Err` is the plain-words reason the agent and the person see.
    async fn admit(&self, run: &SandboxRun) -> Result<SandboxGrant, String>;
    /// The run is over: it existed for `ran` (never more than the lifetime).
    /// Called at least once per grant; implementations must accept repeats.
    async fn settle(&self, grant: &str, run: &SandboxRun, ran: Duration) -> Result<(), String>;
}

static GATE: OnceLock<Arc<dyn PlatformSandboxGate>> = OnceLock::new();

/// Install the deployment's gate, once, before the first sandbox starts.
/// Returns `false` when one was already installed.
pub fn install_gate(gate: Arc<dyn PlatformSandboxGate>) -> bool {
    GATE.set(gate).is_ok()
}

/// The installed gate, if any.
pub fn installed_gate() -> Option<Arc<dyn PlatformSandboxGate>> {
    GATE.get().cloned()
}

/// Whether `id` can be kept as a Kubernetes label value.
pub(crate) fn valid_grant_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 63
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

const ANNOTATION_GRANT: &str = "everruns.io/sandbox-grant";
const ANNOTATION_RUN: &str = "everruns.io/sandbox-run";

/// Bound `pod` by `grant` and record the run on it, so whoever deletes the
/// pod later can settle it without any other state.
pub(crate) fn apply_grant(
    pod: &mut k8s_openapi::api::core::v1::Pod,
    run: &SandboxRun,
    grant: &SandboxGrant,
) -> Result<(), String> {
    if !valid_grant_id(&grant.id) {
        return Err("the sandbox gate returned an unusable grant id".to_string());
    }
    let seconds = i64::try_from(grant.lifetime.as_secs())
        .unwrap_or(i64::MAX)
        .max(1);
    if let Some(spec) = pod.spec.as_mut() {
        spec.active_deadline_seconds = Some(seconds);
    }
    let annotations = pod
        .metadata
        .annotations
        .get_or_insert_with(Default::default);
    annotations.insert(ANNOTATION_GRANT.to_string(), grant.id.clone());
    annotations.insert(ANNOTATION_RUN.to_string(), run.run_id.clone());
    Ok(())
}

/// The grant a live pod still has to settle, with its run and how long it
/// ran by `now`. `None` for a pod without a grant, or one already being
/// deleted (whoever deleted it settled it).
pub(crate) fn unsettled_run(
    pod: &k8s_openapi::api::core::v1::Pod,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<(String, SandboxRun, Duration)> {
    if pod.metadata.deletion_timestamp.is_some() {
        return None;
    }
    let annotations = pod.metadata.annotations.as_ref()?;
    let grant = annotations.get(ANNOTATION_GRANT)?.clone();
    let labels = pod.metadata.labels.clone().unwrap_or_default();
    let run = SandboxRun {
        run_id: annotations.get(ANNOTATION_RUN).cloned().unwrap_or_default(),
        sandbox: pod.metadata.name.clone().unwrap_or_default(),
        org_id: labels
            .get(everruns_contracts::session_sandbox::LABEL_ORG_ID)
            .cloned(),
        session_id: labels
            .get(everruns_contracts::session_sandbox::LABEL_SESSION_ID)
            .cloned(),
        size: annotations
            .get(ANNOTATION_SIZE)
            .cloned()
            .unwrap_or_default(),
    };
    let started = pod
        .metadata
        .creation_timestamp
        .as_ref()
        .and_then(|time| chrono::DateTime::from_timestamp(time.0.as_second(), 0))
        .unwrap_or(now);
    let mut ran = (now - started).to_std().unwrap_or_default();
    if let Some(deadline) = pod
        .spec
        .as_ref()
        .and_then(|spec| spec.active_deadline_seconds)
        .and_then(|seconds| u64::try_from(seconds).ok())
    {
        ran = ran.min(Duration::from_secs(deadline));
    }
    Some((grant, run, ran))
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::Pod;
    use serde_json::json;

    fn run() -> SandboxRun {
        SandboxRun {
            run_id: "r1".into(),
            sandbox: "er-sbx-1".into(),
            org_id: Some("org_1".into()),
            session_id: Some("session_1".into()),
            size: "small".into(),
        }
    }

    fn pod() -> Pod {
        serde_json::from_value(json!({
            "metadata": {
                "name": "er-sbx-1",
                "creationTimestamp": "2026-11-15T10:00:00Z",
                "labels": {"everruns.org_id": "org_1", "everruns.session_id": "session_1"},
                "annotations": {"everruns.size": "small"},
            },
            "spec": {"containers": [{"name": "sandbox"}]},
        }))
        .unwrap()
    }

    #[test]
    fn a_granted_pod_is_bounded_and_settles_what_it_ran() {
        let mut pod = pod();
        let grant = SandboxGrant {
            id: "hold-1".into(),
            lifetime: Duration::from_secs(3600),
        };
        apply_grant(&mut pod, &run(), &grant).unwrap();
        assert_eq!(
            pod.spec.as_ref().unwrap().active_deadline_seconds,
            Some(3600)
        );

        let now = "2026-11-15T10:10:00Z".parse().unwrap();
        let (id, settled, ran) = unsettled_run(&pod, now).unwrap();
        assert_eq!(id, "hold-1");
        assert_eq!(settled, run());
        assert_eq!(ran, Duration::from_secs(600));

        // Never more than the lifetime Kubernetes enforced.
        let later = "2026-11-15T12:00:00Z".parse().unwrap();
        assert_eq!(
            unsettled_run(&pod, later).unwrap().2,
            Duration::from_secs(3600)
        );
    }

    #[test]
    fn pods_without_a_grant_or_already_deleting_settle_nothing() {
        let now = chrono::Utc::now();
        assert!(unsettled_run(&pod(), now).is_none());

        let mut deleting = pod();
        apply_grant(
            &mut deleting,
            &run(),
            &SandboxGrant {
                id: "hold-1".into(),
                lifetime: Duration::from_secs(60),
            },
        )
        .unwrap();
        deleting.metadata.deletion_timestamp =
            Some(serde_json::from_value(json!("2026-11-15T10:05:00Z")).unwrap());
        assert!(unsettled_run(&deleting, now).is_none());
    }

    #[test]
    fn a_grant_id_kubernetes_cannot_keep_is_refused() {
        let mut pod = pod();
        for id in ["", "has space", &"x".repeat(64)] {
            let grant = SandboxGrant {
                id: id.to_string(),
                lifetime: Duration::from_secs(60),
            };
            assert!(apply_grant(&mut pod, &run(), &grant).is_err(), "{id:?}");
        }
    }
}
