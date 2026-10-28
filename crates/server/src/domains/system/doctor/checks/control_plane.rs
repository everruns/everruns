use super::Findings;
use crate::auth::config::AuthMode;
use crate::domains::system::doctor::{CheckArea, DoctorCheck, DoctorEnv, SystemDoctor};

const AREA: CheckArea = CheckArea::ControlPlane;

pub(super) async fn run(doctor: &SystemDoctor) -> Findings {
    let env = &doctor.env;
    let mut found = Findings::default();
    if env.dev_mode {
        found.fact("Development mode (in-process worker)");
    }

    let address = DoctorCheck::new(AREA, "control_plane.public_url", "Public address");
    let local = is_local(&env.base_url);
    found.check(if env.base_url.is_empty() {
        address
            .problem("The public address of the API is not set.")
            .why("Sign-in redirects, OAuth callbacks and links in emails are built from it.")
            .fix("Set AUTH_BASE_URL to the address people use to reach the API.")
    } else if local && env.strict() {
        address
            .warning(format!("The public address is {}.", env.base_url))
            .why(
                "Links in emails and OAuth callbacks will point at this machine, \
                 which nobody else can reach.",
            )
            .fix("Set AUTH_BASE_URL to the address people use to reach the API.")
    } else {
        address.ok(format!("The API is reached at {}.", env.base_url))
    });

    let utility = DoctorCheck::new(AREA, "control_plane.utility_llm", "Utility model");
    found.check(if env.utility_llm_configured {
        utility.ok("A utility model is configured for analysis and system decisions.")
    } else {
        utility
            .warning("No utility model is configured.")
            .why(
                "Session analysis, agent health checks and guardrail decisions are \
                 unavailable; guardrails fail open.",
            )
            .fix("Set UTILITY_OPENAI_API_KEY or UTILITY_OPENROUTER_API_KEY.")
    });
    found.check(email_check(env));
    found
}

/// System email: password resets, invitations and sign-up confirmations.
pub(crate) fn email_check(env: &DoctorEnv) -> DoctorCheck {
    let check = DoctorCheck::new(AREA, "control_plane.email", "System email");
    match &env.email {
        Ok(Some(provider)) => check.ok(format!("System email is sent through {provider}.")),
        Ok(None) if matches!(env.auth_mode, AuthMode::Full | AuthMode::External) => check
            .warning("System email is off.")
            .why("Invitations, password resets and sign-up confirmations are never sent.")
            .fix("Set EMAIL_PROVIDER=resend and RESEND_API_KEY."),
        Ok(None) => check.info("System email is off. Nothing on this install needs it."),
        Err(error) => check
            .problem(format!("System email is misconfigured: {error}."))
            .why("The server sends no email, so invitations and password resets are lost.")
            .fix("Set EMAIL_PROVIDER=resend with RESEND_API_KEY, or unset EMAIL_PROVIDER."),
    }
}

/// Whether `url` points at this machine.
fn is_local(url: &str) -> bool {
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', ':'])
        .next()
        .unwrap_or_default();
    matches!(host, "localhost" | "127.0.0.1" | "0.0.0.0" | "[::1]") || host.ends_with(".localhost")
}

#[cfg(test)]
mod tests {
    #[test]
    fn local_addresses_are_recognised() {
        assert!(super::is_local("http://localhost:9300/api"));
        assert!(super::is_local("http://127.0.0.1"));
        assert!(super::is_local("http://app.localhost"));
        assert!(!super::is_local("https://app.everruns.com/api"));
        assert!(!super::is_local("https://localhost-proxy.example.com"));
    }
}
