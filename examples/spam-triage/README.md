# Spam Triage

Classify a hundred emails with a fast classifier, then send only the ones it was unsure about to a larger model. The interesting decision is not spam or not — it is which emails are worth a second opinion, and that decision is made in your code, from a number.

## What you learn

Reading a classifier's confidence as a value your code branches on, and pairing it with a direct model call for the cases the number says are unsettled.

## Scenario and expected outcome

The corpus is 100 messages from the SpamAssassin public corpus: 50 spam and 50 legitimate, where two thirds of the legitimate half is `hard_ham` — newsletters, receipts, and vendor mail that read spammish. An easy corpus would show nothing; the whole point is the band where a fast model hedges.

`jev-latest` answers one yes/no question per email and returns a probability. The confidence behind its own verdict is `max(p, 1 - p)`; below `--threshold` (0.80 by default) the email is routed to `meta/muse-spark-1.3-contributor` for a one-word verdict, and that verdict wins.

Expect the screen alone to land around 97%, roughly a fifth of the corpus to escalate, and the pipeline to finish at 98–100% for well under a cent of adjudication. The second stage is a reasoning model and is not deterministic even at temperature 0, so two runs of the same corpus can differ by an email or two.

## Run it

Install Rust/Cargo, clone the repository, and run from its root. These folders are self-contained **within the workspace**: their Cargo manifests reference the local Framework crates, so copying one folder alone is not sufficient.

```bash
git clone https://github.com/everruns/everruns.git
cd everruns
bash examples/spam-triage/data/build-corpus.sh
export TYPESAFE_API_KEY="your-key"
export OPENROUTER_API_KEY="your-key"
cargo run -p everruns-spam-triage
```

The configured models are `jev-latest` on TypeSafe and `meta/muse-spark-1.3-contributor` on OpenRouter. Provider access and funded credits are required for both; a model identifier alone does not grant access. Keep keys in your environment, not in source control. Missing variables, provider errors, or a failed call exit nonzero.

Move the threshold, or run a slice of the corpus:

```bash
cargo run -p everruns-spam-triage -- --threshold 0.95   # escalate far more
cargo run -p everruns-spam-triage -- --limit 20         # a shorter, cheaper run
cargo run -p everruns-spam-triage -- --corpus /path/to/your.jsonl
```

## The corpus

The messages are not vendored. Copyright for their text stays with the people who sent them, so `data/build-corpus.sh` downloads the [SpamAssassin public corpus](https://spamassassin.apache.org/old/publiccorpus/) into a gitignored cache and writes `data/corpus.jsonl` beside it. Sampling is deterministic — same counts in, same messages out — so a published run reproduces. Each line is one message:

```json
{"id": "spam/00011", "sender": "…", "subject": "…", "body": "…", "label": "spam"}
```

Point `--corpus` at your own file in that shape to triage your own mail. The corpus these messages came from is a decade old and its spam looks it; treat the accuracy here as a demonstration of the routing pattern, not as a benchmark of either model on mail you would receive today.

## Screen every email

A classifier answers a question as a probability rather than as prose, which is what makes the routing decision a branch instead of a parse. `src/pipeline.rs` asks one question and spells out both sides, so the boundary between spam and a newsletter is stated rather than guessed:

```rust
let answers = classifier
    .about(email.as_state())
    .noul_between(QUESTION_ID, "Is this email spam?", SPAM_MEANS, LEGITIMATE_MEANS)
    .send()
    .await?;
Screening { spam_probability: answers.probability(QUESTION_ID)?, .. }
```

Read the number as a probability, not a grade: 0.5 means spam and legitimate are near-equally likely, not "medium spam". The confidence is the mass behind whichever side won, and `is_settled` is the whole of the routing rule.

## Route only what is unsettled

The second stage has no history to keep and no tool to call, so it is a direct completion rather than an agent:

```rust
let response = model
    .completion()
    .system(include_str!("resources/adjudicator.md"))
    .user(email.as_state())
    .temperature(0.0)
    .reasoning_effort(ReasoningEffort::Low)
    .max_tokens(2048)
    .send()
    .await?;
```

Both stages run `CONCURRENCY` requests at a time and preserve input order, so the report lines up with the corpus. Muse reasons before it answers and will not let that be disabled, so the token budget has to cover the reasoning as well as the one word after it; too tight a cap returns an empty answer, which the report counts as an unreadable escalation rather than hiding it as a verdict.

## Where the threshold goes

The threshold is the only real design decision here, and it is a calibration against one classifier release and one mail mix. On this corpus every email the screen got wrong sat at 0.73 confidence or below, so 0.80 escalates all of them and still settles four fifths of the corpus locally. Raising it to 0.95 escalates about two thirds, costs roughly three times as much, and — on this corpus — buys nothing. `Answers::model` reports the `jev-*` release that answered; pin it before trusting a threshold you measured.

## Validate the behavior

```bash
cargo test -p everruns-spam-triage
bash examples/spam-triage/demo/record.sh --check
```

Tests cover corpus parsing, the confidence rule and its boundary, reading a verdict out of a wordy answer, the fallback when an escalation is unreadable, and the tally that separates what the screen got from what routing added. They do not grade either model: run it live and compare.

CI runs these offline checks without provider credentials. Live model behavior is evaluated separately; passing tests is not proof of answer quality.

## Demo and recording

`demo/transcript.txt` is a real 30-email run, the same command the tape records. Recording the screencast needs the corpus built, credentials exported, and VHS, ffmpeg, and a VHS-compatible browser installed:

```bash
bash examples/spam-triage/demo/record.sh
```

VHS hides most provider wait time but does not replace the models with scripted output.
