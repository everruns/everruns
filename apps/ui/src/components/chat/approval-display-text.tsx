/**
 * Display-only escaping at approval boundaries. Do not reuse this output for
 * arguments, fingerprints or decisions. Preserve joiners and variation selectors
 * used by Persian and emoji; normal tabs/newlines still format the preview.
 */
// THREAT[TM-WEB-026]: reveal directional overrides/isolates and control bytes
// before the browser can use them to disguise the action awaiting consent.
const APPROVAL_CONTROLS =
  /([\u0000-\u0008\u000B-\u001F\u007F-\u009F\u061C\u200E\u200F\u2028-\u202E\u2066-\u2069])/u;

export function ApprovalDisplayText({ children }: { children: string }) {
  return (
    <span>
      {children.split(APPROVAL_CONTROLS).map((part, index) => {
        if (index % 2 === 0) return part;
        const code = part.charCodeAt(0).toString(16).toUpperCase().padStart(4, "0");
        const label = `Unicode control character U+${code}`;
        return (
          <span
            key={index}
            role="img"
            aria-label={label}
            title={label}
            dir="ltr"
            className="inline-block rounded-sm border border-warning/50 bg-warning/10 px-0.5 font-mono text-warning [unicode-bidi:isolate]"
          >
            {`\\u${code}`}
          </span>
        );
      })}
    </span>
  );
}
