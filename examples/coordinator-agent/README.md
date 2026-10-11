# Coordinator agent

A Framework application where one session coordinates and threads do the work.

The person asks the coordinator for a launch. It starts two threads, one for
the announcement and one for a pricing FAQ. Each thread writes its draft to the
shared workspace, keeps a checklist, and finishes with a summary. Each finished
thread wakes the coordinator with an automatic update, and the person resolves
a thread by asking the coordinator.

It runs on OpenAI's `gpt-6.1-sol`, so the wording and the drafts vary from run
to run. A typical run:

```console
$ OPENAI_API_KEY=... cargo run -p everruns-coordinator-agent
person> Plan the launch: write the announcement and draft a pricing FAQ.
coordinator> I started two threads, Announcement and Pricing FAQ. I will tell you when each is ready.

Board
  ReadyForReview   Announcement   2/2 steps  Three-line announcement drafted in announcement.md with the launch date and sign-up link.
  ReadyForReview   Pricing FAQ    2/2 steps  Three pricing FAQ entries drafted in pricing-faq.md, one sentence each.

coordinator> Announcement and Pricing FAQ are ready for review.   (automatic update)

person> The announcement looks good. Resolve that thread.
coordinator> Resolved the announcement thread.

Board
  Resolved         Announcement   2/2 steps  Three-line announcement drafted in announcement.md with the launch date and sign-up link.
  ReadyForReview   Pricing FAQ    2/2 steps  Three pricing FAQ entries drafted in pricing-faq.md, one sentence each.
```

## Run it

```bash
OPENAI_API_KEY=... cargo run -p everruns-coordinator-agent
```

`cargo test -p everruns-coordinator-agent` replays the conversation on a
scripted model that lives only in `tests/team_sim/`, and checks the real state: both threads exist as their own sessions, each wrote its
file and finished its checklist, the coordinator was woken without the person
saying anything, and resolving went through the coordinator.

## How it is built

- `coordinator()` in `src/lib.rs` is the whole agent: the `Coordination`
  capability, a read-write workspace policy so threads can write drafts, and a
  local profile, which coordination requires.
- `everruns::coordination::threads` reads the board.
- `tests/team_sim/` holds `TeamSim`, the test-only scripted model. It reads the
  newest user message and the tools called since, so the coordinator and its
  threads get the same script whatever order they run in. The example binary
  never uses it.

See [Coordinator agents](https://docs.everruns.com/framework/coordination/)
for the model.
