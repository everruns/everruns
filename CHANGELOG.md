# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **⚠️ Important:** There is no automatic migration between versions. Each major/minor release requires a fresh database. Back up any data you need before upgrading.

## [Unreleased]

<!-- New changes go here. Use `/prepare-release X.Y.Z` to generate draft from commits. -->

## [0.8.8] - 2026-03-28

### Highlights

- **Everruns as MCP Server** — ScriptedTool-backed MCP endpoint with OAuth 2.1 PKCE authentication ([#1078](https://github.com/everruns/everruns/pull/1078), [#1092](https://github.com/everruns/everruns/pull/1092))
- **Persistent Memory Layer** — Cross-session memory capability for agents ([#1091](https://github.com/everruns/everruns/pull/1091))
- **New Sandbox Integrations** — Added E2B, Deno Deploy, Sprites, and PI sandbox providers ([#1038](https://github.com/everruns/everruns/pull/1038), [#1047](https://github.com/everruns/everruns/pull/1047), [#1076](https://github.com/everruns/everruns/pull/1076), [#1085](https://github.com/everruns/everruns/pull/1085))
- **API Hardening** — Per-IP rate limiting, configurable resource limits, error sanitization, and account deletion/export ([#1117](https://github.com/everruns/everruns/pull/1117), [#1119](https://github.com/everruns/everruns/pull/1119), [#1116](https://github.com/everruns/everruns/pull/1116), [#1123](https://github.com/everruns/everruns/pull/1123))
- **Prometheus /metrics** — Production-ready metrics endpoint with horizontal scaling support ([#1101](https://github.com/everruns/everruns/pull/1101), [#1106](https://github.com/everruns/everruns/pull/1106))
- **Started Work on Evals Subsystem** — User-facing eval system for agents and harnesses, gated behind experimental feature flag ([#1121](https://github.com/everruns/everruns/pull/1121), [#1122](https://github.com/everruns/everruns/pull/1122))

### What's Changed

- feat(mcp): ScriptedTool-backed MCP endpoint at /mcp ([#1078](https://github.com/everruns/everruns/pull/1078)) by [@chaliy](https://github.com/chaliy)
- feat(auth): add MCP OAuth 2.1 with PKCE for MCP client authentication ([#1092](https://github.com/everruns/everruns/pull/1092)) by [@chaliy](https://github.com/chaliy)
- feat(memory): add persistent cross-session memory capability ([#1091](https://github.com/everruns/everruns/pull/1091)) by [@chaliy](https://github.com/chaliy)
- feat(e2b): add cloud sandbox integration ([#1038](https://github.com/everruns/everruns/pull/1038)) by [@chaliy](https://github.com/chaliy)
- feat(deno): add Deno Deploy sandbox integration ([#1047](https://github.com/everruns/everruns/pull/1047)) by [@chaliy](https://github.com/chaliy)
- feat(deno): bring Deno integration to parity with Daytona ([#1052](https://github.com/everruns/everruns/pull/1052)) by [@chaliy](https://github.com/chaliy)
- feat(sprites): add Sprites sandbox integration ([#1076](https://github.com/everruns/everruns/pull/1076)) by [@chaliy](https://github.com/chaliy)
- feat(pi): add PI sandbox coding agent capability ([#1085](https://github.com/everruns/everruns/pull/1085)) by [@chaliy](https://github.com/chaliy)
- feat(server): add Prometheus /metrics endpoint ([#1101](https://github.com/everruns/everruns/pull/1101)) by [@chaliy](https://github.com/chaliy)
- feat(evals): add user-facing eval system for agents and harnesses ([#1121](https://github.com/everruns/everruns/pull/1121)) by [@chaliy](https://github.com/chaliy)
- feat(evals): gate evals behind experimental feature flag ([#1122](https://github.com/everruns/everruns/pull/1122)) by [@chaliy](https://github.com/chaliy)
- feat(server): add account deletion and data export endpoints ([#1123](https://github.com/everruns/everruns/pull/1123)) by [@chaliy](https://github.com/chaliy)
- feat(api): add global per-IP API rate limiting middleware ([#1119](https://github.com/everruns/everruns/pull/1119)) by [@chaliy](https://github.com/chaliy)
- feat(api): add configurable resource limits for orgs, members, API keys ([#1117](https://github.com/everruns/everruns/pull/1117)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add custom error pages (404, 500) ([#1118](https://github.com/everruns/everruns/pull/1118)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add mermaid diagram rendering to chat messages ([#1073](https://github.com/everruns/everruns/pull/1073)) by [@chaliy](https://github.com/chaliy)
- feat(cli): add --writable flag for initial-files-dir ([#1128](https://github.com/everruns/everruns/pull/1128)) by [@chaliy](https://github.com/chaliy)
- feat(cli): rename .syncignore to .everrunsignore ([#1104](https://github.com/everruns/everruns/pull/1104)) by [@chaliy](https://github.com/chaliy)
- feat(cli): add sessions watch command for real-time monitoring ([#1046](https://github.com/everruns/everruns/pull/1046)) by [@chaliy](https://github.com/chaliy)
- feat(cli): remove default timeout from chat command ([#1044](https://github.com/everruns/everruns/pull/1044)) by [@chaliy](https://github.com/chaliy)
- feat(harness): enable compaction by default on Generic harness ([#1126](https://github.com/everruns/everruns/pull/1126)) by [@chaliy](https://github.com/chaliy)
- feat(apps): support multiple channels per app ([#1088](https://github.com/everruns/everruns/pull/1088)) by [@chaliy](https://github.com/chaliy)
- feat(sessions): add session-level system_prompt and initial_files overrides ([#1095](https://github.com/everruns/everruns/pull/1095)) by [@chaliy](https://github.com/chaliy)
- feat(core): add multi-platform channel abstractions ([#1080](https://github.com/everruns/everruns/pull/1080)) by [@chaliy](https://github.com/chaliy)
- feat(core): add ToolHints to tool definitions ([#1074](https://github.com/everruns/everruns/pull/1074)) by [@chaliy](https://github.com/chaliy)
- feat(events): add tool.output.delta for streamed tool output ([#1086](https://github.com/everruns/everruns/pull/1086)) by [@chaliy](https://github.com/chaliy)
- feat(daytona): stream exec output in real time via tool.output.delta ([#1096](https://github.com/everruns/everruns/pull/1096)) by [@chaliy](https://github.com/chaliy)
- feat(browserless): add tool.progress streaming for status feedback ([#1051](https://github.com/everruns/everruns/pull/1051)) by [@chaliy](https://github.com/chaliy)
- feat(browserless): add secret references in interact steps ([#1042](https://github.com/everruns/everruns/pull/1042)) by [@chaliy](https://github.com/chaliy)
- feat(blueprints): implement agent blueprints infrastructure ([#1055](https://github.com/everruns/everruns/pull/1055)) by [@chaliy](https://github.com/chaliy)
- feat(agent-identities): add identity-scoped connections ([#1034](https://github.com/everruns/everruns/pull/1034)) by [@chaliy](https://github.com/chaliy)
- feat(server): resolve connections from agent identity on session ([#1039](https://github.com/everruns/everruns/pull/1039)) by [@chaliy](https://github.com/chaliy)
- feat(cli): restore --initial-files-dir flag for agents create/update ([#1064](https://github.com/everruns/everruns/pull/1064)) by [@chaliy](https://github.com/chaliy)
- feat(core): add link-following hint to agent_instructions prompt ([e6ac5e28](https://github.com/everruns/everruns/commit/e6ac5e28)) by [@chaliy](https://github.com/chaliy)
- feat(agent-identity): align edit page with agent edit patterns and centralize locale/timezone ([7564d59d](https://github.com/everruns/everruns/commit/7564d59d)) by [@chaliy](https://github.com/chaliy)
- feat(deno): support personal tokens (ddp_...) in generic connection flow ([#1063](https://github.com/everruns/everruns/pull/1063)) by [@chaliy](https://github.com/chaliy)
- feat(ci): build server and worker binaries for Linux releases ([#1120](https://github.com/everruns/everruns/pull/1120)) by [@chaliy](https://github.com/chaliy)
- feat(ci): auto-update Homebrew formula after CLI releases ([#1090](https://github.com/everruns/everruns/pull/1090)) by [@chaliy](https://github.com/chaliy)
- feat(ci): add Sprites integration workflow with live API tests ([#1077](https://github.com/everruns/everruns/pull/1077)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): detect dead session shell and auto-recover ([#1129](https://github.com/everruns/everruns/pull/1129)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): normalize session paths in download_workspace ([#1127](https://github.com/everruns/everruns/pull/1127)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): handle empty path/branch and quote shell args in git_clone ([#1125](https://github.com/everruns/everruns/pull/1125)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): use proper encoding for binary files in download_workspace ([#1124](https://github.com/everruns/everruns/pull/1124)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): migrate exec to Session API with unified streaming ([#1108](https://github.com/everruns/everruns/pull/1108)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): wrap exec polling commands in sh -c for shell redirection ([#1105](https://github.com/everruns/everruns/pull/1105)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): use snapshot-based sizing instead of ignored resource params ([#1099](https://github.com/everruns/everruns/pull/1099)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): auto-renew sandbox lease during long-running exec ([#1087](https://github.com/everruns/everruns/pull/1087)) by [@chaliy](https://github.com/chaliy)
- fix(api): sanitize error responses to prevent internal detail leaks ([#1116](https://github.com/everruns/everruns/pull/1116)) by [@chaliy](https://github.com/chaliy)
- fix(deno): force HTTP/1.1 ALPN and add proxy auth for WebSocket ([#1114](https://github.com/everruns/everruns/pull/1114)) by [@chaliy](https://github.com/chaliy)
- fix(auth): return clear error when CLI login user has no orgs ([#1115](https://github.com/everruns/everruns/pull/1115)) by [@chaliy](https://github.com/chaliy)
- fix(cli): allow .agents/ directory in --initial-files-dir ([#1110](https://github.com/everruns/everruns/pull/1110)) by [@chaliy](https://github.com/chaliy)
- fix(capabilities): resolve dependencies in collect_capabilities ([#1113](https://github.com/everruns/everruns/pull/1113)) by [@chaliy](https://github.com/chaliy)
- fix(browserless): use /chromium path, force HTTP/1.1 ALPN, add proxy support for CDP WebSocket ([#1112](https://github.com/everruns/everruns/pull/1112)) by [@chaliy](https://github.com/chaliy)
- fix(server): make Prometheus metrics correct under horizontal scaling ([#1106](https://github.com/everruns/everruns/pull/1106)) by [@chaliy](https://github.com/chaliy)
- fix(cli): replace chat polling with SSE streaming and efficient snapshot ([#1094](https://github.com/everruns/everruns/pull/1094)) by [@chaliy](https://github.com/chaliy)
- fix(ui): remove duplicate Connections header on agent identity page ([#1109](https://github.com/everruns/everruns/pull/1109)) by [@chaliy](https://github.com/chaliy)
- fix(ui): improve code block styling in light theme ([#1072](https://github.com/everruns/everruns/pull/1072)) by [@chaliy](https://github.com/chaliy)
- fix(ui): widen API key dialog and inline copy button ([#1035](https://github.com/everruns/everruns/pull/1035)) by [@chaliy](https://github.com/chaliy)
- fix(ui,docs): fix code block empty header bar and background contrast ([#1083](https://github.com/everruns/everruns/pull/1083)) by [@chaliy](https://github.com/chaliy)
- fix(worker): prevent concurrent turn corruption (EVE-170) ([#1037](https://github.com/everruns/everruns/pull/1037)) by [@chaliy](https://github.com/chaliy)
- fix(durable): notify workflow when reclaimed tasks are marked dead ([#1061](https://github.com/everruns/everruns/pull/1061)) by [@chaliy](https://github.com/chaliy)
- fix(browserless): add timeouts to CdpSession connect and send_command ([#1059](https://github.com/everruns/everruns/pull/1059)) by [@chaliy](https://github.com/chaliy)
- fix(browserless): accept HTTP 204 from v2 /active endpoint ([#1036](https://github.com/everruns/everruns/pull/1036)) by [@chaliy](https://github.com/chaliy)
- fix(browserless): use correct env var name BROWSERLESS_TOKEN ([#1068](https://github.com/everruns/everruns/pull/1068)) by [@chaliy](https://github.com/chaliy)
- fix(e2b): align response structs with current E2B API format ([#1054](https://github.com/everruns/everruns/pull/1054)) by [@chaliy](https://github.com/chaliy)
- fix(server): include initial_files in agent upsert SQL query ([#1069](https://github.com/everruns/everruns/pull/1069)) by [@chaliy](https://github.com/chaliy)
- fix(images): use SessionId instead of Uuid for upload query parameter ([#1049](https://github.com/everruns/everruns/pull/1049)) by [@chaliy](https://github.com/chaliy)
- fix(cli): robust event filtering in chat polling ([#1062](https://github.com/everruns/everruns/pull/1062)) by [@chaliy](https://github.com/chaliy)
- fix(scripts): remove invalid `local` outside function in setup.sh ([#1093](https://github.com/everruns/everruns/pull/1093)) by [@chaliy](https://github.com/chaliy)
- fix(docs): fix mermaid diagram rendering in docs site ([#1075](https://github.com/everruns/everruns/pull/1075)) by [@chaliy](https://github.com/chaliy)
- fix(core): install rustls CryptoProvider at startup for parallel tool execution ([b1c19357](https://github.com/everruns/everruns/commit/b1c19357)) by [@chaliy](https://github.com/chaliy)
- fix(browserless): declare session_storage as dependency ([b2b360b4](https://github.com/everruns/everruns/commit/b2b360b4)) by [@chaliy](https://github.com/chaliy)
- fix: rustls CryptoProvider in e2b test + renumber duplicate migration ([#1067](https://github.com/everruns/everruns/pull/1067)) by [@chaliy](https://github.com/chaliy)
- fix(ci): replace deprecated macos-13 runner with macos-latest ([#1033](https://github.com/everruns/everruns/pull/1033)) by [@chaliy](https://github.com/chaliy)
- refactor(auth): decouple CLI auth routes from BuiltinAuthBackend (EVE-176) ([#1050](https://github.com/everruns/everruns/pull/1050)) by [@chaliy](https://github.com/chaliy)
- refactor(platform): split platform management tools into read/write ([#1070](https://github.com/everruns/everruns/pull/1070)) by [@chaliy](https://github.com/chaliy)
- refactor(seed): move multi-org harness reconciliation to non-blocking background task ([#1082](https://github.com/everruns/everruns/pull/1082)) by [@chaliy](https://github.com/chaliy)
- refactor(skills): remove redundant bundled_files from activate_skill result ([#1040](https://github.com/everruns/everruns/pull/1040)) by [@chaliy](https://github.com/chaliy)
- perf(core): cache compiled regexes in skill.rs with LazyLock ([ad5f11e4](https://github.com/everruns/everruns/commit/ad5f11e4)) by [@chaliy](https://github.com/chaliy)
- test(mcp): add integration tests for MCP endpoint ([#1100](https://github.com/everruns/everruns/pull/1100)) by [@chaliy](https://github.com/chaliy)
- test(e2b): add integration tests and CI for E2B cloud sandbox ([#1048](https://github.com/everruns/everruns/pull/1048)) by [@chaliy](https://github.com/chaliy)
- test(browserless): add CI jobs for browserless integration tests ([#1053](https://github.com/everruns/everruns/pull/1053)) by [@chaliy](https://github.com/chaliy)
- test(agent-chat): add multi-turn conversation UI test case ([#1097](https://github.com/everruns/everruns/pull/1097)) by [@chaliy](https://github.com/chaliy)
- chore(migrations): squash v0.8.8 SQL migrations ([#1131](https://github.com/everruns/everruns/pull/1131)) by [@chaliy](https://github.com/chaliy)
- chore(deps): fix npm vulnerabilities and update deps ([#1130](https://github.com/everruns/everruns/pull/1130)) by [@chaliy](https://github.com/chaliy)
- chore(deps): upgrade notify 7→8.2, reqwest 0.13.1→0.13.2 ([#1043](https://github.com/everruns/everruns/pull/1043)) by [@chaliy](https://github.com/chaliy)
- chore: remove protoc build dependency ([#1079](https://github.com/everruns/everruns/pull/1079)) by [@chaliy](https://github.com/chaliy)
- chore(specs): add Figma design system reference to brand spec ([#1111](https://github.com/everruns/everruns/pull/1111)) by [@chaliy](https://github.com/chaliy)
- chore(specs): merge mcp-oauth.md into mcp.md ([#1103](https://github.com/everruns/everruns/pull/1103)) by [@chaliy](https://github.com/chaliy)
- chore(specs): remove implementation details that duplicate code ([#1089](https://github.com/everruns/everruns/pull/1089)) by [@chaliy](https://github.com/chaliy)
- chore(specs): remove temporary analysis, clarify durable memory principle ([#1081](https://github.com/everruns/everruns/pull/1081)) by [@chaliy](https://github.com/chaliy)
- chore(test-cases): fix structure, numbering, and spec compliance ([#1098](https://github.com/everruns/everruns/pull/1098)) by [@chaliy](https://github.com/chaliy)
- docs(daytona): mention all autocleanup timeouts in system prompt ([#1084](https://github.com/everruns/everruns/pull/1084)) by [@chaliy](https://github.com/chaliy)
- docs(cli): add Homebrew installation instructions ([#1060](https://github.com/everruns/everruns/pull/1060)) by [@chaliy](https://github.com/chaliy)

## [0.8.7] - 2026-03-22

### Highlights

- **Agent Identities** — Virtual principals for unattended execution across backend, API, DB, and UI ([#1029](https://github.com/everruns/everruns/pull/1029))
- **CLI + Interactive Login** — Install script, interactive OAuth login, file sync commands, and pre-built release binaries ([#1013](https://github.com/everruns/everruns/pull/1013), [#969](https://github.com/everruns/everruns/pull/969), [#968](https://github.com/everruns/everruns/pull/968), [#1000](https://github.com/everruns/everruns/pull/1000))
- **Session Filesystem** — Git version control for session files with hash-gated edit_file tool ([#979](https://github.com/everruns/everruns/pull/979), [#942](https://github.com/everruns/everruns/pull/942))
- **Localization** — Full Ukrainian chat UI coverage ([#1005](https://github.com/everruns/everruns/pull/1005))

### What's Changed

- feat(agent-identities): add agent identities across backend, API, DB, and UI ([#1029](https://github.com/everruns/everruns/pull/1029)) by [@chaliy](https://github.com/chaliy)
- feat(worker): increase default max concurrent tasks from 10 to 1000 ([#1027](https://github.com/everruns/everruns/pull/1027)) by [@chaliy](https://github.com/chaliy)
- feat(daytona): expose cpu, memory, and disk resource options on sandbox creation ([#1024](https://github.com/everruns/everruns/pull/1024)) by [@chaliy](https://github.com/chaliy)
- feat(daytona): add auto-archive and auto-delete lifecycle settings ([#1026](https://github.com/everruns/everruns/pull/1026)) by [@chaliy](https://github.com/chaliy)
- feat(core): protect skill content from context compaction ([#1022](https://github.com/everruns/everruns/pull/1022)) by [@chaliy](https://github.com/chaliy)
- feat(mcp): add auth modes and OAuth connection flow ([#1018](https://github.com/everruns/everruns/pull/1018)) by [@chaliy](https://github.com/chaliy)
- feat(cli): add agents update command and --initial-files-dir flag ([#1020](https://github.com/everruns/everruns/pull/1020)) by [@chaliy](https://github.com/chaliy)
- feat(core): implement SearchCapable for bashkit indexed search ([#1014](https://github.com/everruns/everruns/pull/1014)) by [@chaliy](https://github.com/chaliy)
- feat(cli): add install script and just recipe ([#1013](https://github.com/everruns/everruns/pull/1013)) by [@chaliy](https://github.com/chaliy)
- feat(agents): upsert on import when agent ID exists ([#1010](https://github.com/everruns/everruns/pull/1010)) by [@chaliy](https://github.com/chaliy)
- feat(ci): publish pre-built CLI binaries to GitHub releases ([#1000](https://github.com/everruns/everruns/pull/1000)) by [@chaliy](https://github.com/chaliy)
- feat(server): add git version control for session filesystems ([#979](https://github.com/everruns/everruns/pull/979)) by [@chaliy](https://github.com/chaliy)
- feat(server): seed example agents during org init ([#985](https://github.com/everruns/everruns/pull/985)) by [@chaliy](https://github.com/chaliy)
- feat(cli): interactive login with localhost OAuth callback ([#969](https://github.com/everruns/everruns/pull/969)) by [@chaliy](https://github.com/chaliy)
- feat(skills): add ${SESSION_ID} and ${SKILL_DIR} variable substitution ([#974](https://github.com/everruns/everruns/pull/974)) by [@chaliy](https://github.com/chaliy)
- feat(cli): add file sync commands and comprehensive test coverage ([#968](https://github.com/everruns/everruns/pull/968)) by [@chaliy](https://github.com/chaliy)
- feat(core): add client hints mechanism and gate setup_connection ([#b04d6e28](https://github.com/everruns/everruns/commit/b04d6e28)) by [@chaliy](https://github.com/chaliy)
- feat(server): implement 5-minute timeout for waiting_for_tool_results sessions ([#961](https://github.com/everruns/everruns/pull/961)) by [@chaliy](https://github.com/chaliy)
- feat(core): cap tool result size to 64 KiB before sending to LLM ([#953](https://github.com/everruns/everruns/pull/953)) by [@chaliy](https://github.com/chaliy)
- feat(apps): add slack report-progress reply mode ([#954](https://github.com/everruns/everruns/pull/954)) by [@chaliy](https://github.com/chaliy)
- feat(harness): add instruction hierarchy to Generic harness system prompt ([#950](https://github.com/everruns/everruns/pull/950)) by [@chaliy](https://github.com/chaliy)
- feat(session-file-system): add hash-gated edit_file tool ([#942](https://github.com/everruns/everruns/pull/942)) by [@chaliy](https://github.com/chaliy)
- feat(harness): add inheritance and effective previews ([#932](https://github.com/everruns/everruns/pull/932)) by [@chaliy](https://github.com/chaliy)
- feat(skills): add model frontmatter field for per-skill model override ([#934](https://github.com/everruns/everruns/pull/934)) by [@chaliy](https://github.com/chaliy)
- feat(permissions): add skill-scoped permission rules ([#931](https://github.com/everruns/everruns/pull/931)) by [@chaliy](https://github.com/chaliy)
- feat(embedding): add PlatformDefinition for embeddable runtimes ([#929](https://github.com/everruns/everruns/pull/929)) by [@chaliy](https://github.com/chaliy)
- feat(anthropic): adopt model metadata from /v1/models API ([#925](https://github.com/everruns/everruns/pull/925)) by [@chaliy](https://github.com/chaliy)
- feat(core): add GPT-5.4 mini/nano profiles and tiered pricing support ([#927](https://github.com/everruns/everruns/pull/927)) by [@chaliy](https://github.com/chaliy)
- feat(skills): add context: fork and agent frontmatter fields ([#926](https://github.com/everruns/everruns/pull/926)) by [@chaliy](https://github.com/chaliy)
- feat(skills): add dynamic context injection via !command syntax ([#923](https://github.com/everruns/everruns/pull/923)) by [@chaliy](https://github.com/chaliy)
- feat(skills): positional argument substitution ([#914](https://github.com/everruns/everruns/pull/914)) by [@chaliy](https://github.com/chaliy)
- feat(skills): add disable-model-invocation frontmatter field ([#913](https://github.com/everruns/everruns/pull/913)) by [@chaliy](https://github.com/chaliy)
- feat(skills): add manual-ui-testing skill ([#912](https://github.com/everruns/everruns/pull/912)) by [@chaliy](https://github.com/chaliy)
- feat(core): enable Opus 4.6 1M context and add max_media limit ([#890](https://github.com/everruns/everruns/pull/890)) by [@chaliy](https://github.com/chaliy)
- feat(compaction): multi-strategy context compaction ([#883](https://github.com/everruns/everruns/pull/883)) by [@chaliy](https://github.com/chaliy)
- feat(docs): add Twitter/OG social card preview metadata ([#886](https://github.com/everruns/everruns/pull/886)) by [@chaliy](https://github.com/chaliy)
- feat(server): seed admin user at startup in admin auth mode ([#882](https://github.com/everruns/everruns/pull/882)) by [@chaliy](https://github.com/chaliy)
- fix(durable): strip null bytes from JSON before PostgreSQL jsonb insert ([#1031](https://github.com/everruns/everruns/pull/1031)) by [@chaliy](https://github.com/chaliy)
- fix(worker): treat Pending workflow as takeover-safe and cancel stale tasks ([#1025](https://github.com/everruns/everruns/pull/1025)) by [@chaliy](https://github.com/chaliy)
- fix(core): add fallback parsing for malformed SKILL.md YAML frontmatter ([#1021](https://github.com/everruns/everruns/pull/1021)) by [@chaliy](https://github.com/chaliy)
- fix(localization): finish Ukrainian chat UI coverage ([#1005](https://github.com/everruns/everruns/pull/1005)) by [@chaliy](https://github.com/chaliy)
- fix(auth): support API keys via standard Bearer scheme ([#1016](https://github.com/everruns/everruns/pull/1016)) by [@chaliy](https://github.com/chaliy)
- fix(cli): simplify install-cli recipe, fix version parsing ([#1017](https://github.com/everruns/everruns/pull/1017)) by [@chaliy](https://github.com/chaliy)
- fix(cli): fix four CLI bugs — streaming, upsert, capabilities list, optional harness ([#1009](https://github.com/everruns/everruns/pull/1009)) by [@chaliy](https://github.com/chaliy)
- fix(cli): show credentials path in status and fix macOS path docs ([#1008](https://github.com/everruns/everruns/pull/1008)) by [@chaliy](https://github.com/chaliy)
- fix: remove automatic agent seeding to prevent duplicates with examples ([#1004](https://github.com/everruns/everruns/pull/1004)) by [@chaliy](https://github.com/chaliy)
- fix(grpc): replace 150MB gRPC message limit with presigned URLs for images ([#1001](https://github.com/everruns/everruns/pull/1001)) by [@chaliy](https://github.com/chaliy)
- fix(core): make session_interact schema OpenAI-compatible ([#996](https://github.com/everruns/everruns/pull/996)) by [@chaliy](https://github.com/chaliy)
- fix(core): add missing properties to object tool schemas ([#984](https://github.com/everruns/everruns/pull/984)) by [@chaliy](https://github.com/chaliy)
- fix(platform): default harness_id to Generic in manage_sessions ([#982](https://github.com/everruns/everruns/pull/982)) by [@chaliy](https://github.com/chaliy)
- fix(grpc): unify gRPC error handling across 3 crates ([#980](https://github.com/everruns/everruns/pull/980)) by [@chaliy](https://github.com/chaliy)
- fix(api): validate virtual capability references on write ([#981](https://github.com/everruns/everruns/pull/981)) by [@chaliy](https://github.com/chaliy)
- fix(ui): add informative tooltip to chat sidebar warning badge ([#973](https://github.com/everruns/everruns/pull/973)) by [@chaliy](https://github.com/chaliy)
- fix(worker): use per-provider circuit breaker keys ([#971](https://github.com/everruns/everruns/pull/971)) by [@chaliy](https://github.com/chaliy)
- fix(grpc): add GetMessage RPC to replace O(n) message lookup ([#970](https://github.com/everruns/everruns/pull/970)) by [@chaliy](https://github.com/chaliy)
- fix(ui): replace unsafe type casts with type guards ([#967](https://github.com/everruns/everruns/pull/967)) by [@chaliy](https://github.com/chaliy)
- fix(ui): prevent schedules table horizontal overflow at 1280px viewport ([#4b6fcf4f](https://github.com/everruns/everruns/commit/4b6fcf4f)) by [@chaliy](https://github.com/chaliy)
- fix(core): remove top-level oneOf from edit_file tool schema ([#966](https://github.com/everruns/everruns/pull/966)) by [@chaliy](https://github.com/chaliy)
- fix(ui): make org switching atomic with cookie sync and query invalidation ([#964](https://github.com/everruns/everruns/pull/964)) by [@chaliy](https://github.com/chaliy)
- fix(ui): deduplicate initial events REST fetch on chat page load ([#963](https://github.com/everruns/everruns/pull/963)) by [@chaliy](https://github.com/chaliy)
- fix(ui): unify API error handling through centralized client ([#962](https://github.com/everruns/everruns/pull/962)) by [@chaliy](https://github.com/chaliy)
- fix(security): encrypt app channel_config secrets at rest ([#960](https://github.com/everruns/everruns/pull/960)) by [@chaliy](https://github.com/chaliy)
- fix(auth-sync): support authoritative org membership updates and removals ([#952](https://github.com/everruns/everruns/pull/952)) by [@chaliy](https://github.com/chaliy)
- fix(ui): constrain ScrollArea max-height overflow ([#956](https://github.com/everruns/everruns/pull/956)) by [@chaliy](https://github.com/chaliy)
- fix(ui): remove chat picker inset chrome ([#947](https://github.com/everruns/everruns/pull/947)) by [@chaliy](https://github.com/chaliy)
- fix(api): return 404 for missing app harness and agent references ([#939](https://github.com/everruns/everruns/pull/939)) by [@chaliy](https://github.com/chaliy)
- fix(worker): skip InputAtom when resuming after connection_required ([#bdb47f47](https://github.com/everruns/everruns/commit/bdb47f47)) by [@chaliy](https://github.com/chaliy)
- fix(core): deduplicate tools by name in RuntimeAgentBuilder ([#946](https://github.com/everruns/everruns/pull/946)) by [@chaliy](https://github.com/chaliy)
- fix(worker): skip retries for non-retryable durable task errors ([#944](https://github.com/everruns/everruns/pull/944)) by [@chaliy](https://github.com/chaliy)
- fix(ui): deduplicate utility functions across frontend ([#941](https://github.com/everruns/everruns/pull/941)) by [@chaliy](https://github.com/chaliy)
- fix(api): return 404 for missing harness IDs in update and destroy ([#938](https://github.com/everruns/everruns/pull/938)) by [@chaliy](https://github.com/chaliy)
- fix(api): return empty results for unknown agent_id in session list ([#937](https://github.com/everruns/everruns/pull/937)) by [@chaliy](https://github.com/chaliy)
- fix(api): validate default_model_id in agent upsert ([#935](https://github.com/everruns/everruns/pull/935)) by [@chaliy](https://github.com/chaliy)
- fix(worker): track actual workflow iteration count ([#930](https://github.com/everruns/everruns/pull/930)) by [@chaliy](https://github.com/chaliy)
- fix(auth): resolve org cookie in no-auth mode for multi-org support ([#918](https://github.com/everruns/everruns/pull/918)) by [@chaliy](https://github.com/chaliy)
- fix(auth): update threat model for OAuth state validation ([#922](https://github.com/everruns/everruns/pull/922)) by [@chaliy](https://github.com/chaliy)
- fix(core): deduplicate ModelWithProvider type across crates ([#921](https://github.com/everruns/everruns/pull/921)) by [@chaliy](https://github.com/chaliy)
- fix(core): skip serializing default deferrable policy in tool types ([#920](https://github.com/everruns/everruns/pull/920)) by [@chaliy](https://github.com/chaliy)
- fix(worker): report actual load in heartbeat instead of default ([#919](https://github.com/everruns/everruns/pull/919)) by [@chaliy](https://github.com/chaliy)
- fix(server): downgrade temporary debug logging in event service ([#917](https://github.com/everruns/everruns/pull/917)) by [@chaliy](https://github.com/chaliy)
- fix(ui): show harness names instead of raw IDs in settings dropdowns ([#915](https://github.com/everruns/everruns/pull/915)) by [@chaliy](https://github.com/chaliy)
- fix(ui): fix schedule creation form silently failing ([#911](https://github.com/everruns/everruns/pull/911)) by [@chaliy](https://github.com/chaliy)
- fix(ui): add confirmation dialog for MCP server archive ([#906](https://github.com/everruns/everruns/pull/906)) by [@chaliy](https://github.com/chaliy)
- fix(ui): auto-switch to newly created org ([#910](https://github.com/everruns/everruns/pull/910)) by [@chaliy](https://github.com/chaliy)
- fix(ui): refresh org dropdown after creating new org ([#907](https://github.com/everruns/everruns/pull/907)) by [@chaliy](https://github.com/chaliy)
- fix(ui): redirect to setup page after org creation ([#905](https://github.com/everruns/everruns/pull/905)) by [@chaliy](https://github.com/chaliy)
- fix(scripts): handle empty ui_dev_args under set -u ([#902](https://github.com/everruns/everruns/pull/902)) by [@chaliy](https://github.com/chaliy)
- fix: resolve open Dependabot security alerts (Next.js + lodash) ([#899](https://github.com/everruns/everruns/pull/899)) by [@chaliy](https://github.com/chaliy)
- fix(ui): add missing SSE event types for connection setup and compaction ([#897](https://github.com/everruns/everruns/pull/897)) by [@chaliy](https://github.com/chaliy)
- fix(server): allow waiting_for_tool_results in gRPC set_session_status ([#893](https://github.com/everruns/everruns/pull/893)) by [@chaliy](https://github.com/chaliy)
- fix(ui): remove native picker chrome ([#894](https://github.com/everruns/everruns/pull/894)) by [@chaliy](https://github.com/chaliy)
- fix(docs): render correct sidebar on API reference pages ([#887](https://github.com/everruns/everruns/pull/887)) by [@chaliy](https://github.com/chaliy)
- fix(ui): align chat surface with runtime previews ([#885](https://github.com/everruns/everruns/pull/885)) by [@chaliy](https://github.com/chaliy)
- fix(core): default system_prompt on create and render links in platform chat ([#884](https://github.com/everruns/everruns/pull/884)) by [@chaliy](https://github.com/chaliy)
- refactor(cli): upgrade deps, drop serde_yaml, use server import API ([#1028](https://github.com/everruns/everruns/pull/1028)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): combine agents page into single view with links to full lists ([#1006](https://github.com/everruns/everruns/pull/1006)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): split monolith files (types.ts, settings, queues) ([#1002](https://github.com/everruns/everruns/pull/1002)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): extract magic values to named constants ([#999](https://github.com/everruns/everruns/pull/999)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): extract generic CRUD APIs and hooks ([#994](https://github.com/everruns/everruns/pull/994)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): decompose chat panel and sidebar ([#991](https://github.com/everruns/everruns/pull/991)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): extract QueryStateWrapper for list page boilerplate ([#945](https://github.com/everruns/everruns/pull/945)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): create centralized TOOL_REGISTRY for tool card polymorphism ([#943](https://github.com/everruns/everruns/pull/943)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): move inline CSS strings to dedicated CSS files ([#949](https://github.com/everruns/everruns/pull/949)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): extract useScrollManager and useImageDropZone hooks ([#936](https://github.com/everruns/everruns/pull/936)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): replace derived-state useEffects with inline computation ([#889](https://github.com/everruns/everruns/pull/889)) by [@chaliy](https://github.com/chaliy)
- refactor(api): add ApiResult type alias and impl_auth_state! macro ([#998](https://github.com/everruns/everruns/pull/998)) by [@chaliy](https://github.com/chaliy)
- refactor(store): add StoreResultExt trait and JSON helpers ([#997](https://github.com/everruns/everruns/pull/997)) by [@chaliy](https://github.com/chaliy)
- refactor(infra): replace Docker Compose with native pg_ctl + valkey-server ([#995](https://github.com/everruns/everruns/pull/995)) by [@chaliy](https://github.com/chaliy)
- refactor(llm): extract shared LLM driver helpers ([#993](https://github.com/everruns/everruns/pull/993)) by [@chaliy](https://github.com/chaliy)
- refactor: rename agent templates to examples, install to use ([#992](https://github.com/everruns/everruns/pull/992)) by [@chaliy](https://github.com/chaliy)
- refactor(worker): consolidate adapter wrappers ([#989](https://github.com/everruns/everruns/pull/989), [#990](https://github.com/everruns/everruns/pull/990)) by [@chaliy](https://github.com/chaliy)
- refactor(worker): extract domain logic from workers into atoms and shared modules ([#958](https://github.com/everruns/everruns/pull/958)) by [@chaliy](https://github.com/chaliy)
- refactor(grpc): decompose grpc_service.rs into submodules ([#988](https://github.com/everruns/everruns/pull/988)) by [@chaliy](https://github.com/chaliy)
- refactor(storage): split repositories.rs and memory.rs god objects into per-entity modules ([#986](https://github.com/everruns/everruns/pull/986), [#987](https://github.com/everruns/everruns/pull/987)) by [@chaliy](https://github.com/chaliy)
- refactor(core): simplify ReasonAtom by removing 6 generic type parameters ([#983](https://github.com/everruns/everruns/pull/983)) by [@chaliy](https://github.com/chaliy)
- refactor(durable): replace Option<Option<T>> with UpdateField<T> enum ([#933](https://github.com/everruns/everruns/pull/933)) by [@chaliy](https://github.com/chaliy)
- chore(migrations): squash 008-013 into 008_v0.8.7 ([#1030](https://github.com/everruns/everruns/pull/1030)) by [@chaliy](https://github.com/chaliy)
- chore(cli): bump everruns-sdk to v0.1.5 ([#1019](https://github.com/everruns/everruns/pull/1019)) by [@chaliy](https://github.com/chaliy)
- chore(config): add shared config crate, unify env-loading pattern ([#1007](https://github.com/everruns/everruns/pull/1007)) by [@chaliy](https://github.com/chaliy)
- chore(ship): add structured security review and enforce review comment resolution ([#1015](https://github.com/everruns/everruns/pull/1015)) by [@chaliy](https://github.com/chaliy)
- chore(skills): ship skill should analyze non-blocking review comments ([#1012](https://github.com/everruns/everruns/pull/1012)) by [@chaliy](https://github.com/chaliy)
- chore(core): audit and clean up #[allow(dead_code)] annotations ([#972](https://github.com/everruns/everruns/pull/972)) by [@chaliy](https://github.com/chaliy)
- chore(core): bump bashkit to v0.1.11 ([#940](https://github.com/everruns/everruns/pull/940)) by [@chaliy](https://github.com/chaliy)
- chore(maintenance): review stale in-progress linear issues ([#928](https://github.com/everruns/everruns/pull/928)) by [@chaliy](https://github.com/chaliy)
- chore(shipping): require final review sweep before merge ([#924](https://github.com/everruns/everruns/pull/924)) by [@chaliy](https://github.com/chaliy)
- chore(specs): enforce mandatory smoke testing in shipping requirements ([#903](https://github.com/everruns/everruns/pull/903)) by [@chaliy](https://github.com/chaliy)
- chore(skills): enforce /ship delegation in process-issues skill ([#904](https://github.com/everruns/everruns/pull/904)) by [@chaliy](https://github.com/chaliy)
- chore: convert process-issues command to goal-oriented skill ([#895](https://github.com/everruns/everruns/pull/895)) by [@chaliy](https://github.com/chaliy)
- chore: co-locate integration specs with their crates ([#896](https://github.com/everruns/everruns/pull/896)) by [@chaliy](https://github.com/chaliy)
- chore: add technical debt analysis to maintenance skill ([#891](https://github.com/everruns/everruns/pull/891)) by [@chaliy](https://github.com/chaliy)
- chore(maintenance): add GitHub security checks to maintenance requirements ([#898](https://github.com/everruns/everruns/pull/898)) by [@chaliy](https://github.com/chaliy)
- chore(ui): upgrade Next.js from 16.1.7 to 16.2.0 ([#977](https://github.com/everruns/everruns/pull/977)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump h3 from 1.15.6 to 1.15.9 ([#975](https://github.com/everruns/everruns/pull/975))
- chore(deps): bump rustls-webpki from 0.103.9 to 0.103.10 ([#976](https://github.com/everruns/everruns/pull/976))
- docs: extend all short meta descriptions to 150+ chars for SEO ([#900](https://github.com/everruns/everruns/pull/900)) by [@chaliy](https://github.com/chaliy)
- test(daytona): add UI test case for Daytona OpenUI connection flow ([#916](https://github.com/everruns/everruns/pull/916)) by [@chaliy](https://github.com/chaliy)
- test(ui): add global chat test cases for agent creation and execution ([#901](https://github.com/everruns/everruns/pull/901)) by [@chaliy](https://github.com/chaliy)
- test: add agent, session, and org creation test cases ([#892](https://github.com/everruns/everruns/pull/892)) by [@chaliy](https://github.com/chaliy)

### Migration Notes

**0.8.6 → 0.8.7:** Migrations have been squashed. Requires a fresh database if upgrading from pre-0.8.6.

## [0.8.6] - 2026-03-15

### Highlights

- **Multitenancy & Org Scoping** — Models, providers, capabilities, harnesses, and derived capabilities are now properly scoped to the owning organization with ownership validation on create ([#845](https://github.com/everruns/everruns/pull/845), [#850](https://github.com/everruns/everruns/pull/850), [#851](https://github.com/everruns/everruns/pull/851), [#852](https://github.com/everruns/everruns/pull/852))
- **Permissions Groundwork** — New permission resolver contract wired into AuthState and config endpoints, laying the foundation for fine-grained access control ([#836](https://github.com/everruns/everruns/pull/836), [#862](https://github.com/everruns/everruns/pull/862))
- **Durable Engine Improvements** — Pre-load count check, snapshot path limit, and continue-as-new for long-running workflows; partial output preserved on stream errors ([#839](https://github.com/everruns/everruns/pull/839), [#877](https://github.com/everruns/everruns/pull/877))
- **UI Polish** — Archive/delete entity states, filter dropdowns, model install/uninstall, org setup page, inline connection setup, tools list in LLM details ([#843](https://github.com/everruns/everruns/pull/843), [#814](https://github.com/everruns/everruns/pull/814), [#855](https://github.com/everruns/everruns/pull/855), [#865](https://github.com/everruns/everruns/pull/865))
- **Localization** — Started backend locale propagation support ([#830](https://github.com/everruns/everruns/pull/830))

### What's Changed

- feat(core): add permission resolver contract ([#836](https://github.com/everruns/everruns/pull/836)) by [@chaliy](https://github.com/chaliy)
- feat(server): wire PermissionResolver into AuthState and config endpoints ([#862](https://github.com/everruns/everruns/pull/862)) by [@chaliy](https://github.com/chaliy)
- feat(durable): pre-load count check, snapshot path limit, continue-as-new ([#839](https://github.com/everruns/everruns/pull/839)) by [@chaliy](https://github.com/chaliy)
- feat(session): add backend locale propagation ([#830](https://github.com/everruns/everruns/pull/830)) by [@chaliy](https://github.com/chaliy)
- feat(session): add initial files for agents and harnesses ([#832](https://github.com/everruns/everruns/pull/832)) by [@chaliy](https://github.com/chaliy)
- feat(connections): inline connection setup via client-side tool call ([#814](https://github.com/everruns/everruns/pull/814)) by [@chaliy](https://github.com/chaliy)
- feat(lifecycle): add archive and delete entity states ([#843](https://github.com/everruns/everruns/pull/843)) by [@chaliy](https://github.com/chaliy)
- feat(sdk): update SDK to v0.1.4 and add agents list pagination ([#846](https://github.com/everruns/everruns/pull/846)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add model install/uninstall toggle and org default model selector ([#868](https://github.com/everruns/everruns/pull/868)) by [@chaliy](https://github.com/chaliy)
- feat(ui): replace archive checkboxes with filter dropdown on all list pages ([#855](https://github.com/everruns/everruns/pull/855)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add tools list and copy button to LLM generation details ([#858](https://github.com/everruns/everruns/pull/858)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add org setup page after creation ([#865](https://github.com/everruns/everruns/pull/865)) by [@chaliy](https://github.com/chaliy)
- fix(api): validate provider ownership for model create ([#845](https://github.com/everruns/everruns/pull/845)) by [@chaliy](https://github.com/chaliy)
- fix(api): validate harness and model ownership on session create ([#850](https://github.com/everruns/everruns/pull/850)) by [@chaliy](https://github.com/chaliy)
- fix(api): scope agent and harness default model ids ([#844](https://github.com/everruns/everruns/pull/844)) by [@chaliy](https://github.com/chaliy)
- fix(api): query DB for org membership instead of stale auth context ([#857](https://github.com/everruns/everruns/pull/857)) by [@chaliy](https://github.com/chaliy)
- fix(api): bind session schedule routes to parent session ([#848](https://github.com/everruns/everruns/pull/848)) by [@chaliy](https://github.com/chaliy)
- fix(storage): scope llm model provider joins to org ([#851](https://github.com/everruns/everruns/pull/851)) by [@chaliy](https://github.com/chaliy)
- fix(session): scope derived capabilities to org-owned refs ([#852](https://github.com/everruns/everruns/pull/852)) by [@chaliy](https://github.com/chaliy)
- fix(org): add default and base harness settings ([#849](https://github.com/everruns/everruns/pull/849)) by [@chaliy](https://github.com/chaliy)
- fix(auth): query DB for org memberships in /v1/auth/me (none mode) ([#863](https://github.com/everruns/everruns/pull/863)) by [@chaliy](https://github.com/chaliy)
- fix(auth): grant admin users owner role in default org ([#873](https://github.com/everruns/everruns/pull/873)) by [@chaliy](https://github.com/chaliy)
- fix(worker): record circuit breaker failure on LLM errors ([#853](https://github.com/everruns/everruns/pull/853)) by [@chaliy](https://github.com/chaliy)
- fix(worker): prevent duplicate error events on transient LLM failures ([#869](https://github.com/everruns/everruns/pull/869)) by [@chaliy](https://github.com/chaliy)
- fix(worker): add connection_required handling to durable worker ([#871](https://github.com/everruns/everruns/pull/871)) by [@chaliy](https://github.com/chaliy)
- fix(core): revert tool_search auto-enable, keep capability-driven ([#860](https://github.com/everruns/everruns/pull/860)) by [@chaliy](https://github.com/chaliy)
- fix(core): auto-enable tool_search for GPT-5.4 and remove Daytona prompt duplication ([#859](https://github.com/everruns/everruns/pull/859)) by [@chaliy](https://github.com/chaliy)
- fix(reason): preserve partial output on trailing stream errors ([#877](https://github.com/everruns/everruns/pull/877)) by [@chaliy](https://github.com/chaliy)
- fix(protocol): include missing fields in proto session conversion ([#875](https://github.com/everruns/everruns/pull/875)) by [@chaliy](https://github.com/chaliy)
- fix(config): enforce real user git identity via SessionStart hook ([#861](https://github.com/everruns/everruns/pull/861)) by [@chaliy](https://github.com/chaliy)
- fix(browserless): block internal network targets ([#838](https://github.com/everruns/everruns/pull/838)) by [@chaliy](https://github.com/chaliy)
- fix(openui): add error boundary around Renderer for malformed ElementNode objects ([#835](https://github.com/everruns/everruns/pull/835)) by [@chaliy](https://github.com/chaliy)
- fix(ui-security): fail closed on auth bootstrap errors ([#840](https://github.com/everruns/everruns/pull/840)) by [@chaliy](https://github.com/chaliy)
- fix(ui): self-host Caveat font to avoid Google Fonts CSP drift ([#847](https://github.com/everruns/everruns/pull/847)) by [@chaliy](https://github.com/chaliy)
- fix(ui): always show session title edit button ([#856](https://github.com/everruns/everruns/pull/856)) by [@chaliy](https://github.com/chaliy)
- fix(ui): deduplicate single-row tool activity timeline display ([#864](https://github.com/everruns/everruns/pull/864)) by [@chaliy](https://github.com/chaliy)
- fix(ui): redirect to entity list page on org switch ([#866](https://github.com/everruns/everruns/pull/866)) by [@chaliy](https://github.com/chaliy)
- fix(ui): simplify archive filter label to "Show archived" ([#870](https://github.com/everruns/everruns/pull/870)) by [@chaliy](https://github.com/chaliy)
- fix(ui): match filter button size with sibling buttons ([#867](https://github.com/everruns/everruns/pull/867)) by [@chaliy](https://github.com/chaliy)
- fix(ui): prevent horizontal scroll on schedules page ([#854](https://github.com/everruns/everruns/pull/854)) by [@chaliy](https://github.com/chaliy)
- fix(ui): remove chat composer divider ([#842](https://github.com/everruns/everruns/pull/842)) by [@chaliy](https://github.com/chaliy)
- fix(ui): remove always-visible scroll buttons from select dropdowns ([#878](https://github.com/everruns/everruns/pull/878)) by [@chaliy](https://github.com/chaliy)
- fix(ui): improve connection-required banner readability ([#880](https://github.com/everruns/everruns/pull/880)) by [@chaliy](https://github.com/chaliy)
- refactor(core): remove system prompt duplication with tool definitions ([#879](https://github.com/everruns/everruns/pull/879)) by [@chaliy](https://github.com/chaliy)
- refactor(core): remove stream-level retry from ReasonAtom, classify LLM errors ([#872](https://github.com/everruns/everruns/pull/872)) by [@chaliy](https://github.com/chaliy)
- chore(core): bump bashkit v0.1.8 → v0.1.10 ([#876](https://github.com/everruns/everruns/pull/876)) by [@chaliy](https://github.com/chaliy)
- chore(server): squash post-0.8.5 migrations into single 0.8.6 migration ([#874](https://github.com/everruns/everruns/pull/874)) by [@chaliy](https://github.com/chaliy)
- chore(maintenance): add invokable maintenance skill ([#833](https://github.com/everruns/everruns/pull/833)) by [@chaliy](https://github.com/chaliy)
- chore(ship): move ship workflow into invokable skill ([#834](https://github.com/everruns/everruns/pull/834)) by [@chaliy](https://github.com/chaliy)
- chore(agents): require latest remote main in worktrees ([#837](https://github.com/everruns/everruns/pull/837)) by [@chaliy](https://github.com/chaliy)

### Migration Notes

**0.8.5 → 0.8.6:** Requires fresh database. Run migrations with `just migrate` or start with `just start-all`.

## [0.8.5] - 2026-03-12

### Highlights

- **Browserless Integration** — Browser automation for agents via Browserless ([#776](https://github.com/everruns/everruns/pull/776))
- **Slack Thread Context** — Bot receives full thread context when first mentioned mid-thread ([#768](https://github.com/everruns/everruns/pull/768))
- **Preview of OpenUI Generative UI** — Dynamic generative UI capability for agents ([#790](https://github.com/everruns/everruns/pull/790))
- **Global Search & Command Palette** — Cmd+K to search sessions, navigate, and run commands ([#767](https://github.com/everruns/everruns/pull/767))
- **Performance Improvements** — GIN-indexed tsvector event search, durable snapshot checkpointing, paginated event loading ([#787](https://github.com/everruns/everruns/pull/787), [#794](https://github.com/everruns/everruns/pull/794))

### What's Changed

- feat(durable): add snapshot checkpointing for workflow event replay ([#794](https://github.com/everruns/everruns/pull/794)) by [@chaliy](https://github.com/chaliy)
- feat(openui): implement OpenUI generative UI capability ([#790](https://github.com/everruns/everruns/pull/790)) by [@chaliy](https://github.com/chaliy)
- feat: paginated event loading for large sessions (EVE-82, EVE-83) by [@chaliy](https://github.com/chaliy)
- feat(ui): bottom-anchored chat scroll with new messages indicator ([#781](https://github.com/everruns/everruns/pull/781)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add exponential backoff to SSE reconnection ([#779](https://github.com/everruns/everruns/pull/779)) by [@chaliy](https://github.com/chaliy)
- feat(browserless): add Browserless browser automation integration ([#776](https://github.com/everruns/everruns/pull/776)) by [@chaliy](https://github.com/chaliy)
- feat(search): global search & command palette (Cmd+K) ([#767](https://github.com/everruns/everruns/pull/767)) by [@chaliy](https://github.com/chaliy)
- feat(daytona): add ownership metadata labels to sandbox creation ([#772](https://github.com/everruns/everruns/pull/772)) by [@chaliy](https://github.com/chaliy)
- feat(slack): inject thread context when bot is first mentioned mid-thread ([#768](https://github.com/everruns/everruns/pull/768)) by [@chaliy](https://github.com/chaliy)
- feat(core): set GPT-5.4 as default model ([#762](https://github.com/everruns/everruns/pull/762)) by [@chaliy](https://github.com/chaliy)
- feat(connections): add generic API key verification for connected accounts ([#760](https://github.com/everruns/everruns/pull/760)) by [@chaliy](https://github.com/chaliy)
- feat(ui): move MCP Servers from Settings to Building Blocks ([#761](https://github.com/everruns/everruns/pull/761)) by [@chaliy](https://github.com/chaliy)
- feat(core): add execution phases and iteration tracking ([#759](https://github.com/everruns/everruns/pull/759)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add durable queues management page ([#754](https://github.com/everruns/everruns/pull/754)) by [@chaliy](https://github.com/chaliy)
- feat(ui): collapse durable execution sidebar by default ([#756](https://github.com/everruns/everruns/pull/756)) by [@chaliy](https://github.com/chaliy)
- fix(server): log migration errors through tracing before propagating ([#800](https://github.com/everruns/everruns/pull/800)) by [@chaliy](https://github.com/chaliy)
- fix(ui): fix duplicate React key in command palette navigation ([#798](https://github.com/everruns/everruns/pull/798)) by [@chaliy](https://github.com/chaliy)
- fix(storage): clear existing default model before setting new one ([#797](https://github.com/everruns/everruns/pull/797)) by [@chaliy](https://github.com/chaliy)
- fix(ui): close global search on ESC and route navigation ([#795](https://github.com/everruns/everruns/pull/795)) by [@chaliy](https://github.com/chaliy)
- fix(storage): replace ILIKE event search with GIN-indexed tsvector ([#787](https://github.com/everruns/everruns/pull/787)) by [@chaliy](https://github.com/chaliy)
- fix(durable): use SELECT COUNT(*) for event counting ([#782](https://github.com/everruns/everruns/pull/782)) by [@chaliy](https://github.com/chaliy)
- fix(slack): ensure long_description meets Slack's 174-char minimum ([#780](https://github.com/everruns/everruns/pull/780)) by [@chaliy](https://github.com/chaliy)
- fix(ui): show completed turn duration in chat ([#778](https://github.com/everruns/everruns/pull/778)) by [@chaliy](https://github.com/chaliy)
- fix(docs): upgrade Astro docs site to v6 ([#777](https://github.com/everruns/everruns/pull/777)) by [@chaliy](https://github.com/chaliy)
- fix(slack): expose external_actor in API Message response ([#771](https://github.com/everruns/everruns/pull/771)) by [@chaliy](https://github.com/chaliy)
- fix(apps): auto-complete Slack setup checklist steps 4 and 5 ([#770](https://github.com/everruns/everruns/pull/770)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): use git clone via exec instead of broken /git/clone endpoint ([#766](https://github.com/everruns/everruns/pull/766)) by [@chaliy](https://github.com/chaliy)
- fix(worker): wire all stores into durable act_activity ([#763](https://github.com/everruns/everruns/pull/763)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): use /home/daytona as workspace path ([#765](https://github.com/everruns/everruns/pull/765)) by [@chaliy](https://github.com/chaliy)
- fix(daytona): remove API key prerequisite from Coder agent prompt ([#757](https://github.com/everruns/everruns/pull/757)) by [@chaliy](https://github.com/chaliy)
- fix(ui): simplify inline tool transcript ([#758](https://github.com/everruns/everruns/pull/758)) by [@chaliy](https://github.com/chaliy)
- fix(session-files): return 409 instead of 500 on duplicate file creation ([#755](https://github.com/everruns/everruns/pull/755)) by [@chaliy](https://github.com/chaliy)
- fix(server): increase session files upload body limit to 10MB ([#751](https://github.com/everruns/everruns/pull/751)) by [@chaliy](https://github.com/chaliy)
- fix(ui): render folder action icons inline with folder name ([#752](https://github.com/everruns/everruns/pull/752)) by [@chaliy](https://github.com/chaliy)
- fix(ui): fix code block rendering in chat messages ([#753](https://github.com/everruns/everruns/pull/753)) by [@chaliy](https://github.com/chaliy)
- fix(vfs): correct folder detection in stat() ([#749](https://github.com/everruns/everruns/pull/749)) by [@chaliy](https://github.com/chaliy)
- fix(ui): prevent workspace file tree from overflowing viewport ([#750](https://github.com/everruns/everruns/pull/750)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump fetchkit from 0.1.2 to 0.1.3 ([#802](https://github.com/everruns/everruns/pull/802))
- chore(migrations): squash post-0.8.4 migrations into 006_v0.8.5 ([#792](https://github.com/everruns/everruns/pull/792)) by [@chaliy](https://github.com/chaliy)
- test(slack): enforce credentials, add Slack integration tests to CI ([#789](https://github.com/everruns/everruns/pull/789)) by [@chaliy](https://github.com/chaliy)
- test(daytona): add live API integration tests ([#774](https://github.com/everruns/everruns/pull/774)) by [@chaliy](https://github.com/chaliy)

## [0.8.4] - 2026-03-08

### Highlights

- **Brave Search** — New connection provider and seed agent for Brave Search web search ([#716](https://github.com/everruns/everruns/pull/716))
- **Slack Bot** — Event-driven delivery dispatcher, file/legacy attachment support, per-app manifest generation ([#696](https://github.com/everruns/everruns/pull/696), [#717](https://github.com/everruns/everruns/pull/717), [#689](https://github.com/everruns/everruns/pull/689))
- **Performance Caching** — In-memory caches for encryption keys, model resolution, auth validation, skills, and agent capabilities ([#700](https://github.com/everruns/everruns/pull/700), [#701](https://github.com/everruns/everruns/pull/701), [#702](https://github.com/everruns/everruns/pull/702), [#705](https://github.com/everruns/everruns/pull/705), [#706](https://github.com/everruns/everruns/pull/706))
- **Valkey Rate Limiting** — Distributed rate limiting via Valkey replaces in-process limiters ([#690](https://github.com/everruns/everruns/pull/690))
- **Tool Search** — OpenAI GPT 5.4 tool_search capability for deferred tool loading ([#687](https://github.com/everruns/everruns/pull/687))

### What's Changed

- feat(brave-search): add connection provider, seed agent, and Doppler CI ([#716](https://github.com/everruns/everruns/pull/716)) by [@chaliy](https://github.com/chaliy)
- feat(slack): support file and legacy attachments in messages ([#717](https://github.com/everruns/everruns/pull/717)) by [@chaliy](https://github.com/chaliy)
- feat(ui): unify chat and shell slate styling ([#718](https://github.com/everruns/everruns/pull/718)) by [@chaliy](https://github.com/chaliy)
- feat(ui): show ngrok instructions when Slack webhook URL is localhost ([#714](https://github.com/everruns/everruns/pull/714)) by [@chaliy](https://github.com/chaliy)
- feat(ui): show display names instead of raw IDs in all select dropdowns ([#713](https://github.com/everruns/everruns/pull/713)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add title and experimental badge to Chat page ([#711](https://github.com/everruns/everruns/pull/711)) by [@chaliy](https://github.com/chaliy)
- feat(ui): pluggable logout and createOrganization via AuthContext ([#709](https://github.com/everruns/everruns/pull/709)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add sidebar navigation registry with extension points ([#707](https://github.com/everruns/everruns/pull/707)) by [@chaliy](https://github.com/chaliy)
- feat(build): add optional sccache integration with S3 backend ([#704](https://github.com/everruns/everruns/pull/704)) by [@chaliy](https://github.com/chaliy)
- feat(slack): event-driven delivery dispatcher replaces 120s polling ([#696](https://github.com/everruns/everruns/pull/696)) by [@chaliy](https://github.com/chaliy)
- feat(ci): add sccache S3 backend for shared Rust compilation cache ([#693](https://github.com/everruns/everruns/pull/693)) by [@chaliy](https://github.com/chaliy)
- feat(durable): add generic queue semantics for standalone tasks ([#691](https://github.com/everruns/everruns/pull/691)) by [@chaliy](https://github.com/chaliy)
- feat(server): add Valkey for distributed rate limiting ([#690](https://github.com/everruns/everruns/pull/690)) by [@chaliy](https://github.com/chaliy)
- feat(slack): per-app manifest generation and setup guide ([#689](https://github.com/everruns/everruns/pull/689)) by [@chaliy](https://github.com/chaliy)
- feat(core): implement OpenAI tool_search capability for deferred tool loading ([#687](https://github.com/everruns/everruns/pull/687)) by [@chaliy](https://github.com/chaliy)
- feat(core): add ExternalActor for channel-agnostic user identity ([#688](https://github.com/everruns/everruns/pull/688)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add experimental badge for Chat and Apps ([#684](https://github.com/everruns/everruns/pull/684)) by [@chaliy](https://github.com/chaliy)
- feat(core): add apps feature flag ([#685](https://github.com/everruns/everruns/pull/685)) by [@chaliy](https://github.com/chaliy)
- feat(ui): redesign app creation flow with non-modal Slack config ([#683](https://github.com/everruns/everruns/pull/683)) by [@chaliy](https://github.com/chaliy)
- fix(dev): restore stop-all cleanup and caddy validation ([#728](https://github.com/everruns/everruns/pull/728)) by [@chaliy](https://github.com/chaliy)
- fix(multitenancy): remove DEFAULT_ORG_ID fallbacks from worker runtime paths ([#727](https://github.com/everruns/everruns/pull/727)) by [@chaliy](https://github.com/chaliy)
- fix(dev-parity): implement grep_files in DirectWorkerAdapters by [@chaliy](https://github.com/chaliy)
- fix(dev): isolate worktree port layout ([#726](https://github.com/everruns/everruns/pull/726)) by [@chaliy](https://github.com/chaliy)
- fix(model-sync): use decrypted provider keys and sync across all orgs by [@chaliy](https://github.com/chaliy)
- fix(mcp): pass decrypted API keys to MCP tool execution by [@chaliy](https://github.com/chaliy)
- fix(auth): validate GitHub App installation callback state by [@chaliy](https://github.com/chaliy)
- fix(ui): simplify bash tool result details ([#725](https://github.com/everruns/everruns/pull/725)) by [@chaliy](https://github.com/chaliy)
- fix(ui): move Event Subscriptions card inside grid layout ([#715](https://github.com/everruns/everruns/pull/715)) by [@chaliy](https://github.com/chaliy)
- fix(slack): add missing users:read scope to bot manifest ([#712](https://github.com/everruns/everruns/pull/712)) by [@chaliy](https://github.com/chaliy)
- fix(ui): stop experimental badge overlapping chat content ([#699](https://github.com/everruns/everruns/pull/699)) by [@chaliy](https://github.com/chaliy)
- fix(scripts): prevent init-cloud-env hangs on downloads ([#698](https://github.com/everruns/everruns/pull/698)) by [@chaliy](https://github.com/chaliy)
- fix(core): move tool_search guard to RuntimeAgentBuilder ([#703](https://github.com/everruns/everruns/pull/703)) by [@chaliy](https://github.com/chaliy)
- fix(slack): correct answer mapping, dedup events, and stream progress ([#686](https://github.com/everruns/everruns/pull/686)) by [@chaliy](https://github.com/chaliy)
- fix(docs): correct Slack OAuth scope from app_mentions:events to app_mentions:read ([#682](https://github.com/everruns/everruns/pull/682)) by [@chaliy](https://github.com/chaliy)
- fix: deduplicate moka workspace dep + add sequential merge spec ([#708](https://github.com/everruns/everruns/pull/708)) by [@chaliy](https://github.com/chaliy)
- perf(ci): build all Rust Docker images in single builder stage ([#710](https://github.com/everruns/everruns/pull/710)) by [@chaliy](https://github.com/chaliy)
- perf(encryption): cache decrypted encryption keys in memory ([#706](https://github.com/everruns/everruns/pull/706)) by [@chaliy](https://github.com/chaliy)
- perf(server): deduplicate get_agent_capabilities() calls ([#705](https://github.com/everruns/everruns/pull/705)) by [@chaliy](https://github.com/chaliy)
- perf(skills): cache active skills list per org with 5-min TTL ([#702](https://github.com/everruns/everruns/pull/702)) by [@chaliy](https://github.com/chaliy)
- perf(llm): cache model/provider resolution with 1-hour TTL ([#701](https://github.com/everruns/everruns/pull/701)) by [@chaliy](https://github.com/chaliy)
- perf(auth): cache API key auth validation with 5-min TTL ([#700](https://github.com/everruns/everruns/pull/700)) by [@chaliy](https://github.com/chaliy)
- refactor(server): centralize runtime credential and grep resolution paths ([#731](https://github.com/everruns/everruns/pull/731)) by [@chaliy](https://github.com/chaliy)
- refactor: remove CodeSandbox integration ([#719](https://github.com/everruns/everruns/pull/719), [#720](https://github.com/everruns/everruns/pull/720)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): standardize page headers to inline pattern ([#694](https://github.com/everruns/everruns/pull/694)) by [@chaliy](https://github.com/chaliy)
- test(runtime): add adapter contract coverage and security-focused negative tests ([#730](https://github.com/everruns/everruns/pull/730)) by [@chaliy](https://github.com/chaliy)
- chore(server): squash v0.8.4 migration into 005_v0.8.4.sql ([#733](https://github.com/everruns/everruns/pull/733)) by [@chaliy](https://github.com/chaliy)
- chore(dev): remove Jaeger UI from local dev and example ([#729](https://github.com/everruns/everruns/pull/729)) by [@chaliy](https://github.com/chaliy)
- chore: reorganize Linear tickets to OSS project ([#695](https://github.com/everruns/everruns/pull/695)) by [@chaliy](https://github.com/chaliy)
- refactor(multitenancy): thread org_id through worker and image-resolution interfaces ([#732](https://github.com/everruns/everruns/pull/732)) by [@chaliy](https://github.com/chaliy)
- ci: optimize pipeline — pre-build test binary, 8-core runner, combine test invocations ([#697](https://github.com/everruns/everruns/pull/697)) by [@chaliy](https://github.com/chaliy)

## [0.8.3] - 2026-03-06

### Highlights

- **GPT-5.4 Support** — Full model profiles for GPT-5.4 and GPT-5.4 Pro with input token limits ([#653](https://github.com/everruns/everruns/pull/653), [#654](https://github.com/everruns/everruns/pull/654), [#657](https://github.com/everruns/everruns/pull/657))
- **Custom Commands** — Slash command system with UI autocomplete ([#667](https://github.com/everruns/everruns/pull/667))
- **Security Hardening** — Per-IP rate limiting, structured audit logging, mTLS, security headers, account enumeration prevention ([#627](https://github.com/everruns/everruns/pull/627), [#633](https://github.com/everruns/everruns/pull/633), [#634](https://github.com/everruns/everruns/pull/634), [#636](https://github.com/everruns/everruns/pull/636), [#641](https://github.com/everruns/everruns/pull/641))
- **Durable Engine Scaling** — Multi-instance control plane, capacity-aware fair-share claiming, worker backpressure ([#637](https://github.com/everruns/everruns/pull/637), [#638](https://github.com/everruns/everruns/pull/638), [#639](https://github.com/everruns/everruns/pull/639), [#640](https://github.com/everruns/everruns/pull/640))
- **DuckDuckGo Search** — DuckDuckGo Instant Answer search integration ([#663](https://github.com/everruns/everruns/pull/663))

### What's Changed

- feat(core): add GPT-5.4 and GPT-5.4 Pro model profiles ([#653](https://github.com/everruns/everruns/pull/653)) by [@chaliy](https://github.com/chaliy)
- feat(core): add GPT-5.4 model profiles and integration tests ([#654](https://github.com/everruns/everruns/pull/654)) by [@chaliy](https://github.com/chaliy)
- feat(core): add optional input token limit to LlmModelLimits ([#657](https://github.com/everruns/everruns/pull/657)) by [@chaliy](https://github.com/chaliy)
- feat(commands): add custom commands system with UI autocomplete ([#667](https://github.com/everruns/everruns/pull/667)) by [@chaliy](https://github.com/chaliy)
- feat(duckduckgo): add DuckDuckGo Instant Answer search integration ([#663](https://github.com/everruns/everruns/pull/663)) by [@chaliy](https://github.com/chaliy)
- feat(users): add profile page with full name editing ([#649](https://github.com/everruns/everruns/pull/649)) by [@chaliy](https://github.com/chaliy)
- feat(ui): Claude Code-style bash tool rendering ([#644](https://github.com/everruns/everruns/pull/644)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): add list_capabilities tool to platform management ([#642](https://github.com/everruns/everruns/pull/642)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): add risk level classification and admin approval ([#631](https://github.com/everruns/everruns/pull/631)) by [@chaliy](https://github.com/chaliy)
- feat(grpc): add mutual TLS (mTLS) support for worker-server communication ([#641](https://github.com/everruns/everruns/pull/641)) by [@chaliy](https://github.com/chaliy)
- feat(grpc): add gRPC support for sqldb_store in WorkerAdapters ([#645](https://github.com/everruns/everruns/pull/645)) by [@chaliy](https://github.com/chaliy)
- feat(durable): resource-based worker backpressure ([#638](https://github.com/everruns/everruns/pull/638)) by [@chaliy](https://github.com/chaliy)
- feat(durable): capacity-aware fair-share task claiming ([#639](https://github.com/everruns/everruns/pull/639)) by [@chaliy](https://github.com/chaliy)
- feat(durable): load-proportional claim jitter ([#640](https://github.com/everruns/everruns/pull/640)) by [@chaliy](https://github.com/chaliy)
- feat(server): multi-instance control plane support ([#637](https://github.com/everruns/everruns/pull/637)) by [@chaliy](https://github.com/chaliy)
- feat(server): structured audit logging for auth events ([#636](https://github.com/everruns/everruns/pull/636)) by [@chaliy](https://github.com/chaliy)
- feat(server): add security response headers ([#634](https://github.com/everruns/everruns/pull/634)) by [@chaliy](https://github.com/chaliy)
- feat(auth): add per-IP rate limiting on auth endpoints ([#627](https://github.com/everruns/everruns/pull/627)) by [@chaliy](https://github.com/chaliy)
- feat(storage): add encrypted system_prompt columns ([#630](https://github.com/everruns/everruns/pull/630)) by [@chaliy](https://github.com/chaliy)
- feat(chat): add run-agent, harness-avoidance, and confirmation guidelines to chat system prompt ([#648](https://github.com/everruns/everruns/pull/648)) by [@chaliy](https://github.com/chaliy)
- feat(docs): add horizontal navigation tabs ([#662](https://github.com/everruns/everruns/pull/662)) by [@chaliy](https://github.com/chaliy)
- feat: Slack bot integration with Apps abstraction ([#671](https://github.com/everruns/everruns/pull/671)) by [@chaliy](https://github.com/chaliy)
- fix(ui): provider icons invisible on light theme ([#670](https://github.com/everruns/everruns/pull/670)) by [@chaliy](https://github.com/chaliy)
- fix(ui): render plain URLs as links in chat markdown ([#646](https://github.com/everruns/everruns/pull/646)) by [@chaliy](https://github.com/chaliy)
- fix(ui): default to Generic harness in New Session dialog ([#632](https://github.com/everruns/everruns/pull/632)) by [@chaliy](https://github.com/chaliy)
- fix(vfs): block deletion of readonly files ([#669](https://github.com/everruns/everruns/pull/669)) by [@chaliy](https://github.com/chaliy)
- fix(capabilities): scope platform store to session org and fix public URL default ([#647](https://github.com/everruns/everruns/pull/647)) by [@chaliy](https://github.com/chaliy)
- fix(auth): prevent account enumeration via registration endpoint ([#633](https://github.com/everruns/everruns/pull/633)) by [@chaliy](https://github.com/chaliy)
- fix(api): add regex pattern length limit on grep endpoint ([#629](https://github.com/everruns/everruns/pull/629)) by [@chaliy](https://github.com/chaliy)
- fix(server): warn when DATABASE_URL lacks TLS in production ([#628](https://github.com/everruns/everruns/pull/628)) by [@chaliy](https://github.com/chaliy)
- fix(worker): enforce WorkerAdapters parity at compile time ([#643](https://github.com/everruns/everruns/pull/643)) by [@chaliy](https://github.com/chaliy)
- fix(docker): pin UI builder stage to amd64 to avoid QEMU SIGILL ([#652](https://github.com/everruns/everruns/pull/652)) by [@chaliy](https://github.com/chaliy)
- fix(ci): merge env-var SSE tests to prevent flaky race condition ([#656](https://github.com/everruns/everruns/pull/656)) by [@chaliy](https://github.com/chaliy)
- fix(ci): skip arm64 QEMU build for UI Docker image ([#651](https://github.com/everruns/everruns/pull/651)) by [@chaliy](https://github.com/chaliy)
- refactor(capabilities): adjust risk levels, rename capabilities, add Daytona docs ([#675](https://github.com/everruns/everruns/pull/675)) by [@chaliy](https://github.com/chaliy)
- refactor: rename GRPC_* env vars to WORKER_GRPC_* prefix ([#635](https://github.com/everruns/everruns/pull/635)) by [@chaliy](https://github.com/chaliy)
- revert(server): remove system_prompt_encrypted ([#659](https://github.com/everruns/everruns/pull/659)) by [@chaliy](https://github.com/chaliy)
- docs(capabilities): add Capabilities navigation tab with top 15 capability reference pages ([#672](https://github.com/everruns/everruns/pull/672)) by [@chaliy](https://github.com/chaliy)
- docs: add tutorial for building agents using the Everruns SDK ([#661](https://github.com/everruns/everruns/pull/661)) by [@chaliy](https://github.com/chaliy)
- docs: reduce duplication in building-agents-using-sdk tutorial ([#674](https://github.com/everruns/everruns/pull/674)) by [@chaliy](https://github.com/chaliy)
- docs: improve meta descriptions for SEO ([#660](https://github.com/everruns/everruns/pull/660)) by [@chaliy](https://github.com/chaliy)
- chore: pre-release maintenance — update dependencies ([#668](https://github.com/everruns/everruns/pull/668)) by [@chaliy](https://github.com/chaliy)
- chore(migrations): squash 005_apps into 004_v0.8.3 ([#678](https://github.com/everruns/everruns/pull/678)) by [@chaliy](https://github.com/chaliy)
- chore(db): squash migrations 004-006 into 004_v0.8.3 ([#673](https://github.com/everruns/everruns/pull/673)) by [@chaliy](https://github.com/chaliy)
- chore(deps): bump dompurify from 3.3.1 to 3.3.2 in /apps/docs ([#655](https://github.com/everruns/everruns/pull/655)) by [@dependabot](https://github.com/dependabot)
- chore(deps): bump svgo from 4.0.0 to 4.0.1 in /apps/docs ([#650](https://github.com/everruns/everruns/pull/650)) by [@dependabot](https://github.com/dependabot)
- chore(docs): add IndexNow verification key file ([#658](https://github.com/everruns/everruns/pull/658)) by [@chaliy](https://github.com/chaliy)

### Migration Notes

**0.8.2 → 0.8.3:** Requires fresh database (new migration squash). The `GRPC_*` environment variables have been renamed to `WORKER_GRPC_*` — update your configuration accordingly.

## [0.8.2] - 2026-03-01

### Highlights

- **Global Chat Page** — New global chat page and Chat harness for direct agent conversations ([#602](https://github.com/everruns/everruns/pull/602), [#608](https://github.com/everruns/everruns/pull/608))
- **Platform Management Capability** — New capability for platform operations, wired through Chat harness and gRPC workers ([#587](https://github.com/everruns/everruns/pull/587), [#608](https://github.com/everruns/everruns/pull/608), [#615](https://github.com/everruns/everruns/pull/615), [#622](https://github.com/everruns/everruns/pull/622))
- **SSE Reliability** — Periodic heartbeat comments, HTTP/2 flow control tuning, 1000-connection limit, reliable event ordering via sequence resolution ([#604](https://github.com/everruns/everruns/pull/604), [#584](https://github.com/everruns/everruns/pull/584), [#585](https://github.com/everruns/everruns/pull/585), [#597](https://github.com/everruns/everruns/pull/597), [#606](https://github.com/everruns/everruns/pull/606))
- **Durable Dashboard Metrics** — Time-series graphs, accurate worker counts, throughput rates, and dev-mode support ([#578](https://github.com/everruns/everruns/pull/578), [#590](https://github.com/everruns/everruns/pull/590), [#596](https://github.com/everruns/everruns/pull/596), [#610](https://github.com/everruns/everruns/pull/610))
- **OpenAI Context Caching** — Thread `previous_response_id` for server-side context caching across turns ([#594](https://github.com/everruns/everruns/pull/594))

### What's Changed

- fix(worker): wire platform_store in DurableWorker act_activity path ([#622](https://github.com/everruns/everruns/pull/622))
- chore: add /process-issues command for Linear issue processing ([#619](https://github.com/everruns/everruns/pull/619))
- chore(specs): security audit — 12 findings with threat model updates ([#618](https://github.com/everruns/everruns/pull/618))
- feat(ship): add code simplification and security review phases ([#616](https://github.com/everruns/everruns/pull/616))
- chore(ship): add impact awareness to Phase 5 quality gates ([#617](https://github.com/everruns/everruns/pull/617))
- fix(server): downgrade missing directory log from error to debug ([#614](https://github.com/everruns/everruns/pull/614))
- fix(worker): implement PlatformStore for gRPC workers ([#615](https://github.com/everruns/everruns/pull/615))
- chore(deps): upgrade bashkit v0.1.7 → v0.1.8 ([#613](https://github.com/everruns/everruns/pull/613))
- fix(example): always pull fresh images on start ([#612](https://github.com/everruns/everruns/pull/612))
- chore(deps): upgrade everruns-sdk v0.1.2 → v0.1.3 ([#611](https://github.com/everruns/everruns/pull/611))
- fix(durable): fix dashboard metrics for dev mode and show all statuses ([#610](https://github.com/everruns/everruns/pull/610))
- feat(ui): add preview tabs to harness detail and edit pages ([#607](https://github.com/everruns/everruns/pull/607))
- refactor(bench): always use llmsim-latency model in load tests ([#609](https://github.com/everruns/everruns/pull/609))
- feat(capabilities): register platform_management in Chat harness, rename to Platform Chat ([#608](https://github.com/everruns/everruns/pull/608))
- fix(server): resolve since_id to sequence for reliable event ordering ([#606](https://github.com/everruns/everruns/pull/606))
- chore(deps): bump the npm_and_yarn group across 1 directory with 1 update ([#605](https://github.com/everruns/everruns/pull/605))
- feat(sse): add periodic heartbeat comments to all SSE streams ([#604](https://github.com/everruns/everruns/pull/604))
- feat(ui,server): add global chat page and Chat harness ([#602](https://github.com/everruns/everruns/pull/602))
- fix(server): eliminate env var race in Http2FlowConfig tests ([#601](https://github.com/everruns/everruns/pull/601))
- fix(load-test): remove SSE retry cap to use SDK default unlimited reconnects ([#599](https://github.com/everruns/everruns/pull/599))
- chore(deps): upgrade bashkit v0.1.6 → v0.1.7 ([#598](https://github.com/everruns/everruns/pull/598))
- fix(sse): bump SDK with SSE disconnect fix, configurable cycling ([#597](https://github.com/everruns/everruns/pull/597))
- fix(durable): show workflow/task throughput rates instead of gauges ([#596](https://github.com/everruns/everruns/pull/596))
- fix(docs): block /cdn-cgi/ in robots.txt ([#595](https://github.com/everruns/everruns/pull/595))
- feat(core): thread previous_response_id for OpenAI server-side context caching ([#594](https://github.com/everruns/everruns/pull/594))
- fix(ci): stop cancelling in-progress CI runs on main ([#593](https://github.com/everruns/everruns/pull/593))
- chore: upgrade Node.js from 20 to 22 LTS ([#592](https://github.com/everruns/everruns/pull/592))
- chore(ui): update oxfmt to 0.35.0 ([#591](https://github.com/everruns/everruns/pull/591))
- fix(durable): fix dashboard worker count and metrics accuracy ([#590](https://github.com/everruns/everruns/pull/590))
- fix(docs): resolve SEO crawling issues (308 redirects, excluded pages) ([#589](https://github.com/everruns/everruns/pull/589))
- fix(bench): eagerly connect SSE stream before sending messages ([#588](https://github.com/everruns/everruns/pull/588))
- feat(core): add platform management capability ([#587](https://github.com/everruns/everruns/pull/587))
- fix(server): configure HTTP/2 flow control for high-concurrency SSE ([#584](https://github.com/everruns/everruns/pull/584))
- chore(specs): remove code-derivable content, link to source files ([#586](https://github.com/everruns/everruns/pull/586))
- fix(server): bump per-org SSE connection limit from 50 to 1000 ([#585](https://github.com/everruns/everruns/pull/585))
- fix(bench): use SDK EventStream for SSE reconnection in load test ([#582](https://github.com/everruns/everruns/pull/582))
- feat(server): add types positive filter to events endpoints ([#581](https://github.com/everruns/everruns/pull/581))
- feat(durable): add metrics time-series graphs to overview dashboard ([#578](https://github.com/everruns/everruns/pull/578))
- feat(fake_aws): autonomous Cost & Security Auditor with rich seed data ([#577](https://github.com/everruns/everruns/pull/577))
- feat(bench): replace polling with SSE for turn completion detection ([#580](https://github.com/everruns/everruns/pull/580))
- fix(durable): add summary to workers list response ([#579](https://github.com/everruns/everruns/pull/579))
- fix(server): use axum_extra::extract::Query for SSE exclude param ([#575](https://github.com/everruns/everruns/pull/575))
- chore(ship): expand /ship command to enforce full quality workflow ([#576](https://github.com/everruns/everruns/pull/576))
- feat(llmsim): add latency and streaming simulation for benchmarks ([#574](https://github.com/everruns/everruns/pull/574))
- fix(ui): merge orphan input.message into turn.started in trajectory view ([#573](https://github.com/everruns/everruns/pull/573))
- refactor(server): extract ServerAppBuilder, remove server::run() ([#572](https://github.com/everruns/everruns/pull/572))
- fix(durable): skip postgres-dependent tests without PostgreSQL ([#571](https://github.com/everruns/everruns/pull/571))

## [0.8.1] - 2026-02-22

### Highlights

- **Load Testing Infrastructure** — New load testing framework with llmsim mock LLM server and durable execution race condition fix ([#568](https://github.com/everruns/everruns/pull/568))
- **Dashboard Stats** — Total sessions count and improved session stats accuracy ([#564](https://github.com/everruns/everruns/pull/564))
- **CI & Docs Improvements** — Docker publish fix for release tags, SEO fixes, and bashkit docs ([#563](https://github.com/everruns/everruns/pull/563), [#566](https://github.com/everruns/everruns/pull/566), [#567](https://github.com/everruns/everruns/pull/567))

### What's Changed

- feat(load-test): add load testing infrastructure with llmsim and durable race fix ([#568](https://github.com/everruns/everruns/pull/568))
- docs: fix SEO issues across docs site ([#567](https://github.com/everruns/everruns/pull/567))
- docs(ecosystem): add bashkit overview and hide SRE sidebar ([#566](https://github.com/everruns/everruns/pull/566))
- chore(specs): add performance impact guidelines to pre-PR and maintenance checklists ([#565](https://github.com/everruns/everruns/pull/565))
- feat(dashboard): add total sessions count and fix session stats accuracy ([#564](https://github.com/everruns/everruns/pull/564))
- fix(ci): trigger Docker Publish for release tags via workflow_dispatch ([#563](https://github.com/everruns/everruns/pull/563))

## [0.8.0] - 2026-02-21

### Highlights

- **Built-in Skills Discovery** — Skills capability with system prompt integration and Generic harness support ([#516](https://github.com/everruns/everruns/pull/516), [#532](https://github.com/everruns/everruns/pull/532), [#543](https://github.com/everruns/everruns/pull/543))
- **Daytona Integration** — User connection with API key support and official branding ([#522](https://github.com/everruns/everruns/pull/522), [#533](https://github.com/everruns/everruns/pull/533))
- **Generic Harness Type** — New Generic harness with skills, agent_instructions, and copy endpoints ([#512](https://github.com/everruns/everruns/pull/512), [#518](https://github.com/everruns/everruns/pull/518), [#524](https://github.com/everruns/everruns/pull/524))
- **Claude Sonnet 4.6 & Opus 4.6** — New model profiles for latest Claude models ([#531](https://github.com/everruns/everruns/pull/531))
- **Session-Scoped Task Scheduling** — Cron-based scheduled tasks scoped to sessions ([#536](https://github.com/everruns/everruns/pull/536))

### What's Changed

- docs: fix Daytona integration image ([#561](https://github.com/everruns/everruns/pull/561))
- fix(bash): set executable file mode for script execution ([#559](https://github.com/everruns/everruns/pull/559))
- fix(durable): optimize slow claim_due_schedules scheduler query ([#558](https://github.com/everruns/everruns/pull/558))
- chore(deps): update bashkit v0.1.5 → v0.1.6 ([#556](https://github.com/everruns/everruns/pull/556))
- docs: fix duplicate titles, dark-mode logos, and add Braintrust icon ([#557](https://github.com/everruns/everruns/pull/557))
- docs: fix duplicate titles, logo visibility, and redesign home page ([#555](https://github.com/everruns/everruns/pull/555))
- fix(docs): correct edit page link URL for all doc pages ([#554](https://github.com/everruns/everruns/pull/554))
- chore(docs): add Google site verification meta tag ([#553](https://github.com/everruns/everruns/pull/553))
- refactor(migrations): squash post-0.7.0 migrations into single 003_v0.8.0 ([#552](https://github.com/everruns/everruns/pull/552))
- chore: pre-release maintenance — deps, specs, threat model, code cleanup ([#551](https://github.com/everruns/everruns/pull/551))
- feat(capabilities): add features() for UI-driven tab rendering ([#550](https://github.com/everruns/everruns/pull/550))
- fix(ui): fix llm.generation preview modal layout and rename button to View ([#549](https://github.com/everruns/everruns/pull/549))
- refactor(skills): separate AttachSkillCapability (mount-only) from SkillsCapability (discovery+tools) ([#548](https://github.com/everruns/everruns/pull/548))
- docs(skills): add overview video, split into skills + skills-registry ([#547](https://github.com/everruns/everruns/pull/547))
- feat(ui): add frontmatter support to markdown file preview ([#546](https://github.com/everruns/everruns/pull/546))
- chore(deps): bump devalue from 5.6.2 to 5.6.3 in /apps/docs ([#545](https://github.com/everruns/everruns/pull/545))
- fix(durable): gate postgres integration tests behind feature flag ([#544](https://github.com/everruns/everruns/pull/544))
- feat(skills): include first 15 skill descriptions in system prompt ([#543](https://github.com/everruns/everruns/pull/543))
- fix(ui): fix chat input panel and sidebar layout alignment ([#542](https://github.com/everruns/everruns/pull/542))
- feat: add `just start-production` command ([#541](https://github.com/everruns/everruns/pull/541))
- fix(llm): detect model-not-found errors and surface user-friendly message ([#540](https://github.com/everruns/everruns/pull/540))
- docs: add sitemap.xml with lastmod dates ([#539](https://github.com/everruns/everruns/pull/539))
- feat(docs): add Bing meta validation tag, robots.txt with AI crawler rules ([#538](https://github.com/everruns/everruns/pull/538))
- feat(ui): add drag-and-drop file upload to workspace ([#537](https://github.com/everruns/everruns/pull/537))
- feat(schedules): add session-scoped task scheduling ([#536](https://github.com/everruns/everruns/pull/536))
- fix(ui): rename Preview button to View and restore generation visualization ([#535](https://github.com/everruns/everruns/pull/535))
- fix(capabilities): respect /workspace prefix in skills agent-facing paths ([#534](https://github.com/everruns/everruns/pull/534))
- feat(daytona): add official Daytona logo icon and integration docs ([#533](https://github.com/everruns/everruns/pull/533))
- feat(harness): add skills capability to Generic harness ([#532](https://github.com/everruns/everruns/pull/532))
- feat(models): add Claude Sonnet 4.6 and Opus 4.6 model profiles ([#531](https://github.com/everruns/everruns/pull/531))
- refactor(capabilities): make Capability trait async for dynamic system prompt content ([#530](https://github.com/everruns/everruns/pull/530))
- feat(commands): add /ship command for automated ship flow ([#529](https://github.com/everruns/everruns/pull/529))
- fix(auth): auto-refresh expired tokens and preserve page on re-login ([#528](https://github.com/everruns/everruns/pull/528))
- fix(ui): render connection instructions as markdown ([#527](https://github.com/everruns/everruns/pull/527))
- feat(ui): add llm.generation filter to session events ([#526](https://github.com/everruns/everruns/pull/526))
- chore(agents): update pre-PR checklist and add shipping definition ([#525](https://github.com/everruns/everruns/pull/525))
- fix(harness): include agent_instructions in Generic harness ([#524](https://github.com/everruns/everruns/pull/524))
- fix(ui): fix workspace refresh button and auto-refresh on tab switch ([#523](https://github.com/everruns/everruns/pull/523))
- feat(connections): add Daytona user connection with API key support ([#522](https://github.com/everruns/everruns/pull/522))
- feat(ui): reorganize sidebar navigation ([#521](https://github.com/everruns/everruns/pull/521))
- fix(worker): increase control-plane connection timeout from 5s to 30s ([#520](https://github.com/everruns/everruns/pull/520))
- fix(ui): fix settings panel not filling full height ([#519](https://github.com/everruns/everruns/pull/519))
- feat(agents,harnesses): add copy endpoints ([#518](https://github.com/everruns/everruns/pull/518))
- fix(worker): register harness capability tools when agent_id is absent ([#517](https://github.com/everruns/everruns/pull/517))
- feat(capabilities): add built-in skills discovery capability ([#516](https://github.com/everruns/everruns/pull/516))
- feat(ui): add Schedules link to sidebar navigation ([#515](https://github.com/everruns/everruns/pull/515))
- chore(deps): update bashkit from v0.1.4 to v0.1.5 ([#514](https://github.com/everruns/everruns/pull/514))
- fix(seed): upsert seed data with change detection ([#513](https://github.com/everruns/everruns/pull/513))
- feat(harness): rename Default to Base, add Generic harness type ([#512](https://github.com/everruns/everruns/pull/512))
- test: remove 11 ineffective tests ([#511](https://github.com/everruns/everruns/pull/511))

### Migration Notes

**0.7.0 → 0.8.0:** This release includes database schema changes (session-scoped scheduling, Generic harness type, migration squash). A fresh database is required — no automatic migration is supported.

## [0.7.0] - 2026-02-13

### Highlights

- **Skills Registry** — Agent skills registry with top-level navigation and agentskills.io format ([#460](https://github.com/everruns/everruns/pull/460))
- **Harness Abstraction** — New Harness entity between Organization and Agent for flexible grouping ([#434](https://github.com/everruns/everruns/pull/434))
- **Google Gemini Support** — Native Gemini API driver with parametrized LLM integration tests ([#437](https://github.com/everruns/everruns/pull/437))
- **AGENTS.md Support** — New agent_instructions capability for dynamic project instructions ([#449](https://github.com/everruns/everruns/pull/449))
- **Client-Side Tool Calls & Native Images** — Support for client-side tool execution and native image support in tool results ([#443](https://github.com/everruns/everruns/pull/443), [#442](https://github.com/everruns/everruns/pull/442))

### What's Changed

- refactor(migrations): squash SQL migrations to base and durable ([#462](https://github.com/everruns/everruns/pull/462))
- fix: address 3 urgent Linear issues (EVE-5, EVE-6, EVE-8) ([#461](https://github.com/everruns/everruns/pull/461))
- feat(skills): add skills registry with top-level navigation ([#460](https://github.com/everruns/everruns/pull/460))
- fix: rename LINEAR_MCP_API_KEY to LINEAR_API_KEY ([#459](https://github.com/everruns/everruns/pull/459))
- chore: add Linear MCP server configuration ([#458](https://github.com/everruns/everruns/pull/458))
- docs(ui): remove dev-focused sections from Management UI doc ([#457](https://github.com/everruns/everruns/pull/457))
- docs: rename sidebar entries, promote Event Reference, clean up UI doc ([#456](https://github.com/everruns/everruns/pull/456))
- feat(core): wrap system prompt sections in XML tags ([#455](https://github.com/everruns/everruns/pull/455))
- feat(harness): add Harness abstraction between Organization and Agent ([#434](https://github.com/everruns/everruns/pull/434))
- chore(deps): update everruns-sdk to 0.1.2 ([#454](https://github.com/everruns/everruns/pull/454))
- chore(specs): add SDK doc check to maintenance checklist ([#453](https://github.com/everruns/everruns/pull/453))
- chore(deps): upgrade fetchkit to 0.1.1 from crates.io ([#452](https://github.com/everruns/everruns/pull/452))
- chore(specs): align provider type model with app-layer validation ([#451](https://github.com/everruns/everruns/pull/451))
- chore(build): reduce debug binary size and disable incremental in cloud ([#450](https://github.com/everruns/everruns/pull/450))
- feat(core): add agent_instructions capability (AGENTS.md support) ([#449](https://github.com/everruns/everruns/pull/449))
- chore(docs): remove redundant cloud legacy section ([#448](https://github.com/everruns/everruns/pull/448))
- chore(dev): clarify doppler cloud-secret workflow ([#447](https://github.com/everruns/everruns/pull/447))
- feat(test): add SKIP_LLM_INTEGRATION_TESTS_PROVIDERS env var ([#446](https://github.com/everruns/everruns/pull/446))
- feat(auth): pluggable auth backend for SaaS repo support ([#445](https://github.com/everruns/everruns/pull/445))
- fix(ci): handle multiline commit messages in release workflow ([#444](https://github.com/everruns/everruns/pull/444))
- feat(core): add client-side tool calls support ([#443](https://github.com/everruns/everruns/pull/443))
- feat(core): native image support in tool results ([#442](https://github.com/everruns/everruns/pull/442))
- feat(gemini): add Google Gemini API support and parametrize LLM integration tests ([#437](https://github.com/everruns/everruns/pull/437))
- docs: add concepts page with entity diagrams ([#441](https://github.com/everruns/everruns/pull/441))

### Migration Notes

**0.6.0 → 0.7.0:** This release includes database schema changes (Harness abstraction, migration squash). A fresh database is required — no automatic migration is supported.

## [0.6.0] - 2026-02-10

### Highlights

- **Session-Scoped SQL Databases** — Agents can create and query SQLite databases scoped to their session ([#425](https://github.com/everruns/everruns/pull/425))
- **OpenTelemetry Observability** — Full-featured OTel with 13 event types, span hierarchy, and content recording ([#427](https://github.com/everruns/everruns/pull/427))
- **Virtual Bash Capability** — Sandboxed bash execution for agents using bashkit ([#399](https://github.com/everruns/everruns/pull/399))
- **Scheduled Tasks** — Cron-based scheduled task execution for durable workflows ([#405](https://github.com/everruns/everruns/pull/405))
- **Agent Trajectory Visualization** — New UI for visualizing agent execution paths in sessions ([#436](https://github.com/everruns/everruns/pull/436))

### What's Changed

- chore(deps): pre-release maintenance — update deps, specs, and docs ([#439](https://github.com/everruns/everruns/pull/439))
- feat(examples): add HackerNews reader agent example ([#438](https://github.com/everruns/everruns/pull/438))
- feat(ui): agent trajectory visualization in session UI ([#436](https://github.com/everruns/everruns/pull/436))
- chore: add Doppler CLI for secrets management ([#435](https://github.com/everruns/everruns/pull/435))
- chore(deps): update everruns-sdk 0.1→0.1.1 and bashkit v0.1.2→v0.1.4 ([#433](https://github.com/everruns/everruns/pull/433))
- refactor(migrations): squash 6 migrations into 2 logical groups ([#432](https://github.com/everruns/everruns/pull/432))
- chore(specs): add comprehensive threat model with stable IDs ([#431](https://github.com/everruns/everruns/pull/431))
- fix(deps): upgrade llmsim from 0.2.0 to 0.2.1 ([#429](https://github.com/everruns/everruns/pull/429))
- fix(ui): fix workspace file browser display issues ([#428](https://github.com/everruns/everruns/pull/428))
- feat(otel): full-featured OTel with 13 event types, span hierarchy, content recording ([#427](https://github.com/everruns/everruns/pull/427))
- feat(agents): dual-ID pattern with public_id and upsert semantics ([#426](https://github.com/everruns/everruns/pull/426))
- feat(session-sqldb): session-scoped SQL databases ([#425](https://github.com/everruns/everruns/pull/425))
- fix(core): update bashkit to v0.1.2, fix file size in virtual bash ([#424](https://github.com/everruns/everruns/pull/424))
- feat(core): update model profiles for Claude 4.6 and GPT 5.2/5.3 ([#423](https://github.com/everruns/everruns/pull/423))
- feat(ui): replace FileBrowser with AI Elements FileTree ([#422](https://github.com/everruns/everruns/pull/422))
- fix(ci): suppress 'no jobs were run' notifications in release workflow ([#421](https://github.com/everruns/everruns/pull/421))
- fix(ui): remove duplicate Workspace label and fix breadcrumbs ([#419](https://github.com/everruns/everruns/pull/419))
- docs(features): add SDK documentation page ([#417](https://github.com/everruns/everruns/pull/417))
- feat(ui): improve Workspace breadcrumbs visibility ([#415](https://github.com/everruns/everruns/pull/415))
- refactor(test): restructure integration tests with in-process testing and CI optimization ([#395](https://github.com/everruns/everruns/pull/395))
- feat(ci): add UI Jest tests to CI pipeline ([#413](https://github.com/everruns/everruns/pull/413))
- feat(ui): add file previews for Workspace ([#410](https://github.com/everruns/everruns/pull/410))
- feat(durable): add scheduled tasks with cron-based execution ([#405](https://github.com/everruns/everruns/pull/405))
- feat(ui): implement Streamdown for streaming markdown in messages ([#408](https://github.com/everruns/everruns/pull/408))
- fix(api): handle /workspace prefix in filesystem API ([#407](https://github.com/everruns/everruns/pull/407))
- fix(api): accept prefixed EventId for since_id query parameter ([#406](https://github.com/everruns/everruns/pull/406))
- fix(durable): enforce max_attempts when claiming tasks ([#403](https://github.com/everruns/everruns/pull/403))
- test(capabilities): add security limit tests for virtual bash ([#401](https://github.com/everruns/everruns/pull/401))
- feat(capabilities): add virtual bash capability using bashkit ([#399](https://github.com/everruns/everruns/pull/399))
- feat(api): add session-level capabilities configuration ([#396](https://github.com/everruns/everruns/pull/396))
- feat(ci): add CLI e2e tests ([#394](https://github.com/everruns/everruns/pull/394))
- refactor(cli): migrate to everruns-sdk for API client ([#393](https://github.com/everruns/everruns/pull/393))
- fix(example): distinguish local vs example compose containers ([#392](https://github.com/everruns/everruns/pull/392))
- fix(durable): prevent draining workers from claiming new tasks ([#391](https://github.com/everruns/everruns/pull/391))
- fix(ui): remove redundant refresh button from Worker Pool page ([#388](https://github.com/everruns/everruns/pull/388))
- fix(ci): fix release workflow syntax and add manual trigger ([#390](https://github.com/everruns/everruns/pull/390))

### Migration Notes

**0.5.0 → 0.6.0:** This release includes database schema changes (session-scoped SQL databases, migration squash, dual-ID pattern). A fresh database is required — no automatic migration is supported.

## [0.5.0] - 2026-01-30

### Highlights

- **OpenResponses Support** - Added support for [OpenResponses](https://www.openresponses.org/) specification
- **Braintrust Integration** - LLM tracing and observability with Braintrust ([#340](https://github.com/everruns/everruns/pull/340))
- **Simplified API Structure** - Removed org from API paths, now automatically inferred from API key ([#363](https://github.com/everruns/everruns/pull/363))
- **Sessions as Top-Level Entities** - Reworked sessions to be top-level entities under organizations ([#351](https://github.com/everruns/everruns/pull/351))
- **Improved SSE Reliability** - Enhanced durability and graceful disconnect handling for SSE connections ([#387](https://github.com/everruns/everruns/pull/387), [#370](https://github.com/everruns/everruns/pull/370))
- **Automatic Compaction** - Support for `/v1/responses/compact` endpoint with reactive compaction ([#371](https://github.com/everruns/everruns/pull/371))
- **Extended Thinking for Anthropic** - Support for Claude's extended thinking mode with streaming budget tokens ([#338](https://github.com/everruns/everruns/pull/338))

### What's Changed

- fix(durable): graceful SSE disconnect on errors and add comprehensive tests ([#387](https://github.com/everruns/everruns/pull/387))
- chore(deps): update Rust dependencies to latest major versions ([#386](https://github.com/everruns/everruns/pull/386))
- chore(deps): update Rust and UI dependencies ([#385](https://github.com/everruns/everruns/pull/385))
- fix(example): export EVERRUNS_TAG in pull recipe ([#384](https://github.com/everruns/everruns/pull/384))
- fix(durable): populate worker stats in API response ([#383](https://github.com/everruns/everruns/pull/383))
- fix(examples): docker-compose YAML fixes and add image tag option ([#382](https://github.com/everruns/everruns/pull/382))
- feat(ui): adopt oxfmt for JS/TS formatting ([#381](https://github.com/everruns/everruns/pull/381))
- fix(durable): add Resume button for drained workers ([#380](https://github.com/everruns/everruns/pull/380))
- fix(durable): add index for stale task reclaim query ([#379](https://github.com/everruns/everruns/pull/379))
- fix(telemetry): switch from gRPC to HTTP OTLP to fix DNS errors ([#378](https://github.com/everruns/everruns/pull/378))
- fix(ui): wrap EventFilter menu content in DropdownMenuGroup ([#377](https://github.com/everruns/everruns/pull/377))
- feat(example): add pull command to update docker images ([#376](https://github.com/everruns/everruns/pull/376))
- feat(example): add logs command to listen to docker-compose logs ([#375](https://github.com/everruns/everruns/pull/375))
- feat(durable): add fail-rs failure injection testing ([#374](https://github.com/everruns/everruns/pull/374))
- feat(example): pass through OPENAI/ANTHROPIC API keys to docker compose ([#373](https://github.com/everruns/everruns/pull/373))
- feat(core): add comprehensive OpenResponses types module from OpenAPI spec ([#372](https://github.com/everruns/everruns/pull/372))
- feat(llm): add support for /v1/responses/compact endpoint with reactive compaction ([#371](https://github.com/everruns/everruns/pull/371))
- feat(sse): add connection cycling and retry hints for SSE endpoints ([#370](https://github.com/everruns/everruns/pull/370))
- refactor(migrations): squash migrations 003-007 into base schema ([#369](https://github.com/everruns/everruns/pull/369))
- chore(deps): bump next from 16.1.1 to 16.1.5 in /apps/ui ([#368](https://github.com/everruns/everruns/pull/368))
- feat(llm): add automatic retry for rate limit errors ([#367](https://github.com/everruns/everruns/pull/367))
- fix(auth): ensure org cookie is set on auth and UI waits for org initialization ([#366](https://github.com/everruns/everruns/pull/366))
- fix(durable): optimize task claiming query with better index order ([#365](https://github.com/everruns/everruns/pull/365))
- feat(server): auto-apply database migrations on startup ([#364](https://github.com/everruns/everruns/pull/364))
- refactor(api): remove org from API paths, derive from auth context ([#363](https://github.com/everruns/everruns/pull/363))
- refactor: rename MessageRole::Assistant to MessageRole::Agent ([#362](https://github.com/everruns/everruns/pull/362))
- feat(docker): add just example subcommand for docker-compose-full ([#361](https://github.com/everruns/everruns/pull/361))
- feat(ui): unify ID handling with copy buttons ([#360](https://github.com/everruns/everruns/pull/360))
- refactor: rename everruns-control-plane to everruns-server ([#359](https://github.com/everruns/everruns/pull/359))
- feat(ui): add organisation settings page ([#358](https://github.com/everruns/everruns/pull/358))
- feat(events): document events contract and add contract tests ([#357](https://github.com/everruns/everruns/pull/357))
- feat(events): add event filtering with exclude parameter ([#356](https://github.com/everruns/everruns/pull/356))
- feat(ui): implement Slate design system ([#355](https://github.com/everruns/everruns/pull/355))
- feat(core): add in-memory LLM integration tests ([#354](https://github.com/everruns/everruns/pull/354))
- refactor(events): rework event types for input/output symmetry ([#353](https://github.com/everruns/everruns/pull/353))
- feat(ui): show full agent_id instead of truncated version ([#352](https://github.com/everruns/everruns/pull/352))
- refactor(api): make sessions top-level entities under organizations ([#351](https://github.com/everruns/everruns/pull/351))
- fix(scripts): use robust PostgreSQL check instead of port-only check ([#350](https://github.com/everruns/everruns/pull/350))
- chore(ci): add workflow permissions blocks ([#349](https://github.com/everruns/everruns/pull/349))
- feat(core): add InMemoryAgenticLoop and TurnStateMachine ([#348](https://github.com/everruns/everruns/pull/348))
- docs: cleanup AGENTS.md and reorganize documentation ([#347](https://github.com/everruns/everruns/pull/347))
- fix(braintrust): convert message roles to OpenAI-compatible format ([#346](https://github.com/everruns/everruns/pull/346))
- refactor(worker): unify in-memory and durable worker implementations ([#345](https://github.com/everruns/everruns/pull/345))
- fix(observability): fix Braintrust timeline view for reason and act spans ([#344](https://github.com/everruns/everruns/pull/344))
- fix(ui): bypass Next.js proxy for image uploads ([#343](https://github.com/everruns/everruns/pull/343))
- fix(worker): add restart-on-crash logic for worker startup ([#342](https://github.com/everruns/everruns/pull/342))
- refactor(core): use typed IDs throughout codebase for type safety ([#341](https://github.com/everruns/everruns/pull/341))
- feat(observability): add Braintrust integration for LLM tracing ([#340](https://github.com/everruns/everruns/pull/340))
- fix(scripts): improve PostgreSQL detection with /dev/tcp ([#339](https://github.com/everruns/everruns/pull/339))
- feat(core): add extended thinking support for Claude models ([#338](https://github.com/everruns/everruns/pull/338))
- feat(db): add updated_at column to sessions table ([#337](https://github.com/everruns/everruns/pull/337))
- fix(core): isolate event listener errors to prevent crash propagation ([#336](https://github.com/everruns/everruns/pull/336))
- refactor(proto): make ResolveImageResponse and TaskNotification fields required ([#335](https://github.com/everruns/everruns/pull/335))
- fix(worker): add startup retry for control-plane connection ([#334](https://github.com/everruns/everruns/pull/334))
- refactor(core): remove CapabilityId constants and factory methods ([#333](https://github.com/everruns/everruns/pull/333))
- fix(ui): reorder session tabs to Chat, Files, Storage, Events ([#332](https://github.com/everruns/everruns/pull/332))
- fix(scripts): improve Ctrl+C signal handling in start-all/start-dev ([#331](https://github.com/everruns/everruns/pull/331))
- refactor(ui): split components and consolidate utilities ([#330](https://github.com/everruns/everruns/pull/330))
- docs: add OpenAI Platform Traces API to dismissed options ([#329](https://github.com/everruns/everruns/pull/329))
- fix(ui): shorten session ID display with copy button ([#328](https://github.com/everruns/everruns/pull/328))
- chore: code cleanup - centralize utilities, deps, and shared components ([#327](https://github.com/everruns/everruns/pull/327))
- docs(specs): fix API paths to include org prefix and correct event types ([#326](https://github.com/everruns/everruns/pull/326))
- feat(core): add metadata to LLM API requests for tracking ([#325](https://github.com/everruns/everruns/pull/325))
- chore: rename api service to control-plane in docker-compose-full ([#324](https://github.com/everruns/everruns/pull/324))
- docs: rebrand from platform to agentic harness engine ([#323](https://github.com/everruns/everruns/pull/323))
- feat(ui): move info icon to footer row in chat messages ([#322](https://github.com/everruns/everruns/pull/322))
- feat: simplify dev setup for cloud environments ([#321](https://github.com/everruns/everruns/pull/321))
- feat(core): standardize ID schema with Stripe-style prefixed IDs ([#320](https://github.com/everruns/everruns/pull/320))
- fix(scripts): fetch github remote main branch for gh pr merge ([#319](https://github.com/everruns/everruns/pull/319))
- feat(core): add composable message filter abstraction ([#318](https://github.com/everruns/everruns/pull/318))
- feat(ui): add capability settings editor for Docker custom image ([#317](https://github.com/everruns/everruns/pull/317))
- feat(capabilities): add session storage capability with UI ([#316](https://github.com/everruns/everruns/pull/316))
- feat(docker): add docker_logs tool to get container logs ([#315](https://github.com/everruns/everruns/pull/315))
- refactor(ui): rename File System tab to Files with subtitle ([#314](https://github.com/everruns/everruns/pull/314))
- chore: replace dev.sh with just command runner ([#313](https://github.com/everruns/everruns/pull/313))
- feat(ui): simplify chat UI with minimal + icon style ([#312](https://github.com/everruns/everruns/pull/312))
- fix(test): use std::time::Duration in cancel turn test ([#310](https://github.com/everruns/everruns/pull/310))
- fix(api): make cancel turn endpoint idempotent with response body ([#309](https://github.com/everruns/everruns/pull/309))
- feat(metrics): add time-to-first-token tracking for LLM calls ([#308](https://github.com/everruns/everruns/pull/308))
- feat(openai): adopt Open Responses API as default, remove Azure support ([#307](https://github.com/everruns/everruns/pull/307))
- feat(capabilities): add capability dependencies with automatic resolution ([#306](https://github.com/everruns/everruns/pull/306))
- fix(ui): use TooltipPrimitive.Provider for tooltips to work ([#305](https://github.com/everruns/everruns/pull/305))
- feat(session): add cancel turn functionality ([#304](https://github.com/everruns/everruns/pull/304))
- feat(dev): auto-check UI dependencies in start-all and start-dev ([#303](https://github.com/everruns/everruns/pull/303))
- feat(events): include system message in llm.generation event ([#302](https://github.com/everruns/everruns/pull/302))
- feat(capabilities): add experimental Docker container capability ([#301](https://github.com/everruns/everruns/pull/301))
- chore: update default model to gpt-5.2 ([#300](https://github.com/everruns/everruns/pull/300))
- feat(ui): add pagination and auto-refresh to session events ([#299](https://github.com/everruns/everruns/pull/299))
- fix(api): use cached tools when viewing MCP capability details ([#298](https://github.com/everruns/everruns/pull/298))
- fix(deps): update llmsim to 0.2.0 to fix dependency vulnerabilities ([#297](https://github.com/everruns/everruns/pull/297))
- feat(llm): add user-friendly error for request-too-large errors ([#296](https://github.com/everruns/everruns/pull/296))
- feat(models): add automatic model discovery from LLM provider APIs ([#295](https://github.com/everruns/everruns/pull/295))
- feat(ui): auto-focus message input when session loads ([#294](https://github.com/everruns/everruns/pull/294))
- feat(ui): add real-time streaming chat with thinking indicator ([#293](https://github.com/everruns/everruns/pull/293))
- ci: add lockfile check to verify Cargo.lock is up to date ([#292](https://github.com/everruns/everruns/pull/292))
- feat(ui): add LLM message history visualization component ([#291](https://github.com/everruns/everruns/pull/291))
- feat(ui): add markdown support for capability descriptions ([#290](https://github.com/everruns/everruns/pull/290))
- chore: update lock files and document release requirements ([#289](https://github.com/everruns/everruns/pull/289))
- fix(ci): quote if expression in release workflow ([#288](https://github.com/everruns/everruns/pull/288))
- refactor(capabilities): replace webfetch with fetchkit library ([#279](https://github.com/everruns/everruns/pull/279))
- feat(skill): update ui-screenshots to use agent-browser ([#197](https://github.com/everruns/everruns/pull/197))

### Migration Notes

**0.4.0 → 0.5.0:** No backward compatibility. This release includes schema changes (migrations squashed) and API path changes (org removed from paths). Export agents via API, reset database, re-import.

## [0.4.0] - 2025-01-17

### Highlights

- **Organization-scoped Multitenancy** - Full tenant isolation with organization-based resource scoping
- **MCP Support** - Model Context Protocol integration (without Auth)
- **Push-based Work Scheduling** - Real-time task distribution replacing polling for durable execution
- **DEV_MODE** - Run without PostgreSQL using in-memory storage for quick development
- **Multimodality support for images** - Attach and process multiple images in messages

### What's Changed

- fix(ci): update Docker tag strategy for stable latest ([#287](https://github.com/everruns/everruns/pull/287)) by [@chaliy](https://github.com/chaliy)
- refactor: remove outdated decision comments from Cargo.toml ([#285](https://github.com/everruns/everruns/pull/285)) by [@chaliy](https://github.com/chaliy)
- fix(deps): address security vulnerabilities ([#284](https://github.com/everruns/everruns/pull/284)) by [@chaliy](https://github.com/chaliy)
- refactor(db): squash migrations into two files ([#283](https://github.com/everruns/everruns/pull/283)) by [@chaliy](https://github.com/chaliy)
- feat(release): add automated release workflow with CHANGELOG.md as source of truth ([#282](https://github.com/everruns/everruns/pull/282)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): add capability mounting to session filesystem ([#281](https://github.com/everruns/everruns/pull/281)) by [@chaliy](https://github.com/chaliy)
- feat(tests): add agent integration tests for tool calls across providers ([#280](https://github.com/everruns/everruns/pull/280)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add agent preview mode to show final agent shape ([#276](https://github.com/everruns/everruns/pull/276)) by [@chaliy](https://github.com/chaliy)
- docs: refactor README for users, move dev setup to CONTRIBUTING ([#278](https://github.com/everruns/everruns/pull/278)) by [@chaliy](https://github.com/chaliy)
- feat: implement organization-scoped multitenancy ([#277](https://github.com/everruns/everruns/pull/277)) by [@chaliy](https://github.com/chaliy)
- feat(mcp): add MCP agent support with virtual capabilities ([#275](https://github.com/everruns/everruns/pull/275)) by [@chaliy](https://github.com/chaliy)
- feat(images): add multi-image attachment support for messages ([#274](https://github.com/everruns/everruns/pull/274)) by [@chaliy](https://github.com/chaliy)
- refactor: remove database trigger, implement usage tracking in Rust ([#273](https://github.com/everruns/everruns/pull/273)) by [@chaliy](https://github.com/chaliy)
- fix: remove outdated Temporal reference from dev.sh ([#272](https://github.com/everruns/everruns/pull/272)) by [@chaliy](https://github.com/chaliy)
- feat(worker): implement push-based task notifications ([#270](https://github.com/everruns/everruns/pull/270)) by [@chaliy](https://github.com/chaliy)
- feat(durable): add PostgreSQL-backed load test benchmarks ([#268](https://github.com/everruns/everruns/pull/268)) by [@chaliy](https://github.com/chaliy)
- fix(worker): reduce poll interval from 1s to 100ms ([#262](https://github.com/everruns/everruns/pull/262)) by [@chaliy](https://github.com/chaliy)
- feat(models): add favorite LLM models support ([#265](https://github.com/everruns/everruns/pull/265)) by [@chaliy](https://github.com/chaliy)
- feat(durable): integrate circuit breaker for LLM provider protection ([#263](https://github.com/everruns/everruns/pull/263)) by [@chaliy](https://github.com/chaliy)
- feat(ui): update session status and usage via SSE in real-time ([#258](https://github.com/everruns/everruns/pull/258)) by [@chaliy](https://github.com/chaliy)
- feat(dev): add llmsim provider support and seed data ([#261](https://github.com/everruns/everruns/pull/261)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): add per-agent capability configuration ([#260](https://github.com/everruns/everruns/pull/260)) by [@chaliy](https://github.com/chaliy)
- fix(dev): fix dev mode LLM errors and improve DX ([#259](https://github.com/everruns/everruns/pull/259)) by [@chaliy](https://github.com/chaliy)
- fix(ui): remove max-width constraint from agent edit page ([#256](https://github.com/everruns/everruns/pull/256)) by [@chaliy](https://github.com/chaliy)
- fix(ui): maintain input focus after sending chat message ([#257](https://github.com/everruns/everruns/pull/257)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): replace durable polling with SSE streaming ([#254](https://github.com/everruns/everruns/pull/254)) by [@chaliy](https://github.com/chaliy)
- feat: add LLM token usage tracking and visualization ([#250](https://github.com/everruns/everruns/pull/250)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add SessionCard component with status display and info button ([#247](https://github.com/everruns/everruns/pull/247)) by [@chaliy](https://github.com/chaliy)
- feat(mcp): add MCP server registration and management ([#246](https://github.com/everruns/everruns/pull/246)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add message info icon with metadata tooltip ([#243](https://github.com/everruns/everruns/pull/243)) by [@chaliy](https://github.com/chaliy)
- feat(api,ui): add pagination to sessions API ([#244](https://github.com/everruns/everruns/pull/244)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add centralized capability icons ([#237](https://github.com/everruns/everruns/pull/237)) by [@chaliy](https://github.com/chaliy)

### Migration Notes

**0.3.x → 0.4.0:** No automatic migration. This release includes schema changes for multitenancy and capabilities. Export agents via API, reset database, re-import.

## [0.3.0] - 2025-01-09

### Highlights

- **Durable Execution Engine** - Custom PostgreSQL-backed workflow engine replacing Temporal
- **CLI Tool** - Command-line interface for agent and session management
- **OpenTelemetry Integration** - Distributed tracing with gen-ai semantic conventions
- **SSE Events** - Real-time session status updates replacing polling

### What's Changed

- feat(durable): add custom durable execution engine (Phases 1-4) ([#154](https://github.com/everruns/everruns/pull/154)) by [@chaliy](https://github.com/chaliy)
- refactor(telemetry): implement event-listener-based OTel with gen-ai semantic conventions ([#161](https://github.com/everruns/everruns/pull/161)) by [@chaliy](https://github.com/chaliy)
- feat(cli): add everruns CLI for agent and session management ([#163](https://github.com/everruns/everruns/pull/163)) by [@chaliy](https://github.com/chaliy)
- feat(docs): add auto-generated API Reference from OpenAPI spec ([#164](https://github.com/everruns/everruns/pull/164)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): add fake demo tools and agents for warehouse, AWS, CRM, and financial operations ([#168](https://github.com/everruns/everruns/pull/168)) by [@chaliy](https://github.com/chaliy)
- feat(api): add agent import/export endpoints ([#172](https://github.com/everruns/everruns/pull/172)) by [@chaliy](https://github.com/chaliy)
- feat(api): add input validation for agent create/update/import ([#173](https://github.com/everruns/everruns/pull/173)) by [@chaliy](https://github.com/chaliy)
- feat(dev): add seed agent markdown files and upload-agents command ([#176](https://github.com/everruns/everruns/pull/176)) by [@chaliy](https://github.com/chaliy)

### Migration Notes

**0.2.x → 0.3.0:** No automatic migration. Export agents via API, reset database, re-import.

## [0.2.0] - 2024-12

### Highlights

- **Temporal Integration** - Workflow orchestration via Temporal
- **PostgreSQL Storage** - Database layer with SQLx
- **Management UI** - Next.js dashboard for agent management

### What's Changed

- Initial implementation with Temporal-based workflow orchestration
- Complete rewrite from early POC architecture

### Migration Notes

**0.1.x → 0.2.0:** Complete rewrite. Manual migration required.

## [0.1.0] - 2024-11

### Highlights

- Initial proof-of-concept release
- Basic agent execution with simple message handling

---

## Versioning Policy

- **Major versions** (1.0, 2.0): Breaking API changes, architectural shifts
- **Minor versions** (0.3, 0.4): New features, schema changes requiring fresh DB
- **Patch versions** (0.3.1): Bug fixes, no schema changes
