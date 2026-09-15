# Slack App (UI)

* [TC001: Slack app setup — manifest, publish, webhook verification, first message](TC001_slack_app_setup.md) - Verifies the Slack setup checklist can be completed end to end, that the generated manifest carries event_subscriptions so no hand-editing is needed, and that the checklist advances from observed webhook state.
* [TC002: Slack agent pane — streaming, status, title, stop](TC002_agent_pane.md) - Verifies the assistant pane streams replies progressively, shows a status line while a tool runs without leaking tool names, sets the thread title, and cancels the turn from the stop button.
* [TC003: Slack channel thread — mention, markdown, attribution, history](TC003_channel_thread.md) - Verifies channel-thread behaviour: an @mention starts one threaded session, markdown renders as blocks, multiple speakers are attributed, and joining mid-thread backfills prior history.
* [TC004: Slack failure surfacing — no turn ends in silence](TC004_failure_surfacing.md) - Verifies that a failed turn, an exhausted budget, and a turn producing no output each post exactly one terse notice with a session link, and that no internal error detail reaches a public channel.
