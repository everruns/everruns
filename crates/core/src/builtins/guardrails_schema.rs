//! JSON Schema for the guardrails capability config.
//!
//! Display titles (`oneOf` const + title) are what schema clients show instead
//! of the raw wire values.

use serde_json::{Value, json};

use super::{
    MAX_CHECK_ID_LEN, MAX_CHECKS, MAX_ENTRIES_PER_CHECK, MAX_ENTRY_LEN, MAX_JUDGE_PROMPT_LEN,
    MAX_MCP_REF_LEN, MAX_REPLACEMENT_LEN,
};

pub(super) fn config_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "mode": {
                "type": "string",
                "title": "Mode",
                "default": "active",
                "description": "Advisory runs all checks but only logs hits — use it to tune checks against false positives before enforcing.",
                "oneOf": [
                    {"const": "active", "title": "Active"},
                    {"const": "advisory", "title": "Advisory"}
                ]
            },
            "checks": {
                "type": "array",
                "title": "Checks",
                "maxItems": MAX_CHECKS,
                "items": {
                    "type": "object",
                    "title": "Check",
                    "required": ["stage", "type"],
                    "properties": {
                        "id": {
                            "type": "string",
                            "maxLength": MAX_CHECK_ID_LEN,
                            "description": "Stable identifier surfaced in reason codes and logs."
                        },
                        "stage": {
                            "type": "string",
                            "title": "Stage",
                            "description": "Where the check runs: streamed model output, tool calls before execution, or tool results before they enter context.",
                            "oneOf": [
                                {"const": "output", "title": "Model output"},
                                {"const": "tool_use", "title": "Tool call"},
                                {"const": "tool_output", "title": "Tool result"}
                            ]
                        },
                        "type": {
                            "type": "string",
                            "title": "Type",
                            "oneOf": [
                                {"const": "regex", "title": "Regex"},
                                {"const": "blocklist", "title": "Blocklist"},
                                {"const": "tool_pattern", "title": "Tool name"},
                                {"const": "llm_judge", "title": "Policy judge"},
                                {"const": "mcp", "title": "MCP guardrail"},
                                {"const": "moderation", "title": "Moderation"}
                            ],
                            "description": "regex/blocklist match stage text; tool_pattern matches tool names (tool_use stage only); llm_judge evaluates a natural-language policy via the utility LLM (tool_use/tool_output stages only); mcp delegates the decision to an external guardrail served over scoped MCP (tool_use/tool_output stages only — sends stage content off-platform); moderation scores the finalized assistant message via the utility LLM as a content decisions (output stage only — runs on the end-of-message seam, sends the message to the utility model)."
                        },
                        "patterns": {
                            "type": "array",
                            "items": {"type": "string", "maxLength": MAX_ENTRY_LEN},
                            "maxItems": MAX_ENTRIES_PER_CHECK,
                            "description": "Regex patterns (type=regex)."
                        },
                        "words": {
                            "type": "array",
                            "items": {"type": "string", "maxLength": MAX_ENTRY_LEN},
                            "maxItems": MAX_ENTRIES_PER_CHECK,
                            "description": "Words or phrases matched as substrings (type=blocklist)."
                        },
                        "case_sensitive": {
                            "type": "boolean",
                            "default": false,
                            "description": "Blocklist matching case sensitivity."
                        },
                        "tools": {
                            "type": "array",
                            "items": {"type": "string", "maxLength": MAX_ENTRY_LEN},
                            "maxItems": MAX_ENTRIES_PER_CHECK,
                            "description": "Tool name patterns with * wildcards (type=tool_pattern)."
                        },
                        "on_fail": {
                            "type": "string",
                            "title": "When it matches",
                            "default": "block",
                            "description": "Block stops the output or tool call. Log records the hit and continues.",
                            "oneOf": [
                                {"const": "block", "title": "Block"},
                                {"const": "log", "title": "Log"}
                            ]
                        },
                        "prompt": {
                            "type": "string",
                            "maxLength": MAX_JUDGE_PROMPT_LEN,
                            "description": "Natural-language policy prompt for llm_judge. Example: 'Block any tool call that reads files outside /home/user.' Evaluated by the engine selected in `engine`; fails open on timeout or error."
                        },
                        "server": {
                            "type": "string",
                            "maxLength": MAX_MCP_REF_LEN,
                            "description": "Scoped-MCP server reference for type=mcp (sanitized server name). Required for mcp checks."
                        },
                        "tool": {
                            "type": "string",
                            "maxLength": MAX_MCP_REF_LEN,
                            "description": "Guardrail tool/method to call on the MCP server for type=mcp. Required for mcp checks. Sends a bounded stage payload off-platform; fails open on timeout, connection error, parse failure, or server-not-configured."
                        },
                        "engine": {
                            "type": "string",
                            "title": "Answered by",
                            "default": "utility_llm",
                            "oneOf": [
                                {"const": "utility_llm", "title": "Utility model"},
                                {"const": "jev", "title": "Decision API"}
                            ],
                            "description": "Which system model answers a model-backed check (type=llm_judge or moderation). utility_llm prompts the utility model for a verdict, one request per check. jev asks TypeSafe's Jev model a typed question and gets a calibrated probability back; every jev check on a stage rides a single request, and `threshold` decides the verdict. jev requires UTILITY_TYPESAFE_API_KEY on the deployment, or an organization that answers system decisions with its own decision model; without either the check is skipped. Both fail open."
                        },
                        "categories": {
                            "type": "array",
                            "items": {"type": "string", "maxLength": MAX_ENTRY_LEN},
                            "maxItems": MAX_ENTRIES_PER_CHECK,
                            "description": "Moderation categories to score (type=moderation). Defaults to a built-in safety set (hate, harassment, self_harm, sexual, violence, illicit) when omitted."
                        },
                        "threshold": {
                            "type": "integer",
                            "minimum": 0,
                            "maximum": 100,
                            "default": 50,
                            "description": "Block threshold as a percentage (0-100). For type=moderation, a category scoring at or above this value trips the check. For type=llm_judge with engine=jev, the judged probability of a violation at or above this value trips the check; the utility_llm engine returns a verdict directly and ignores it."
                        },
                        "replacement": {
                            "type": "string",
                            "maxLength": MAX_REPLACEMENT_LEN,
                            "description": "Text shown in place of blocked output or as the user-facing message for blocked tool calls."
                        }
                    }
                }
            }
        }
    })
}
