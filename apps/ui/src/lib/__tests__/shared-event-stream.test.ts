import {
  createEventStream,
  setEventStreamFactory,
  clearSharedEventStreams,
  type EventStreamLike,
} from "@/lib/event-stream";

const sources: Array<EventStreamLike & { listeners: Map<string, (event: MessageEvent) => void> }> =
  [];
const urls: string[] = [];
beforeEach(() => {
  setEventStreamFactory((url) => {
    urls.push(url);
    const source = {
      listeners: new Map(),
      addEventListener: (type: string, listener: (event: MessageEvent) => void) => {
        source.listeners.set(type, listener);
      },
      close: jest.fn(),
      onopen: null,
      onerror: null,
    };
    sources.push(source);
    return source;
  });
});
afterEach(() => {
  setEventStreamFactory(null);
  sources.length = 0;
  urls.length = 0;
});
const url = "/api/v1/sessions/session_1/sse";
const scope = { shareScope: "alice:org" };
function fire(index: number, type: string, data = "{}") {
  sources[index].listeners.get(type)?.(new MessageEvent(type, { data }));
}

it("shares a transcript with snapshot consumers and releases on the last close", () => {
  const chat = createEventStream(url + "?since_id=event_1", scope);
  const tasks = createEventStream(url, { ...scope, snapshotOnly: true });
  const chatEvent = jest.fn();
  const taskEvent = jest.fn();
  chat.addEventListener("task.updated", chatEvent);
  tasks.addEventListener("task.updated", taskEvent);
  fire(0, "task.updated");
  expect(sources).toHaveLength(1);
  expect(chatEvent).toHaveBeenCalledTimes(1);
  expect(taskEvent).toHaveBeenCalledTimes(1);
  chat.close();
  expect(sources[0].close).not.toHaveBeenCalled();
  tasks.close();
  tasks.close();
  expect(sources[0].close).toHaveBeenCalledTimes(1);
});

it("promotes a task-first connection using the transcript's exact replay cursor", () => {
  const task = createEventStream(url, { ...scope, snapshotOnly: true });
  const connected = jest.fn();
  task.addEventListener("connected", connected);
  createEventStream(url + "?after_sequence=0", scope);
  expect(urls).toEqual([url, url + "?after_sequence=0"]);
  expect(sources[0].close).toHaveBeenCalledTimes(1);
  fire(0, "connected");
  expect(connected).not.toHaveBeenCalled();
  fire(1, "connected");
  expect(connected).toHaveBeenCalledTimes(1);
});

it("notifies a late snapshot subscriber so it can refresh its REST snapshot", async () => {
  const chat = createEventStream(url + "?since_id=event_1", scope);
  chat.addEventListener("connected", jest.fn());
  fire(0, "connected");
  const tasks = createEventStream(url, { ...scope, snapshotOnly: true });
  const connected = jest.fn();
  tasks.addEventListener("connected", connected);
  await Promise.resolve();
  expect(connected).toHaveBeenCalledTimes(1);
  expect(sources).toHaveLength(1);
});

it("never combines transcript subscribers with independently loaded snapshots", () => {
  createEventStream(url + "?since_id=event_1", scope);
  createEventStream(url + "?since_id=event_2", scope);
  expect(sources).toHaveLength(2);
});

it("separates accounts, organizations, credentials, and event filters", () => {
  createEventStream(url, { ...scope, snapshotOnly: true });
  createEventStream(url, { shareScope: "bob:org", snapshotOnly: true });
  createEventStream(url, { shareScope: "alice:other", snapshotOnly: true });
  createEventStream(url, { ...scope, withCredentials: false, snapshotOnly: true });
  createEventStream(url + "?exclude=task.updated", { ...scope, snapshotOnly: true });
  expect(sources).toHaveLength(5);
});

it("delivers an error to all consumers even when the first closes itself", () => {
  const chat = createEventStream(url, scope);
  const tasks = createEventStream(url, { ...scope, snapshotOnly: true });
  chat.onerror = () => chat.close();
  tasks.onerror = jest.fn();
  sources[0].onerror?.(null);
  expect(tasks.onerror).toHaveBeenCalledTimes(1);
});

it("logout disposes sources and suppresses already-queued events", () => {
  const chat = createEventStream(url, scope);
  const listener = jest.fn();
  chat.addEventListener("input.message", listener);
  clearSharedEventStreams();
  fire(0, "input.message");
  expect(listener).not.toHaveBeenCalled();
  expect(sources[0].close).toHaveBeenCalledTimes(1);
});
