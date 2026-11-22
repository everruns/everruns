---
title: "Main chat, side chats and threads, beside any page"
description: "Every chat now hands longer work to threads shown in a panel on its right, side chats replace subchats, and Chat opens beside the page you are on."
date: 2026-11-10
kind: improvement
significance: major
products: [platform, cloud]
release: "0.52.0"
surfaces: [ui]
links:
  docs: [/built-ins/harnesses/platform-chat/]
---

Ask **Main chat** for something long and it starts a thread, then keeps talking to
you while the thread works. **Threads** opens a panel on the right, sorted into
Needs you, Working and Ready for review. The panel opens when a new thread
starts. Click a thread to read it next to the chat.

![Threads panel beside Main chat with one thread needing an answer, two working with step counts, and one ready for review](./threads-panel@2x.png)

**Side chats** replace subchats and can start their own threads. The
conversation sits beside the page in the default
[Chat first layout](/changelog/2026-11-11-chat-first-layout/). Cmd/Ctrl-Shift-J
toggles full screen. Chat workspace and background work are on for every
organization. Home is now **Overview**, under Building, with a count of the
items waiting there. See [Platform Chat](/built-ins/harnesses/platform-chat/).

## Update 2026-11-20

Main chat recovers from a rolled-back first attempt. Side chats open
independently of the Main chat lookup. Playground setup can also start a new
private Platform Chat.
