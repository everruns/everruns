import { compileDeclarativeMatchers, type DeepsecPlugin } from "deepsec/config";

const specs = [
  {
    "version": 1,
    "slug": "slack-request-signature-verifier",
    "description": "Identifies Everruns' Slack HMAC request-signature verification boundary.",
    "noiseTier": "normal",
    "filePatterns": [
      "crates/server/src/api/slack_events/api.rs"
    ],
    "requires": {
      "tech": [
        "rust"
      ]
    },
    "patterns": [
      {
        "source": "^pub\\(crate\\)\\s+fn\\s+verify_slack_signature\\s*\\(",
        "flags": "m",
        "label": "Slack HMAC request verifier"
      }
    ],
    "examples": [
      "pub(crate) fn verify_slack_signature("
    ],
    "closesSurfaceIds": [
      "inbound-webhooks"
    ]
  },
  {
    "version": 1,
    "slug": "mcp-tool-catalog-builders",
    "description": "Identifies the MCP endpoint's tool-definition registry and executable command-toolset builder.",
    "noiseTier": "normal",
    "filePatterns": [
      "crates/server/src/api/mcp_endpoint/tool_registry.rs",
      "crates/server/src/api/mcp_endpoint/catalog.rs"
    ],
    "requires": {
      "tech": [
        "rust"
      ]
    },
    "patterns": [
      {
        "source": "^pub\\s+fn\\s+tool_definitions\\s*\\(",
        "flags": "m",
        "label": "MCP tool-definition registry"
      },
      {
        "source": "^pub\\s+fn\\s+build_toolset\\s*\\(",
        "flags": "m",
        "label": "MCP command-toolset builder"
      }
    ],
    "examples": [
      "pub fn tool_definitions(",
      "pub fn build_toolset("
    ],
    "closesSurfaceIds": [
      "mcp-agent-tools"
    ]
  },
  {
    "version": 1,
    "slug": "rust-background-task-spawners",
    "description": "Identifies server functions that register Everruns' long-running periodic background tasks.",
    "noiseTier": "normal",
    "filePatterns": [
      "crates/server/src/session_scheduler.rs",
      "crates/server/src/tool_result_timeout.rs",
      "crates/server/src/event_retention.rs",
      "crates/server/src/blob_gc.rs",
      "crates/server/src/domains/reporting/background.rs"
    ],
    "requires": {
      "tech": [
        "rust"
      ]
    },
    "patterns": [
      {
        "source": "^pub\\s+fn\\s+spawn_(?:session_scheduler|tool_result_timeout_sweep|retention_task|blob_gc_task|reporting_background_task)\\s*\\(",
        "flags": "m",
        "label": "Periodic background-task registration"
      }
    ],
    "examples": [
      "pub fn spawn_session_scheduler(",
      "pub fn spawn_tool_result_timeout_sweep(",
      "pub fn spawn_retention_task(",
      "pub fn spawn_blob_gc_task(",
      "pub fn spawn_reporting_background_task("
    ],
    "closesSurfaceIds": [
      "scheduled-background-jobs"
    ]
  },
  {
    "version": 1,
    "slug": "rust-clap-command-surface",
    "description": "Identifies Everruns CLI command declarations, command dispatch functions, and its credential-resolution boundary.",
    "noiseTier": "normal",
    "filePatterns": [
      "crates/cli/src/main.rs",
      "crates/cli/src/auth.rs",
      "crates/cli/src/commands/**/*.rs"
    ],
    "requires": {
      "tech": [
        "rust"
      ]
    },
    "patterns": [
      {
        "source": "^#\\[derive\\((?:Parser|Subcommand)(?:,\\s*Debug)?\\)\\]",
        "flags": "m",
        "label": "Clap command declaration"
      },
      {
        "source": "^pub\\s+(?:async\\s+)?fn\\s+run\\s*\\(",
        "flags": "m",
        "label": "CLI command dispatcher"
      },
      {
        "source": "^pub\\s+fn\\s+resolve_credentials\\s*\\(",
        "flags": "m",
        "label": "CLI credential-resolution boundary"
      }
    ],
    "examples": [
      "#[derive(Parser)]",
      "#[derive(Subcommand, Debug)]",
      "pub async fn run(",
      "pub fn run(",
      "pub fn resolve_credentials("
    ],
    "closesSurfaceIds": [
      "command-line-client"
    ]
  },
  {
    "version": 1,
    "slug": "runtime-tool-executor",
    "description": "Identifies the core execution interface through which model-selected tool calls are dispatched.",
    "noiseTier": "normal",
    "filePatterns": [
      "crates/core/src/tool_execution.rs"
    ],
    "requires": {
      "tech": [
        "rust"
      ]
    },
    "patterns": [
      {
        "source": "^pub\\s+trait\\s+ToolExecutor\\s*:\\s*Send\\s*\\+\\s*Sync\\s*\\{",
        "flags": "m",
        "label": "Runtime tool-execution interface"
      }
    ],
    "examples": [
      "pub trait ToolExecutor: Send + Sync {"
    ],
    "closesSurfaceIds": [
      "runtime-agent-capability-tools"
    ]
  }
];

export const generatedMatchersPlugin: DeepsecPlugin = {
  name: "deepsec-generated-matchers",
  matchers: compileDeclarativeMatchers(specs),
};
