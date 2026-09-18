"""Turn SpamAssassin corpus directories into one labeled JSONL file.

Each line is {"id", "sender", "subject", "body", "label"} with `label` either
"spam" or "legitimate". Bodies are decoded, stripped of HTML, collapsed, and
truncated: the pipeline reads what a human would skim, not raw MIME.
"""

import email
import email.policy
import html
import html.parser
import json
import os
import re
import sys

BODY_CHARS = 1200


class _Strip(html.parser.HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.parts = []
        self.skip = 0

    def handle_starttag(self, tag, attrs):
        if tag in ("script", "style"):
            self.skip += 1

    def handle_endtag(self, tag):
        if tag in ("script", "style") and self.skip:
            self.skip -= 1

    def handle_data(self, data):
        if not self.skip:
            self.parts.append(data)


def strip_html(text):
    parser = _Strip()
    try:
        parser.feed(text)
        parser.close()
    except Exception:
        return re.sub(r"<[^>]*>", " ", text)
    return "".join(parser.parts)


def body_of(message):
    part = message
    if message.is_multipart():
        part = message.get_body(preferencelist=("plain", "html")) or message
    try:
        text = part.get_content()
    except Exception:
        payload = part.get_payload(decode=True) or b""
        text = payload.decode("utf-8", "replace")
    if not isinstance(text, str):
        text = str(text)
    if (part.get_content_subtype() or "").lower() == "html" or "<html" in text.lower():
        text = strip_html(text)
    text = html.unescape(text)
    text = re.sub(r"\s+", " ", text).strip()
    return text[:BODY_CHARS]


def header(message, name):
    value = message.get(name, "")
    value = re.sub(r"\s+", " ", str(value)).strip()
    return value[:200]


def sample(directory, count):
    """Deterministic: sort by name, then take an even stride across the set."""
    names = sorted(n for n in os.listdir(directory) if not n.startswith("cmds"))
    if count >= len(names):
        return [os.path.join(directory, n) for n in names]
    stride = len(names) / count
    return [os.path.join(directory, names[int(i * stride)]) for i in range(count)]


def read(path, label):
    """`id` locates the message in the cache: "<corpus dir>/<message number>"."""
    with open(path, "rb") as handle:
        message = email.message_from_binary_file(handle, policy=email.policy.default)
    body = body_of(message)
    subject = header(message, "Subject")
    if not body and not subject:
        return None
    return {
        "id": os.path.basename(os.path.dirname(path)) + "/" + os.path.basename(path).split(".")[0],
        "sender": header(message, "From"),
        "subject": subject,
        "body": body,
        "label": label,
    }


def main():
    sources = [
        (os.environ["SPAM_DIR"], int(os.environ["SPAM_COUNT"]), "spam"),
        (os.environ["HARD_HAM_DIR"], int(os.environ["HARD_HAM_COUNT"]), "legitimate"),
        (os.environ["EASY_HAM_DIR"], int(os.environ["EASY_HAM_COUNT"]), "legitimate"),
    ]
    records = []
    for directory, count, label in sources:
        for path in sample(directory, count):
            record = read(path, label)
            if record is not None:
                records.append(record)

    # Interleave so a truncated run is not all of one label.
    spam = [r for r in records if r["label"] == "spam"]
    ham = [r for r in records if r["label"] == "legitimate"]
    ordered = []
    for index in range(max(len(spam), len(ham))):
        if index < len(ham):
            ordered.append(ham[index])
        if index < len(spam):
            ordered.append(spam[index])

    with open(os.environ["OUT"], "w", encoding="utf-8") as out:
        for record in ordered:
            out.write(json.dumps(record, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    sys.exit(main())
