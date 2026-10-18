---
title: "Slack agents answer DMs and keep the files people send"
description: "One-click Slack apps open a Messages tab, answer a direct message in the DM, store files sent with a message, and use the agent's avatar as the app icon."
date: 2026-10-13
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [agent, ui]
release: "0.48.0"
links:
  docs: [/capabilities/slack/]
  prs: [everruns/everruns#4597, everruns/everruns#4603, everruns/everruns#4677, everruns/everruns#4545]
---

An agent added to Slack with the one-click app now works as a direct-message
contact. The generated app enables the Messages tab, so people can DM the bot,
and a top-level DM gets its answer inline in the DM rather than in a thread. A
message written inside a thread still gets its reply in that thread.

Files sent with a Slack message reach the agent. Everruns downloads them with
the bot token through the organization's egress policy, up to 10 files of
25 MiB each, and stores them read-only in the session:

```text
/attachments/<upload-id>/quarterly-report.pdf
```

Changing the agent's avatar also updates the Slack app icon. See
[Connect an Agent to Slack](/capabilities/slack/#connect-an-agent-to-slack).

Watch [Kids Duties in Everruns and Slack](https://www.youtube.com/watch?v=mbbYFif9juY)
for a real configuration, saved execution and Slack conversation. The original
messages are Ukrainian; narration and captions are English.

<iframe src="https://www.youtube-nocookie.com/embed/mbbYFif9juY?rel=0" title="Slack integration: Kids Duties agent in Everruns" loading="lazy" style="width:100%;aspect-ratio:16/9;border:0" allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" referrerpolicy="strict-origin-when-cross-origin" allowfullscreen></iframe>
