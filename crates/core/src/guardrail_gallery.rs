// Guardrail gallery — ready-made `GuardrailsConfig` presets.
//
// A curated catalogue of deterministic guardrail configs that an author can
// adopt as a starting point instead of authoring checks from scratch. See
// knowledge/execution/guardrails.md ("guardrail gallery").
//
// Design constraints:
//  - Adoption is client-side config composition. A gallery item carries a
//    full `GuardrailsConfig`; the client drops it into an agent's `guardrails`
//    capability config (merging or replacing checks). There is no new
//    persisted resource — guardrail configs already live in agent capability
//    config — so the gallery is a read-only catalogue, mirroring the
//    harness-examples pattern.
//  - Every preset must `compile()` (enforced by a test) so an adopted preset
//    is always valid against the engine's limits.
//  - Presets may be deterministic (in-process, no egress) or model-backed
//    (`llm_judge` sends a bounded content excerpt to the utility LLM). Each
//    preset's `data_egress` is derived from its check types so a UI can warn
//    before adoption. MCP-served presets will add another marker when a preset
//    uses that check type.

use crate::guardrail_checks::{
    GuardrailCheck, GuardrailOnFail, GuardrailRule, GuardrailStage, GuardrailsConfig,
};

/// Where a preset's checks send data when they run. Derived from the preset's
/// check types (see [`GuardrailGalleryItem::data_egress`]), not hand-authored,
/// so it stays correct as presets mix deterministic and model-backed checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataEgress {
    /// Runs entirely in-process; no data leaves the platform.
    None,
    /// Sends a bounded content excerpt to the org's configured utility LLM
    /// (`llm_judge` / `moderation` checks) for evaluation.
    UtilityLlm,
}

impl DataEgress {
    pub fn as_str(&self) -> &'static str {
        match self {
            DataEgress::None => "none",
            DataEgress::UtilityLlm => "utility_llm",
        }
    }
}

/// A read-only, adoptable guardrails preset.
pub struct GuardrailGalleryItem {
    /// Stable slug used to reference the preset (e.g. `secret-detection`).
    pub name: &'static str,
    /// Human-facing label.
    pub display_name: &'static str,
    /// What the preset protects against and how to tune it.
    pub description: &'static str,
    /// Free-form tags for grouping/filtering in a picker.
    pub tags: Vec<&'static str>,
    /// The adoptable config. Always compiles (see tests).
    pub config: GuardrailsConfig,
}

impl GuardrailGalleryItem {
    /// Distinct rule types used across the preset's checks, in first-seen
    /// order. This is the "check-type composition" trust signal.
    pub fn check_types(&self) -> Vec<&'static str> {
        let mut seen = Vec::new();
        for check in &self.config.checks {
            let t = check.rule.rule_type();
            if !seen.contains(&t) {
                seen.push(t);
            }
        }
        seen
    }

    /// Distinct stages the preset's checks run in, in first-seen order.
    pub fn stages(&self) -> Vec<&'static str> {
        let mut seen = Vec::new();
        for check in &self.config.checks {
            let s = check.stage.as_str();
            if !seen.contains(&s) {
                seen.push(s);
            }
        }
        seen
    }

    /// Where this preset sends data, derived from its check types so it stays
    /// correct as new types (model/MCP) are added. A check that sends content
    /// to the utility LLM (`llm_judge` / `moderation`) marks the preset as
    /// utility-LLM egress; otherwise everything runs in-process.
    pub fn data_egress(&self) -> DataEgress {
        let uses_utility_llm = self
            .config
            .checks
            .iter()
            .any(|c| matches!(c.rule.rule_type(), "llm_judge" | "moderation"));
        if uses_utility_llm {
            DataEgress::UtilityLlm
        } else {
            DataEgress::None
        }
    }
}

// ---- check builders -------------------------------------------------------

fn check(
    id: &'static str,
    stage: GuardrailStage,
    on_fail: GuardrailOnFail,
    replacement: Option<&'static str>,
    rule: GuardrailRule,
) -> GuardrailCheck {
    GuardrailCheck {
        id: Some(id.to_string()),
        stage,
        on_fail,
        replacement: replacement.map(str::to_string),
        rule,
    }
}

fn regex(patterns: &[&str]) -> GuardrailRule {
    GuardrailRule::Regex {
        patterns: patterns.iter().map(|p| p.to_string()).collect(),
    }
}

fn blocklist(words: &[&str]) -> GuardrailRule {
    GuardrailRule::Blocklist {
        words: words.iter().map(|w| w.to_string()).collect(),
        case_sensitive: false,
    }
}

fn tool_pattern(tools: &[&str]) -> GuardrailRule {
    GuardrailRule::ToolPattern {
        tools: tools.iter().map(|t| t.to_string()).collect(),
    }
}

fn llm_judge(prompt: &str) -> GuardrailRule {
    GuardrailRule::LlmJudge {
        prompt: prompt.to_string(),
    }
}

/// Natural-language policy for the secret-leak judge. Describes the *class* of
/// content to block (secret material rendered in cleartext) rather than any
/// specific value, so it catches opaque secrets not known at config time.
const SECRET_LEAK_JUDGE_POLICY: &str = "Block any tool call, tool argument, tool result, log, diff, or \
    other content that displays or transmits secret or credential material in cleartext — API keys, \
    access tokens, passwords, private keys, connection strings, or values read from a secrets manager. Allow \
    comparisons that only reveal a hash, fingerprint, length, or redacted form. Allow reads or \
    writes that store or move a secret without displaying its value.";

fn config(checks: Vec<GuardrailCheck>) -> GuardrailsConfig {
    GuardrailsConfig {
        mode: crate::guardrail_checks::GuardrailMode::Active,
        checks,
    }
}

/// The adoptable guardrail presets, in display order.
pub fn guardrail_gallery() -> Vec<GuardrailGalleryItem> {
    use GuardrailOnFail::{Block, Log};
    use GuardrailStage::{Output, ToolOutput, ToolUse};

    // High-precision secret formats. Blocked on output (model echoing a
    // secret) and on tool_output (a fetched file/page carrying one), which is
    // the untrusted-content trust boundary.
    let secret_patterns: &[&str] = &[
        r"AKIA[0-9A-Z]{16}",                   // AWS access key id
        r"ghp_[A-Za-z0-9]{36}",                // GitHub personal access token
        r"xox[baprs]-[A-Za-z0-9-]{10,}",       // Slack token
        r"AIza[0-9A-Za-z\-_]{35}",             // Google API key
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----", // PEM private key header
    ];

    vec![
        GuardrailGalleryItem {
            name: "secret-detection",
            display_name: "Secret & Credential Detection",
            description: "Blocks well-known credential formats (AWS, GitHub, Slack, Google keys, PEM \
                 private keys) in model output and in tool results before they reach context. \
                 High-precision patterns; safe to run active.",
            tags: vec!["security", "secrets"],
            config: config(vec![
                check(
                    "secret-output",
                    Output,
                    Block,
                    Some("[Response withheld: appears to contain a credential.]"),
                    regex(secret_patterns),
                ),
                check(
                    "secret-tool-output",
                    ToolOutput,
                    Block,
                    Some("[Tool output withheld: appears to contain a credential.]"),
                    regex(secret_patterns),
                ),
            ]),
        },
        GuardrailGalleryItem {
            name: "secret-leak-judge",
            display_name: "Secret Leak Prevention (LLM judge)",
            description: "Model-backed guardrail that blocks tool calls (and tool results) revealing \
                 secret or credential material in cleartext, without the value being known in \
                 advance. Complements `secret-detection`: that catches known credential formats by \
                 pattern; this catches opaque secrets by intent. Evaluated by the utility LLM (a \
                 bounded content excerpt leaves the generating path); async and fail-open. Blocked \
                 tool calls are recoverable — the model self-corrects to a safe form (e.g. comparing \
                 a hash). Run advisory first to tune false positives before enforcing.",
            tags: vec!["security", "secrets"],
            config: config(vec![
                check(
                    "secret-leak-tool-use",
                    ToolUse,
                    Block,
                    Some(
                        "This tool call was blocked: it would reveal secret or credential material. \
                         Retry without printing the secret — compare a hash or redacted form instead.",
                    ),
                    llm_judge(SECRET_LEAK_JUDGE_POLICY),
                ),
                check(
                    "secret-leak-tool-output",
                    ToolOutput,
                    Block,
                    Some(
                        "[Tool output withheld: appears to reveal secret or credential material.]",
                    ),
                    llm_judge(SECRET_LEAK_JUDGE_POLICY),
                ),
            ]),
        },
        GuardrailGalleryItem {
            name: "pii-detection",
            display_name: "PII Detection (email, SSN, phone)",
            description: "Logs likely PII (emails, US SSNs, phone numbers) in output and tool results. \
                 Regex PII is noisy, so this ships as log-only — review hits, then switch \
                 individual checks to block (or run the capability in advisory mode) once tuned.",
            tags: vec!["privacy", "pii"],
            config: config(vec![
                check(
                    "pii-output",
                    Output,
                    Log,
                    None,
                    regex(&[
                        r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}",
                        r"\b\d{3}-\d{2}-\d{4}\b",
                        r"\b\d{3}[-.\s]\d{3}[-.\s]\d{4}\b",
                    ]),
                ),
                check(
                    "pii-tool-output",
                    ToolOutput,
                    Log,
                    None,
                    regex(&[
                        r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}",
                        r"\b\d{3}-\d{2}-\d{4}\b",
                        r"\b\d{3}[-.\s]\d{3}[-.\s]\d{4}\b",
                    ]),
                ),
            ]),
        },
        GuardrailGalleryItem {
            name: "profanity-filter",
            display_name: "Profanity Filter (starter)",
            description: "Blocks output containing words from a small starter list (case-insensitive). \
                 Extend `words` with your own terms — the shipped list is intentionally minimal.",
            tags: vec!["content", "profanity"],
            config: config(vec![check(
                "profanity",
                Output,
                Block,
                Some("[Response withheld: contains filtered language.]"),
                blocklist(&["damn", "crap"]),
            )]),
        },
        GuardrailGalleryItem {
            name: "dangerous-shell-commands",
            display_name: "Dangerous Shell Commands",
            description: "Blocks tool calls whose arguments contain destructive shell patterns \
                 (recursive force-remove of root, mkfs, dd to a device, curl|wget piped to a \
                 shell). Matches serialized tool arguments at the tool_use stage.",
            tags: vec!["security", "tools"],
            config: config(vec![check(
                "dangerous-shell",
                ToolUse,
                Block,
                Some("This command was blocked as potentially destructive."),
                regex(&[
                    r"\brm\s+-[a-zA-Z]*r[a-zA-Z]*f",
                    r"\brm\s+-[a-zA-Z]*f[a-zA-Z]*r",
                    r"\bmkfs\.[a-z0-9]+\b",
                    r"\bdd\s+if=.*\bof=/dev/",
                    r"(?:curl|wget)\s+[^|]*\|\s*(?:sudo\s+)?(?:ba)?sh\b",
                ]),
            )]),
        },
        GuardrailGalleryItem {
            name: "block-shell-access",
            display_name: "Block Shell & Code Execution",
            description: "Refuses tool calls to shell/exec-style tools by name pattern. Tool names vary \
                 by deployment — adjust `tools` to match the runtime's shell/code tools.",
            tags: vec!["security", "tools"],
            config: config(vec![check(
                "no-shell",
                ToolUse,
                Block,
                Some("Shell and code execution are disabled for this agent."),
                tool_pattern(&["bash*", "*shell*", "*exec*", "run_command*"]),
            )]),
        },
        GuardrailGalleryItem {
            name: "prompt-injection-heuristics",
            display_name: "Prompt-Injection Heuristics (tool output)",
            description: "Logs common indirect prompt-injection phrasings in tool results — the \
                 untrusted-content trust boundary. Heuristic and noisy, so it ships as \
                 log-only; review hits before switching to block.",
            tags: vec!["security", "prompt-injection"],
            config: config(vec![check(
                "injection-phrases",
                ToolOutput,
                Log,
                None,
                regex(&[
                    r"(?i)ignore (all )?(previous|prior|above) instructions",
                    r"(?i)disregard (the )?(previous|above|system|prior)",
                    r"(?i)you are now ",
                    r"(?i)new instructions:",
                ]),
            )]),
        },
    ]
}

/// Look up a gallery preset by its `name` slug.
pub fn find_guardrail_gallery_item(name: &str) -> Option<GuardrailGalleryItem> {
    guardrail_gallery().into_iter().find(|i| i.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_compiles() {
        for item in guardrail_gallery() {
            item.config
                .compile()
                .unwrap_or_else(|e| panic!("preset '{}' must compile: {e}", item.name));
        }
    }

    #[test]
    fn preset_names_are_unique_nonempty_slugs() {
        let items = guardrail_gallery();
        let mut names: Vec<&str> = items.iter().map(|i| i.name).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(count, names.len(), "duplicate gallery slugs");
        for item in &items {
            assert!(!item.name.is_empty(), "empty slug");
            assert!(!item.display_name.is_empty(), "empty display name");
            assert!(!item.config.checks.is_empty(), "preset has no checks");
        }
    }

    #[test]
    fn trust_metadata_is_derived_from_config() {
        let secret = find_guardrail_gallery_item("secret-detection").expect("present");
        assert_eq!(secret.check_types(), vec!["regex"]);
        assert_eq!(secret.stages(), vec!["output", "tool_output"]);
        assert_eq!(secret.data_egress(), DataEgress::None);

        // The model-backed secret-leak judge derives utility-LLM egress from
        // its check types, not from a hand-authored marker.
        let judge = find_guardrail_gallery_item("secret-leak-judge").expect("present");
        assert_eq!(judge.check_types(), vec!["llm_judge"]);
        assert_eq!(judge.stages(), vec!["tool_use", "tool_output"]);
        assert_eq!(judge.data_egress(), DataEgress::UtilityLlm);

        let shell = find_guardrail_gallery_item("block-shell-access").expect("present");
        assert_eq!(shell.check_types(), vec!["tool_pattern"]);
        assert_eq!(shell.stages(), vec!["tool_use"]);
    }

    #[test]
    fn secret_leak_judge_policy_covers_tool_results() {
        let item = find_guardrail_gallery_item("secret-leak-judge").expect("present");
        let check = item
            .config
            .checks
            .iter()
            .find(|check| check.stage == GuardrailStage::ToolOutput)
            .expect("tool-output check");
        let GuardrailRule::LlmJudge { prompt } = &check.rule else {
            panic!("tool-output check must use an LLM judge");
        };
        assert!(prompt.contains("tool result"));
    }

    #[test]
    fn find_returns_none_for_unknown() {
        assert!(find_guardrail_gallery_item("nope").is_none());
    }

    #[test]
    fn secret_detection_blocks_aws_key_on_tool_output() {
        let item = find_guardrail_gallery_item("secret-detection").expect("present");
        let compiled = item.config.compile().expect("compiles");
        let hits = compiled.evaluate(
            GuardrailStage::ToolOutput,
            "the key is AKIAIOSFODNN7EXAMPLE here",
            None,
            &|_| false,
        );
        assert_eq!(hits.len(), 1, "expected a single secret hit");
        assert_eq!(
            hits[0].action,
            crate::guardrail_checks::GuardrailAction::Block
        );
    }

    #[test]
    fn pii_detection_logs_not_blocks() {
        let item = find_guardrail_gallery_item("pii-detection").expect("present");
        let compiled = item.config.compile().expect("compiles");
        let hits = compiled.evaluate(
            GuardrailStage::Output,
            "reach me at jane.doe@example.com",
            None,
            &|_| false,
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].action,
            crate::guardrail_checks::GuardrailAction::Log,
            "PII preset must ship log-only"
        );
    }

    #[test]
    fn dangerous_shell_blocks_rm_rf_in_args() {
        let item = find_guardrail_gallery_item("dangerous-shell-commands").expect("present");
        let compiled = item.config.compile().expect("compiles");
        let args = serde_json::json!({"cmd": "rm -rf /"}).to_string();
        let hits = compiled.evaluate(
            GuardrailStage::ToolUse,
            &args,
            Some("bashkit_exec"),
            &|_| false,
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].action,
            crate::guardrail_checks::GuardrailAction::Block
        );
    }
}
