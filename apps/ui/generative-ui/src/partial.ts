export function parseA2UI(code: string): unknown {
  const trimmed = code.trim();
  if (!trimmed) return null;

  try {
    return JSON.parse(trimmed);
  } catch {
    // Attempt progressive truncation for streaming tolerance.
    return tryPartialParse(trimmed);
  }
}

/**
 * Try to recover a parseable JSON prefix by closing unbalanced brackets.
 * This is best-effort: if it cannot produce valid JSON, returns null.
 */
function tryPartialParse(text: string): unknown {
  // Walk the text tracking brackets + string state; emit a "closed" version
  // by padding the right number of close brackets.
  let inString = false;
  let escape = false;
  const stack: Array<"{" | "["> = [];
  let lastSafeIndex = -1;

  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (escape) {
      escape = false;
      continue;
    }
    if (ch === "\\") {
      if (inString) escape = true;
      continue;
    }
    if (ch === '"') {
      inString = !inString;
      if (!inString) lastSafeIndex = i;
      continue;
    }
    if (inString) continue;

    if (ch === "{" || ch === "[") {
      stack.push(ch);
      lastSafeIndex = i;
    } else if (ch === "}" || ch === "]") {
      stack.pop();
      lastSafeIndex = i;
    } else if (!/\s/.test(ch) && stack.length === 0) {
      // After the top-level object closes, anything else is garbage.
      break;
    } else if (stack.length > 0) {
      lastSafeIndex = i;
    }
  }

  if (lastSafeIndex < 0) return null;

  // Truncate to last safe point and close remaining brackets.
  let truncated = text.slice(0, lastSafeIndex + 1);

  // If we stopped mid-key/value (e.g. "foo": ), strip trailing comma/colon.
  truncated = truncated.replace(/[,:\s]+$/, "");

  // Re-scan truncated to recount open brackets.
  const closes: string[] = [];
  let inStr = false;
  let esc = false;
  for (const ch of truncated) {
    if (esc) {
      esc = false;
      continue;
    }
    if (ch === "\\") {
      if (inStr) esc = true;
      continue;
    }
    if (ch === '"') {
      inStr = !inStr;
      continue;
    }
    if (inStr) continue;
    if (ch === "{") closes.push("}");
    else if (ch === "[") closes.push("]");
    else if (ch === "}" || ch === "]") closes.pop();
  }

  const candidate = truncated + closes.reverse().join("");
  try {
    return JSON.parse(candidate);
  } catch {
    return null;
  }
}
