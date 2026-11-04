use super::Findings;
use crate::domains::system::doctor::{CheckArea, DoctorCheck, SystemDoctor};

const AREA: CheckArea = CheckArea::Models;

pub(super) async fn run(doctor: &SystemDoctor) -> Findings {
    let mut found = Findings::default();
    let keys = &doctor.env.provider_keys;
    let check = DoctorCheck::new(AREA, "models.platform_keys", "Platform model keys");
    found.check(if keys.is_empty() {
        check.info(
            "No platform model keys are set, so every organization adds its own provider \
             keys before its agents can answer.",
        )
    } else {
        check.ok(format!(
            "Platform keys are set for {}; providers without their own key fall back to them.",
            keys.join(", ")
        ))
    });
    found
}
