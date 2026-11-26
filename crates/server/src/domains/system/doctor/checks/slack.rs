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
        "Add to Slack compatibility",
    );
    let Some(last) = last else {
        return check.info(
            "Slack has not been asked yet. The check runs a few minutes after the server \
             starts and then daily, once Slack app creation is set up.",
        );
    };
    let mut check = check;
    check.observed_at = last.checked_at;
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
                .warning(format!(
                    "New Add to Slack installations may fail for some agent names or descriptions. \
                     Slack rejected {} synthetic compatibility test(s) on {when}.",
                    refusals.len()
                ))
                .why(format!(
                    "This is a platform compatibility check, not a failed installation of one of your agents. \
                     It does not change existing Slack connections. Test cases: {detail}."
                ))
                .fix("Everruns maintainers need to investigate the rejected fields and update manifest generation. \
                      If Add to Slack fails for your agent, try a shorter Slack app name and description in its Branding settings, \
                      then retry. No changes are needed to working connections. The compatibility test runs again daily; \
                      refreshing this report shows the last result.")
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
    fn a_synthetic_refusal_explains_impact_owner_and_workaround() {
        let check = manifest_check(at(SlackManifestOutcome::Refused {
            refusals: vec![(
                "long-ascii".to_string(),
                vec!["/display_information/description: too long".to_string()],
            )],
        }));
        assert_eq!(check.status, CheckStatus::Warning);
        assert!(check.summary.contains("synthetic"));
        assert!(!check.summary.contains("long-ascii"));
        assert!(check.why.as_deref().unwrap().contains("long-ascii"));
        assert!(
            check
                .why
                .as_deref()
                .unwrap()
                .contains("/display_information/description")
        );
        let fix = check.fix.as_deref().unwrap();
        assert!(fix.contains("Everruns maintainers"));
        assert!(fix.contains("Branding"));
        assert!(fix.contains("daily"));
    }

    #[test]
    fn report_refresh_keeps_the_actual_slack_probe_time() {
        let checked_at = chrono::Utc::now() - chrono::Duration::hours(12);
        let check = manifest_check(Some(SlackManifestHealth {
            checked_at,
            outcome: SlackManifestOutcome::Accepted { cases: 6 },
        }));
        assert_eq!(check.observed_at, checked_at);
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
