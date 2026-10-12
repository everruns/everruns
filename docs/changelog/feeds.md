---
title: Changelog feeds
description: RSS feeds for the Everruns changelog, one for every change and one per product, plus the Markdown versions that coding agents can read.
---

Every feed carries the full text of each entry, with its product and kind as
categories.

| Feed | URL |
| --- | --- |
| All changes | [/changelog/rss.xml](https://everruns.com/docs/changelog/rss.xml) |
| Cloud | [/changelog/cloud/rss.xml](https://everruns.com/docs/changelog/cloud/rss.xml) |
| Platform | [/changelog/platform/rss.xml](https://everruns.com/docs/changelog/platform/rss.xml) |
| Framework | [/changelog/framework/rss.xml](https://everruns.com/docs/changelog/framework/rss.xml) |
| SDK | [/changelog/sdk/rss.xml](https://everruns.com/docs/changelog/sdk/rss.xml) |
| CLI | [/changelog/cli/rss.xml](https://everruns.com/docs/changelog/cli/rss.xml) |
| Yolop | [/changelog/yolop/rss.xml](https://everruns.com/docs/changelog/yolop/rss.xml) |
| Mira | [/changelog/mira/rss.xml](https://everruns.com/docs/changelog/mira/rss.xml) |
| Bashkit | [/changelog/bashkit/rss.xml](https://everruns.com/docs/changelog/bashkit/rss.xml) |

## For coding agents

Every entry is also served as Markdown: add `index.md` to its address, for
example `https://everruns.com/docs/changelog/index.md` for the whole list. The
changelog is a section of [llms.txt](https://everruns.com/docs/llms.txt).

To list only what changed after the version you run, add `?since=` to the
changelog address, for example `/changelog/?since=0.45.0`.
