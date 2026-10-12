"""Load driver: N sessions with a mix of turn shapes over the public API.

Each session follows its events over SSE (one stream per session, as the UI
does) and reconnects with `after_sequence` to another server when its stream
drops, so a killed API server is a reconnect, not a lost session. Turn
completion and latency come from the stream; the end-of-run assertions do not
trust it and re-read every session's events with paged GETs.

Session kinds (assigned round robin by weight so a run is reproducible). Turn
shape comes from llmsim scenario markers (`[[llmsim:<name>]]`, built-ins in
`crates/drivers/llmsim/scenarios/`); a message without a marker gets the
instant `llmsim-default` answer, which is the platform-overhead probe.

- plain:    3 sequential unmarked turns (instant model).
- tools:    `research-3-tools` then `parallel-tools` with `bashkit_shell`:
            4 and 2 reason steps around tool rounds.
- midturn:  `slow-first-token` (4-8 s), and a second message once the turn
            has started: the server steers it into the running turn
            (delivery `steered`) or starts a follow-up turn.
- cancel:   `slow-first-token` cancelled after it starts, then one unmarked
            turn to show the session is usable afterwards.
- approval: `llmsim-realistic` (calls `bash`) with `bashkit_shell` and
            `tool_approval` (mode `protective`): the call parks the turn on an
            `approve_tool_call` request, answered `allow_always` via
            `POST /v1/sessions/{id}/tool-approvals`.
"""

from __future__ import annotations

import http.client
import json
import random
import threading
import time
import urllib.parse
import uuid
from dataclasses import dataclass, field

from api import Api, ApiError

TERMINAL = ("turn.completed", "turn.failed", "turn.cancelled", "turn.sealed")
KIND_WEIGHTS = [("plain", 40), ("tools", 20), ("midturn", 15), ("cancel", 15), ("approval", 10)]


def turn_id_of(e: dict) -> str | None:
    d = e.get("data") or {}
    return d.get("turn_id") or (e.get("context") or {}).get("turn_id")


@dataclass
class SentMessage:
    text: str
    client_message_id: str
    sent_at: float
    id: str | None = None
    delivery: str | None = None
    expect: str = "completed"  # completed | cancel_ok | any
    done_at: float | None = None  # client time the terminal event arrived
    turn_id: str | None = None
    post_error: str | None = None


@dataclass
class SessionRun:
    index: int
    kind: str
    session_id: str | None = None
    messages: list[SentMessage] = field(default_factory=list)
    approvals: int = 0
    cancels: list[float] = field(default_factory=list)
    errors: list[str] = field(default_factory=list)
    started_at: float = 0.0
    finished_at: float = 0.0


class SessionStream:
    """Follows one session's SSE stream across server restarts."""

    def __init__(self, api: Api, session_id: str):
        self.api = api
        self.session_id = session_id
        self.events: list[dict] = []
        self.seen: set = set()
        self.last_seq = 0
        self.cond = threading.Condition()
        self.stop_flag = False
        self.reconnects = 0
        self.thread = threading.Thread(target=self._run, daemon=True)
        self.thread.start()

    def close(self):
        self.stop_flag = True
        with self.cond:
            self.cond.notify_all()

    def _run(self):
        attempt = 0
        while not self.stop_flag:
            base = self.api.base_urls[attempt % len(self.api.base_urls)]
            attempt += 1
            try:
                self._stream_once(base)
            except Exception:
                pass
            if self.stop_flag:
                return
            self.reconnects += 1
            time.sleep(0.2)

    def _stream_once(self, base: str):
        p = urllib.parse.urlparse(base)
        conn = http.client.HTTPConnection(p.hostname, p.port, timeout=30)
        try:
            q = urllib.parse.urlencode({"after_sequence": self.last_seq})
            conn.request(
                "GET",
                f"{self.api.prefix}/v1/sessions/{self.session_id}/sse?{q}",
                headers={"Accept": "text/event-stream"},
            )
            resp = conn.getresponse()
            if resp.status != 200:
                resp.read()
                time.sleep(0.5)
                return
            data_lines: list[str] = []
            while not self.stop_flag:
                raw = resp.fp.readline()
                if not raw:
                    return
                line = raw.decode("utf-8", "replace").rstrip("\r\n")
                if line == "":
                    if data_lines:
                        self._dispatch("\n".join(data_lines))
                        data_lines = []
                    continue
                if line.startswith(":"):
                    continue
                if line.startswith("data:"):
                    data_lines.append(line[5:].lstrip())
        finally:
            conn.close()

    def _dispatch(self, payload: str):
        try:
            ev = json.loads(payload)
        except ValueError:
            return
        if not isinstance(ev, dict):
            return
        ev.setdefault("event_type", ev.get("type"))
        if not ev["event_type"]:
            return
        key = ev.get("id") or (ev.get("sequence"), ev.get("event_type"))
        with self.cond:
            if key in self.seen:
                return
            self.seen.add(key)
            ev["_recv"] = time.time()
            self.events.append(ev)
            seq = ev.get("sequence")
            if isinstance(seq, int) and seq > self.last_seq:
                self.last_seq = seq
            self.cond.notify_all()

    def wait_for(self, pred, timeout: float):
        deadline = time.time() + timeout
        with self.cond:
            while True:
                r = pred(self.events)
                if r:
                    return r
                left = deadline - time.time()
                if left <= 0 or self.stop_flag:
                    return None
                self.cond.wait(min(left, 1.0))


class Driver:
    def __init__(self, api: Api, args, models: dict, agents: dict):
        self.api = api
        self.args = args
        self.models = models
        self.agents = agents
        self.runs: list[SessionRun] = []
        self.lock = threading.Lock()

    # --- message helpers ---------------------------------------------------

    def send(self, run: SessionRun, text: str, expect="completed") -> SentMessage:
        m = SentMessage(text=text, client_message_id=str(uuid.uuid4()), sent_at=time.time(), expect=expect)
        run.messages.append(m)
        body = {
            "message": {"role": "user", "content": [{"type": "text", "text": text}]},
            "client_message_id": m.client_message_id,
        }
        try:
            r = self.api.post(f"/v1/sessions/{run.session_id}/messages", body)
            m.id = r.get("id")
            m.delivery = r.get("delivery")
        except Exception as e:  # noqa: BLE001
            m.post_error = str(e)
            run.errors.append(f"post message: {e}")
        return m

    @staticmethod
    def _turn_started_for(msg_id):
        def pred(evs):
            for e in evs:
                if e["event_type"] == "turn.started" and (e.get("data") or {}).get("input_message_id") == msg_id:
                    return turn_id_of(e)
            return None

        return pred

    @staticmethod
    def _terminal_for(turn_id):
        def pred(evs):
            for e in evs:
                if e["event_type"] in TERMINAL and turn_id_of(e) == turn_id:
                    return e
            return None

        return pred

    def wait_turn(self, run: SessionRun, stream: SessionStream, m: SentMessage, timeout: float):
        if not m.id:
            return None
        tid = stream.wait_for(self._turn_started_for(m.id), timeout)
        if not tid:
            run.errors.append(f"no turn.started for {m.id} (delivery={m.delivery})")
            return None
        m.turn_id = tid
        term = stream.wait_for(self._terminal_for(tid), timeout)
        if term is None:
            run.errors.append(f"turn {tid} not terminal within {timeout}s")
            return None
        m.done_at = term["_recv"]
        return term

    @staticmethod
    def _idle(evs):
        open_turns = set()
        for e in evs:
            t = e["event_type"]
            if t == "turn.started":
                open_turns.add(turn_id_of(e))
            elif t in TERMINAL:
                open_turns.discard(turn_id_of(e))
        return bool(evs) and not open_turns and evs[-1]["event_type"] in ("session.idled",) + TERMINAL

    # --- session kinds -----------------------------------------------------

    def run_session(self, index: int, kind: str) -> SessionRun:
        run = SessionRun(index=index, kind=kind, started_at=time.time())
        with self.lock:
            self.runs.append(run)
        a = self.args
        agent, model = {
            "plain": ("plain", "instant"),
            "tools": ("tools", "instant"),
            "midturn": ("plain", "instant"),
            "cancel": ("plain", "instant"),
            "approval": ("approval", "realistic"),
        }[kind]
        try:
            s = self.api.post(
                "/v1/sessions",
                {
                    "agent_id": self.agents[agent],
                    "model_id": self.models[model],
                    "title": f"chaos {kind} {index}",
                    "tags": ["chaos", kind],
                },
            )
        except Exception as e:  # noqa: BLE001
            run.errors.append(f"create session: {e}")
            run.finished_at = time.time()
            return run
        run.session_id = s["id"]
        stream = SessionStream(self.api, run.session_id)
        T = a.turn_timeout
        try:
            stream.wait_for(lambda evs: True, 0.3)  # let the stream connect
            if kind == "plain":
                for k in range(a.plain_turns):
                    m = self.send(run, f"plain turn {k} of session {index}")
                    self.wait_turn(run, stream, m, T)
            elif kind == "tools":
                for k, sc in enumerate(("research-3-tools", "parallel-tools")):
                    m = self.send(run, f"[[llmsim:{sc}]] tools turn {k} of session {index}")
                    self.wait_turn(run, stream, m, T)
            elif kind == "midturn":
                m1 = self.send(run, f"[[llmsim:slow-first-token]] slow first message of session {index}")
                if m1.id and stream.wait_for(self._turn_started_for(m1.id), T):
                    time.sleep(random.uniform(0.1, 0.6))
                    self.send(run, f"[[llmsim:chat-short]] follow-up sent mid-turn in session {index}")
                stream.wait_for(self._idle, T)
                for m in run.messages:
                    if m.id and m.done_at is None:
                        tid = stream.wait_for(self._turn_started_for(m.id), 0.01)
                        if tid:
                            m.turn_id = tid
                            term = stream.wait_for(self._terminal_for(tid), 0.01)
                            if term:
                                m.done_at = term["_recv"]
            elif kind == "cancel":
                m1 = self.send(run, f"[[llmsim:slow-first-token]] slow message to cancel in session {index}", expect="cancel_ok")
                if m1.id and stream.wait_for(self._turn_started_for(m1.id), T):
                    time.sleep(random.uniform(0.2, 1.0))
                    run.cancels.append(time.time())
                    try:
                        self.api.post(f"/v1/sessions/{run.session_id}/cancel")
                    except Exception as e:  # noqa: BLE001
                        run.errors.append(f"cancel: {e}")
                    self.wait_turn(run, stream, m1, T)
                m2 = self.send(run, f"after cancel in session {index}")
                self.wait_turn(run, stream, m2, T)
            elif kind == "approval":
                self.run_approval(run, stream, index, T)
            stream.wait_for(self._idle, 30)
        except Exception as e:  # noqa: BLE001
            run.errors.append(f"driver exception: {e!r}")
        finally:
            stream.close()
            run.finished_at = time.time()
        return run

    def run_approval(self, run: SessionRun, stream: SessionStream, index: int, T: float):
        m = self.send(run, f"run a command that needs approval, session {index}")
        if not m.id:
            return
        answered: set = set()
        deadline = time.time() + T

        def pending(evs):
            # approve_tool_call requests the engine raised and nobody answered.
            reqs, done = [], set()
            for e in evs:
                d = e.get("data") or {}
                if e["event_type"] == "tool.call_requested":
                    for c in d.get("tool_calls") or []:
                        if c.get("name") == "approve_tool_call":
                            reqs.append(c.get("id"))
                elif e["event_type"] == "tool.completed":
                    done.add(d.get("tool_call_id"))
            out = [r for r in reqs if r and r not in done and r not in answered]
            return out or None

        def finished(evs):
            # The message's turn is terminal and nothing is parked.
            return self._idle(evs) and not pending(evs)

        while time.time() < deadline:
            got = stream.wait_for(lambda evs: pending(evs) or finished(evs), max(1, deadline - time.time()))
            if got is True or got is None:
                break
            if isinstance(got, list):
                decisions = [{"tool_call_id": r, "decision": "allow_always"} for r in got]
                # The request event lands before the session flips to
                # waiting_for_tool_results; the API answers 409 until it has.
                for _ in range(100):
                    try:
                        self.api.post(f"/v1/sessions/{run.session_id}/tool-approvals", {"decisions": decisions})
                        run.approvals += len(got)
                        break
                    except ApiError as e:
                        if e.status == 409 and "not waiting" in e.body:
                            time.sleep(0.2)
                            continue
                        run.errors.append(f"tool-approvals: {e}")
                        break
                    except Exception as e:  # noqa: BLE001
                        run.errors.append(f"tool-approvals: {e}")
                        break
                else:
                    run.errors.append("tool-approvals: session never reached waiting_for_tool_results")
                answered.update(got)
        tid = stream.wait_for(self._turn_started_for(m.id), 1)
        if tid:
            m.turn_id = tid
            # The parked turn may end and a resumed one carry on; latency is to idle.
            if stream.wait_for(self._idle, 1):
                m.done_at = stream.events[-1]["_recv"]


def plan_kinds(n: int, only: str | None = None) -> list[str]:
    if only:
        return [only] * n
    total = sum(w for _, w in KIND_WEIGHTS)
    kinds = []
    acc = {k: 0.0 for k, _ in KIND_WEIGHTS}
    for _ in range(n):
        for k, w in KIND_WEIGHTS:
            acc[k] += w / total
        k = max(acc, key=acc.get)
        acc[k] -= 1
        kinds.append(k)
    return kinds
