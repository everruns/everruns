use super::Findings;
use crate::domains::system::doctor::{CheckArea, DoctorCheck, PROBE_TIMEOUT, SystemDoctor};
use crate::live_updates::nats;
use crate::valkey::ValkeyClient;

const AREA: CheckArea = CheckArea::Messaging;

pub(super) async fn run(doctor: &SystemDoctor) -> Findings {
    let mut found = Findings::default();
    let env = &doctor.env;

    let (nats_check, valkey_check) = tokio::join!(
        nats_check(env.nats_url.as_deref()),
        valkey_check(env.valkey_url.as_deref())
    );
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

async fn nats_check(url: Option<&str>) -> DoctorCheck {
    let check = DoctorCheck::new(AREA, "messaging.nats", "NATS");
    let Some(url) = url else {
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

async fn valkey_check(url: Option<&str>) -> DoctorCheck {
    let check = DoctorCheck::new(AREA, "messaging.valkey", "Valkey");
    let Some(url) = url else {
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
