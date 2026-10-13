# Web search providers, a Mira eval

A [Mira](https://github.com/everruns/mira) study that grades the result lists of
every web search provider Everruns can use, through the **shipped** drivers.

Prices differ by two orders of magnitude, from Ceramic at $0.25 per 1,000
searches to model provider search at about $14. Price says nothing about the
question that matters for picking a default: can an agent answer from the list?

```text
dataset.jsonl ──► provider axis ──► shipped SearchDriver ──► LLM judge ──► scorers
 (59 agent-style   free, brave,      (direct egress)          gpt-6.1-sol    answerable
  queries + need)  parallel,         native: OpenAI           grades each    relevance (nDCG@5)
                   ceramic, exa,     Responses web_search     hit 0-3        current
                   linkup, native    on gpt-6-luna                           returned
```

## Running it

```sh
# Credentials: BRAVE_SEARCH_API_KEY, PARALLEL_API_KEY, CLOUDFLARE_ACCOUNT_ID +
# CLOUDFLARE_API_TOKEN (ceramic, exa, linkup), OPENAI_API_KEY (native and the judge).
doppler run -- mira --bin web_search_providers
# Without the Mira host: cases.jsonl and summary.md under results/
doppler run -- cargo run --release -- report results/
```

`WEB_SEARCH_PROVIDERS=brave,ceramic` narrows the provider axis. A provider
without credentials skips: unmeasured, not bad.

## How it grades

The judge sees what an agent sees (title, URL, date, snippet), never which
provider produced the list, and grades every hit 0 to 3 against the case's
stated need. It also says whether an agent could meet the need from the list
(**answerable**, the headline number) and, for time-sensitive cases, whether the
list shows the current state (**current**). A failed search scores as a miss,
as it would for an agent. Rate limits are retried with backoff and counted.

## What it found (2026-10-13)

59 queries in seven groups: documentation, error messages, current versions and
news, facts, pricing, niche projects, research. Five results per search, run
from a datacenter IP like a hosted worker.

| Provider | Answerable | nDCG@5 | Current (time-sensitive) | Failed | Empty | Median latency | Cost per 1,000 |
|---|---|---|---|---|---|---|---|
| free | 6/59 (10%) | 0.05 | 2/19 | 45 | 3 | 1.3 s | $0 |
| brave | 58/59 (98%) | 0.75 | 18/19 | 0 | 0 | 0.5 s | $5 |
| parallel | **59/59 (100%)** | 0.78 | 13/19 | 0 | 0 | 0.7 s | $5 |
| ceramic (Cloudflare) | 29/59 (49%) | 0.33 | **0/19** | 0 | 6 | 0.6 s | $0.25 |
| exa (Cloudflare) | 58/59 (98%) | 0.80 | 15/19 | 1 | 0 | 0.5 s | $7 |
| linkup (Cloudflare) | 50/59 (85%) | 0.61 | 12/19 | 0 | 0 | 0.7 s | $5 |
| native (gpt-6-luna) | 57/59 (97%) | **0.82** | 5/19* | 0 | 0 | 6.1 s | $14 |

| Group | free | brave | parallel | ceramic | exa | linkup | native |
|---|---|---|---|---|---|---|---|
| docs | 0/15 | 15/15 | 15/15 | 9/15 | 15/15 | 15/15 | 15/15 |
| errors | 0/5 | 4/5 | 5/5 | 5/5 | 4/5 | 5/5 | 5/5 |
| facts | 3/7 | 7/7 | 7/7 | 7/7 | 7/7 | 7/7 | 7/7 |
| current | 3/15 | 15/15 | 15/15 | 1/15 | 15/15 | 11/15 | 13/15 |
| niche | 0/7 | 7/7 | 7/7 | 2/7 | 7/7 | 6/7 | 7/7 |
| pricing | 0/6 | 6/6 | 6/6 | 3/6 | 6/6 | 3/6 | 6/6 |
| research | 0/4 | 4/4 | 4/4 | 2/4 | 4/4 | 3/4 | 4/4 |

- **Ceramic is not good enough to be a default.** It answers half the cases and
  none of the time-sensitive ones: "latest stable Rust release" returns the
  Rust book's install page, "current Node.js LTS" returns one unrelated page,
  and docs queries surface spam next to the right page. It is a keyword index
  at a keyword-index price.
- **Brave, Parallel and Exa are interchangeable on quality** (98 to 100%
  answerable, nDCG 0.75 to 0.80). Brave is the freshest (18/19) and the fastest;
  Parallel answered every case; Exa ranks best of the three but costs more and
  keeps data.
- **Free search barely works from a datacenter.** DuckDuckGo and Mojeek blocked
  45 of 59 searches outright. Free is a last resort, not a default.
- **Native search ranks best but costs about 3x and takes 6 s**, and it is
  graded generously: a model re-queries before it cites. *Its low "current"
  count is a measurement artifact, not a finding: native sources carry no
  snippets or dates, so the judge cannot see how recent they are.

**The warning:** one run, 59 queries, one judge. The gaps above (49% against
98%) are far larger than that noise; the differences among Brave, Parallel and
Exa are not.
