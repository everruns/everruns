---
title: TypeSafe
description: Give agents typed classification — calibrated probabilities, single-choice routing, and graded scores — from TypeSafe's System One model, instead of asking a chat model for an opinion. Requires a TypeSafe API key.
---

<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="52.0" height="52.0" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" style="float: right; margin-left: 16px;"><path d="M12 3v18M7 21h10M3 8l4-3 4 3M3 8a4 4 0 0 0 8 0M13 8l4-3 4 3M13 8a4 4 0 0 0 8 0M12 5l5-2M12 5 7 3"/></svg>

Everruns integrates with [TypeSafe](https://typesafe.ai) so agents can ask for a
**judgment** rather than an opinion. TypeSafe's System One model answers typed
questions about content and returns numbers your agent — and your code — can act
on directly: the probability that something is true, which option out of a set
applies, or where something falls on a scale you define.

It does not write prose. That is the point: there is no answer to interpret, and
no JSON to parse out of a paragraph.

## What You Get

- **Yes/no with a probability**: "Is a refund being requested?" → `0.97`, not "Yes, it appears so."
- **Single choice with a distribution**: pick one option and see how close the runners-up were
- **Graded scores**: rate against ordered levels you write, with the probability of each level
- **Confidence**: how concentrated the answer is, so the agent can escalate instead of guessing
- **One call, many questions**: every question in a call is answered together over the same content

## Quick Start

### 1. Get Your API Key

1. Sign in at [typesafe.ai](https://typesafe.ai)
2. Create an **API key** in the dashboard
3. Copy it

### 2. Connect in Everruns

1. Go to **Settings** > **Connections**
2. Find **TypeSafe** in the available providers
3. Click **Connect** and paste your API key

Once connected, the TypeSafe capability is available in agent sessions.

### 3. Use in Sessions

Agents with the TypeSafe capability get one tool:

| Tool | Description |
|------|-------------|
| `jev_evaluate` | Ask typed questions about content and get calibrated answers |

A call gives it the content plus the questions to ask about it:

```json
{
  "state": "Why did the chicken cross the road? To get to the other side.",
  "questions": [
    {
      "id": "is_funny",
      "type": "noul",
      "instructions": "Would a general audience laugh at this?"
    },
    {
      "id": "humor",
      "type": "score",
      "instructions": "How funny is this joke?",
      "levels": ["Not funny at all", "Mildly amusing", "Genuinely funny", "Hilarious"]
    }
  ]
}
```

And gets back numbers, not a review:

```json
{
  "model": "jev-1.13.0",
  "answers": {
    "is_funny": { "type": "noul", "probability_yes": 0.43 },
    "humor": {
      "type": "score",
      "score": 0.58,
      "normalized": 0.19,
      "level": 1,
      "label": "Mildly amusing",
      "probabilities": { "0": 0.44, "1": 0.55, "2": 0.01, "3": 0.0 },
      "confidence": 0.57
    }
  }
}
```

## Question Types

| Type | Ask it when | You get |
|------|-------------|---------|
| `noul` | A condition either holds or it doesn't | The probability of yes, from 0 to 1 |
| `choice` | Exactly one option out of a set applies | The selected option, every option's probability, and a confidence |
| `score` | Something falls somewhere on a scale | A weighted position across your levels, each level's probability, and a confidence |

Two things worth knowing when you write the questions:

- A `noul` near **0.5** means yes and no are roughly equally likely. It does not
  mean "somewhat" — for degree, use a `score`.
- `choice` options and `score` levels must each describe a concrete situation and
  stand on their own. The question id is never shown to the model, so the
  instructions have to carry the whole meaning.

## Good Fits

- **Verification**: does this answer actually follow from the source it cites?
- **Rating**: how severe is this report, how good is this draft, how funny is this joke
- **Routing**: which handler, team, or tool should take this — with a confidence to gate on
- **Screening**: does this content match a policy, and how clearly

## Guardrails

The same model backs Everruns [guardrails](/capabilities/guardrails/) when a
`llm_judge` or `moderation` check sets `"engine": "jev"`. Instead of asking
the utility model to write a verdict, the check gets a calibrated probability and
your configured `threshold` decides — and every check on a stage is answered in a
single call. That path uses a deployment-owned key (`UTILITY_TYPESAFE_API_KEY`), not your
personal connection.

## Security

- The API key is stored as a user connection and never exposed to the agent or
  written into session transcripts.
- Content passed to `jev_evaluate` leaves the platform for TypeSafe, like
  any other integration that inspects content. Calls are capped at 20 questions
  and 32 KiB of content.
- The content being judged is sent as **data**, and every question states so — a
  document that tries to instruct the model is being rated, not obeyed.
