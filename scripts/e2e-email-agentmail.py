#!/usr/bin/env python3
"""Opt-in paid email round trip against a disposable, local v2 stack.

EMAIL_E2E_LIVE=1 AGENTMAIL_API_KEY=... python3 scripts/e2e-email-agentmail.py
Requires AUTH_MODE=none, data plane v2 and a configured real gpt-6-luna model. The key is
encrypted into the local test database. Only everruns-testing@agentmail.to is
used. No provider webhook is created: this exercises real polling recovery.
Self-mail is deliberately held by loop protection, then explicitly reviewed.
The created inbox is disabled on exit. Credentials and message bodies are never
logged. Override EMAIL_E2E_BASE_URL (default http://localhost:27200/api).
EMAIL_E2E_RESUME_MESSAGE can select a previously received, held test email to
qualify backlog resumption without sending another initial email.
"""

import base64
import json
import os
import secrets
import time
import urllib.error
import urllib.parse
import urllib.request
from email.utils import parseaddr


def request(url, method="GET", body=None, headers=None):
    encoded = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(
        url, encoded, {"Content-Type": "application/json", **(headers or {})}, method=method
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as response:
            raw = response.read()
            return json.loads(raw) if raw else None
    except urllib.error.HTTPError as error:
        # Never include request headers, credentials or returned mail in errors.
        raise RuntimeError(f"{method} {urllib.parse.urlsplit(url).path}: HTTP {error.code}") from None
    except urllib.error.URLError:
        raise RuntimeError(f"{method} {urllib.parse.urlsplit(url).path}: connection unavailable") from None


def wait_for(label, operation, timeout=240):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        value = operation()
        if value:
            print(f"PASS {label}", flush=True)
            return value
        time.sleep(3)
    raise RuntimeError(f"Timed out waiting for {label}")


def matches_provider_reply(detail, marker):
    # AgentMail appends the quoted thread to reply alternatives. The immutable
    # outbox is checked for exact equality before sending; readback checks the
    # new first line independently of provider-added quoted history.
    lines = (detail.get("text") or "").strip().splitlines()
    return bool(lines) and lines[0].strip() == marker and marker in (detail.get("html") or "")


def main():
    assert os.environ.get("EMAIL_E2E_LIVE") == "1", "Set EMAIL_E2E_LIVE=1 for paid live testing"
    base = os.environ.get("EMAIL_E2E_BASE_URL", "http://localhost:27200/api").rstrip("/")
    parsed = urllib.parse.urlsplit(base)
    assert parsed.scheme == "http" and parsed.hostname in ("localhost", "127.0.0.1"), "Local stack only"
    key = os.environ["AGENTMAIL_API_KEY"]
    inbox_address = "everruns-testing@agentmail.to"
    provider_base = "https://api.agentmail.to/v0/inboxes/" + inbox_address
    provider_headers = {"Authorization": "Bearer " + key}
    org = None

    def api(path, method="GET", body=None):
        return request(base + "/v1/platform" + path, method, body, {"X-Org-Id": org} if org else {})

    assert api("/auth/config")["mode"] == "none", "Disposable AUTH_MODE=none stack required"
    org = api("/auth/me")["organizations"][0]["public_id"]
    model = next((m for m in api("/models")["data"] if m["model_id"] == "gpt-6-luna"), None)
    assert model, "Configure the real gpt-6-luna model before running the live test"
    nonce = secrets.token_hex(6)
    resume = None
    source_marker = None
    if requested := os.environ.get("EMAIL_E2E_RESUME_MESSAGE"):
        old_inbox = next((i for i in api("/email/inboxes") if i["address"] == inbox_address), None)
        assert old_inbox and not old_inbox["enabled"], "Resume requires the disabled testing inbox"
        resume = next((m for m in api(f'/email/inboxes/{old_inbox["id"]}/messages')
                       if m["id"] == requested), None)
        assert resume and resume["state"] == "held", "Resume only an unexecuted held test message"
        prefix = "Everruns local email round trip "
        library_prefix = "Everruns inbox E2E "
        if resume["subject"].startswith(prefix):
            nonce = resume["subject"][len(prefix):]
            assert len(nonce) == 12 and all(c in "0123456789abcdef" for c in nonce)
        else:
            # The coordinated Channelkit smoke test also sends only to this
            # designated inbox. Resuming its held message tests real backlog.
            assert resume["subject"].startswith(library_prefix), "Only generated test mail can be resumed"
            nonce = resume["subject"][len(library_prefix):]
            assert nonce.isdigit() and 16 <= len(nonce) <= 22
            source_marker = "Test message " + nonce + ". No action required."
    marker = "EVERRUNS_EMAIL_" + nonce
    source_marker = source_marker or marker
    account = api(f"/orgs/{org}/organization-connections/providers/agentmail_inbox", "POST", {
        "name": "Email E2E " + nonce, "api_key": key,
    })
    inbox = None
    channel_path = None
    try:
        setup = {
            "name": "Email E2E " + nonce, "account_id": account["id"],
            "address": inbox_address,
            # Local random secret enables signed ingress, but no external
            # webhook is registered. The real poller supplies the event.
            "signing_secret": "whsec_" + base64.b64encode(secrets.token_bytes(32)).decode(),
        }
        existing = next((i for i in api("/email/inboxes") if i["address"] == inbox_address), None)
        if existing:
            assert not existing["enabled"] and existing["name"].startswith("Email E2E "), "Inbox already in use"
            inbox = existing["id"]
            api(f"/email/inboxes/{inbox}", "PATCH", {"enabled": True, "signing_secret": setup["signing_secret"]})
        else:
            inbox = api("/email/inboxes", "POST", setup)["id"]
        agent = api("/agents", "POST", {
            "name": "email-e2e-" + nonce + "-" + secrets.token_hex(3),
            "default_model_id": model["id"],
            "system_prompt": "You handle an email integration test. First use the email_read tool "
                "to read the current message, using the message_id from the incoming envelope. "
                "Then reply with exactly " + marker + ". Include no additional text.",
        })
        channel = api(f'/agents/{agent["id"]}/channels', "POST", {
            "channel_type": "email", "channel_config": {
                "inbox_id": inbox, "purpose": "Answer integration-test confirmation emails.",
                "auto_dispatch": False, "auto_reply": False,
            },
        })
        channel_path = f'/agents/{agent["id"]}/channels/{channel["id"]}'
        api(channel_path + "/publish", "POST", {})
        alias = "everruns-testing+e2e-" + nonce + "@agentmail.to"
        receipt = resume or request(provider_base + "/messages/send", "POST", {
                "to": [alias], "subject": "Everruns local email round trip " + nonce,
                "text": "Please confirm the integration test with " + marker,
                "html": "<p>Please confirm the integration test with " + marker + "</p>",
            }, provider_headers)

        def incoming():
            return next((m for m in api(f"/email/inboxes/{inbox}/messages")
                         if m["message_id"] == receipt["message_id"]), None)

        mail = wait_for("persisted real-provider backlog resumption" if resume else
                        "provider plus-address delivery and durable poll intake", incoming)
        mail = wait_for("incoming review gate", lambda: (m if (m := incoming()) and m["state"] == "held" else None))
        assert parseaddr(mail["sender"])[1] == inbox_address, "Reply only to the designated testing inbox"
        full = api(f'/email/inboxes/{inbox}/messages/{mail["id"]}?view=full')
        assert source_marker in full["content"], "Review the exact generated test message"
        api(f'/email/inboxes/{inbox}/messages/{mail["id"]}/approve', "POST", {})

        def reply_ready():
            current = incoming()
            if current and current["state"] in ("failed", "held"):
                raise RuntimeError("Agent execution held or failed: " + str(current.get("reason")))
            rows = api(f"/email/inboxes/{inbox}/outbox")
            return next((r for r in rows if r["intake_id"] == mail["id"]
                         and current and current["state"] == "completed"), None)

        reply = wait_for("real agent turn and immutable outbox", reply_ready)
        assert reply["state"] == "held", "Sending requires its separate approval"
        assert reply["body"]["to"] == [inbox_address]
        assert reply["body"]["text"].strip() == marker
        assert marker in reply["body"]["html"]
        session = "session_" + reply["session_id"].replace("-", "")
        events = api(f"/sessions/{session}/events")["data"]
        assert any(e["type"] == "tool.completed" and e["data"].get("success")
                   and e["data"].get("tool_name") == "mcp_email__email_read"
                   for e in events), "The real agent must successfully call its scoped email read tool"
        print("PASS authenticated, session-scoped email tool execution", flush=True)
        api(f'/email/inboxes/{inbox}/outbox/{reply["id"]}/approve', "POST", {})

        def provider_reply():
            messages = request(provider_base + "/messages?limit=30", headers=provider_headers)["messages"]
            for candidate in messages:
                if candidate["thread_id"] != receipt["thread_id"] or candidate["message_id"] == receipt["message_id"]:
                    continue
                detail = request(provider_base + "/messages/" + urllib.parse.quote(candidate["message_id"], safe=""), headers=provider_headers)
                if matches_provider_reply(detail, marker):
                    return True
            return False

        wait_for("provider threaded reply readback with both alternatives", provider_reply)
        print("PASS local v2 email round trip (" + ("backlog resume" if resume else "polling") +
              ", scoped read, explicit review, real model, real AgentMail)", flush=True)
    finally:
        if inbox:
            api(f"/email/inboxes/{inbox}", "PATCH", {"enabled": False})
        if channel_path:
            api(channel_path + "/unpublish", "POST", {})
            api(channel_path, "DELETE")


if __name__ == "__main__":
    main()
