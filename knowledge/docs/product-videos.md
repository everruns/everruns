---
type: Playbook
title: Product Video Knowledge
description: Ownership and discovery of the canonical Everruns product-video guide and published product walkthroughs.
tags:
  - everruns
  - documentation
  - video
---

# Product Video Knowledge

The canonical production knowledge lives in the **private landing repository**:
[Product Videos](https://github.com/everruns/landing/blob/main/knowledge/content/product-videos.md).
Its linked
[product-video skill](https://github.com/everruns/landing/blob/main/.claude/skills/product-video/SKILL.md)
covers real browser/native-app capture, scene timing, fluid zooms, floating text
cards, Sarah narration, brand pronunciation, audio mixing, captions, upload and
verification. Read those before producing the next walkthrough. Repository access
is required; these links are for maintainers, not public product documentation.

Landing owns the detailed guide because video production serves the website,
launches and product documentation together. Keep one canonical procedure there;
update this pointer if its ownership moves. Do not duplicate credentials, private
account details, raw recordings or capture-specific scripts into OSS knowledge.

The approved public channel is [Everruns @everrunshq](https://www.youtube.com/@everrunshq).
The reference is
[Slack integration: Kids Duties agent in Everruns](https://www.youtube.com/watch?v=mbbYFif9juY),
a public 29-second walkthrough with English Sarah narration/captions. It shows an
existing configuration and saved run followed by actual Slack messages; the
original messages remain Ukrainian. It does not claim to create the agent from
scratch or demonstrate a Slack pane that was disabled in that setup.

The baseline is casual description, a smoothly joined "ever-runs" pronunciation,
quick eased zooms followed by reading holds, and short navy/gold text cards placed
clear of the relevant controls. **Improvements and deviations are welcome** when
they improve clarity or naturalness; the reference is not a rigid template.

Publishing the video is separate from deploying its containing page. The docs
source now owns the [Slack landing page](../../docs/integrations/slack.mdx),
including the public embed and English conversation examples. The detailed
[Slack reference](../../docs/capabilities/slack.md) retains configuration/actions.
Verify deployment before describing a containing page as live.

For public entries, link the real demo rather than a private recording directory.

The [PACT account walkthrough](https://www.youtube.com/watch?v=9FZiYv5PsWY)
uses a local personal-agent demo client and synthetic store data. It shows scoped
consent, cancellation refused before approval, and a verified signed receipt.
Its release card also names Poppy support; the demonstrated account flow is PACT.
The public embeds live in the A2A/PACT and Poppy feature pages and PACT changelog.

The Slack walkthrough illustrates both
[Slack DMs and files](../../docs/changelog/2026-10-13-slack-agents-dms-and-files/index.md)
and [scheduled Slack briefings](../../docs/changelog/2026-10-12-scheduled-turns-post-to-slack/index.md).

The approved Sandbox Builder walkthrough is served with the
[Sandbox Templates guide](../../docs/features/sandbox-templates.md#build-a-reusable-snapshot)
and [snapshot-builder changelog](../../docs/changelog/2026-10-29-sandbox-snapshot-builder/index.md).
It uses the real local app and a live Daytona build to show an install script,
fresh-snapshot verification, readable software notes, separate build/runtime
network policies and an immutable revision. Provider waiting time is shortened.
The curated 52-second Sarah cut includes English captions; raw captures and
production files stay outside Git.
