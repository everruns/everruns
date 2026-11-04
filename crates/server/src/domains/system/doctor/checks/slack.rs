//! Whether Slack accepts the app manifests Add to Slack builds. The answer
//! comes from the daily check in `channels::slack::provisioning`, which asks
//! Slack itself; this only reports it.

use super::Findings;
use crate::channels::slack::provisioning::{
    SlackManifestHealth, SlackManifestOutcome, last_manifest_health,
};
use crate::domains::system::doctor::{CheckArea, DoctorCheck};

pub(super) fn run() -> Findings {
    let mut found = Findings::default();
    found.check(manifest_check(last_manifest_health()));
    found
}

pub(super) fn manifest_check(last: Option<SlackManifestHealth>) -> DoctorCheck {
    let check = DoctorCheck::new(
        CheckArea::Integrations,
        "integrations.slack_manifests",
        "Slack app manifests",
    );
    let Some(last) = last else {
        return check.info(
            "Slack has not been asked yet. The check runs a few minutes after the server \
             starts and then daily, once Slack app creation is set up.",
        );
    };
    let when = last.checked_at.format("%Y-%m-%d %H:%M UTC");
    match last.outcome {
        SlackManifestOutcome::Accepted { cases } => check.ok(format!(
            "Slack accepted all {cases} worst-case app manifests on {when}."
        )),
        SlackManifestOutcome::NoWorkspace => check.info(
            "No Slack workspace is connected, so there is no token to ask Slack with. \
             Add to Slack is not in use.",
        ),
        SlackManifestOutcome::Failed(error) => check
            .warning(format!("Could not ask Slack about app manifests on {when}: {error}"))
            .fix("It runs again within a day. If it keeps failing, test the workspace connection in Settings → Slack workspaces."),
        SlackManifestOutcome::Refused { refusals } => {
            let detail = refusals
                .iter()
                .map(|(case, problems)| {
                    if problems.is_empty() {
                        case.clone()
                    } else {
                        format!("{case} ({})", problems.join("; "))
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            check
                .problem(format!(
                    "Slack refused {} of the worst-case app manifests on {when}: {detail}. \
                     Add to Slack fails with invalid_manifest for agents like these.",
                    refusals.len()
                ))
                .fix("Tighten the manifest limits in channels/slack/events/manifest.rs to what Slack accepts.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::system::doctor::CheckStatus;

    fn at(outcome: SlackManifestOutcome) -> Option<SlackManifestHealth> {
        Some(SlackManifestHealth {
            checked_at: chrono::Utc::now(),
            outcome,
        })
    }

    #[test]
    fn a_refused_manifest_is_a_problem_that_names_the_case_and_field() {
        let check = manifest_check(at(SlackManifestOutcome::Refused {
            refusals: vec![(
                "long-ascii".to_string(),
                vec!["/display_information/description: too long".to_string()],
            )],
        }));
        assert_eq!(check.status, CheckStatus::Problem);
        assert!(check.summary.contains("long-ascii"), "{}", check.summary);
        assert!(check.summary.contains("/display_information/description"));
    }

    #[test]
    fn accepted_is_ok_and_unknown_states_never_fail_the_report() {
        assert_eq!(
            manifest_check(at(SlackManifestOutcome::Accepted { cases: 6 })).status,
            CheckStatus::Ok
        );
        assert_eq!(manifest_check(None).status, CheckStatus::Info);
        assert_eq!(
            manifest_check(at(SlackManifestOutcome::NoWorkspace)).status,
            CheckStatus::Info
        );
        assert_eq!(
            manifest_check(at(SlackManifestOutcome::Failed("timeout".into()))).status,
            CheckStatus::Warning
        );
    }
}
