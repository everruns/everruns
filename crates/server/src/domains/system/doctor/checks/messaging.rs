use super::Findings;
use super::replicas::{self, Replicas};
use crate::domains::system::doctor::instances::InstanceRow;
use crate::domains::system::doctor::{CheckArea, DoctorCheck, PROBE_TIMEOUT, SystemDoctor};
use crate::live_updates::nats;
use crate::valkey::ValkeyClient;

const AREA: CheckArea = CheckArea::Messaging;

pub(super) async fn run(doctor: &SystemDoctor, replicas: &Replicas) -> Findings {
    let mut found = Findings::default();
    let env = &doctor.env;
    let servers = replicas.live_count(replicas::this_id(doctor));

    let (nats_check, valkey_check) = tokio::join!(
        nats_check(env.nats_url.as_deref(), servers),
        valkey_check(env.valkey_url.as_deref(), servers)
    );
    let nats_check = disagreement(nats_check, &replicas.live, "NATS", |row| row.details.nats);
    let valkey_check = disagreement(valkey_check, &replicas.live, "Valkey", |row| {
        row.details.valkey
    });
    if env.nats_url.is_some() {
        found.fact("NATS");
    }
    if env.valkey_url.is_some() {
        found.fact("Valkey");
    }
    found.check(nats_check);
    found.check(valkey_check);
    found
}

pub(crate) async fn nats_check(url: Option<&str>, servers: usize) -> DoctorCheck {
    let check = DoctorCheck::new(AREA, "messaging.nats", "NATS");
    let Some(url) = url else {
        if servers > 1 {
            return check
                .warning(format!(
                    "Not used, and {servers} servers run: live updates and task wake-ups go \
                     through PostgreSQL."
                ))
                .why(
                    "Each server polls the database for what the others did, which adds load \
                     and makes chat updates arrive late.",
                )
                .fix("Run NATS and set NATS_URL on every server and worker.");
        }
        return check.info(
            "Not used: live updates and task wake-ups go through PostgreSQL. Fine for one \
             server; set NATS_URL when you run more than one, or when updates feel slow.",
        );
    };
    match tokio::time::timeout(PROBE_TIMEOUT, nats::probe(url)).await {
        Ok(Ok(())) => check.ok("Connected; live updates and task wake-ups go through NATS."),
        Ok(Err(error)) => unreachable_nats(check, &nats::error_chain(&error)),
        Err(_) => unreachable_nats(check, "no answer within 3 seconds"),
    }
}

fn unreachable_nats(check: DoctorCheck, reason: &str) -> DoctorCheck {
    check
        .problem(format!(
            "NATS_URL is set but NATS cannot be reached: {reason}."
        ))
        .why("Chat updates arrive late or not at all, and workers pick up work slowly.")
        .fix("Check that NATS is running and that NATS_URL, including its credentials, is right.")
}

pub(crate) async fn valkey_check(url: Option<&str>, servers: usize) -> DoctorCheck {
    let check = DoctorCheck::new(AREA, "messaging.valkey", "Valkey");
    let Some(url) = url else {
        if servers > 1 {
            return check
                .warning(format!(
                    "Not used, and {servers} servers run: each counts rate limits on its own."
                ))
                .why(format!(
                    "Every server allows the full limit, so the real limit is {servers} times \
                     the configured one, sign-in attempts included."
                ))
                .fix("Run Valkey and set VALKEY_URL on every server.");
        }
        return check.info(
            "Not used: rate limits are counted by each server on its own. Fine for one \
             server; with more, each allows the full limit, so set VALKEY_URL.",
        );
    };
    let probe = async {
        let client = ValkeyClient::connect(url).await?;
        let pinged = client.ping().await;
        client.quit().await;
        pinged
    };
    match tokio::time::timeout(PROBE_TIMEOUT, probe).await {
        Ok(Ok(())) => check.ok("Connected; rate limits are shared across servers."),
        Ok(Err(error)) => unreachable_valkey(check, &format!("{error:#}")),
        Err(_) => unreachable_valkey(check, "no answer within 3 seconds"),
    }
}

fn unreachable_valkey(check: DoctorCheck, reason: &str) -> DoctorCheck {
    check
        .problem(format!(
            "VALKEY_URL is set but Valkey cannot be reached: {reason}."
        ))
        .why("Rate limits that depend on it, such as sign-in attempts, stop working as intended.")
        .fix("Check that Valkey is running and that VALKEY_URL is right.")
}

/// Turn a healthy or unused `check` into a warning when running servers
/// disagree on whether they use `what`.
pub(crate) fn disagreement(
    check: DoctorCheck,
    live: &[InstanceRow],
    what: &str,
    uses: impl Fn(&InstanceRow) -> bool,
) -> DoctorCheck {
    let (with, without): (Vec<&InstanceRow>, Vec<&InstanceRow>) =
        live.iter().partition(|row| uses(row));
    if with.is_empty()
        || without.is_empty()
        || check.status > crate::domains::system::doctor::CheckStatus::Info
    {
        return check;
    }
    let names = |rows: &[&InstanceRow]| {
        rows.iter()
            .map(|row| row.label())
            .collect::<Vec<_>>()
            .join(", ")
    };
    check
        .warning(format!(
            "Servers disagree on {what}: {} use it, {} do not.",
            names(&with),
            names(&without)
        ))
        .why("What people see depends on which server their request lands on.")
        .fix(format!("Give every server the same {what} setting."))
}
