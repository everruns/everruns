#!/usr/bin/env python3
"""Reconcile source declarations; review decisions must be entered explicitly."""
import argparse
import collections
import hashlib
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[2]
LEDGER = ROOT / 'knowledge/evaluation/test-quality-ledger.jsonl'


def digest(text):
    return hashlib.sha256(text.encode()).hexdigest()


def mask_rust(text):
    # Preserve offsets and newlines. Nested block comments and raw strings can
    # contain braces, attributes, or entire apparent tests.
    out = list(text)
    i = 0
    while i < len(text):
        end = None
        if text.startswith('//', i):
            end = text.find('\n', i)
            if end < 0:
                end = len(text)
        elif text.startswith('/*', i):
            end, depth = i + 2, 1
            while end < len(text) and depth:
                if text.startswith('/*', end):
                    depth += 1
                    end += 2
                elif text.startswith('*/', end):
                    depth -= 1
                    end += 2
                else:
                    end += 1
        else:
            raw = re.compile(r'(?:br|cr|r)(#{0,255})"').match(text, i) if text[i] in 'bcr' else None
            if raw:
                closing = '"' + raw.group(1)
                position = text.find(closing, i + len(raw.group()))
                end = len(text) if position < 0 else position + len(closing)
            elif text[i] == '"' or (text[i] == "'" and re.match(r"'(?:\\(?:u\{[\da-fA-F]+\}|x[\da-fA-F]{2}|.)|[^'\\\n])'", text[i:])):
                quote, end = text[i], i + 1
                while end < len(text):
                    if text[end] == '\\':
                        end += 2
                    elif text[end] == quote:
                        end += 1
                        break
                    else:
                        end += 1
        if end is None:
            i += 1
        else:
            out[i:end] = ['\n' if char == '\n' else ' ' for char in text[i:end]]
            i = end
    return ''.join(out)


def rust_tests(path, text):
    masked = mask_rust(text)
    stack, closing = [], {}
    brackets, attribute_end = [], {}
    for offset, char in enumerate(masked):
        if char == '[':
            brackets.append(offset)
        elif char == ']' and brackets:
            attribute_end[brackets.pop()] = offset + 1
        if char == '{':
            stack.append(offset)
        elif char == '}' and stack:
            closing[stack.pop()] = offset + 1
    modules = [(match.group(1), match.start(), closing.get(match.end()-1, len(text)))
               for match in re.finditer(r'\bmod\s+(\w+)\s*\{', masked)]
    seen = set()
    for match in re.finditer(r'#\[\s*(?:\w+::)*(?:test|rstest|test_case)\b', masked):
        position = attribute_end.get(match.start()+1)
        if position is None:
            raise ValueError(f'unclosed attribute at {path}:{match.start()}')
        while True:
            gap = re.match(r'\s*', masked[position:]).end()
            position += gap
            if not masked.startswith('#[', position):
                break
            position = attribute_end[position+1]
        fn = re.match(r'(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)\s*\(', masked[position:])
        if not fn:
            continue
        start = position
        if start in seen:
            continue
        seen.add(start)
        opening = masked.find('{', start)
        if opening not in closing:
            raise ValueError(f'unclosed test at {path}:{start}')
        end = closing[opening]
        module = [name for name, left, right in modules if left < start < right]
        yield dict(path=path, name='::'.join([*module, fn.group(1)]), language='rust',
                   start=start, end=end, line=text.count('\n', 0, start)+1,
                   end_line=text.count('\n', 0, end)+1, body=text[start:end])


def inventory():
    files = subprocess.check_output(['git', 'ls-files', '-z'], cwd=ROOT, text=True).split('\0')[:-1]
    rows, sources = [], {}
    js_files = []
    for path in files:
        if path.endswith('.rs'):
            text = (ROOT/path).read_text()
            sources[path] = text
            rows.extend(rust_tests(path, text))
        elif re.search(r'\.(?:[cm]?[jt]sx?)$', path) and (
                re.search(r'\.(?:test|spec)\.', path) or '/__tests__/' in path):
            js_files.append(path)
    parsed = json.loads(subprocess.check_output(
        ['node', str(ROOT/'scripts/test-quality/javascript.cjs')],
        input=json.dumps(js_files), cwd=ROOT, text=True))
    rows.extend(parsed)
    identities = collections.Counter()
    for row in sorted(rows, key=lambda row: (row['path'], row['start'])):
        path = row['path']
        if path not in sources:
            sources[path] = (ROOT/path).read_text()
        identity = path + '::' + row['name']
        identities[identity] += 1
        row['id'] = identity + (f"#{identities[identity]}" if identities[identity] > 1 else '')
        row['body_hash'] = digest(row.pop('body'))
        row['file_hash'] = digest(sources[path])
        row.pop('start')
        row.pop('end')
        # Consumer fixture crates live below tests/ but their src/ unit tests
        # still belong to the unit inventory.
        row['scope'] = ('e2e' if '/e2e/' in path else
                        'unit' if row['language'] == 'rust' and '/src/' in path else
                        'integration' if '/tests/' in path or path.startswith('tests/') else 'unit')
        yield row


def read_ledger(path=LEDGER):
    rows = [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []
    seen = set()
    for row in rows:
        if row['id'] in seen:
            raise ValueError(f"duplicate ledger identity: {row['id']}")
        seen.add(row['id'])
        review = row.get('review')
        if review:
            if review.get('decision') not in {'keep', 'improved', 'finding'}:
                raise ValueError(f"invalid review decision: {row['id']}")
            for key in ('rationale', 'body_hash', 'file_hash'):
                if not isinstance(review.get(key), str) or not review[key].strip():
                    raise ValueError(f"review missing {key}: {row['id']}")
    return rows


def status(row):
    if row.get('retired'):
        return 'retired'
    review = row.get('review')
    if not review:
        return 'pending'
    if review.get('body_hash') != row['body_hash'] or review.get('file_hash') != row['file_hash']:
        return 'stale'
    return review['decision']


def reconcile(current, previous):
    old = {row['id']: row for row in previous}
    result = []
    for row in current:
        prior = old.pop(row['id'], {})
        for key in ('review', 'prior_review', 'resolution'):
            if key in prior:
                row[key] = prior[key]
        result.append(row)
    result.extend(dict(row, retired=True) for row in old.values())
    return sorted(result, key=lambda row: row['id'])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['sync', 'check', 'summary', 'show'])
    parser.add_argument('--path', help='exact source path to display')
    args = parser.parse_args()
    old = read_ledger()
    if args.command in ('sync', 'check'):
        rows = reconcile(inventory(), old)
        content = ''.join(json.dumps(row, separators=(',', ':'), ensure_ascii=False)+'\n' for row in rows)
        if args.command == 'sync':
            LEDGER.write_text(content)
        elif not LEDGER.exists() or LEDGER.read_text() != content:
            raise SystemExit('Ledger differs from current source; run ledger.py sync.')
    else:
        rows = old
    if args.command == 'show':
        if not args.path:
            parser.error('show requires --path')
        lines = (ROOT/args.path).read_text().splitlines()
        for row in sorted(rows, key=lambda row: row['line']):
            if row['path'] == args.path and not row.get('retired'):
                print(f"\n{row['id']} [{status(row)}]")
                print('\n'.join(f'{i+1}: {lines[i]}' for i in range(row['line']-1, min(row['end_line'], len(lines)))))
    else:
        unit = [row for row in rows if row['scope'] == 'unit' and not row.get('retired')]
        print(json.dumps({'active_unit_candidates': len(unit), 'review':dict(collections.Counter(map(status, unit))),
                          'retired':sum(bool(row.get('retired')) for row in rows)}, indent=2))


if __name__ == '__main__':
    main()
