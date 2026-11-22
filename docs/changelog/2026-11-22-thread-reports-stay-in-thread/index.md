---
title: Thread reports stay in their thread
description: Chat no longer repeats a thread's report under its card. A finished thread is at most one line with View thread; the coordinator's reply says what matters.
date: 2026-11-22
kind: improvement
significance: minor
products: [platform, cloud]
surfaces: [ui]
links:
  docs: [/features/ui/#chat-and-threads, /framework/coordination/]
---

When a thread finished, Chat used to add a row under its card with the thread's
whole report, task id and all. That text was written for the coordinator, not for you.

Now the report stays in the thread. If the coordinator answers right after the
thread's card, nothing extra appears: the card already says **Succeeded** or
**Failed**, and the reply is your summary. If the chat moved on in between, one
line says which thread finished, failed, or asks a question, with **View thread**
beside it. Progress messages from a running thread no longer show at all. The
coordinator still receives the full text, and the message info icon shows it
under **What the agent received**.

![A finished thread right after its card adds no row; further down it is one line with Open session](./thread-results@2x.png)
