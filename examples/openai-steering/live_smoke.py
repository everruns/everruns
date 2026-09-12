#!/usr/bin/env python3
"""Optional billable live check. OPENAI_API_KEY and GPT-6 Astra access required."""

import argparse
import asyncio
import json
import os

from steering import Journal, run_owner, usage_totals


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--db", required=True, help="New private journal to retain as evidence"
    )
    args = parser.parse_args()
    journal = Journal.create(
        args.db,
        "Draft a detailed implementation plan for a task-tracking app.",
        {"max_output_tokens": 3000},
    )
    try:
        journal.enqueue(
            "smoke-update-1",
            "Keep scope to two weeks for one developer. End with STEERING_CONFIRMED.",
        )
        asyncio.run(
            run_owner(
                journal, os.environ["OPENAI_API_KEY"], stop_when_idle=True, timeout=120
            )
        )
        state = journal.load()
        if state["phase"] != "complete" or state["intents"][0]["status"] != "applied":
            raise RuntimeError(
                f"Live check did not complete: {state['phase']}; {state['error']}"
            )
        output = state["responses"][state["current"]]["output"]
        text = "".join(
            part["text"]
            for item in output
            if item["type"] == "message"
            for part in item["content"]
            if part["type"] == "output_text"
        )
        if "STEERING_CONFIRMED" not in text:
            raise RuntimeError("Successor did not include the requested marker")
        print(
            json.dumps(
                {
                    "result": "passed",
                    "responses": list(state["responses"]),
                    "usage": usage_totals(state),
                    "text": text,
                },
                indent=2,
            )
        )
    finally:
        journal.db.close()


if __name__ == "__main__":
    main()
