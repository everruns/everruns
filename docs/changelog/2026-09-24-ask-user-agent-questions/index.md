---
title: Ask User lets agents pause and ask you questions mid-run
description: Agents with the ask_user capability pause a run to ask one to four choice, text or secret questions, shown as an inline card that you answer in place.
date: 2026-09-24
kind: feature
significance: major
products: [platform, cloud, framework]
surfaces: [ui, api, code]
release: "0.31.0"
visual: "Ask User inline card in a chat session with a choice question and a text question, Continue and Decline buttons visible; route /chat/{session}; crop to the card."
links:
  docs: [/capabilities/ask-user/, /framework/ask-user/]
  prs: [everruns/everruns#3735, everruns/everruns#3738, everruns/everruns#3747, everruns/everruns#3757, everruns/everruns#3790]
---

**In the UI:** an agent with the `ask_user` capability can stop and ask one to
four structured questions instead of guessing. The chat shows a card with
choice options, a free-form text field, or a secret field; answering resumes
the turn. An unanswered choice falls back to the option the model recommended,
while text and secret questions are declined. A secret answer never enters the
event log or the agent's context. The default chat harnesses enable it.

**In code:** implement the `AskUser` trait to answer questions from your own
interface when you embed the Framework.

See [Ask User](/capabilities/ask-user/) and
[Answer an agent's questions](/framework/ask-user/). PRs [#3735](https://github.com/everruns/everruns/pull/3735),
[#3738](https://github.com/everruns/everruns/pull/3738), [#3747](https://github.com/everruns/everruns/pull/3747), [#3757](https://github.com/everruns/everruns/pull/3757), [#3790](https://github.com/everruns/everruns/pull/3790).
