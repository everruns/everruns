// A shared Chat session can already contain completed turns before the test sends.
export function eventsForMessage(events, messageId) {
  const started = events.find(
    (event) => event.type === "turn.started" && event.data?.input_message_id === messageId,
  );
  const turnId = started?.data?.turn_id;
  return events.filter(
    (event) =>
      event.context?.input_message_id === messageId ||
      (turnId && event.context?.turn_id === turnId),
  );
}
