"""Dependency-free lexical retrieval baselines for the MCP scale evaluation.

The Everruns baseline mirrors crates/core/src/builtins/tool_search.rs, including
its top-score-band cutoff and exact-name bonus. BM25 is intentionally simple
and deterministic so results can be reproduced without a service or model.
"""
from __future__ import annotations

import math
import re
from collections import Counter
from typing import Any

MAX_RESULTS = 8
NAME_TERM_WEIGHT = 3
DESC_TERM_WEIGHT = 1
EXACT_NAME_BONUS = 100
_TOKEN_RE = re.compile(r"[a-z0-9]+", re.IGNORECASE)


def _trim_non_alphanumeric(text: str) -> str:
    start, end = 0, len(text)
    while start < end and not text[start].isalnum():
        start += 1
    while end > start and not text[end - 1].isalnum():
        end -= 1
    return text[start:end]


def _tokens(text: str) -> list[str]:
    # Split underscores and camel case, which lets a natural phrase match names
    # such as drive_file_download without stemming away useful distinctions.
    text = re.sub(r"([a-z0-9])([A-Z])", r"\1 \2", text)
    return [t.lower() for t in _TOKEN_RE.findall(text)]


def rank_everruns(tools: list[dict[str, Any]], query: str, limit: int = MAX_RESULTS) -> list[dict[str, Any]]:
    """Reproduce current Everruns ToolSearchTool keyword-overlap ranking."""
    normalized = _trim_non_alphanumeric(query.strip()).lower()
    terms = [_trim_non_alphanumeric(t).lower() for t in query.split()]
    terms = [t for t in terms if t]
    scored: list[tuple[int, int, dict[str, Any]]] = []
    for index, tool in enumerate(tools):
        name = str(tool.get("name", ""))
        if name == "tool_search":
            continue
        desc = str(tool.get("description", ""))
        score = 0
        for term in terms:
            if term in name.lower():
                score += NAME_TERM_WEIGHT
            elif term in desc.lower():
                score += DESC_TERM_WEIGHT
        if normalized == name.lower():
            score += EXACT_NAME_BONUS
        if score:
            scored.append((score, index, tool))
    if not terms:
        return [tool for tool in tools if tool.get("name") != "tool_search"][:max(0, min(limit, MAX_RESULTS))]
    scored.sort(key=lambda item: (-item[0], item[1]))
    if scored:
        cutoff = (scored[0][0] + 1) // 2
        scored = [item for item in scored if item[0] >= cutoff]
    return [tool for _, _, tool in scored[:max(0, min(limit, MAX_RESULTS))]]


def rank_bm25(tools: list[dict[str, Any]], query: str, limit: int = MAX_RESULTS,
              *, include_aliases: bool = False) -> list[dict[str, Any]]:
    """Rank tools by BM25 over tool name/description and MCP server description.

    Returned items are the original dictionaries, preserving tool_id,
    server_id/name, inputSchema, schema_digest, and any other metadata.
    """
    query_terms = _tokens(query)
    if not query_terms or not tools or limit <= 0:
        return []
    corpus: list[list[str]] = []
    for tool in tools:
        text = (f"{tool.get('name', '')} {tool.get('description', '')} "
                f"{tool.get('server_description', '')}")
        if include_aliases:
            aliases = tool.get("aliases", [])
            if isinstance(aliases, str):
                text += " " + aliases
            else:
                text += " " + " ".join(str(x) for x in aliases)
        corpus.append(_tokens(text))
    n = len(corpus)
    avgdl = sum(map(len, corpus)) / n if n else 0.0
    if not avgdl:
        return []
    document_frequency: Counter[str] = Counter()
    term_frequency: list[Counter[str]] = []
    for doc in corpus:
        frequencies = Counter(doc)
        term_frequency.append(frequencies)
        document_frequency.update(frequencies.keys())
    scores: list[tuple[float, int, dict[str, Any]]] = []
    k1, b = 1.5, 0.75
    for index, tool in enumerate(tools):
        dl = len(corpus[index])
        score = 0.0
        for term in query_terms:
            tf = term_frequency[index][term]
            if not tf:
                continue
            df = document_frequency[term]
            # BM25+positive IDF avoids negative scores for very common terms.
            idf = math.log(1 + (n - df + 0.5) / (df + 0.5))
            score += idf * (tf * (k1 + 1)) / (tf + k1 * (1 - b + b * dl / avgdl))
        if score > 0:
            scores.append((score, index, tool))
    scores.sort(key=lambda item: (-item[0], item[1]))
    return [tool for _, _, tool in scores[:min(limit, MAX_RESULTS)]]
