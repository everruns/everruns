use everruns::capability::serde_json::{self, Value};

fn submission(id: &str) -> Result<Value, String> {
    let submissions: Value = serde_json::from_str(include_str!("resources/submissions.json"))
        .map_err(|error| error.to_string())?;
    submissions
        .get(id)
        .cloned()
        .ok_or_else(|| format!("Unknown demo submission: {id}"))
}

#[everruns::tool]
/// Read one open-mic submission: the comic, the set length, and the bit itself.
pub async fn read_submission(submission_id: String) -> Result<String, String> {
    submission(&submission_id).map(|value| value.to_string())
}

#[everruns::tool]
/// Read the club's booking rules: which questions to ask, and the thresholds each slot needs.
pub async fn read_house_rules() -> Result<String, String> {
    Ok(include_str!("resources/house-rules.md").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUSE_RULES: &str = include_str!("resources/house-rules.md");
    const INSTRUCTIONS: &str = include_str!("resources/instructions.md");

    #[test]
    fn submissions_cover_one_case_per_slot() {
        // Clean and funny, inside the cap: the main stage case.
        let pun = submission("mic_pun").unwrap();
        assert_eq!(pun["minutes"], 5);
        // Funny, but drinking is the punchline: clean fails, laugh does not.
        assert_eq!(submission("mic_late").unwrap()["minutes"], 5);
        // An office in-joke: clean passes, a room full of strangers does not laugh.
        assert_eq!(submission("mic_flat").unwrap()["first_timer"], true);
        // Over the cap, so the length rule decides before any number does.
        assert_eq!(submission("mic_long").unwrap()["minutes"], 9);
    }

    #[test]
    fn every_submission_carries_a_bit_to_measure() {
        for id in ["mic_pun", "mic_late", "mic_flat", "mic_long"] {
            let bit = submission(id).unwrap();
            let text = bit["bit"].as_str().unwrap_or_default();
            assert!(text.len() > 80, "{id} has no bit worth classifying");
        }
    }

    #[test]
    fn unknown_submission_is_not_silently_replaced() {
        assert!(submission("../submissions.json").is_err());
        assert!(submission("not-a-submission").is_err());
    }

    #[test]
    fn house_rules_name_every_question_the_prompt_asks_for() {
        // The prompt tells the agent to ask "all four house questions"; the ids
        // and their thresholds live in the rules. Keep the two from drifting.
        for question in ["laugh", "clean", "style", "polish"] {
            assert!(
                HOUSE_RULES.contains(question),
                "house rules do not define the {question} question"
            );
        }
        assert!(INSTRUCTIONS.contains("jev_evaluate"));
        assert!(HOUSE_RULES.contains("jev_evaluate"));
    }

    #[test]
    fn house_rules_state_thresholds_as_numbers() {
        // Thresholds a reader can check against a printed answer, not adjectives.
        // The values are calibrated against these fixtures; see README.md.
        assert!(HOUSE_RULES.contains("0.35"));
        assert!(HOUSE_RULES.contains("0.75"));
        assert!(HOUSE_RULES.contains("Five minutes is the hard cap"));
    }
}
